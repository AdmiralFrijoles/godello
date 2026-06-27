//! The Godello desktop app.
//!
//! This is the second front end over the same core the CLI uses. It keeps no
//! logic of its own beyond presentation. Every real action calls into the core
//! through the shared context.
//!
//! The window is a left navigation sidebar and a content area. There is an
//! engines screen, a projects screen, and a settings screen with a theme picker
//! and a cache control. A full settings form comes later.

mod cache;
mod icons;
mod message;
mod progress;
mod screens;
mod state;
mod style;
mod tasks;
mod theme;
mod widgets;

use std::path::Path;
use std::time::Duration;

use iced::widget::{
    button, center, column, container, mouse_area, opaque, pick_list, progress_bar, row, space,
    stack, text, text_input,
};
use iced::{Alignment, Color, Element, Length, Size, Subscription, Task, Theme};

use godello_core::{
    BlockReason, GodotProject, GodotVersion, LaunchError, ProjectList, SystemLauncher,
    UpdateOutcome, Variant, VersionPattern, engine_for_project, open_path, open_version,
};

pub use message::Message;
use state::{
    App, CloneDialog, EnginesTab, InstallOffer, Load, PendingLaunch, PinChoice, PinEditor, Screen,
    Toast, ToastKind,
};

/// How often the toast timer ticks. About sixty times a second, so the countdown
/// bar moves smoothly rather than in visible steps.
const TOAST_TICK: Duration = Duration::from_millis(16);
/// How often the projects screen rechecks version control status, so local
/// changes show without the user asking.
const VCS_REFRESH: Duration = Duration::from_secs(10);

/// Open the desktop app and run until the window closes.
///
/// iced owns the event loop and its own runtime, so this blocks. The boot hook
/// clones the shared context into the starting state and kicks off the first
/// load of installed engines.
pub fn launch(ctx: crate::context::Context) -> iced::Result {
    iced::application(
        move || {
            (
                App::new(ctx.clone()),
                // Load the installed engines, and the projects for the landing
                // screen. Navigate to Projects triggers its first load.
                Task::batch([
                    Task::done(Message::RefreshInstalled),
                    Task::done(Message::Navigate(Screen::Projects)),
                ]),
            )
        },
        update,
        view,
    )
    .title("Godello")
    .theme(|state: &App| state.theme.clone())
    .subscription(subscription)
    .window(iced::window::Settings {
        size: Size::new(1024.0, 620.0),
        min_size: Some(Size::new(1024.0, 620.0)),
        ..iced::window::Settings::default()
    })
    .run()
}

/// The timers the app needs: the toast countdown while toasts are showing, and a
/// periodic version control recheck while the projects screen is open.
fn subscription(state: &App) -> Subscription<Message> {
    let mut subs = Vec::new();
    if !state.toasts.is_empty() {
        subs.push(iced::time::every(TOAST_TICK).map(|_| Message::ToastTick));
    }
    if state.screen == Screen::Projects && matches!(state.projects, Load::Loaded(_)) {
        subs.push(iced::time::every(VCS_REFRESH).map(|_| Message::RefreshGitStatuses));
    }
    Subscription::batch(subs)
}

