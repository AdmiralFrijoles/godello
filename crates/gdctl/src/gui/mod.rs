//! The Godello desktop app.
//!
//! This is the second front end over the same core the CLI uses. It keeps no
//! logic of its own beyond presentation. Every real action calls into the core
//! through the shared context.
//!
//! The window is a left navigation sidebar, a content area that uses a list and
//! detail split, and a footer status bar. Pass one ships the engines screen and
//! a theme picker. Projects and a full settings form come in later passes.

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

use std::time::Duration;

use iced::widget::{
    button, center, column, container, mouse_area, opaque, progress_bar, row, space, stack, text,
};
use iced::{Alignment, Color, Element, Length, Size, Subscription, Task, Theme};

use godello_core::{SystemLauncher, open_path, open_version};

pub use message::Message;
use state::{App, EnginesTab, Load, Screen, Toast, ToastKind};

/// How often the toast timer ticks. About sixty times a second, so the countdown
/// bar moves smoothly rather than in visible steps.
const TOAST_TICK: Duration = Duration::from_millis(16);

/// Open the desktop app and run until the window closes.
///
/// iced owns the event loop and its own runtime, so this blocks. The boot hook
/// clones the shared context into the starting state and kicks off the first
/// load of installed engines.
pub fn launch(ctx: crate::context::Context) -> iced::Result {
    iced::application(
        move || (App::new(ctx.clone()), Task::done(Message::RefreshInstalled)),
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

/// Tick the toast timer only while there are toasts to age.
fn subscription(state: &App) -> Subscription<Message> {
    if state.toasts.is_empty() {
        Subscription::none()
    } else {
        iced::time::every(TOAST_TICK).map(|_| Message::ToastTick)
    }
}

/// Apply a message to the state and return any follow up work.
fn update(state: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(screen) => {
            state.screen = screen;
            state.menu_open = None;
            // Load the available list the first time it is needed.
            if screen == Screen::Engines
                && state.engines_tab == EnginesTab::Available
                && matches!(state.remote, Load::Idle)
            {
                return load_remote(state, false);
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

        Message::Install { variant, version } => {
            if state.is_installing(variant, version) {
                return Task::none();
            }
            // Make the install cancelable. The abort handle stops the download,
            // the cancel flag stops the extract that runs off the executor.
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
                }
                Err(err) => {
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
        Screen::Projects => screens::placeholder(
            "Projects",
            "The projects screen is coming soon. It will list your projects, bind each to an engine version, and launch the editor.",
        ),
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

    // Lay the remove confirmation over everything when it is open.
    match state.confirm_remove {
        Some((variant, version)) => modal(layered, confirm_remove_dialog(variant, version)),
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
/// backdrop cancels. The opaque wrappers stop clicks reaching the base.
fn modal<'a>(base: Element<'a, Message>, dialog: Element<'a, Message>) -> Element<'a, Message> {
    stack![
        base,
        opaque(
            mouse_area(center(opaque(dialog)).style(|_theme| {
                container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                    ..container::Style::default()
                }
            }))
            .on_press(Message::CancelRemove)
        )
    ]
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
