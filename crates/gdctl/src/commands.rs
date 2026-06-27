//! Carrying out the commands.
//!
//! Each function here turns a parsed command into real work against the core
//! library, then prints a short result. Network and disk side effects go through
//! the context, so this layer stays focused on flow and presentation. The offer
//! to install a missing engine and the safe handling of a git reset live here.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use godello_core::{
    BlockReason, DEFAULT_MAIN_BRANCH, EngineRepository, Git, GodotProject, GodotVersion,
    LaunchError, NoProgress, ProjectList, RepoStatus, Settings, SyncState, SystemCommandRunner,
    SystemLauncher, Target, UpdateOutcome, Variant, VersionControl, VersionPattern,
    engine_for_project, find_project_dir, open_editor, open_version, run_project,
};

use crate::cli::{Command, ProjectCommand, SettingsCommand};
use crate::context::Context;
use crate::progress::BarProgress;

/// Print a line to stdout unless the run is silent. Errors still reach stderr
/// through the normal error path, so silent only hides the status output. The
/// message is not even built when silent, so this is cheap to leave in place.
macro_rules! say {
    ($ctx:expr, $($arg:tt)*) => {
        if !$ctx.silent {
            println!($($arg)*);
        }
    };
}

/// Run a parsed command to completion.
pub async fn dispatch(ctx: &mut Context, command: Command) -> Result<()> {
    match command {
        Command::Install { version, variant } => install(ctx, version, variant.selected()).await,
        Command::Remove { version, variant } => remove(ctx, version, variant.selected()),
        Command::List { remote, pre } => {
            if remote {
                list_remote(ctx, pre).await
            } else {
                list_local(ctx)
            }
        }
        Command::Search { text } => search(ctx, &text).await,
        Command::Open {
            version,
            variant,
            detach,
        } => open(ctx, version, variant.selected(), detach.selected()).await,
        Command::Project { command } => project(ctx, command).await,
        Command::Clone { url, dir } => clone(ctx, &url, dir).await,
        Command::Run { no_build, detach } => run_current(ctx, no_build, detach.selected()).await,
        Command::Edit { no_build, detach } => edit_current(ctx, no_build, detach.selected()).await,
        Command::Settings { command } => settings(ctx, command),
    }
}

// Engine commands.

async fn install(ctx: &Context, pattern: VersionPattern, variant: Option<Variant>) -> Result<()> {
    let variant = variant.unwrap_or(ctx.settings.default_variant);
    say!(ctx, "Finding a {variant} build for {pattern}...");
    let release = ctx
        .repository()
        .resolve(pattern, variant, ctx.settings.include_prereleases)
        .await
        .with_context(|| format!("could not find a {variant} release for {pattern}"))?;
    install_version(ctx, variant, release.version).await
}

fn remove(ctx: &Context, pattern: VersionPattern, variant: Option<Variant>) -> Result<()> {
    let variant = variant.unwrap_or(ctx.settings.default_variant);
    let manager = ctx.install_manager();
    let installed = installed_versions(ctx, variant)?;
    let version = pattern
        .best_match(&installed)
        .ok_or_else(|| anyhow!("no installed {variant} engine matches {pattern}"))?;
    manager.remove(variant, version)?;
    say!(ctx, "Removed {} {variant}", version.to_tag());
    Ok(())
}

fn list_local(ctx: &Context) -> Result<()> {
    let mut engines = ctx.install_manager().list_installed()?;
    if engines.is_empty() {
        say!(ctx, "No engines installed.");
        return Ok(());
    }
    engines.sort_by(|a, b| a.variant.cmp(&b.variant).then(a.version.cmp(&b.version)));
    for engine in engines {
        say!(
            ctx,
            "{:14} {}",
            engine.version.to_tag(),
            engine.variant.as_str()
        );
    }
    Ok(())
}