/// Apply a message to the state and return any follow up work.
fn update(state: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(screen) => {
            state.screen = screen;
            state.menu_open = None;
            state.project_menu_open = None;
            // Load each list the first time its screen is shown.
            if screen == Screen::Engines
                && state.engines_tab == EnginesTab::Available
                && matches!(state.remote, Load::Idle)
            {
                return load_remote(state, false);
            }
            if screen == Screen::Projects && matches!(state.projects, Load::Idle) {
                return reload_projects(state);
            }
            Task::none()
        }
        Message::DismissToast(id) => {
            state.dismiss_toast(id);
            Task::none()
        }
        Message::ToastTick => {
            state.tick_toasts(TOAST_TICK.as_secs_f32());
            Task::none()
        }
        Message::HoverToasts(over) => {
            state.toast_paused = over;
            Task::none()
        }
        Message::SetTheme(theme) => {
            state.theme = theme;
            Task::none()
        }
        Message::CopyPath(path) => {
            // No banner. The copy button shows its own color feedback while
            // pressed, which is enough to confirm the action.
            iced::clipboard::write(path)
        }

        Message::RefreshInstalled => {
            reload_installed(state);
            Task::none()
        }

        Message::LoadRemote { force } => load_remote(state, force),
        Message::ClearCache => {
            cache::clear(&state.ctx.paths.manifest_cache());
            // Drop the loaded list so the next visit fetches a fresh one.
            state.remote = Load::Idle;
            state.toast(ToastKind::Info, "Cleared the cached version list.");
            Task::none()
        }
        Message::RemoteLoaded(Ok(mut releases)) => {
            // Newest first, matching the CLI listing.
            releases.sort_by(|a, b| b.version.cmp(&a.version));
            state.remote = Load::Loaded(releases);
            Task::none()
        }
        Message::RemoteLoaded(Err(err)) => {
            state.remote = Load::Failed(err.clone());
            state.toast(
                ToastKind::Error,
                format!("Could not load available versions: {err}"),
            );
            Task::none()
        }
        Message::SetEnginesTab(tab) => {
            state.engines_tab = tab;
            if tab == EnginesTab::Available && matches!(state.remote, Load::Idle) {
                return load_remote(state, false);
            }
            Task::none()
        }
        Message::SetChannel(channel) => {
            state.channel = channel;
            Task::none()
        }
        Message::FilterChanged(text) => {
            state.filter = text;
            Task::none()
        }

        Message::ToggleEngineMenu { variant, version } => {
            let key = Some((variant, version));
            state.menu_open = if state.menu_open == key { None } else { key };
            Task::none()
        }
        Message::CloseEngineMenu => {
            state.menu_open = None;
            Task::none()
        }
        Message::OpenFolder { variant, version } => {
            state.menu_open = None;
            let dir = state.ctx.install_manager().install_dir(variant, version);
            if let Err(err) = open_path(&dir, &SystemLauncher) {
                state.toast(
                    ToastKind::Error,
                    format!("Could not open the folder: {err}"),
                );
            }
            Task::none()
        }
        Message::OpenProjectManager { variant, version } => {
            state.menu_open = None;
            let manager = state.ctx.install_manager();
            if let Err(err) = open_version(&manager, version, variant, true, &SystemLauncher) {
                state.toast(
                    ToastKind::Error,
                    format!("Could not open the project manager: {err}"),
                );
            }
            Task::none()
        }
        Message::RequestRemove { variant, version } => {
            state.menu_open = None;
            state.confirm_remove = Some((variant, version));
            Task::none()
        }
        Message::CancelRemove => {
            state.confirm_remove = None;
            Task::none()
        }
        Message::Remove { variant, version } => {
            state.confirm_remove = None;
            match state.ctx.install_manager().remove(variant, version) {
                Ok(()) => {
                    state.toast(
                        ToastKind::Info,
                        format!("Removed {} {}", version.to_tag(), variant.as_str()),
                    );
                    reload_installed(state);
                }
                Err(err) => {
                    state.toast(ToastKind::Error, format!("Could not remove: {err}"));
                }
            }
            Task::none()
        }

        Message::Install { variant, version } => start_install(state, variant, version),
        Message::CancelInstall { variant, version } => {
            if let Some(job) = state.job(variant, version) {
                // Stop the download, and ask the extract to stop if it has begun.
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                job.abort.abort();
            }
            state
                .jobs
                .retain(|job| !(job.variant == variant && job.version == version));
            state.toast(
                ToastKind::Info,
                format!(
                    "Cancelled installing {} {}",
                    version.to_tag(),
                    variant.as_str()
                ),
            );
            Task::none()
        }
        Message::InstallProgress {
            variant,
            version,
            event,
        } => {
            if let Some(job) = state.job_mut(variant, version) {
                match event {
                    progress::ProgressEvent::Started { total } => {
                        job.total = total;
                        job.downloaded = 0;
                    }
                    progress::ProgressEvent::Advanced { downloaded } => {
                        job.downloaded = downloaded;
                    }
                    progress::ProgressEvent::Finished => {
                        // The download is done. The verify and extract that follow
                        // have no progress, so show the row as busy until the
                        // Installed message, which is the real end.
                        job.installing = true;
                    }
                }
            }
            Task::none()
        }
        Message::Installed {
            variant,
            version,
            result,
        } => {
            // If the job is gone, it was cancelled, so ignore the late result.
            if state.job(variant, version).is_none() {
                return Task::none();
            }
            state
                .jobs
                .retain(|job| !(job.variant == variant && job.version == version));
            match result {
                Ok(()) => {
                    state.toast(
                        ToastKind::Info,
                        format!("Installed {} {}", version.to_tag(), variant.as_str()),
                    );
                    reload_installed(state);
                    // Resume a launch that was waiting on this install.
                    if let Some(pending) = state.pending_launch.take() {
                        return Task::done(Message::LaunchProject {
                            dir: pending.dir,
                            run: pending.run,
                        });
                    }
                }
                Err(err) => {
                    state.pending_launch = None;
                    state.toast(
                        ToastKind::Error,
                        format!(
                            "Could not install {} {}: {err}",
                            version.to_tag(),
                            variant.as_str()
                        ),
                    );
                }
            }
            Task::none()
        }

        // Projects.
        Message::AddProject => tasks::pick_project_folder(),
        Message::ProjectFolderPicked(None) => Task::none(),
        Message::ProjectFolderPicked(Some(dir)) => {
            add_project(state, &dir);
            reload_projects(state)
        }
        Message::RemoveProject(dir) => {
            state.project_menu_open = None;
            let file = state.ctx.paths.projects_file();
            match ProjectList::load(&file) {
                Ok(mut list) => {
                    if list.remove(&dir) {
                        let _ = list.save(&file);
                        state.toast(ToastKind::Info, "Forgot the project.");
                    }
                }
                Err(err) => {
                    state.toast(
                        ToastKind::Error,
                        format!("Could not update the project list: {err}"),
                    );
                }
            }
            state.git_status.remove(&dir);
            reload_projects(state)
        }
        Message::OpenProjectFolder(dir) => {
            state.project_menu_open = None;
            if let Err(err) = open_path(&dir, &SystemLauncher) {
                state.toast(
                    ToastKind::Error,
                    format!("Could not open the folder: {err}"),
                );
            }
            Task::none()
        }
        Message::ToggleProjectMenu(dir) => {
            state.project_menu_open = if state.project_menu_open.as_deref() == Some(dir.as_path()) {
                None
            } else {
                Some(dir)
            };
            Task::none()
        }
        Message::CloseProjectMenu => {
            state.project_menu_open = None;
            Task::none()
        }
        Message::LaunchProject { dir, run } => {
            state.project_menu_open = None;
            prepare_launch(state, dir, run)
        }
        Message::LaunchFinished(Ok(())) => Task::none(),
        Message::LaunchFinished(Err(err)) => {
            state.toast(ToastKind::Error, format!("Could not launch: {err}"));
            Task::none()
        }
        Message::OfferResolved {
            dir,
            run,
            variant,
            result,
        } => {
            match result {
                Ok(version) => {
                    let label = format!("{} {}", version.to_tag(), variant.as_str());
                    state.install_offer = Some(InstallOffer {
                        variant,
                        version,
                        label,
                        dir,
                        run,
                    });
                }
                Err(err) => {
                    state.toast(
                        ToastKind::Error,
                        format!("Could not find the engine to install: {err}"),
                    );
                }
            }
            Task::none()
        }
        Message::AcceptOffer => match state.install_offer.take() {
            Some(offer) => {
                state.pending_launch = Some(PendingLaunch {
                    dir: offer.dir,
                    run: offer.run,
                });
                start_install(state, offer.variant, offer.version)
            }
            None => Task::none(),
        },
        Message::DismissOffer => {
            state.install_offer = None;
            Task::none()
        }

        // Pinning.
        Message::OpenPinEditor(dir) => {
            state.project_menu_open = None;
            let (choices, selected) = pin_choices(state, &dir);
            state.pin_editor = Some(PinEditor {
                dir,
                choices,
                selected,
            });
            Task::none()
        }
        Message::PinSelected(choice) => {
            if let Some(editor) = &mut state.pin_editor {
                editor.selected = Some(choice);
            }
            Task::none()
        }
        Message::CancelPin => {
            state.pin_editor = None;
            Task::none()
        }
        Message::SavePin => {
            let Some(editor) = state.pin_editor.take() else {
                return Task::none();
            };
            let Some(choice) = editor.selected else {
                return Task::none();
            };
            match GodotProject::set_pin(&editor.dir, choice.pattern) {
                Ok(()) => {
                    state.toast(ToastKind::Info, format!("Pinned to {}.", choice.pattern));
                    return reload_projects(state);
                }
                Err(err) => {
                    state.toast(ToastKind::Error, format!("Could not pin: {err}"));
                }
            }
            Task::none()
        }

        // Version control.
        Message::RefreshGitStatuses => match &state.projects {
            Load::Loaded(entries) => {
                let checks: Vec<Task<Message>> = entries
                    .iter()
                    .map(|entry| tasks::git_status(entry.path.clone()))
                    .collect();
                Task::batch(checks)
            }
            _ => Task::none(),
        },
        Message::GitStatusLoaded { dir, status } => {
            match status {
                Some(status) => {
                    state.git_status.insert(dir, status);
                }
                None => {
                    state.git_status.remove(&dir);
                }
            }
            Task::none()
        }
        Message::UpdateProject(dir) => {
            state.project_menu_open = None;
            state.toast(ToastKind::Info, "Checking the remote...");
            tasks::update_project(dir)
        }
        Message::ProjectUpdated { dir, result } => {
            match result {
                Ok(outcome) => state.toast(ToastKind::Info, describe_update(outcome)),
                Err(err) => state.toast(ToastKind::Error, format!("Could not update: {err}")),
            }
            tasks::git_status(dir)
        }

        // Cloning.
        Message::OpenCloneDialog => {
            state.clone_dialog = Some(CloneDialog::default());
            Task::none()
        }
        Message::CloneUrlChanged(url) => {
            if let Some(dialog) = &mut state.clone_dialog {
                dialog.url = url;
            }
            Task::none()
        }
        Message::CancelClone => {
            state.clone_dialog = None;
            Task::none()
        }
        Message::StartClone => {
            let Some(dialog) = state.clone_dialog.take() else {
                return Task::none();
            };
            let url = dialog.url.trim().to_string();
            if url.is_empty() {
                state.toast(ToastKind::Error, "Enter a repository url.");
                return Task::none();
            }
            tasks::pick_clone_destination(url)
        }
        Message::CloneDestinationPicked { url, dest } => match dest {
            Some(dest) => {
                state.toast(ToastKind::Info, "Cloning...");
                tasks::clone_repo(url, dest, state.ctx.paths.projects_file())
            }
            None => Task::none(),
        },
        Message::Cloned(Ok(entry)) => {
            match entry {
                Some(_) => state.toast(ToastKind::Info, "Cloned and added the project."),
                None => state.toast(
                    ToastKind::Info,
                    "Cloned, but there was no project.godot so it was not added.",
                ),
            }
            reload_projects(state)
        }
        Message::Cloned(Err(err)) => {
            state.toast(ToastKind::Error, format!("Could not clone: {err}"));
            Task::none()
        }
    }
}

