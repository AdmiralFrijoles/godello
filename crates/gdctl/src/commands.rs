//! Carrying out the commands.
//!
//! Each function here turns a parsed command into real work against the core
//! library, then prints a short result. Network and disk side effects go through
//! the context, so this layer stays focused on flow and presentation. The offer
//! to install a missing engine and the safe handling of a git reset live here.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use godello_core::{
    BlockReason, EngineRepository, Git, GodotProject, GodotVersion, LaunchError, ProjectList,
    RepoStatus, SyncState, SystemCommandRunner, SystemLauncher, Target, UpdateOutcome, Variant,
    VersionControl, VersionPattern, engine_for_project, find_project_dir, open_editor,
    open_version, run_project,
};

use crate::cli::{Command, ProjectCommand, SettingsCommand};
use crate::context::Context;
use crate::progress::BarProgress;

/// Run a parsed command to completion.
pub async fn dispatch(ctx: &mut Context, command: Command) -> Result<()> {
    match command {
        Command::Install { version, variant } => install(ctx, version, variant).await,
        Command::Remove { version, variant } => remove(ctx, version, variant),
        Command::List { remote, pre } => {
            if remote {
                list_remote(ctx, pre).await
            } else {
                list_local(ctx)
            }
        }
        Command::Search { text } => search(ctx, &text).await,
        Command::Open { version, variant } => open(ctx, version, variant).await,
        Command::Project { command } => project(ctx, command).await,
        Command::Clone { url, dir } => clone(ctx, &url, dir).await,
        Command::Run => run_current(ctx).await,
        Command::Edit => edit_current(ctx).await,
        Command::Settings { command } => settings(ctx, command),
    }
}

// Engine commands.

async fn install(ctx: &Context, pattern: VersionPattern, variant: Option<Variant>) -> Result<()> {
    let variant = variant.unwrap_or(ctx.settings.default_variant);
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
    println!("Removed {variant} {}", version.to_tag());
    Ok(())
}

fn list_local(ctx: &Context) -> Result<()> {
    let mut engines = ctx.install_manager().list_installed()?;
    if engines.is_empty() {
        println!("No engines installed.");
        return Ok(());
    }
    engines.sort_by(|a, b| a.variant.cmp(&b.variant).then(a.version.cmp(&b.version)));
    for engine in engines {
        println!("{:9} {}", engine.variant.as_str(), engine.version.to_tag());
    }
    Ok(())
}

async fn list_remote(ctx: &Context, pre: bool) -> Result<()> {
    let include = pre || ctx.settings.include_prereleases;
    let mut releases = ctx.repository().list_releases(include).await?;
    if releases.is_empty() {
        println!("No versions available.");
        return Ok(());
    }
    // Newest first reads best for a human scanning the list.
    releases.sort_by(|a, b| b.version.cmp(&a.version));
    for release in releases {
        println!(
            "{:14} ({})",
            release.version.to_tag(),
            variant_list(&release.variants)
        );
    }
    Ok(())
}

async fn search(ctx: &Context, text: &str) -> Result<()> {
    // A search is an explicit ask, so it always looks at prereleases too.
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
        println!("No versions match {text}.");
        return Ok(());
    }
    releases.sort_by(|a, b| b.version.cmp(&a.version));
    for release in releases {
        println!(
            "{:14} ({})",
            release.version.to_tag(),
            variant_list(&release.variants)
        );
    }
    Ok(())
}

async fn open(ctx: &Context, pattern: VersionPattern, variant: Option<Variant>) -> Result<()> {
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
            let label = format!("{variant} {}", release.version.to_tag());
            if !ctx.confirm(&format!("{label} is not installed. Install it now?"), true) {
                bail!("{label} is not installed");
            }
            install_version(ctx, variant, release.version).await?;
            release.version
        }
    };
    open_version(
        &ctx.install_manager(),
        version,
        variant,
        ctx.settings.launch_detached,
        &SystemLauncher,
    )
    .context("could not open the editor")?;
    Ok(())
}

/// Download and install one resolved version, or note it is already present.
async fn install_version(ctx: &Context, variant: Variant, version: GodotVersion) -> Result<()> {
    let manager = ctx.install_manager();
    if manager.is_installed(variant, version) {
        println!("{variant} {} is already installed.", version.to_tag());
        return Ok(());
    }
    let target = Target::current(variant);
    let asset = ctx
        .repository()
        .asset(version, target)
        .await
        .with_context(|| format!("no download for {variant} {}", version.to_tag()))?;
    println!("Downloading {variant} {}...", version.to_tag());
    let progress = BarProgress::new(format!("{variant} {}", version.to_tag()));
    manager
        .install(&asset, variant, version, ctx.client(), &progress)
        .await?;
    println!("Installed {variant} {}", version.to_tag());
    Ok(())
}

// Project commands.

async fn project(ctx: &mut Context, command: ProjectCommand) -> Result<()> {
    match command {
        ProjectCommand::Add { path } => project_add(ctx, &path),
        ProjectCommand::List => project_list(ctx),
        ProjectCommand::Pin { path, version } => project_pin(&path, version),
        ProjectCommand::Edit { path } => {
            let dir = existing_dir(&path)?;
            edit_project(ctx, &dir).await
        }
        ProjectCommand::Run { path } => {
            let dir = existing_dir(&path)?;
            run_project_dir(ctx, &dir).await
        }
        ProjectCommand::Remove { path } => project_remove(ctx, &path),
        ProjectCommand::Status { path } => project_status(&path),
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
        println!("Added {label} at {}", dir.display());
    } else {
        println!("Updated {label} at {}", dir.display());
    }
    Ok(())
}

