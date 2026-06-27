//! The settings screen.
//!
//! For now it holds the theme picker and a cache control. The full settings form,
//! bound to the saved settings, comes in a later pass, so this also notes what is
//! coming.

use iced::widget::{button, column, pick_list, row, text};
use iced::{Alignment, Element, Length};

use crate::gui::Message;
use crate::gui::state::App;
use crate::gui::{style, themes};

/// Build the settings screen from the current state.
pub fn view(state: &App) -> Element<'_, Message> {
    let theme_row = row![
        text("Theme").width(Length::Fixed(160.0)),
        pick_list(themes(), Some(state.theme.clone()), Message::SetTheme).style(style::pick_list),
    ]
    .spacing(style::GAP_M)
    .align_y(Alignment::Center);

    let cache_row = row![
        column![
            text("Version list cache").size(style::TEXT_BODY),
            text("The available versions are cached to load faster. Clear it to fetch a fresh list next time.")
                .size(style::TEXT_CAPTION),
        ]
        .spacing(style::GAP_XS)
        .width(Length::Fill),
        button(text("Clear cache"))
            .padding(style::BTN_PAD)
            .style(style::button_secondary)
            .on_press(Message::ClearCache),
    ]
    .spacing(style::GAP_M)
    .align_y(Alignment::Center);

    column![
        text("Settings").size(style::TEXT_TITLE),
        text("Appearance").size(style::TEXT_HEADING),
        theme_row,
        text("Cache").size(style::TEXT_HEADING),
        cache_row,
        text("More settings are coming. The build, variant, and engine folder options will live here.")
            .size(style::TEXT_CAPTION),
    ]
    .spacing(style::GAP_M)
    .into()
}