async fn list_remote(ctx: &Context, pre: bool) -> Result<()> {
    let include = pre || ctx.settings.include_prereleases;
    say!(ctx, "Fetching the available versions...");
    let mut releases = ctx.repository().list_releases(include).await?;
    if releases.is_empty() {
        say!(ctx, "No versions available.");
        return Ok(());
    }
    // Newest first reads best for a human scanning the list.
    releases.sort_by(|a, b| b.version.cmp(&a.version));
    for release in releases {
        say!(
            ctx,
            "{:14} ({})",
            release.version.to_tag(),
            variant_list(&release.variants)
        );
    }
    Ok(())
}

async fn search(ctx: &Context, text: &str) -> Result<()> {
    // A search is an explicit ask, so it always looks at prereleases too.
    say!(ctx, "Searching the available versions...");
    let mut releases = ctx.repository().list_releases(true).await?;
    let needle = text.to_ascii_lowercase();
    releases.retain(|release| {
        release
            .version
            .to_tag()
            .to_ascii_lowercase()
            .contains(&needle)
    });
    if releases.is_empty() {
        say!(ctx, "No versions match {text}.");
        return Ok(());
    }
    releases.sort_by(|a, b| b.version.cmp(&a.version));
    for release in releases {
        say!(
            ctx,
            "{:14} ({})",
            release.version.to_tag(),
            variant_list(&release.variants)
        );
    }
    Ok(())
}

async fn open(
    ctx: &Context,
    pattern: VersionPattern,
    variant: Option<Variant>,
    detached: Option<bool>,
) -> Result<()> {
    let variant = variant.unwrap_or(ctx.settings.default_variant);
    let installed = installed_versions(ctx, variant)?;
    let version = match pattern.best_match(&installed) {
        Some(version) => version,
        None => {
            let release = ctx
                .repository()
                .resolve(pattern, variant, ctx.settings.include_prereleases)
                .await
                .with_context(|| format!("could not find a {variant} release for {pattern}"))?;
            let label = format!("{} {variant}", release.version.to_tag());
            if !ctx.confirm(&format!("{label} is not installed. Install it now?"), true) {
                bail!("{label} is not installed");
            }
            install_version(ctx, variant, release.version).await?;
            release.version
        }
    };
    say!(
        ctx,
        "Opening the project manager with {} {variant}...",
        version.to_tag()
    );
    // The project manager has no C# build, so the detached choice applies cleanly.
    let detached = detached.unwrap_or(ctx.settings.launch_detached);
    open_version(
        &ctx.install_manager(),
        version,
        variant,
        detached,
        &SystemLauncher,
    )
    .context("could not open the editor")?;
    Ok(())
}

/// Download and install one resolved version, or note it is already present.
async fn install_version(ctx: &Context, variant: Variant, version: GodotVersion) -> Result<()> {
    let manager = ctx.install_manager();
    if manager.is_installed(variant, version) {
        say!(ctx, "{} {variant} is already installed.", version.to_tag());
        return Ok(());
    }
    let target = Target::current(variant);
    let asset = ctx
        .repository()
        .asset(version, target)
        .await
        .with_context(|| format!("no download for {} {variant}", version.to_tag()))?;
    say!(ctx, "Downloading {} {variant}...", version.to_tag());
    // A silent run shows no bar either. The bar draws to stderr, but silent means
    // quiet, so swap in the sink that reports nothing.
    if ctx.silent {
        manager
            .install(&asset, variant, version, ctx.client(), &NoProgress)
            .await?;
    } else {
        let progress = BarProgress::new(format!("{} {variant}", version.to_tag()));
        manager
            .install(&asset, variant, version, ctx.client(), &progress)
            .await?;
    }
    say!(ctx, "Installed {} {variant}", version.to_tag());
    Ok(())
}

// Project commands.

