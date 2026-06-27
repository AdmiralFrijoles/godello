//! Core library for Godello.
//!
//! Holds the shared logic used by the gdctl CLI and the iced GUI. That covers
//! discovering and downloading Godot engine versions, managing installed
//! versions on disk, and tracking projects with their bound engine version.
//!
//! Nothing of substance lives here yet. This crate is the scaffold for the
//! coming implementation work.

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
