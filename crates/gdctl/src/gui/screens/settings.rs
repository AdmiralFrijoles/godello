//! The settings screen.
//!
//! A form bound to the saved settings plus the theme picker and a cache control.
//! Every control applies its change right away and saves to disk, so there is no
//! separate apply or revert step.

use godello_core::{CsharpBuildTool, Variant};
use iced::widget::{Row, button, checkbox, column, container, pick_list, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::gui::state::{App, SettingsTab};
use crate::gui::{Message, style, themes, widgets};

/// The variants offered for the default variant setting.
const VARIANTS: [Variant; 2] = [Variant::Standard, Variant::Mono];
/// The tools offered for the C# build setting.
const BUILD_TOOLS: [CsharpBuildTool; 2] = [CsharpBuildTool::Godot, CsharpBuildTool::Dotnet];

/// Build the settings screen from the current state. A tab bar picks one group
/// of settings, and only that group's form shows below it.
pub fn view(state: &App) -> Element<'_, Message> {
    let form = match state.settings_tab {
        SettingsTab::Appearance => appearance(state),
        SettingsTab::Engines => engines(state),
        SettingsTab::Projects => projects(state),
        SettingsTab::Csharp => csharp(state),
        SettingsTab::Cache => cache(),
    };

    column![
        text("Settings").size(style::TEXT_TITLE),
        tab_bar(state),
        scrollable(container(form).padding([0.0, style::GAP_S]))
            .spacing(style::GAP_S)
            .height(Length::Fill),
    ]
    .spacing(style::GAP_M)
    .height(Length::Fill)
    .into()
}

/// The row of tabs across the top, one per group, with the current one marked.
fn tab_bar(state: &App) -> Element<'_, Message> {
    let mut bar = row![].spacing(style::GAP_XS);
    for tab in SettingsTab::ALL {
        bar = bar.push(
            button(text(tab.label()))
                .padding(style::BTN_PAD)
                .style(style::segment(state.settings_tab == tab))
                .on_press(Message::SetSettingsTab(tab)),
        );
    }
    bar.into()
}

/// The appearance settings: the theme picker.
fn appearance(state: &App) -> Element<'_, Message> {
    let theme_control = pick_list(themes(), Some(state.theme.clone()), Message::SetTheme)
        .style(style::pick_list)
        .into();

    column![field("Theme", "The color theme of the app.", theme_control)]
        .spacing(style::GAP_M)
        .into()
}

/// The engine settings: install location, default variant, and prereleases.
fn engines(state: &App) -> Element<'_, Message> {
    let settings = &state.ctx.settings;

    let engines_dir = settings.effective_engines_dir(&state.ctx.paths);
    let custom_dir = settings.engine_install_dir.is_some();
    let mut dir_buttons = row![
        button(text("Choose..."))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::ChooseEngineDir),
    ]
    .spacing(style::GAP_S)
    .align_y(Alignment::Center);
    if custom_dir {
        dir_buttons = dir_buttons.push(
            button(text("Reset"))
                .padding(style::BTN_PAD)
                .style(style::button_tertiary)
                .on_press(Message::ResetEngineDir),
        );
    }
    let engine_dir = row![
        column![
            text("Engine install location").size(style::TEXT_BODY),
            text(if custom_dir {
                "Engines install into this folder."
            } else {
                "Engines install into the default folder."
            })
            .size(style::TEXT_CAPTION),
            widgets::path_label(engines_dir.display().to_string()),
        ]
        .spacing(style::GAP_XS)
        .width(Length::Fill),
        dir_buttons,
    ]
    .spacing(style::GAP_M)
    .align_y(Alignment::Center);

    let variant_control = pick_list(
        &VARIANTS[..],
        Some(settings.default_variant),
        Message::SetDefaultVariant,
    )
    .style(style::pick_list)
    .into();

    let prereleases_control = checkbox(settings.include_prereleases)
        .on_toggle(Message::SetIncludePrereleases)
        .into();

    column![
        engine_dir,
        field(
            "Default variant",
            "The build used when nothing else decides it.",
            variant_control,
        ),
        field(
            "Include prereleases",
            "Offer release candidate, beta, and dev builds when resolving versions.",
            prereleases_control,
        ),
    ]
    .spacing(style::GAP_M)
    .into()
}