async fn project(ctx: &mut Context, command: ProjectCommand) -> Result<()> {
    match command {
        ProjectCommand::Add { path } => project_add(ctx, &path),
        ProjectCommand::List => project_list(ctx),
        ProjectCommand::Pin { path, version } => project_pin(ctx, &path, version),
        ProjectCommand::Edit {
            path,
            no_build,
            detach,
        } => {
            let dir = existing_dir(&path)?;
            edit_project(ctx, &dir, no_build, detach.selected()).await
        }
        ProjectCommand::Run {
            path,
            no_build,
            detach,
        } => {
            let dir = existing_dir(&path)?;
            run_project_dir(ctx, &dir, no_build, detach.selected()).await
        }
        ProjectCommand::Remove { path } => project_remove(ctx, &path),
        ProjectCommand::Status { path } => project_status(ctx, &path),
        ProjectCommand::Update { path, reset } => project_update(ctx, &path, reset),
    }
}

fn project_add(ctx: &Context, path: &Path) -> Result<()> {
    let dir = existing_dir(path)?;
    let project = GodotProject::load(&dir)
        .with_context(|| format!("could not read the project in {}", dir.display()))?;
    let file = ctx.paths.projects_file();
    let mut list = ProjectList::load(&file)?;
    let added = list.add(&dir, project.name.clone());
    list.save(&file)?;
    let label = project.name.as_deref().unwrap_or("project");
    if added {
        say!(ctx, "Added {label} at {}", dir.display());
    } else {
        say!(ctx, "Updated {label} at {}", dir.display());
    }
    Ok(())
}

fn project_list(ctx: &Context) -> Result<()> {
    let list = ProjectList::load(&ctx.paths.projects_file())?;
    if list.is_empty() {
        say!(ctx, "No projects added.");
        return Ok(());
    }
    for entry in list.entries() {
        let name = entry.name.as_deref().unwrap_or("(unnamed)");
        say!(ctx, "{name}  {}", entry.path.display());
    }
    Ok(())
}

fn project_pin(ctx: &Context, path: &Path, version: VersionPattern) -> Result<()> {
    let dir = existing_dir(path)?;
    GodotProject::set_pin(&dir, version)
        .with_context(|| format!("could not pin the project in {}", dir.display()))?;
    say!(ctx, "Pinned {} to {version}", dir.display());
    Ok(())
}

fn project_remove(ctx: &Context, path: &Path) -> Result<()> {
    // The folder may be gone, so fall back to the path as given.
    let dir = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file = ctx.paths.projects_file();
    let mut list = ProjectList::load(&file)?;
    if list.remove(&dir) {
        list.save(&file)?;
        say!(ctx, "Forgot {}", dir.display());
    } else {
        say!(ctx, "{} was not in the project list.", dir.display());
    }
    Ok(())
}

fn project_status(ctx: &Context, path: &Path) -> Result<()> {
    let dir = existing_dir(path)?;
    let git = Git::new(SystemCommandRunner);
    if !git.is_repo(&dir) {
        say!(
            ctx,
            "{} is not a version control working copy.",
            dir.display()
        );
        return Ok(());
    }
    say!(ctx, "Checking the working copy against its remote...");
    let status = git
        .status(&dir, true)
        .map_err(|err| anyhow!("could not read the status: {err}"))?;
    print_status(ctx, &status);
    Ok(())
}