/// Start a cancelable install and track it as a job.
fn start_install(state: &mut App, variant: Variant, version: GodotVersion) -> Task<Message> {
    if state.is_installing(variant, version) {
        return Task::none();
    }
    // The abort handle stops the download, the cancel flag stops the extract that
    // runs off the executor.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (task, abort) = tasks::install_engine(
        state.ctx.repository(),
        state.ctx.install_manager(),
        state.ctx.client().clone(),
        variant,
        version,
        cancel.clone(),
    )
    .abortable();
    state.jobs.push(state::InstallJob {
        variant,
        version,
        total: None,
        downloaded: 0,
        installing: false,
        abort,
        cancel,
    });
    task
}

/// Load a project from a folder and add it to the saved list.
fn add_project(state: &mut App, dir: &Path) {
    match GodotProject::load(dir) {
        Ok(project) => {
            let file = state.ctx.paths.projects_file();
            match ProjectList::load(&file) {
                Ok(mut list) => {
                    let added = list.add(dir, project.name.clone());
                    if let Err(err) = list.save(&file) {
                        state.toast(ToastKind::Error, format!("Could not save: {err}"));
                        return;
                    }
                    state.toast(
                        ToastKind::Info,
                        if added {
                            "Added the project."
                        } else {
                            "Updated the project."
                        },
                    );
                }
                Err(err) => {
                    state.toast(
                        ToastKind::Error,
                        format!("Could not read the project list: {err}"),
                    );
                }
            }
        }
        Err(err) => state.toast(ToastKind::Error, format!("Not a Godot project: {err}")),
    }
}

