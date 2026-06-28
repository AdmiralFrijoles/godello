//! The projects screen.
//!
//! A full page list of project row cards. Each shows the project name, its
//! location, the engine it needs, whether it uses C#, and its version control
//! status. A menu on each row opens the editor, runs the project, sets the
//! engine version, opens the folder, updates from the remote, or forgets it.

use std::path::Path;

use godello_core::{GodotProject, ProjectEntry, RepoStatus, SyncState, Variant, VersionPattern};
use iced::widget::{button, column, container, row, scrollable, space, text, tooltip};
use iced::{Alignment, Element, Length};
use iced_aw::{DropDown, Spinner, drop_down};

use crate::gui::Message;
use crate::gui::state::{App, Load};
use crate::gui::{icons, style, widgets};

/// The width of a project row menu.
const MENU_WIDTH: f32 = 220.0;

/// Build the projects screen from the current state.
pub fn view(state: &App) -> Element<'_, Message> {
    column![
        header(),
        scrollable(project_list(state))
            .spacing(style::GAP_S)
            .height(Length::Fill),
    ]
    .spacing(style::GAP_M)
    .height(Length::Fill)
    .into()
}

/// The title and the add and clone buttons.
fn header() -> Element<'static, Message> {
    row![
        text("Projects").size(style::TEXT_TITLE),
        space::horizontal(),
        button(text("Clone"))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::OpenCloneDialog),
        button(text("Add"))
            .padding(style::BTN_PAD)
            .style(style::button_primary)
            .on_press(Message::AddProject),
    ]
    .spacing(style::GAP_S)
    .align_y(Alignment::Center)
    .into()
}

/// The list of project row cards.
fn project_list(state: &App) -> Element<'_, Message> {
    match &state.projects {
        Load::Idle | Load::Loading => hint("Reading your projects..."),
        Load::Failed(err) => hint(format!("Could not read your projects: {err}")),
        Load::Loaded(entries) if entries.is_empty() => {
            hint("No projects yet. Add one or clone a repository.")
        }
        Load::Loaded(entries) => {
            let rows: Vec<Element<'_, Message>> =
                entries.iter().map(|e| project_row(state, e)).collect();
            column(rows).spacing(style::GAP_S).into()
        }
    }
}

/// One project row card.
fn project_row<'a>(state: &'a App, entry: &'a ProjectEntry) -> Element<'a, Message> {
    let dir = entry.path.clone();
    let project = state.project_info.get(&entry.path);

    let display_name = project
        .and_then(|p| p.name.clone())
        .or_else(|| entry.name.clone())
        .unwrap_or_else(|| "(unnamed)".to_string());

    let mut badges = row![text(display_name).size(style::TEXT_HEADING)]
        .spacing(style::GAP_S)
        .align_y(Alignment::Center);

    match project {
        Some(project) => {
            badges = badges.push(engine_badge(state, project));
            if project.uses_csharp {
                badges = badges.push(pill("C#"));
            }
            if let Some(status) = state.git_status.get(&entry.path) {
                let (label, tone, tip) = git_label(status);
                badges = badges.push(git_badge(label, tone, tip));
            }
        }
        None => {
            badges = badges.push(pill("missing"));
        }
    }

    let name = column![
        badges,
        widgets::path_label(entry.path.display().to_string())
    ]
    .spacing(style::GAP_XS)
    .width(Length::Fill);

    let is_repo = state.git_status.contains_key(&entry.path);
    let menu_open = state.project_menu_open.as_deref() == Some(entry.path.as_path());
    let menu_button = button(icons::menu())
        .padding(style::BTN_PAD_COMPACT)
        .style(style::button_tertiary)
        .on_press(Message::ToggleProjectMenu(dir.clone()));
    let menu = DropDown::new(menu_button, project_menu(&dir, is_repo), menu_open)
        .width(Length::Fixed(MENU_WIDTH))
        .alignment(drop_down::Alignment::BottomEnd)
        .offset([0.0, style::GAP_XS])
        .on_dismiss(Message::CloseProjectMenu);

    let mut content = row![name].spacing(style::GAP_S).align_y(Alignment::Center);

    // When the project is busy, for example installing the engine it needs before
    // a launch, show that work in place of the launch buttons.
    match state.project_activity.get(&entry.path) {
        Some(activity) => {
            content = content.push(busy_indicator(activity.label()));
        }
        None => {
            // Quick actions beside the menu. They are also in the menu.
            let edit = button(text("Edit"))
                .padding(style::BTN_PAD)
                .style(style::button_primary)
                .on_press(Message::LaunchProject {
                    dir: dir.clone(),
                    run: false,
                });
            let run = button(text("Run"))
                .padding(style::BTN_PAD)
                .style(style::button_secondary)
                .on_press(Message::LaunchProject {
                    dir: dir.clone(),
                    run: true,
                });
            content = content.push(edit).push(run);
        }
    }
    content = content.push(menu);

    container(content)
        .padding(style::GAP_M)
        .width(Length::Fill)
        .style(style::card)
        .into()
}

/// A spinner and a short label, shown on a row while it is busy.
fn busy_indicator<'a>(label: &'a str) -> Element<'a, Message> {
    row![
        Spinner::new()
            .width(Length::Fixed(16.0))
            .height(Length::Fixed(16.0))
            .circle_radius(2.0),
        text(label).size(style::TEXT_BODY),
    ]
    .spacing(style::GAP_S)
    .align_y(Alignment::Center)
    .into()
}