fn project_update(ctx: &Context, path: &Path, reset: bool) -> Result<()> {
    let dir = existing_dir(path)?;
    let git = Git::new(SystemCommandRunner);
    if !git.is_repo(&dir) {
        bail!("{} is not a version control working copy", dir.display());
    }
    if reset {
        // The reset flag is the explicit ask. In interactive mode confirm once
        // more since it loses work. In non interactive mode the flag is consent.
        let proceed = ctx.yes
            || ctx.confirm(
                "This discards local changes and local commits. Continue?",
                false,
            );
        if !proceed {
            bail!("reset cancelled");
        }
        say!(ctx, "Resetting to the tracked remote...");
        git.reset_to_remote(&dir)
            .map_err(|err| anyhow!("could not reset: {err}"))?;
        say!(ctx, "Reset to the tracked remote.");
        return Ok(());
    }
    // The main branch to update from. A project can name its own, otherwise the
    // default is used.
    let main_branch = GodotProject::load(&dir)
        .map(|project| project.main_branch().to_string())
        .unwrap_or_else(|_| DEFAULT_MAIN_BRANCH.to_string());

    // When there are local changes, confirm before the update since the merge
    // touches the working copy. The yes flag is consent in non interactive mode.
    let status = git
        .status(&dir, false)
        .map_err(|err| anyhow!("could not read the status: {err}"))?;
    if status.has_local_changes {
        let proceed = ctx.yes
            || ctx.confirm(
                "The working copy has local changes. Merge updates in anyway?",
                false,
            );
        if !proceed {
            bail!("update cancelled");
        }
    }

    say!(ctx, "Updating from {main_branch}...");
    match git
        .update(&dir, &main_branch)
        .map_err(|err| anyhow!("could not update: {err}"))?
    {
        UpdateOutcome::AlreadyUpToDate => say!(ctx, "Already up to date."),
        UpdateOutcome::Advanced => say!(ctx, "Updated to the latest from the remote."),
        UpdateOutcome::Blocked(reason) => say!(ctx, "{}", describe_block(reason)),
    }
    Ok(())
}

// Current folder shortcuts.

async fn edit_current(ctx: &Context, no_build: bool, detached: Option<bool>) -> Result<()> {
    let dir = current_project_dir()?;
    edit_project(ctx, &dir, no_build, detached).await
}

async fn run_current(ctx: &Context, no_build: bool, detached: Option<bool>) -> Result<()> {
    let dir = current_project_dir()?;
    run_project_dir(ctx, &dir, no_build, detached).await
}

async fn edit_project(
    ctx: &Context,
    dir: &Path,
    no_build: bool,
    detached: Option<bool>,
) -> Result<()> {
    let project = GodotProject::load(dir)
        .with_context(|| format!("could not read the project in {}", dir.display()))?;
    ensure_project_engine(ctx, &project).await?;
    let settings = launch_settings(&ctx.settings, no_build, detached);
    announce_build(ctx, &settings, &project);
    let label = project.name.as_deref().unwrap_or("the project");
    // The launch message prints after any build, just before the editor starts.
    open_editor(
        &ctx.install_manager(),
        &settings,
        &project,
        &SystemCommandRunner,
        &SystemLauncher,
        || say!(ctx, "Opening the editor for {label}..."),
    )
    .context("could not open the editor")?;
    Ok(())
}

async fn run_project_dir(
    ctx: &Context,
    dir: &Path,
    no_build: bool,
    detached: Option<bool>,
) -> Result<()> {
    let project = GodotProject::load(dir)
        .with_context(|| format!("could not read the project in {}", dir.display()))?;
    ensure_project_engine(ctx, &project).await?;
    let settings = launch_settings(&ctx.settings, no_build, detached);
    announce_build(ctx, &settings, &project);
    let label = project.name.as_deref().unwrap_or("the project");
    // The launch message prints after any build, just before the project runs.
    run_project(
        &ctx.install_manager(),
        &settings,
        &project,
        &SystemCommandRunner,
        &SystemLauncher,
        || say!(ctx, "Running {label}..."),
    )
    .context("could not run the project")?;
    Ok(())
}

/// The settings to launch with, after applying the per launch overrides. The
/// no_build flag turns the C# build off for this one launch, leaving the saved
/// setting alone, and only ever turns it off. The detached override sets whether
/// the launch is detached when given, otherwise the saved setting stands.
fn launch_settings(base: &Settings, no_build: bool, detached: Option<bool>) -> Settings {
    let mut settings = base.clone();
    if no_build {
        settings.build_csharp_before_launch = false;
    }
    if let Some(detached) = detached {
        settings.launch_detached = detached;
    }
    settings
}

