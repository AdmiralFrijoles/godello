//! The seam between the GUI and the async core calls.
//!
//! Each function here wraps a core call in a task and maps the core error to a
//! string at the boundary, because an iced message must be Clone and the core
//! errors are not. The synchronous core calls (listing and removing installs)
//! are not here. Those run inline in the update step since they are quick local
//! work and need no task.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use godello_core::{
    EngineRepository, Git, GodotProject, GodotVersion, InstallManager, LaunchError, LaunchPhase,
    ProjectEntry, ProjectList, Settings, SystemCommandRunner, SystemLauncher, Target, Variant,
    VersionControl, VersionPattern, clone_destination, find_project_dir_in_tree, open_editor,
    run_project,
};
use iced::Task;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::context::Repository;
use crate::gui::cache;
use crate::gui::message::{LaunchFailure, Message};
use crate::gui::progress::{ChannelProgress, ProgressEvent};
use crate::net::WebClient;

/// Open the native file picker to add a project by its project.godot file. The
/// picker is filtered to the project file so the user points right at it, and the
/// folder that holds it is the project.
pub fn pick_project_file() -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .set_title("Select a project.godot file")
                .add_filter("Godot project", &["godot"])
                .pick_file()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        Message::ProjectFilePicked,
    )
}

/// Open the native folder picker to choose the default project folder.
pub fn pick_project_dir() -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .set_title("Choose the default project folder")
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        Message::ProjectDirPicked,
    )
}

/// Open the native folder picker to choose where engines are installed.
pub fn pick_engine_dir() -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        Message::EngineDirPicked,
    )
}

/// Open the native folder picker to choose where a clone goes. The result fills
/// in the clone dialog rather than starting the clone.
pub fn pick_clone_dir() -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .set_title("Choose a folder to clone into")
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        Message::CloneDirPicked,
    )
}

/// Open the editor or run a project. The build and launch run on a blocking
/// thread so the editor build does not freeze the UI. The launch is detached.
pub fn launch_project(
    manager: InstallManager,
    settings: Settings,
    project: GodotProject,
    run: bool,
) -> Task<Message> {
    let dir = project.dir.clone();

    // The phase hook fires after any C# build, as the launch moves through
    // importing and then starting. We forward each phase as a message so the row
    // can show the current step. The sender drops when the launch finishes, which
    // ends this stream on its own.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LaunchPhase>();
    let phase_dir = dir.clone();
    let phase = Task::run(UnboundedReceiverStream::new(rx), move |phase| match phase {
        LaunchPhase::Importing => Message::LaunchImporting {
            dir: phase_dir.clone(),
        },
        LaunchPhase::Starting => Message::LaunchStarting {
            dir: phase_dir.clone(),
            run,
        },
    });

    let finish_dir = dir;
    let launch = Task::perform(
        async move {
            let joined = tokio::task::spawn_blocking(move || {
                let on_phase = move |phase| {
                    let _ = tx.send(phase);
                };
                let result = if run {
                    run_project(
                        &manager,
                        &settings,
                        &project,
                        &SystemCommandRunner,
                        &SystemLauncher,
                        on_phase,
                    )
                } else {
                    open_editor(
                        &manager,
                        &settings,
                        &project,
                        &SystemCommandRunner,
                        &SystemLauncher,
                        on_phase,
                    )
                };
                // Keep a C# build failure distinct so an edit can offer to open
                // the editor anyway.
                result.map_err(|err| match err {
                    LaunchError::Csharp(err) => LaunchFailure::Compile(err.to_string()),
                    other => LaunchFailure::Other(other.to_string()),
                })
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(join) => Err(LaunchFailure::Other(join.to_string())),
            }
        },
        move |result| Message::LaunchFinished {
            dir: finish_dir.clone(),
            run,
            result,
        },
    );

    Task::batch([phase, launch])
}

/// Resolve the release a project needs so the install offer can name a version.
pub fn resolve_offer(
    repository: Repository,
    pattern: VersionPattern,
    variant: Variant,
    include_pre: bool,
    dir: PathBuf,
    run: bool,
) -> Task<Message> {
    Task::perform(
        async move {
            repository
                .resolve(pattern, variant, include_pre)
                .await
                .map(|release| release.version)
                .map_err(|err| err.to_string())
        },
        move |result| Message::OfferResolved {
            dir,
            run,
            variant,
            result,
        },
    )
}

/// Read a project's version control status without contacting the remote, so the
/// list shows quickly. None means the folder is not a working copy.
pub fn git_status(dir: PathBuf) -> Task<Message> {
    let for_message = dir.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let git = Git::new(SystemCommandRunner);
                if !git.is_repo(&dir) {
                    return None;
                }
                git.status(&dir, false).ok()
            })
            .await
            .ok()
            .flatten()
        },
        move |status| Message::GitStatusLoaded {
            dir: for_message,
            status,
        },
    )
}

