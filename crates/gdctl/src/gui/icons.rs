//! Bundled UI icons.
//!
//! These are solid icons embedded as SVG, so the app needs no icon files at
//! runtime. Each icon is tinted from the theme so it matches the text around it,
//! and brightens to the accent while the pointer is over it.

use std::sync::LazyLock;

use iced::widget::svg::Handle;
use iced::widget::{Svg, svg};
use iced::{Length, Theme};

/// The standard inline icon size, matching the caption text it sits beside.
const ICON_SIZE: f32 = 16.0;
/// A larger size for an icon that is the whole control, like a row menu button.
const MENU_ICON_SIZE: f32 = 20.0;

static COPY: LazyLock<Handle> = LazyLock::new(|| {
    Handle::from_memory(include_bytes!("icons/document-duplicate.svg").as_slice())
});
static MENU: LazyLock<Handle> =
    LazyLock::new(|| Handle::from_memory(include_bytes!("icons/ellipsis-vertical.svg").as_slice()));

/// A copy icon, for copy to clipboard actions.
pub fn copy<'a>() -> Svg<'a> {
    interactive(COPY.clone(), ICON_SIZE)
}

/// A vertical three dot icon, for opening a row menu.
pub fn menu<'a>() -> Svg<'a> {
    interactive(MENU.clone(), MENU_ICON_SIZE)
}

/// Build an icon at the inline size. It reads as the surrounding text and turns
/// the accent color while the pointer is over it, which is the press feedback for
/// the button it sits in (iced svg reports idle or hovered, and a press happens
/// while hovered).
fn interactive<'a>(handle: Handle, size: f32) -> Svg<'a> {
    svg(handle)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(|theme: &Theme, status| {
            let palette = theme.extended_palette();
            let color = match status {
                svg::Status::Hovered => palette.primary.base.color,
                svg::Status::Idle => palette.background.base.text,
            };
            svg::Style { color: Some(color) }
        })
}
