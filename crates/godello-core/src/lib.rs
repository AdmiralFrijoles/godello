//! Core library for Godello.
//!
//! Holds the shared logic used by the gdctl CLI and the iced GUI. That covers
//! discovering and downloading Godot engine versions, managing installed
//! versions on disk, and tracking projects with their bound engine version.

pub mod config;
pub mod godot_github;
pub mod install;
pub mod platform;
pub mod project;
pub mod repository;
pub mod version;

pub use config::{ConfigError, Paths, ProjectEntry, ProjectList, Settings};
pub use godot_github::GodotGitHubRepository;
pub use install::{Downloader, InstallError, InstallManager, InstalledEngine};
pub use platform::{Arch, Os, PlatformError, Target};
pub use project::{GodotProject, ProjectError, find_project_dir};
pub use repository::{
    Asset, Checksum, ChecksumAlgorithm, EngineRepository, HttpClient, Release, RepositoryError,
};
pub use version::{GodotVersion, Stage, Variant, VersionParseError, VersionPattern};

/// Name of the application, shown in user facing output.
pub const APP_NAME: &str = "Godello";