/// The project settings: the default folder for new projects and clones.
fn projects(state: &App) -> Element<'_, Message> {
    let settings = &state.ctx.settings;

    let project_dir = match &settings.default_project_dir {
        Some(dir) => {
            let buttons = row![
                button(text("Choose..."))
                    .padding(style::BTN_PAD)
                    .style(style::button_secondary)
                    .on_press(Message::ChooseProjectDir),
                button(text("Reset"))
                    .padding(style::BTN_PAD)
                    .style(style::button_tertiary)
                    .on_press(Message::ResetProjectDir),
            ]
            .spacing(style::GAP_S)
            .align_y(Alignment::Center);
            project_dir_row(
                "New projects and clones start in this folder.",
                Some(dir.display().to_string()),
                buttons,
            )
        }
        None => {
            let buttons = row![
                button(text("Choose..."))
                    .padding(style::BTN_PAD)
                    .style(style::button_secondary)
                    .on_press(Message::ChooseProjectDir),
            ]
            .align_y(Alignment::Center);
            project_dir_row(
                "No default is set, so you pick a folder each time you clone.",
                None,
                buttons,
            )
        }
    };

    column![project_dir].spacing(style::GAP_M).into()
}

/// The C# settings: whether to build before a launch and which tool builds it.
fn csharp(state: &App) -> Element<'_, Message> {
    let settings = &state.ctx.settings;

    let build_control = checkbox(settings.build_csharp_before_launch)
        .on_toggle(Message::SetBuildCsharp)
        .into();

    let tool_control = pick_list(
        &BUILD_TOOLS[..],
        Some(settings.csharp_build_tool),
        Message::SetCsharpBuildTool,
    )
    .style(style::pick_list)
    .into();

    column![
        field(
            "Build before launching",
            "Build the C# solution before opening or running a C# project.",
            build_control,
        ),
        field(
            "Build tool",
            "Build with the Godot editor, or with the dotnet command line tool.",
            tool_control,
        ),
    ]
    .spacing(style::GAP_M)
    .into()
}

/// The cache settings: clear the cached version list.
fn cache() -> Element<'static, Message> {
    let cache_control = button(text("Clear cache"))
        .padding(style::BTN_PAD)
        .style(style::button_secondary)
        .on_press(Message::ClearCache)
        .into();

    column![field(
        "Version list cache",
        "Available versions are cached to load faster. Clear it to fetch a fresh list next time.",
        cache_control,
    )]
    .spacing(style::GAP_M)
    .into()
}

/// The default project folder row: a title and description on the left, the
/// chosen path when set, and the choose and reset buttons on the right.
fn project_dir_row<'a>(
    description: &'a str,
    path: Option<String>,
    buttons: Row<'a, Message>,
) -> Element<'a, Message> {
    let mut info = column![
        text("Default project folder").size(style::TEXT_BODY),
        text(description).size(style::TEXT_CAPTION),
    ]
    .spacing(style::GAP_XS)
    .width(Length::Fill);
    if let Some(path) = path {
        info = info.push(widgets::path_label(path));
    }
    row![info, buttons]
        .spacing(style::GAP_M)
        .align_y(Alignment::Center)
        .into()
}

/// One setting row: a title and a short description on the left, the control on
/// the right.
fn field<'a>(
    title: &'a str,
    description: &'a str,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(style::TEXT_BODY),
            text(description).size(style::TEXT_CAPTION),
        ]
        .spacing(style::GAP_XS)
        .width(Length::Fill),
        control,
    ]
    .spacing(style::GAP_M)
    .align_y(Alignment::Center)
    .into()
}
