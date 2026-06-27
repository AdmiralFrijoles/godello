//! The shared look of the app: spacing, shape, surfaces, and controls.
//!
//! Everything visual goes through here so the app stays consistent. Colors come
//! from the theme palette, which guarantees readable text on every theme, so a
//! theme switch reskins the whole app for free.
//!
//! Design rules, so the same choice is made the same way everywhere:
//!
//! Spacing. One eight point scale (with a four point half step). Use it for every
//! gap and padding. Do not invent one off numbers.
//!
//! Shape. Three radii. Small for controls (buttons, inputs, chips, segments, the
//! sidebar items). Medium for panels and cards. Pill for badges. Nothing square,
//! nothing more rounded than these.
//!
//! Layers. Depth comes from lighter or darker surface tiers, not shadows. The app
//! canvas is the plain background. Panels and cards sit one tier up with a hair
//! line border. An inset control (the path chip) sits one tier down so it reads
//! as recessed. Overlays such as tooltips use a card.
//!
//! Borders. One hair line border, one pixel, drawn from a neutral tier. Used to
//! separate a surface from the one behind it, never as decoration.
//!
//! Buttons, by emphasis. Primary is the filled accent and marks the single main
//! action of a view (install, add, apply). Secondary is a quiet neutral button
//! for supporting actions (refresh, cancel, browse). Tertiary is text only for
//! the lowest emphasis (dismiss). Danger is an outline that fills on hover, for a
//! destructive action (remove). Use exactly one primary per view.

use iced::widget::{button, container};
use iced::widget::{pick_list as pick_list_widget, text_input as text_input_widget};
use iced::{Border, Color, Shadow, Theme, Vector, border};

// Spacing. An eight point scale with a four point half step.
pub const GAP_XS: f32 = 4.0;
pub const GAP_S: f32 = 8.0;
pub const GAP_M: f32 = 16.0;
pub const GAP_L: f32 = 24.0;

// Shape. Small for controls, medium for panels, pill for badges.
pub const RADIUS_SM: f32 = 6.0;
pub const RADIUS_MD: f32 = 10.0;
pub const RADIUS_PILL: f32 = 999.0;

/// The one hair line border width.
pub const BORDER_WIDTH: f32 = 1.0;

/// The fixed width of the navigation sidebar.
pub const SIDEBAR_WIDTH: f32 = 210.0;

// Text sizes, largest to smallest.
pub const TEXT_TITLE: f32 = 22.0;
pub const TEXT_HEADING: f32 = 16.0;
pub const TEXT_BODY: f32 = 14.0;
pub const TEXT_CAPTION: f32 = 12.0;

// Button padding. The compact form suits dense rows and inline controls.
pub const BTN_PAD: [f32; 2] = [6.0, 12.0];
pub const BTN_PAD_COMPACT: [f32; 2] = [3.0, 8.0];

/// A hair line border in a neutral tier, at the given radius.
fn hairline(theme: &Theme, radius: f32) -> Border {
    Border {
        color: theme.extended_palette().background.strong.color,
        width: BORDER_WIDTH,
        radius: radius.into(),
    }
}

// Surfaces.

/// The sidebar panel. One tier up from the canvas, no border, since its edge is
/// already clear against the content.
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        ..container::Style::default()
    }
}

/// A card or panel. One tier up from the canvas with a hair line border and a
/// medium radius. The home for grouped content and detail.
pub fn card(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        border: hairline(theme, RADIUS_MD),
        ..container::Style::default()
    }
}

/// A small pill that marks a variant or a state. A quiet neutral fill.
pub fn badge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.secondary.weak.color.into()),
        text_color: Some(palette.secondary.weak.text),
        border: border::rounded(RADIUS_PILL),
        ..container::Style::default()
    }
}

/// A floating toast, tinted by the accent for info or the danger color for an
/// error. It carries a soft shadow so it reads as floating over the content.
pub fn toast(error: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let pair = if error {
            palette.danger.weak
        } else {
            palette.primary.weak
        };
        container::Style {
            background: Some(pair.color.into()),
            text_color: Some(pair.text),
            border: border::rounded(RADIUS_SM),
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                offset: Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..container::Style::default()
        }
    }
}

/// The thin countdown bar along the bottom of a toast. The filled part uses the
/// accent or danger color, over a transparent track.
pub fn toast_progress(error: bool) -> impl Fn(&Theme) -> iced::widget::progress_bar::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let bar = if error {
            palette.danger.base.color
        } else {
            palette.primary.base.color
        };
        iced::widget::progress_bar::Style {
            background: Color::TRANSPARENT.into(),
            bar: bar.into(),
            border: border::rounded(RADIUS_PILL),
        }
    }
}

