//! Small reusable widgets shared across screens.

use iced::widget::text::Wrapping;
use iced::widget::{button, container, progress_bar, responsive, row, text, tooltip};
use iced::{Alignment, Element, Length, Size};
use iced_aw::Spinner;

use crate::gui::Message;
use crate::gui::state::InstallJob;
use crate::gui::{icons, style};

/// The progress display for an install. While downloading it is a determinate bar
/// with the percent. Once the download finishes the verify and extract have no
/// progress, so it becomes an indeterminate spinner. The caller labels it, so
/// this carries no words of its own.
pub fn install_indicator(job: &InstallJob, bar_length: Length) -> Element<'static, Message> {
    if job.installing {
        return Spinner::new()
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(18.0))
            .circle_radius(2.0)
            .into();
    }

    match job.fraction() {
        Some(fraction) => row![
            progress_bar(0.0..=1.0, fraction)
                .length(bar_length)
                .girth(8.0),
            text(format!("{}%", (fraction * 100.0).round() as u32)).size(style::TEXT_CAPTION),
        ]
        .spacing(style::GAP_S)
        .align_y(Alignment::Center)
        .into(),
        None => text("Starting...").size(style::TEXT_CAPTION).into(),
    }
}

/// The fewest characters to show before giving up and just clipping hard.
const MIN_PATH_CHARS: usize = 8;
/// An upper bound so a very wide pane does not build a giant string.
const MAX_PATH_CHARS: usize = 256;
/// Roughly how wide one caption character is, as a fraction of the font size.
/// Used to estimate how many characters fit the available width. It leans a
/// little wide so the clipped text does not overflow.
const CHAR_WIDTH_RATIO: f32 = 0.58;
/// Width taken by the chip padding, the copy icon, and the gap, reserved from the
/// measured width before fitting the text.
const CHIP_RESERVE: f32 = 40.0;
/// The fixed height the chip occupies, so the responsive area is well defined.
const CHIP_HEIGHT: f32 = 26.0;

/// A path display, shown as a small inset chip that copies the path when clicked
/// and shows a copy icon on its right.
///
/// The chip hugs the path text. The text stays on one line and clips in the
/// middle to fit the width available, keeping the start and the end since those
/// carry the most meaning, with three dots where the clip happened. The clip is
/// measured from the real width, so it reflows as the window resizes. Hovering
/// shows the full path, and clicking copies the full path, so nothing is lost.
pub fn path_label<'a>(path: impl Into<String>) -> Element<'a, Message> {
    let full = path.into();

    // responsive sits on the outside so it can measure the width the row gives
    // us. The chip built inside shrinks to the clipped text, so it hugs the text
    // instead of stretching across the row.
    container(responsive(move |size| chip(&full, size)))
        .width(Length::Fill)
        .height(Length::Fixed(CHIP_HEIGHT))
        .into()
}

/// Build the chip clipped to fit the given size.
fn chip(full: &str, size: Size) -> Element<'static, Message> {
    let char_width = (style::TEXT_CAPTION * CHAR_WIDTH_RATIO).max(1.0);
    let available = (size.width - CHIP_RESERVE).max(0.0);
    let max_chars =
        ((available / char_width).floor() as usize).clamp(MIN_PATH_CHARS, MAX_PATH_CHARS);
    let shown = clip_middle(full, max_chars);

    let label = tooltip(
        text(shown)
            .size(style::TEXT_CAPTION)
            .wrapping(Wrapping::None),
        container(text(full.to_string()).size(style::TEXT_CAPTION))
            .padding(style::GAP_S)
            .style(style::card),
        tooltip::Position::Top,
    )
    .gap(style::GAP_XS)
    .delay(style::TOOLTIP_DELAY);

    // The whole chip is the copy button. The path text takes the button text
    // color, so it turns the accent color while the mouse is down.
    let chip = button(
        row![label, icons::copy()]
            .spacing(style::GAP_XS)
            .align_y(Alignment::Center),
    )
    .padding([2.0, style::GAP_S])
    .style(style::chip)
    .on_press(Message::CopyPath(full.to_string()));

    container(chip).center_y(Length::Fill).into()
}

/// Clip a string in the middle when it is longer than max characters, keeping the
/// start and the end and joining them with three dots. Counts characters, not
/// bytes, so multi byte paths are not split inside a character.
fn clip_middle(value: &str, max: usize) -> String {
    let count = value.chars().count();
    if count <= max {
        return value.to_string();
    }
    let keep = max.saturating_sub(3);
    let head_len = keep / 2;
    let tail_len = keep - head_len;
    let head: String = value.chars().take(head_len).collect();
    let tail: String = value.chars().skip(count - tail_len).collect();
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::clip_middle;

    #[test]
    fn a_short_path_is_left_alone() {
        assert_eq!(clip_middle("/home/user/game", 48), "/home/user/game");
    }

    #[test]
    fn a_path_at_the_limit_is_left_alone() {
        let exact = "a".repeat(48);
        assert_eq!(clip_middle(&exact, 48), exact);
    }

    #[test]
    fn a_long_path_is_clipped_in_the_middle() {
        let path = "/home/jason/.local/share/godello/engines/standard/4.6.2-stable";
        let clipped = clip_middle(path, 30);
        assert_eq!(clipped.chars().count(), 30);
        assert!(clipped.contains("..."));
        assert!(clipped.starts_with("/home"));
        assert!(clipped.ends_with("stable"));
    }

    #[test]
    fn one_over_the_limit_is_clipped() {
        let over = "b".repeat(49);
        assert!(clip_middle(&over, 48).contains("..."));
    }

    #[test]
    fn clipping_keeps_whole_characters() {
        // A multi byte character must not be cut in half.
        let path = "/files/café/проект/game-directory-with-a-long-name";
        let clipped = clip_middle(path, 20);
        assert_eq!(clipped.chars().count(), 20);
        assert!(clipped.contains("..."));
    }
}
