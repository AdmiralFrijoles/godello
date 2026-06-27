//! The engines screen.
//!
//! A toggle at the top switches between the engines on disk and the ones
//! available to install. Installed engines are row cards with a remove control.
//! The available list is a compact table, one row per version, with a download
//! action per variant side by side, split into released and prerelease channels.

use godello_core::{GodotVersion, Release, Variant};
use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length};
use iced_aw::{DropDown, drop_down};

use crate::gui::Message;
use crate::gui::state::{App, Channel, EnginesTab, InstallJob, Load};
use crate::gui::{icons, style, widgets};

/// The variant columns of the available table, in a fixed order so the controls
/// line up the same way on every row.
const VARIANT_COLUMNS: [Variant; 2] = [Variant::Standard, Variant::Mono];
/// The width of each variant download column. Sized to hold the longest state,
/// the "Installed" label and the download progress with its cancel control, so
/// nothing shifts or clips as a row changes state. The columns sit on the right.
const VARIANT_COL_WIDTH: f32 = 120.0;
/// The width of the release date column.
const DATE_COL_WIDTH: f32 = 130.0;
/// The width of the row dropdown menu.
const MENU_WIDTH: f32 = 220.0;

/// Build the engines screen from the current state.
pub fn view(state: &App) -> Element<'_, Message> {
    let list = match state.engines_tab {
        EnginesTab::Installed => installed_list(state),
        EnginesTab::Available => available_list(state),
    };

    column![
        header(state),
        controls(state),
        // Embed the scrollbar with spacing so it reserves its own room and never
        // sits on top of the row content.
        scrollable(list).spacing(style::GAP_S).height(Length::Fill),
    ]
    .spacing(style::GAP_M)
    .height(Length::Fill)
    .into()
}

/// The title and the installed versus available toggle.
fn header(state: &App) -> Element<'_, Message> {
    let toggle = row![
        button(text("Installed"))
            .padding(style::BTN_PAD)
            .style(style::segment(state.engines_tab == EnginesTab::Installed))
            .on_press(Message::SetEnginesTab(EnginesTab::Installed)),
        button(text("Available"))
            .padding(style::BTN_PAD)
            .style(style::segment(state.engines_tab == EnginesTab::Available))
            .on_press(Message::SetEnginesTab(EnginesTab::Available)),
    ]
    .spacing(style::GAP_XS);

    row![
        text("Engines").size(style::TEXT_TITLE),
        space::horizontal(),
        toggle,
    ]
    .align_y(Alignment::Center)
    .into()
}

/// The search box, plus the channel toggle and a refresh button when the
/// available list is showing.
fn controls(state: &App) -> Element<'_, Message> {
    let search = text_input("Filter by version", &state.filter)
        .on_input(Message::FilterChanged)
        .style(style::text_input)
        .width(Length::Fixed(240.0));

    let mut controls = row![search]
        .spacing(style::GAP_M)
        .align_y(Alignment::Center);

    if state.engines_tab == EnginesTab::Available {
        let (released, prerelease) = channel_counts(state);
        let channel = row![
            channel_button(
                format!("Released ({released})"),
                Channel::Released,
                state.channel,
            ),
            channel_button(
                format!("Prerelease ({prerelease})"),
                Channel::Prerelease,
                state.channel,
            ),
        ]
        .spacing(style::GAP_XS);
        let refresh_label = match &state.remote {
            Load::Idle => "Load",
            _ => "Refresh",
        };
        controls = controls.push(channel).push(space::horizontal()).push(
            button(text(refresh_label))
                .padding(style::BTN_PAD)
                .style(style::button_secondary)
                .on_press(Message::LoadRemote { force: true }),
        );
    }

    controls.into()
}

/// One channel toggle segment.
fn channel_button(label: String, target: Channel, current: Channel) -> Element<'static, Message> {
    button(text(label).size(style::TEXT_CAPTION))
        .padding(style::BTN_PAD_COMPACT)
        .style(style::segment(target == current))
        .on_press(Message::SetChannel(target))
        .into()
}

/// The installed engines as row cards, with any installs in progress shown first
/// so the user sees them land here while they download.
fn installed_list(state: &App) -> Element<'_, Message> {
    // Loading and error states only matter when there is nothing else to show.
    if state.jobs.is_empty() {
        match &state.installed {
            Load::Idle | Load::Loading => return hint("Reading installed engines..."),
            Load::Failed(err) => {
                return hint(format!("Could not read installed engines: {err}"));
            }
            Load::Loaded(_) => {}
        }
    }

    let mut rows: Vec<Element<'_, Message>> = Vec::new();

    // Installs in progress, at the top.
    for job in &state.jobs {
        if matches_filter(&state.filter, job.version, job.variant) {
            rows.push(installing_row(job));
        }
    }

    // Engines already on disk.
    if let Load::Loaded(engines) = &state.installed {
        for engine in engines {
            if !matches_filter(&state.filter, engine.version, engine.variant) {
                continue;
            }
            let menu_open = state.menu_open == Some((engine.variant, engine.version));
            rows.push(installed_row(
                engine.version,
                engine.variant,
                engine.path.display().to_string(),
                menu_open,
            ));
        }
    }

    if rows.is_empty() {
        hint("No engines match.")
    } else {
        column(rows).spacing(style::GAP_S).into()
    }
}