fn project_list(ctx: &Context) -> Result<()> {
    let list = ProjectList::load(&ctx.paths.projects_file())?;
    if list.is_empty() {
        println!("No projects added.");
        return Ok(());
    }
    for entry in list.entries() {
        let name = entry.name.as_deref().unwrap_or("(unnamed)");
        println!("{name}  {}", entry.path.display());
    }
    Ok(())
}

fn project_pin(path: &Path, version: VersionPattern) -> Result<()> {
    let dir = existing_dir(path)?;
    GodotProject::set_pin(&dir, version)
        .with_context(|| format!("could not pin the project in {}", dir.display()))?;
    println!("Pinned {} to {version}", dir.display());
    Ok(())
}

fn project_remove(ctx: &Context, path: &Path) -> Result<()> {
    // The folder may be gone, so fall back to the path as given.
    let dir = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file = ctx.paths.projects_file();
    let mut list = ProjectList::load(&file)?;
    if list.remove(&dir) {
        list.save(&file)?;
        println!("Forgot {}", dir.display());
    } else {
        println!("{} was not in the project list.", dir.display());
    }
    Ok(())
}

fn project_status(path: &Path) -> Result<()> {
    let dir = existing_dir(path)?;
    let git = Git::new(SystemCommandRunner);
    if !git.is_repo(&dir) {
        println!("{} is not a version control working copy.", dir.display());
        return Ok(());
    }
    let status = git
        .status(&dir, true)
        .map_err(|err| anyhow!("could not read the status: {err}"))?;
    print_status(&status);
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
        git.reset_to_remote(&dir)
            .map_err(|err| anyhow!("could not reset: {err}"))?;
        println!("Reset to the tracked remote.");
        return Ok(());
    }
    match git
        .update(&dir)
        .map_err(|err| anyhow!("could not update: {err}"))?
    {
        UpdateOutcome::AlreadyUpToDate => println!("Already up to date."),
        UpdateOutcome::Advanced => println!("Updated to the latest from the remote."),
        UpdateOutcome::Blocked(reason) => println!("{}", describe_block(reason)),
    }
    Ok(())
}

// Current folder shortcuts.

async fn edit_current(ctx: &Context) -> Result<()> {
    let dir = current_project_dir()?;
    edit_project(ctx, &dir).await
}

async fn run_current(ctx: &Context) -> Result<()> {
    let dir = current_project_dir()?;
    run_project_dir(ctx, &dir).await
}

async fn edit_project(ctx: &Context, dir: &Path) -> Result<()> {
    let project = GodotProject::load(dir)
        .with_context(|| format!("could not read the project in {}", dir.display()))?;
    ensure_project_engine(ctx, &project).await?;
    open_editor(
        &ctx.install_manager(),
        &ctx.settings,
        &project,
        &SystemCommandRunner,
        &SystemLauncher,
    )
    .context("could not open the editor")?;
    Ok(())
}

async fn run_project_dir(ctx: &Context, dir: &Path) -> Result<()> {
    let project = GodotProject::load(dir)
        .with_context(|| format!("could not read the project in {}", dir.display()))?;
    ensure_project_engine(ctx, &project).await?;
    run_project(
        &ctx.install_manager(),
        &ctx.settings,
        &project,
        &SystemCommandRunner,
        &SystemLauncher,
    )
    .context("could not run the project")?;
    Ok(())
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
            let label = format!("{req_variant} {}", release.version.to_tag());
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
    git.clone_repo(url, &dest)
        .map_err(|err| anyhow!("could not clone: {err}"))?;
    println!("Cloned into {}", dest.display());

    // Add it as a project when it has a project.godot. A repo without one is
    // still cloned, it just is not tracked.
    match GodotProject::load(&dest) {
        Ok(project) => {
            let canonical = std::fs::canonicalize(&dest).unwrap_or_else(|_| dest.clone());
            let file = ctx.paths.projects_file();
            let mut list = ProjectList::load(&file)?;
            list.add(&canonical, project.name.clone());
            list.save(&file)?;
            println!("Added it to your project list.");
        }
        Err(_) => {
            println!("Note: no project.godot was found, so it was not added as a project.");
        }
    }
    Ok(())
}

// Settings.

fn settings(ctx: &mut Context, command: SettingsCommand) -> Result<()> {
    match command {
        SettingsCommand::Get { key } => settings_get(ctx, &key),
        SettingsCommand::Set { key, value } => settings_set(ctx, &key, &value),
    }
}

fn settings_get(ctx: &Context, key: &str) -> Result<()> {
    if let Some(value) = ctx.settings.get_field(key) {
        println!("{value}");
        return Ok(());
    }
    // get_field returns None for an unknown key and for an unset engine dir.
    // Tell those two apart so an unset path shows the default in use.
    if key == "engine_install_dir" {
        println!(
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
    println!("Set {key} to {value}");
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
fn print_status(status: &RepoStatus) {
    match &status.branch {
        Some(branch) => println!("Branch: {branch}"),
        None => println!("Branch: (detached)"),
    }
    if let Some(remote) = &status.tracked_remote {
        println!("Remote: {remote}");
    }
    println!("State: {}", describe_sync(&status.sync));
    println!(
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
            "Update blocked. The working copy has local changes. Commit or set them aside first."
                .to_string()
        }
        BlockReason::Diverged => {
            "Update blocked. The local and remote histories diverged. Use gdctl project update --reset to match the remote, which loses local commits."
                .to_string()
        }
        BlockReason::NoRemote => {
            "Update blocked. There is no tracked remote to update from.".to_string()
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
        assert!(describe_block(BlockReason::LocalChanges).contains("local changes"));
        assert!(describe_block(BlockReason::Diverged).contains("--reset"));
        assert!(describe_block(BlockReason::NoRemote).contains("no tracked remote"));
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
