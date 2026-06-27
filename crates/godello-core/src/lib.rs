//! Core library for Godello.
//!
//! Holds the shared logic used by the gdctl CLI and the iced GUI. That covers
//! discovering and downloading Godot engine versions, managing installed
//! versions on disk, and tracking projects with their bound engine version.
//!
//! This is the start of the implementation. See docs/plan.md for the full plan.

pub mod version;

pub use version::{GodotVersion, Stage, Variant, VersionParseError, VersionPattern};

/// Name of the application, shown in user facing output.
pub const APP_NAME: &str = "Godello";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_set() {
        assert_eq!(APP_NAME, "Godello");
    }
}
