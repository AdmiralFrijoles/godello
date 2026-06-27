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
    EngineRepository, Git, GodotProject, GodotVersion, InstallManager, ProjectEntry, ProjectList,
    Settings, SystemCommandRunner, SystemLauncher, Target, Variant, VersionControl, VersionPattern,
    open_editor, run_project,
};
use iced::Task;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::context::Repository;
use crate::gui::cache;
use crate::gui::message::Message;
use crate::gui::progress::{ChannelProgress, ProgressEvent};
use crate::net::WebClient;

/// Open the native folder picker to add a project.
pub fn pick_project_folder() -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        Message::ProjectFolderPicked,
    )
}

/// Open the native folder picker to choose where to clone, carrying the url.
pub fn pick_clone_destination(url: String) -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .pick_folder()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
        move |dest| Message::CloneDestinationPicked { url, dest },
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
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let result = if run {
                    run_project(
                        &manager,
                        &settings,
                        &project,
                        &SystemCommandRunner,
                        &SystemLauncher,
                        || {},
                    )
                } else {
                    open_editor(
                        &manager,
                        &settings,
                        &project,
                        &SystemCommandRunner,
                        &SystemLauncher,
                        || {},
                    )
                };
                result.map_err(|err| err.to_string())
            })
            .await
            .map_err(|err| err.to_string())?
        },
        Message::LaunchFinished,
    )
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

/// Bring a project up to date with its remote, contacting it.
pub fn update_project(dir: PathBuf) -> Task<Message> {
    let for_message = dir.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                Git::new(SystemCommandRunner)
                    .update(&dir)
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

/// Clone a repository into a destination, then add it as a project when it has a
/// project.godot. Returns the added entry, or None when there was no project.
pub fn clone_repo(url: String, dest: PathBuf, projects_file: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let git = Git::new(SystemCommandRunner);
                git.clone_repo(&url, &dest).map_err(|err| err.to_string())?;
                match GodotProject::load(&dest) {
                    Ok(project) => {
                        let canonical = std::fs::canonicalize(&dest).unwrap_or(dest);
                        let mut list =
                            ProjectList::load(&projects_file).map_err(|err| err.to_string())?;
                        list.add(&canonical, project.name.clone());
                        list.save(&projects_file).map_err(|err| err.to_string())?;
                        Ok(Some(ProjectEntry {
                            path: canonical,
                            name: project.name,
                        }))
                    }
                    Err(_) => Ok(None),
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