/// One row card for an install in progress: the version, a progress bar with the
/// percent, and a cancel control.
fn installing_row(job: &InstallJob) -> Element<'_, Message> {
    // The version, then an "Installing" label beside the progress bar or spinner,
    // so the row reads clearly as an install in progress.
    let name = column![
        text(job.version.to_tag()).size(style::TEXT_HEADING),
        row![
            text("Installing").size(style::TEXT_CAPTION),
            widgets::install_indicator(job, Length::Fill),
        ]
        .spacing(style::GAP_S)
        .align_y(Alignment::Center),
    ]
    .spacing(style::GAP_XS)
    .width(Length::Fill);

    let cancel = button(text("Cancel"))
        .padding(style::BTN_PAD)
        .style(style::button_secondary)
        .on_press(Message::CancelInstall {
            variant: job.variant,
            version: job.version,
        });

    container(
        row![name, variant_pill(job.variant), cancel]
            .spacing(style::GAP_M)
            .align_y(Alignment::Center),
    )
    .padding(style::GAP_M)
    .width(Length::Fill)
    .style(style::card)
    .into()
}

/// One installed engine row card. The menu button on the right opens a floating
/// dropdown of actions anchored under it.
fn installed_row<'a>(
    version: GodotVersion,
    variant: Variant,
    path: String,
    menu_open: bool,
) -> Element<'a, Message> {
    let name = column![
        text(version.to_tag()).size(style::TEXT_HEADING),
        widgets::path_label(path),
    ]
    .spacing(style::GAP_XS)
    .width(Length::Fill);

    let menu_button = button(icons::menu())
        .padding(style::BTN_PAD_COMPACT)
        .style(style::button_tertiary)
        .on_press(Message::ToggleEngineMenu { variant, version });

    // A real floating dropdown anchored under the button. It overlays the rest of
    // the content rather than pushing it around, and an outside click dismisses
    // it through on_dismiss.
    let menu = DropDown::new(menu_button, engine_menu(variant, version), menu_open)
        .width(Length::Fixed(MENU_WIDTH))
        .alignment(drop_down::Alignment::BottomEnd)
        .offset([0.0, style::GAP_XS])
        .on_dismiss(Message::CloseEngineMenu);

    container(
        row![name, variant_pill(variant), menu]
            .spacing(style::GAP_M)
            .align_y(Alignment::Center),
    )
    .padding(style::GAP_M)
    .width(Length::Fill)
    .style(style::card)
    .into()
}

/// The dropdown contents for an installed engine: a floating card of actions.
fn engine_menu<'a>(variant: Variant, version: GodotVersion) -> Element<'a, Message> {
    container(
        column![
            menu_item(
                "Open folder",
                false,
                Message::OpenFolder { variant, version }
            ),
            menu_item(
                "Open project manager",
                false,
                Message::OpenProjectManager { variant, version },
            ),
            menu_item("Remove", true, Message::RequestRemove { variant, version }),
        ]
        .spacing(style::GAP_XS),
    )
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

/// A small chip showing the variant.
fn variant_pill<'a>(variant: Variant) -> Element<'a, Message> {
    container(text(variant.as_str().to_string()).size(style::TEXT_CAPTION))
        .padding([2.0, style::GAP_S])
        .style(style::badge)
        .into()
}

/// The available versions as a compact table for the selected channel. One row
/// per version, with a download action for each variant side by side.
fn available_list(state: &App) -> Element<'_, Message> {
    match &state.remote {
        Load::Idle => hint("Choose load to fetch the available versions."),
        Load::Loading => hint("Loading available versions..."),
        Load::Failed(err) => hint(format!("Could not load available versions: {err}")),
        Load::Loaded(releases) => {
            let want_prerelease = state.channel == Channel::Prerelease;
            let rows: Vec<&Release> = releases
                .iter()
                .filter(|release| release.version.is_prerelease() == want_prerelease)
                .filter(|release| release_matches(&state.filter, release))
                .collect();

            if rows.is_empty() {
                return hint("No versions match.");
            }

            let mut table = column![table_head()].spacing(0);
            for (index, release) in rows.iter().enumerate() {
                table = table.push(version_row(state, release, index % 2 == 1));
            }
            table.into()
        }
    }
}