/// The dropdown menu for a project.
fn project_menu<'a>(dir: &Path, is_repo: bool) -> Element<'a, Message> {
    let dir = dir.to_path_buf();
    let mut items = column![
        menu_item(
            "Open editor",
            false,
            Message::LaunchProject {
                dir: dir.clone(),
                run: false,
            },
        ),
        menu_item(
            "Run project",
            false,
            Message::LaunchProject {
                dir: dir.clone(),
                run: true,
            },
        ),
        menu_item(
            "Set engine version",
            false,
            Message::OpenPinEditor(dir.clone()),
        ),
        menu_item(
            "Open folder",
            false,
            Message::OpenProjectFolder(dir.clone()),
        ),
    ]
    .spacing(style::GAP_XS);

    if is_repo {
        items = items.push(menu_item(
            "Update from remote",
            false,
            Message::UpdateProject(dir.clone()),
        ));
    }

    items = items.push(menu_item("Remove", true, Message::RemoveProject(dir)));

    container(items)
        .padding(style::GAP_XS)
        .width(Length::Fill)
        .style(style::popover)
        .into()
}

/// One full width menu row. The danger flag colors a destructive item.
fn menu_item<'a>(label: &'a str, danger: bool, message: Message) -> Element<'a, Message> {
    button(text(label).size(style::TEXT_BODY).width(Length::Fill))
        .padding(style::BTN_PAD_COMPACT)
        .width(Length::Fill)
        .style(style::menu_item(danger))
        .on_press(message)
        .into()
}

/// A pill showing the engine the project needs, or that it names none. When the
/// project names a version that is not installed the pill turns to the danger
/// tone, so a missing engine stands out.
fn engine_badge<'a>(state: &App, project: &GodotProject) -> Element<'a, Message> {
    match project.required_engine() {
        Some((pattern, variant)) => {
            let label = format!("{pattern} {variant}");
            if engine_missing(state, &pattern, variant) {
                toned_pill(label, style::BadgeTone::Danger)
            } else {
                pill(label)
            }
        }
        None => pill("no engine set"),
    }
}

/// True when the installed list is loaded and holds no engine matching the
/// version and variant. While the list is still loading the answer is false, so
/// the pill does not flash an error before the engines are known.
fn engine_missing(state: &App, pattern: &VersionPattern, variant: Variant) -> bool {
    matches!(
        &state.installed,
        Load::Loaded(engines)
            if !engines.iter().any(|engine| {
                engine.variant == variant && pattern.matches(&engine.version)
            })
    )
}

/// A short version control status: the badge label, the tone that colors it, and
/// a plain English explanation for the tooltip. Whether there is an upstream is
/// not reported.
fn git_label(status: &RepoStatus) -> (&'static str, style::BadgeTone, &'static str) {
    if status.has_local_changes {
        return (
            "changes",
            style::BadgeTone::Warning,
            "You have changes that are not saved to version control yet.",
        );
    }
    match status.sync {
        SyncState::Behind { .. } => (
            "behind",
            style::BadgeTone::Info,
            "Updates are available to download.",
        ),
        SyncState::Ahead { .. } => (
            "ahead",
            style::BadgeTone::Info,
            "You have changes that have not been uploaded yet.",
        ),
        SyncState::Diverged => (
            "diverged",
            style::BadgeTone::Danger,
            "Your copy and the online copy have both changed.",
        ),
        // We do not care whether there is an upstream, so with no local changes
        // these all read as up to date.
        SyncState::UpToDate | SyncState::NoRemote | SyncState::Unknown => (
            "clean",
            style::BadgeTone::Success,
            "Everything is up to date.",
        ),
    }
}

/// A small rounded pill.
fn pill<'a>(label: impl Into<String>) -> Element<'a, Message> {
    container(text(label.into()).size(style::TEXT_CAPTION))
        .padding([2.0, style::GAP_S])
        .style(style::badge)
        .into()
}

/// A small rounded pill in a status tone, for a state that needs attention.
fn toned_pill<'a>(label: impl Into<String>, tone: style::BadgeTone) -> Element<'a, Message> {
    container(text(label.into()).size(style::TEXT_CAPTION))
        .padding([2.0, style::GAP_S])
        .style(style::badge_toned(tone))
        .into()
}

/// The joined git badge: a "git" pill flush against a status pill, so the pair
/// reads as one. The status half is colored by its tone, so the two tone pair is
/// always visible. Hovering the pair shows a plain English explanation.
fn git_badge<'a>(status: &'a str, tone: style::BadgeTone, tip: &'a str) -> Element<'a, Message> {
    let left = container(text("git").size(style::TEXT_CAPTION))
        .padding([2.0, style::GAP_S])
        .style(style::badge_left);
    let right = container(text(status).size(style::TEXT_CAPTION))
        .padding([2.0, style::GAP_S])
        .style(style::badge_right(tone));
    let pair = row![left, right];

    tooltip(
        pair,
        container(text(tip).size(style::TEXT_CAPTION))
            .padding(style::GAP_S)
            .style(style::card),
        tooltip::Position::Top,
    )
    .gap(style::GAP_XS)
    .delay(style::TOOLTIP_DELAY)
    .into()
}

/// A muted hint line for empty and loading states.
fn hint(message: impl Into<String>) -> Element<'static, Message> {
    text(message.into()).size(style::TEXT_BODY).into()
}