/// Print the build note when a launch will build the C# solution first. The
/// matching launch note is printed afterward by the launch hook, so the two read
/// in the order they happen.
fn announce_build(ctx: &Context, settings: &Settings, project: &GodotProject) {
    if settings.build_csharp_before_launch && project.uses_csharp {
        say!(ctx, "Building the C# solution first...");
    }
}

/// Make sure an engine the project can use is installed. When none matches, offer
/// to install the version the project names. A project that names nothing and has
/// no installed engine cannot be resolved, so it asks the user to pin a version.
async fn ensure_project_engine(ctx: &Context, project: &GodotProject) -> Result<()> {
    match engine_for_project(&ctx.install_manager(), project) {
        Ok(_) => Ok(()),
        Err(LaunchError::NotInstalled { variant, .. }) => {
            let Some((pattern, req_variant)) = project.required_engine() else {
                bail!(
                    "the project names no engine version and no {variant} engine is installed. pin one with gdctl project pin"
                );
            };
            let release = ctx
                .repository()
                .resolve(pattern, req_variant, ctx.settings.include_prereleases)
                .await
                .with_context(|| format!("could not find a {req_variant} release for {pattern}"))?;
            let label = format!("{} {req_variant}", release.version.to_tag());
            if !ctx.confirm(
                &format!("{label} is required but not installed. Install it now?"),
                true,
            ) {
                bail!("the required engine {label} is not installed");
            }
            install_version(ctx, req_variant, release.version).await
        }
        Err(other) => Err(anyhow!("{other}")),
    }
}

// Clone.

async fn clone(ctx: &Context, url: &str, dir: Option<PathBuf>) -> Result<()> {
    let dest = dir.unwrap_or_else(|| PathBuf::from(dir_from_url(url)));
    if dest.exists() && !is_empty_dir(&dest) {
        bail!("{} already exists and is not empty", dest.display());
    }
    let git = Git::new(SystemCommandRunner);
    say!(ctx, "Cloning {url}...");
    git.clone_repo(url, &dest)
        .map_err(|err| anyhow!("could not clone: {err}"))?;
    say!(ctx, "Cloned into {}", dest.display());

    // Add it as a project when it has a project.godot. A repo without one is
    // still cloned, it just is not tracked.
    match GodotProject::load(&dest) {
        Ok(project) => {
            let canonical = std::fs::canonicalize(&dest).unwrap_or_else(|_| dest.clone());
            let file = ctx.paths.projects_file();
            let mut list = ProjectList::load(&file)?;
            list.add(&canonical, project.name.clone());
            list.save(&file)?;
            say!(ctx, "Added it to your project list.");
        }
        Err(_) => {
            say!(
                ctx,
                "Note: no project.godot was found, so it was not added as a project."
            );
        }
    }
    Ok(())
}

// Settings.

fn settings(ctx: &mut Context, command: SettingsCommand) -> Result<()> {
    match command {
        SettingsCommand::List => settings_list(ctx),
        SettingsCommand::Get { key } => settings_get(ctx, &key),
        SettingsCommand::Set { key, value } => settings_set(ctx, &key, &value),
    }
}

fn settings_list(ctx: &Context) -> Result<()> {
    for key in Settings::FIELD_NAMES {
        // Only the engine dir reads back as None, and only while it is unset, so
        // show the default that is in effect instead of a blank.
        let value = ctx.settings.get_field(key).unwrap_or_else(|| {
            format!(
                "(unset, using {})",
                ctx.paths.default_engines_dir().display()
            )
        });
        say!(ctx, "{key:27} {value}");
    }
    Ok(())
}

