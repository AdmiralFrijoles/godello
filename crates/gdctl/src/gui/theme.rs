//! The app themes.
//!
//! Godello ships its own dark and light themes built from a small palette. iced
//! turns each palette into a full set of readable color pairs and surface tiers,
//! so the whole app restyles when the theme changes.
//!
//! The palettes follow current practice for tool interfaces. The neutrals carry
//! a slight cool blue undertone rather than flat grey, the dark base is a soft
//! dark grey and the light base a soft off white rather than pure black or white,
//! the text is an off white or a soft near black, and the accent is a calm
//! indigo. The status colors are an amber, a green, and a red, kept calm so they
//! read as cues, not alarms.

use std::sync::LazyLock;

use iced::theme::Palette;
use iced::{Color, Theme};

/// The dark palette. A soft dark cool grey base, off white text, and a calm
/// indigo accent. iced lightens the base in small steps for the raised surfaces.
fn dark_palette() -> Palette {
    Palette {
        background: Color::from_rgb8(0x1c, 0x1e, 0x24),
        text: Color::from_rgb8(0xed, 0xee, 0xf0),
        primary: Color::from_rgb8(0x6e, 0x7b, 0xe8),
        success: Color::from_rgb8(0x30, 0xa4, 0x6c),
        warning: Color::from_rgb8(0xfb, 0xbf, 0x24),
        danger: Color::from_rgb8(0xe5, 0x48, 0x4d),
    }
}

/// The light palette. A soft off white cool grey base, a soft near black text,
/// and a slightly deeper indigo so the accent keeps its weight on a light
/// background.
fn light_palette() -> Palette {
    Palette {
        background: Color::from_rgb8(0xed, 0xee, 0xf2),
        text: Color::from_rgb8(0x1c, 0x20, 0x24),
        primary: Color::from_rgb8(0x4f, 0x5b, 0xd0),
        success: Color::from_rgb8(0x30, 0xa4, 0x6c),
        warning: Color::from_rgb8(0xfb, 0xbf, 0x24),
        danger: Color::from_rgb8(0xe5, 0x48, 0x4d),
    }
}

// Build each theme once so they share one instance. That keeps theme comparison
// cheap and makes the settings picker selection match.
static DARK: LazyLock<Theme> = LazyLock::new(|| Theme::custom("Godello Dark", dark_palette()));
static LIGHT: LazyLock<Theme> = LazyLock::new(|| Theme::custom("Godello Light", light_palette()));
static ALL: LazyLock<Vec<Theme>> = LazyLock::new(|| vec![DARK.clone(), LIGHT.clone()]);

/// The default theme.
pub fn dark() -> Theme {
    DARK.clone()
}

/// The theme for a saved name, or the default when the name is not known. The
/// returned theme is one of the shared instances, so the settings picker matches
/// it by identity.
pub fn by_name(name: &str) -> Theme {
    match name {
        "light" => LIGHT.clone(),
        _ => dark(),
    }
}

/// The short saved name for a theme, the inverse of by_name.
pub fn name_of(theme: &Theme) -> &'static str {
    if *theme == *LIGHT { "light" } else { "dark" }
}

/// All themes offered in settings.
pub fn all() -> &'static [Theme] {
    &ALL
}
