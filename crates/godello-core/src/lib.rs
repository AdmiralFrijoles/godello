//! Core library for Godello.
//!
//! Holds the shared logic used by the gdctl CLI and the iced GUI. That covers
//! discovering and downloading Godot engine versions, managing installed
//! versions on disk, and tracking projects with their bound engine version.

pub mod godot_github;
pub mod platform;
pub mod repository;
pub mod version;

pub use godot_github::GodotGitHubRepository;
pub use platform::{Arch, Os, PlatformError, Target};
pub use repository::{
    Asset, Checksum, ChecksumAlgorithm, EngineRepository, HttpClient, Release, RepositoryError,
};
pub use version::{GodotVersion, Stage, Variant, VersionParseError, VersionPattern};

/// Name of the application, shown in user facing output.
pub const APP_NAME: &str = "Godello";