/// Reload the project list and the parsed info for each, and start a git status
/// check for each project.
fn reload_projects(state: &mut App) -> Task<Message> {
    let file = state.ctx.paths.projects_file();
    match ProjectList::load(&file) {
        Ok(list) => {
            let entries = list.entries().to_vec();
            state.project_info.clear();
            let mut checks = Vec::new();
            for entry in &entries {
                if let Ok(project) = GodotProject::load(&entry.path) {
                    state.project_info.insert(entry.path.clone(), project);
                }
                checks.push(tasks::git_status(entry.path.clone()));
            }
            state.projects = Load::Loaded(entries);
            Task::batch(checks)
        }
        Err(err) => {
            state.projects = Load::Failed(err.to_string());
            Task::none()
        }
    }
}

/// Begin an edit or run. Launches when an engine is ready, otherwise resolves the
/// engine the project needs and raises the install offer.
fn prepare_launch(state: &mut App, dir: std::path::PathBuf, run: bool) -> Task<Message> {
    let project = match GodotProject::load(&dir) {
        Ok(project) => project,
        Err(err) => {
            state.toast(
                ToastKind::Error,
                format!("Could not read the project: {err}"),
            );
            return Task::none();
        }
    };
    match engine_for_project(&state.ctx.install_manager(), &project) {
        Ok(_) => {
            let mut settings = state.ctx.settings.clone();
            settings.launch_detached = true;
            tasks::launch_project(state.ctx.install_manager(), settings, project, run)
        }
        Err(LaunchError::NotInstalled { .. }) => match project.required_engine() {
            Some((pattern, variant)) => tasks::resolve_offer(
                state.ctx.repository(),
                pattern,
                variant,
                state.ctx.settings.include_prereleases,
                dir,
                run,
            ),
            None => {
                state.toast(
                    ToastKind::Error,
                    "This project names no engine version. Pin one first.",
                );
                Task::none()
            }
        },
        Err(other) => {
            state.toast(ToastKind::Error, format!("Could not launch: {other}"));
            Task::none()
        }
    }
}