/// Bring a project up to date by fetching its main branch and merging it into the
/// working branch. The main branch can be set per project, otherwise the default.
pub fn update_project(dir: PathBuf, main_branch: String) -> Task<Message> {
    let for_message = dir.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                Git::new(SystemCommandRunner)
                    .update(&dir, &main_branch)
                    .map_err(|err| err.to_string())
            })
            .await
            .map_err(|err| err.to_string())?
        },
        move |result| Message::ProjectUpdated {
            dir: for_message,
            result,
        },
    )
}

/// Clone a repository into the chosen folder, then add it as a project when it
/// has a project.godot. Returns the added entry, or None when there was no
/// project. When the chosen folder already holds files the clone goes into a
/// subfolder named after the repository, so it does not mix with what is there.
pub fn clone_repo(url: String, chosen: PathBuf, projects_file: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let dest = clone_destination(&url, &chosen);
                let git = Git::new(SystemCommandRunner);
                git.clone_repo(&url, &dest).map_err(|err| err.to_string())?;
                // The project.godot may sit in a subfolder of the repository, so
                // search the cloned tree for it rather than only the top.
                match find_project_dir_in_tree(&dest)
                    .and_then(|dir| GodotProject::load(&dir).ok().map(|project| (dir, project)))
                {
                    Some((dir, project)) => {
                        let canonical = std::fs::canonicalize(&dir).unwrap_or(dir);
                        let mut list =
                            ProjectList::load(&projects_file).map_err(|err| err.to_string())?;
                        list.add(&canonical, project.name.clone());
                        list.save(&projects_file).map_err(|err| err.to_string())?;
                        Ok(Some(ProjectEntry {
                            path: canonical,
                            name: project.name,
                        }))
                    }
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| err.to_string())?
        },
        Message::Cloned,
    )
}

/// Fetch the list of versions available to install, through the cache.
///
/// When force is false a fresh enough cached list is used and no network call is
/// made. Otherwise, or when the cache is missing or stale, it fetches the list
/// and writes it back to the cache. The command line does not go through here, so
/// it always fetches live.
pub fn load_remote(
    repository: Repository,
    include_pre: bool,
    cache_path: PathBuf,
    force: bool,
) -> Task<Message> {
    Task::perform(
        async move {
            if !force {
                if let Some(releases) = cache::load(&cache_path, cache::TTL_SECS) {
                    return Ok(releases);
                }
            }
            let releases = repository
                .list_releases(include_pre)
                .await
                .map_err(|err| err.to_string())?;
            cache::store(&cache_path, &releases);
            Ok(releases)
        },
        Message::RemoteLoaded,
    )
}

/// Resolve the download for a version and variant, then install it.
///
/// This returns two tasks batched together. One drains progress events from a
/// channel into messages while the download runs. The other performs the install
/// and yields the final result. The progress sink is moved into the install
/// future and dropped when it finishes, which ends the progress stream on its
/// own, so there is no lifecycle to manage by hand.
pub fn install_engine(
    repository: Repository,
    manager: InstallManager,
    downloader: WebClient,
    variant: Variant,
    version: GodotVersion,
    cancel: Arc<AtomicBool>,
) -> Task<Message> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let sink = ChannelProgress::new(tx);

    let progress = Task::run(UnboundedReceiverStream::new(rx), move |event| {
        Message::InstallProgress {
            variant,
            version,
            event,
        }
    });

    let install = Task::perform(
        async move {
            let target = Target::current(variant);
            let asset = repository
                .asset(version, target)
                .await
                .map_err(|err| err.to_string())?;
            let checksum = asset.checksum.clone();
            // Download. This is cancelable by aborting the task.
            let archive = manager
                .fetch(&asset, variant, version, &downloader, &sink)
                .await
                .map_err(|err| err.to_string())?;
            // Verify and extract on a blocking thread so the hashing and unzip do
            // not freeze the executor, which keeps the UI responsive and lets the
            // cancel flag take effect.
            let outcome = tokio::task::spawn_blocking(move || {
                manager.install_archive(&archive, variant, version, checksum.as_ref(), &cancel)
            })
            .await;
            match outcome {
                Ok(Ok(_engine)) => Ok(()),
                Ok(Err(err)) => Err(err.to_string()),
                Err(_join) => Err("the install was interrupted".to_string()),
            }
        },
        move |result| Message::Installed {
            variant,
            version,
            result,
        },
    );

    Task::batch([progress, install])
}