/// A floating popover, such as a dropdown menu. A raised surface with a border, a
/// medium radius, and a soft shadow so it reads as floating above the content.
pub fn popover(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        border: hairline(theme, RADIUS_MD),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        },
        ..container::Style::default()
    }
}

/// A list row background. Shaded rows get a faint fill so a long list is easy to
/// scan. Plain rows are transparent.
pub fn table_row(shaded: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        if shaded {
            container::Style {
                background: Some(theme.extended_palette().background.weak.color.into()),
                ..container::Style::default()
            }
        } else {
            container::Style::default()
        }
    }
}

// Buttons, by emphasis.

/// Primary: the filled accent. The single main action of a view.
pub fn button_primary(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        border: border::rounded(RADIUS_SM),
        ..button::primary(theme, status)
    }
}

/// Secondary: a quiet neutral button for supporting actions. A soft fill with a
/// hair line border that lifts on hover.
pub fn button_secondary(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
        _ => palette.background.weak.color,
    };
    button::Style {
        background: Some(background.into()),
        text_color: palette.background.base.text,
        border: hairline(theme, RADIUS_SM),
        ..button::Style::default()
    }
}

/// Tertiary: text only, no fill, for the lowest emphasis.
pub fn button_tertiary(theme: &Theme, status: button::Status) -> button::Style {
    button::text(theme, status)
}

/// Danger: an outline that fills on hover, for a destructive action.
pub fn button_danger(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(palette.danger.base.color.into()),
            text_color: palette.danger.base.text,
            border: border::rounded(RADIUS_SM),
            ..button::Style::default()
        },
        _ => button::Style {
            background: None,
            text_color: palette.danger.base.color,
            border: Border {
                color: palette.danger.base.color,
                width: BORDER_WIDTH,
                radius: RADIUS_SM.into(),
            },
            ..button::Style::default()
        },
    }
}

/// A segment of a toggle. The active segment is the filled accent, the rest read
/// as quiet secondary buttons, so the current choice stands out.
pub fn segment(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if active {
            button_primary(theme, status)
        } else {
            button_secondary(theme, status)
        }
    }
}

/// A navigation or list selection row. The selected row is a soft accent fill, a
/// hovered row lifts a little, the rest are flat, so the current place is clear.
pub fn nav_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let background = if selected {
            palette.primary.weak.color
        } else if matches!(status, button::Status::Hovered) {
            palette.background.strong.color
        } else {
            Color::TRANSPARENT
        };
        let text_color = if selected {
            palette.primary.weak.text
        } else {
            palette.background.base.text
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: border::rounded(RADIUS_SM),
            ..button::Style::default()
        }
    }
}

/// A row in a popover menu. Flat and full width. On hover it fills with a clear
/// highlight, the accent for a normal item and a red tint for a destructive one,
/// so the highlight stands apart from the popover behind it.
pub fn menu_item(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);

        let (background, text_color) = if hovered {
            let highlight = if danger {
                palette.danger.weak
            } else {
                palette.primary.weak
            };
            (Some(highlight.color.into()), highlight.text)
        } else {
            let text = if danger {
                palette.danger.base.color
            } else {
                palette.background.base.text
            };
            (None, text)
        };

        button::Style {
            background,
            text_color,
            border: border::rounded(RADIUS_SM),
            ..button::Style::default()
        }
    }
}

/// A text input that matches the button corner radius. Otherwise the iced
/// default look for the theme.
pub fn text_input(theme: &Theme, status: text_input_widget::Status) -> text_input_widget::Style {
    let mut style = text_input_widget::default(theme, status);
    style.border.radius = RADIUS_SM.into();
    style
}

/// A pick list (dropdown) that matches the button corner radius.
pub fn pick_list(theme: &Theme, status: pick_list_widget::Status) -> pick_list_widget::Style {
    let mut style = pick_list_widget::default(theme, status);
    style.border.radius = RADIUS_SM.into();
    style
}

/// The path chip: an inset control that copies on click. A recessed surface one
/// tier below the card, with a hair line border. The text takes the accent color
/// while pressed.
pub fn chip(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let pressed = matches!(status, button::Status::Pressed);
    button::Style {
        background: Some(palette.background.base.color.into()),
        text_color: if pressed {
            palette.primary.base.color
        } else {
            palette.background.base.text
        },
        border: hairline(theme, RADIUS_SM),
        ..button::Style::default()
    }
}