/// Build the pin dropdown choices for a project, with its detected version
/// suggested at the top, then the installed versions of its variant. The
/// selected choice starts on the suggestion.
fn pin_choices(state: &App, dir: &Path) -> (Vec<PinChoice>, Option<PinChoice>) {
    let project = state.project_info.get(dir);
    let variant = project.map(|project| {
        if project.uses_csharp {
            Variant::Mono
        } else {
            Variant::Standard
        }
    });
    let detected = project.and_then(|project| project.pinned_version.or(project.feature_version));

    let mut choices: Vec<PinChoice> = Vec::new();
    if let Some(pattern) = detected {
        choices.push(PinChoice {
            pattern,
            label: format!("{pattern}  (suggested)"),
        });
    }

    if let Load::Loaded(engines) = &state.installed {
        let mut versions: Vec<GodotVersion> = engines
            .iter()
            .filter(|engine| variant.is_none_or(|variant| engine.variant == variant))
            .map(|engine| engine.version)
            .collect();
        versions.sort_by(|a, b| b.cmp(a));
        versions.dedup();
        for version in versions {
            if let Ok(pattern) = version.to_tag().parse::<VersionPattern>() {
                // Skip the one already shown as the suggestion.
                if detected == Some(pattern) {
                    continue;
                }
                choices.push(PinChoice {
                    pattern,
                    label: version.to_tag(),
                });
            }
        }
    }

    let selected = choices.first().cloned();
    (choices, selected)
}

