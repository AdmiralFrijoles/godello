//! The screens of the desktop app. Pass one ships the engines screen and a small
//! settings screen with a theme picker. Projects is a placeholder for now.

pub mod engines;
pub mod settings;

use iced::Element;
use iced::widget::{column, text};

use crate::gui::Message;
use crate::gui::style;

/// A simple stand in for a screen that is not built yet.
pub fn placeholder<'a>(title: &str, body: &str) -> Element<'a, Message> {
    column![
        text(title.to_string()).size(style::TEXT_TITLE),
        text(body.to_string()).size(style::TEXT_BODY),
    ]
    .spacing(style::GAP_S)
    .into()
}