fn settings_get(ctx: &Context, key: &str) -> Result<()> {
    if let Some(value) = ctx.settings.get_field(key) {
        say!(ctx, "{value}");
        return Ok(());
    }
    // get_field returns None for an unknown key and for an unset engine dir.
    // Tell those two apart so an unset path shows the default in use.
    if key == "engine_install_dir" {
        say!(
            ctx,
            "(unset, using {})",
            ctx.paths.default_engines_dir().display()
        );
        Ok(())
    } else {
        bail!("unknown setting {key}");
    }
}

fn settings_set(ctx: &mut Context, key: &str, value: &str) -> Result<()> {
    ctx.settings.set_field(key, value)?;
    ctx.settings.save(&ctx.paths.settings_file())?;
    say!(ctx, "Set {key} to {value}");
    Ok(())
}

// Shared helpers.

/// The installed versions of one variant.
fn installed_versions(ctx: &Context, variant: Variant) -> Result<Vec<GodotVersion>> {
    Ok(ctx
        .install_manager()
        .list_installed()?
        .into_iter()
        .filter(|engine| engine.variant == variant)
        .map(|engine| engine.version)
        .collect())
}

/// Resolve a path to an existing folder, canonicalized so it matches stored
/// entries. An empty path means the current folder.
fn existing_dir(path: &Path) -> Result<PathBuf> {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    std::fs::canonicalize(path).with_context(|| format!("{} does not exist", path.display()))
}

/// Find the project that contains the current folder.
fn current_project_dir() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("could not read the current folder")?;
    find_project_dir(&cwd)
        .ok_or_else(|| anyhow!("no Godot project found in {} or its parents", cwd.display()))
}