/// A short message for the result of a project update.
fn describe_update(outcome: UpdateOutcome) -> &'static str {
    match outcome {
        UpdateOutcome::AlreadyUpToDate => "Already up to date.",
        UpdateOutcome::Advanced => "Updated to the latest from the remote.",
        UpdateOutcome::Blocked(BlockReason::LocalChanges) => {
            "Not updated. There are local changes."
        }
        UpdateOutcome::Blocked(BlockReason::Diverged) => {
            "Not updated. The history has diverged from the remote."
        }
        UpdateOutcome::Blocked(BlockReason::NoRemote) => "Not updated. There is no tracked remote.",
    }
}

/// Start loading the available versions and mark the list as loading. We always
/// fetch prereleases too, then split them between the released and prerelease
/// channels on the screen. The list goes through the cache unless force is set.
fn load_remote(state: &mut App, force: bool) -> Task<Message> {
    state.remote = Load::Loading;
    tasks::load_remote(
        state.ctx.repository(),
        true,
        state.ctx.paths.manifest_cache(),
        force,
    )
}

/// Re read the installed engines from disk into state. This is a quick local
/// scan, so it runs inline rather than as a task.
fn reload_installed(state: &mut App) {
    match state.ctx.install_manager().list_installed() {
        Ok(mut engines) => {
            engines.sort_by(|a, b| a.variant.cmp(&b.variant).then(a.version.cmp(&b.version)));
            state.installed = Load::Loaded(engines);
        }
        Err(err) => {
            state.installed = Load::Failed(err.to_string());
            state.toast(
                ToastKind::Error,
                format!("Could not read installed engines: {err}"),
            );
        }
    }
}

/// The whole window: a sidebar on the left, the active screen on the right, and a
/// footer status bar across the bottom.
fn view(state: &App) -> Element<'_, Message> {
    let screen = match state.screen {
        Screen::Engines => screens::engines::view(state),
        Screen::Projects => screens::projects::view(state),
        Screen::Settings => screens::settings::view(state),
    };

    let content = container(screen)
        .padding(style::GAP_L)
        .width(Length::Fill)
        .height(Length::Fill);

    let base = row![sidebar(state), content].height(Length::Fill);

    // Float the toasts over the content. They do not push anything around. The
    // areas around a toast pass clicks through to the content below.
    let layered: Element<'_, Message> = if state.toasts.is_empty() {
        base.into()
    } else {
        stack![base, toast_layer(state)].into()
    };

    // Lay one dialog over everything when it is open. The backdrop dismisses it.
    let dialog: Option<(Element<'_, Message>, Message)> =
        if let Some((variant, version)) = state.confirm_remove {
            Some((
                confirm_remove_dialog(variant, version),
                Message::CancelRemove,
            ))
        } else if let Some(offer) = &state.install_offer {
            Some((offer_dialog(offer), Message::DismissOffer))
        } else if let Some(editor) = &state.pin_editor {
            Some((pin_dialog(editor), Message::CancelPin))
        } else {
            state
                .clone_dialog
                .as_ref()
                .map(|clone| (clone_dialog_view(clone), Message::CancelClone))
        };

    match dialog {
        Some((dialog, dismiss)) => modal(layered, dialog, dismiss),
        None => layered,
    }
}