/// The table header row with a divider under it. The columns match the version
/// rows below: version, release date, then one column per variant.
fn table_head() -> Element<'static, Message> {
    let mut head = row![
        text("Version")
            .size(style::TEXT_CAPTION)
            .width(Length::Fill),
        column_label("Released", DATE_COL_WIDTH),
    ]
    .spacing(style::GAP_M)
    .align_y(Alignment::Center);

    for variant in VARIANT_COLUMNS {
        head = head.push(
            container(text(variant_title(variant).to_string()).size(style::TEXT_CAPTION))
                .align_right(Length::Fixed(VARIANT_COL_WIDTH)),
        );
    }

    column![head.padding([0.0, style::GAP_S]), rule::horizontal(1)]
        .spacing(style::GAP_XS)
        .into()
}

/// A fixed width header label, so each header sits over its column.
fn column_label(label: &str, width: f32) -> Element<'static, Message> {
    container(text(label.to_string()).size(style::TEXT_CAPTION))
        .width(Length::Fixed(width))
        .into()
}

/// One version row laid out in fixed columns: version, release date, then a
/// download control per variant. The fixed widths keep every control lined up
/// with the others above and below it. Alternate rows are shaded for scanning.
fn version_row<'a>(state: &'a App, release: &'a Release, shaded: bool) -> Element<'a, Message> {
    let version = release.version;
    let date = release
        .release_date
        .clone()
        .unwrap_or_else(|| "-".to_string());

    let mut cells = row![
        text(version.to_tag())
            .size(style::TEXT_BODY)
            .width(Length::Fill),
        container(text(date).size(style::TEXT_CAPTION)).width(Length::Fixed(DATE_COL_WIDTH)),
    ]
    .spacing(style::GAP_M)
    .align_y(Alignment::Center);

    // One cell per variant column. Offered variants get a control, the rest get
    // an empty cell of the same width so the columns stay aligned.
    for variant in VARIANT_COLUMNS {
        let cell: Element<'a, Message> = if release.variants.contains(&variant) {
            variant_control(state, variant, version)
        } else {
            space::horizontal().into()
        };
        cells = cells.push(container(cell).align_right(Length::Fixed(VARIANT_COL_WIDTH)));
    }

    container(cells)
        .padding([style::GAP_XS, style::GAP_S])
        .width(Length::Fill)
        .style(style::table_row(shaded))
        .into()
}

/// The download control for one variant. While an install runs it just reads
/// "Installing", the progress lives in the footer and the installed list. The
/// variant is named by the column, so the control does not repeat it.
fn variant_control(state: &App, variant: Variant, version: GodotVersion) -> Element<'_, Message> {
    if state.is_installing(variant, version) {
        return text("Installing").size(style::TEXT_CAPTION).into();
    }

    if state.is_installed(variant, version) {
        return text("Installed").size(style::TEXT_CAPTION).into();
    }

    button(text("Install").size(style::TEXT_CAPTION))
        .padding(style::BTN_PAD_COMPACT)
        .style(style::button_primary)
        .on_press(Message::Install { variant, version })
        .into()
}

/// The display name for a variant column header.
fn variant_title(variant: Variant) -> &'static str {
    match variant {
        Variant::Standard => "Standard",
        Variant::Mono => "Mono",
    }
}

/// A muted hint line for empty and loading states.
fn hint(message: impl Into<String>) -> Element<'static, Message> {
    text(message.into()).size(style::TEXT_BODY).into()
}

/// Count the released and prerelease versions in the loaded remote list.
fn channel_counts(state: &App) -> (usize, usize) {
    match &state.remote {
        Load::Loaded(releases) => {
            let prerelease = releases
                .iter()
                .filter(|release| release.version.is_prerelease())
                .count();
            (releases.len() - prerelease, prerelease)
        }
        _ => (0, 0),
    }
}

/// True when the filter is empty, or the version tag or any variant of the
/// release contains the filter text.
fn release_matches(filter: &str, release: &Release) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_ascii_lowercase();
    if release
        .version
        .to_tag()
        .to_ascii_lowercase()
        .contains(&needle)
    {
        return true;
    }
    release
        .variants
        .iter()
        .any(|variant| variant.as_str().contains(&needle))
}

/// True when the version or variant contains the filter text. An empty filter
/// matches everything.
fn matches_filter(filter: &str, version: GodotVersion, variant: Variant) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_ascii_lowercase();
    version.to_tag().to_ascii_lowercase().contains(&needle) || variant.as_str().contains(&needle)
}