/// A short label listing the variants a release offers.
fn variant_list(variants: &[Variant]) -> String {
    variants
        .iter()
        .map(|variant| variant.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Print a working copy status in a few lines.
fn print_status(ctx: &Context, status: &RepoStatus) {
    match &status.branch {
        Some(branch) => say!(ctx, "Branch: {branch}"),
        None => say!(ctx, "Branch: (detached)"),
    }
    if let Some(remote) = &status.tracked_remote {
        say!(ctx, "Remote: {remote}");
    }
    say!(ctx, "State: {}", describe_sync(&status.sync));
    say!(
        ctx,
        "Local changes: {}",
        if status.has_local_changes {
            "yes"
        } else {
            "no"
        }
    );
}

/// A human phrase for a sync state.
fn describe_sync(sync: &SyncState) -> String {
    match sync {
        SyncState::UpToDate => "up to date with the remote".to_string(),
        SyncState::Behind { commits: Some(n) } => format!("behind the remote by {n}"),
        SyncState::Behind { commits: None } => "behind the remote".to_string(),
        SyncState::Ahead { commits: Some(n) } => format!("ahead of the remote by {n}"),
        SyncState::Ahead { commits: None } => "ahead of the remote".to_string(),
        SyncState::Diverged => "diverged from the remote".to_string(),
        SyncState::NoRemote => "no tracked remote".to_string(),
        SyncState::Unknown => "unknown".to_string(),
    }
}

/// A human explanation for why an update did not happen.
fn describe_block(reason: BlockReason) -> String {
    match reason {
        BlockReason::LocalChanges => {
            "Update blocked. Local changes would be overwritten. Commit or set them aside first."
                .to_string()
        }
        BlockReason::Diverged => {
            "Update blocked. The local and remote histories diverged. Use gdctl project update --reset to match the remote, which loses local commits."
                .to_string()
        }
        BlockReason::NoRemote => {
            "Update blocked. There is no tracked remote to update from.".to_string()
        }
        BlockReason::Conflict => {
            "Update blocked. Merging the updates in would cause conflicts, so nothing changed."
                .to_string()
        }
    }
}

/// True when the folder has no entries, or cannot be read.
fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

/// Derive a folder name from a repository url. Drops a trailing slash and a
/// trailing .git, and takes the last path or host segment.
fn dir_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        "repository".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_from_url_handles_common_forms() {
        assert_eq!(dir_from_url("https://github.com/owner/game.git"), "game");
        assert_eq!(dir_from_url("https://github.com/owner/game"), "game");
        assert_eq!(dir_from_url("https://github.com/owner/game/"), "game");
        assert_eq!(dir_from_url("git@github.com:owner/game.git"), "game");
        assert_eq!(dir_from_url("game"), "game");
    }

    #[test]
    fn dir_from_url_falls_back_when_empty() {
        assert_eq!(dir_from_url("https://example.test/"), "example.test");
        assert_eq!(dir_from_url("/"), "repository");
    }

    #[test]
    fn describe_sync_covers_each_state() {
        assert!(describe_sync(&SyncState::UpToDate).contains("up to date"));
        assert!(describe_sync(&SyncState::Behind { commits: Some(3) }).contains("3"));
        assert!(describe_sync(&SyncState::Behind { commits: None }).contains("behind"));
        assert!(describe_sync(&SyncState::Ahead { commits: Some(2) }).contains("2"));
        assert!(describe_sync(&SyncState::Ahead { commits: None }).contains("ahead"));
        assert!(describe_sync(&SyncState::Diverged).contains("diverged"));
        assert!(describe_sync(&SyncState::NoRemote).contains("no tracked remote"));
        assert!(describe_sync(&SyncState::Unknown).contains("unknown"));
    }

    #[test]
    fn describe_block_mentions_the_remedy() {
        assert!(describe_block(BlockReason::LocalChanges).contains("Local changes"));
        assert!(describe_block(BlockReason::Diverged).contains("--reset"));
        assert!(describe_block(BlockReason::NoRemote).contains("no tracked remote"));
        assert!(describe_block(BlockReason::Conflict).contains("conflicts"));
    }

    #[test]
    fn no_build_only_turns_the_csharp_build_off() {
        let on = Settings {
            build_csharp_before_launch: true,
            ..Settings::default()
        };
        // The flag overrides an enabled build for this launch.
        assert!(!launch_settings(&on, true, None).build_csharp_before_launch);
        // Without the flag the setting is untouched.
        assert!(launch_settings(&on, false, None).build_csharp_before_launch);

        let off = Settings {
            build_csharp_before_launch: false,
            ..Settings::default()
        };
        // It never turns a build on, it only leaves it off.
        assert!(!launch_settings(&off, true, None).build_csharp_before_launch);
        assert!(!launch_settings(&off, false, None).build_csharp_before_launch);
    }

    #[test]
    fn detached_override_sets_either_way_or_keeps_the_default() {
        let attached = Settings {
            launch_detached: false,
            ..Settings::default()
        };
        // The override can force detached on or off for this launch.
        assert!(launch_settings(&attached, false, Some(true)).launch_detached);
        assert!(!launch_settings(&attached, false, Some(false)).launch_detached);
        // None leaves the saved setting in place, in both directions.
        assert!(!launch_settings(&attached, false, None).launch_detached);

        let detached = Settings {
            launch_detached: true,
            ..Settings::default()
        };
        assert!(!launch_settings(&detached, false, Some(false)).launch_detached);
        assert!(launch_settings(&detached, false, None).launch_detached);
    }

    #[test]
    fn variant_list_joins_names() {
        assert_eq!(
            variant_list(&[Variant::Standard, Variant::Mono]),
            "standard, mono"
        );
        assert_eq!(variant_list(&[Variant::Mono]), "mono");
        assert_eq!(variant_list(&[]), "");
    }

    #[test]
    fn is_empty_dir_reports_contents() {
        let dir = std::env::temp_dir()
            .join("godello-cmd-tests")
            .join("empty-check");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(is_empty_dir(&dir));
        std::fs::write(dir.join("a"), b"x").unwrap();
        assert!(!is_empty_dir(&dir));
        // A missing folder reads as empty so a clone into it can proceed.
        assert!(is_empty_dir(&dir.join("missing")));
    }
}