/// The toasts, stacked at the bottom center of the window.
fn toast_layer(state: &App) -> Element<'_, Message> {
    let toasts: Vec<Element<'_, Message>> = state.toasts.iter().map(toast_view).collect();

    container(
        column(toasts)
            .spacing(style::GAP_S)
            .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(style::GAP_L)
    .align_x(Alignment::Center)
    .align_y(Alignment::End)
    .into()
}

/// One toast: a floating message that dismisses on click and pauses its timer
/// while the pointer is over it. A thin bar along the bottom edge shrinks from
/// full to empty to show the time left.
fn toast_view(toast: &Toast) -> Element<'_, Message> {
    let error = toast.kind == ToastKind::Error;

    let message = container(text(toast.message.clone()).size(style::TEXT_BODY))
        .padding([style::GAP_S, style::GAP_M]);

    // Inset the bar horizontally past the corner radius so it sits along the
    // straight part of the bottom edge and never pokes out of the rounded
    // corners. Its own ends are rounded so it reads as a pill.
    let countdown = container(
        progress_bar(0.0..=1.0, toast.fraction())
            .girth(3.0)
            .style(style::toast_progress(error)),
    )
    .padding([0.0, style::RADIUS_SM]);

    let card = container(column![message, countdown])
        .max_width(520.0)
        .style(style::toast(error));

    mouse_area(card)
        .on_press(Message::DismissToast(toast.id))
        .on_enter(Message::HoverToasts(true))
        .on_exit(Message::HoverToasts(false))
        .into()
}

/// Lay a dialog over the base view behind a dimmed backdrop. Clicking the
/// backdrop sends the dismiss message. The opaque wrappers stop clicks reaching
/// the base.
fn modal<'a>(
    base: Element<'a, Message>,
    dialog: Element<'a, Message>,
    dismiss: Message,
) -> Element<'a, Message> {
    stack![
        base,
        opaque(
            mouse_area(center(opaque(dialog)).style(|_theme| {
                container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                    ..container::Style::default()
                }
            }))
            .on_press(dismiss)
        )
    ]
    .into()
}

/// The install offer dialog, raised when a launch needs an engine that is not
/// installed.
fn offer_dialog<'a>(offer: &InstallOffer) -> Element<'a, Message> {
    let actions = row![
        space::horizontal(),
        button(text("Not now"))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::DismissOffer),
        button(text("Install"))
            .padding(style::BTN_PAD)
            .style(style::button_primary)
            .on_press(Message::AcceptOffer),
    ]
    .spacing(style::GAP_S)
    .align_y(Alignment::Center);

    container(
        column![
            text(format!("Install {}?", offer.label)).size(style::TEXT_HEADING),
            text("This project needs an engine version that is not installed.")
                .size(style::TEXT_BODY),
            actions,
        ]
        .spacing(style::GAP_M),
    )
    .padding(style::GAP_L)
    .max_width(460.0)
    .style(style::card)
    .into()
}

/// The pin editor dialog for setting a project's engine version. It is a dropdown
/// of installed versions, with the project's detected version suggested.
fn pin_dialog<'a>(editor: &'a PinEditor) -> Element<'a, Message> {
    let mut actions = row![space::horizontal()]
        .spacing(style::GAP_S)
        .align_y(Alignment::Center);
    actions = actions.push(
        button(text("Cancel"))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::CancelPin),
    );

    let body: Element<'a, Message> = if editor.choices.is_empty() {
        text("No installed engine versions to pin. Install one first.")
            .size(style::TEXT_BODY)
            .into()
    } else {
        actions = actions.push(
            button(text("Save"))
                .padding(style::BTN_PAD)
                .style(style::button_primary)
                .on_press(Message::SavePin),
        );
        pick_list(
            editor.choices.as_slice(),
            editor.selected.clone(),
            Message::PinSelected,
        )
        .style(style::pick_list)
        .width(Length::Fill)
        .into()
    };

    container(
        column![
            text("Set engine version").size(style::TEXT_HEADING),
            text("Choose an installed version. It is saved in project.godot.")
                .size(style::TEXT_CAPTION),
            body,
            actions,
        ]
        .spacing(style::GAP_M),
    )
    .padding(style::GAP_L)
    .max_width(460.0)
    .style(style::card)
    .into()
}

/// The clone dialog for cloning a repository as a new project.
fn clone_dialog_view<'a>(dialog: &CloneDialog) -> Element<'a, Message> {
    let actions = row![
        space::horizontal(),
        button(text("Cancel"))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::CancelClone),
        button(text("Choose folder and clone"))
            .padding(style::BTN_PAD)
            .style(style::button_primary)
            .on_press(Message::StartClone),
    ]
    .spacing(style::GAP_S)
    .align_y(Alignment::Center);

    container(
        column![
            text("Clone a repository").size(style::TEXT_HEADING),
            text("Enter a git url. You pick the destination folder next.")
                .size(style::TEXT_CAPTION),
            text_input("https://example.com/game.git", &dialog.url)
                .on_input(Message::CloneUrlChanged)
                .on_submit(Message::StartClone)
                .style(style::text_input)
                .width(Length::Fill),
            actions,
        ]
        .spacing(style::GAP_M),
    )
    .padding(style::GAP_L)
    .max_width(480.0)
    .style(style::card)
    .into()
}

/// The remove confirmation dialog card.
fn confirm_remove_dialog<'a>(
    variant: godello_core::Variant,
    version: godello_core::GodotVersion,
) -> Element<'a, Message> {
    let actions = row![
        space::horizontal(),
        button(text("Cancel"))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::CancelRemove),
        button(text("Remove"))
            .padding(style::BTN_PAD)
            .style(style::button_danger)
            .on_press(Message::Remove { variant, version }),
    ]
    .spacing(style::GAP_S)
    .align_y(Alignment::Center);

    container(
        column![
            text(format!("Remove {} {}?", version.to_tag(), variant.as_str()))
                .size(style::TEXT_HEADING),
            text("This deletes the engine from disk and cannot be undone.").size(style::TEXT_BODY),
            actions,
        ]
        .spacing(style::GAP_M),
    )
    .padding(style::GAP_L)
    .max_width(440.0)
    .style(style::card)
    .into()
}

/// The left navigation sidebar. It holds the app name, the screen links, and the
/// settings link pinned to the bottom.
fn sidebar(state: &App) -> Element<'_, Message> {
    let active = state.screen;
    let nav = column![
        text("Godello").size(style::TEXT_TITLE),
        nav_link("Projects", Screen::Projects, active),
        nav_link("Engines", Screen::Engines, active),
    ]
    .spacing(style::GAP_S)
    .width(Length::Fill);

    let bottom = column![nav_link("Settings", Screen::Settings, active)].width(Length::Fill);

    let inner = column![nav, iced::widget::space::vertical(), bottom]
        .spacing(style::GAP_M)
        .padding(style::GAP_M)
        .height(Length::Fill);

    container(inner)
        .width(Length::Fixed(style::SIDEBAR_WIDTH))
        .height(Length::Fill)
        .style(style::sidebar)
        .into()
}

/// One sidebar link. The active screen is filled so the current place is clear.
fn nav_link(label: &str, target: Screen, active: Screen) -> Element<'_, Message> {
    button(text(label.to_string()))
        .width(Length::Fill)
        .padding(style::BTN_PAD)
        .style(style::nav_button(target == active))
        .on_press(Message::Navigate(target))
        .into()
}

/// The themes offered in the settings picker.
pub fn themes() -> &'static [Theme] {
    theme::all()
}
