//! Core library for Godello.
//!
//! Holds the shared logic used by the gdctl CLI and the iced GUI. That covers
//! discovering and downloading Godot engine versions, managing installed
//! versions on disk, and tracking projects with their bound engine version.

pub mod config;
pub mod csharp;
pub mod godot_github;
pub mod install;
pub mod launch;
pub mod platform;
pub mod process;
pub mod project;
pub mod repository;
pub mod version;

pub use config::{ConfigError, Paths, ProjectEntry, ProjectList, Settings};
pub use csharp::{CsharpBuildTool, CsharpError, build_solutions};
pub use godot_github::GodotGitHubRepository;
pub use install::{Downloader, InstallError, InstallManager, InstalledEngine};
pub use launch::{
    LaunchError, Launcher, SystemLauncher, engine_for_project, open_editor, open_version,
    run_project,
};
pub use platform::{Arch, Os, PlatformError, Target};
pub use process::{CommandOutcome, CommandRunner, ProcessError, SystemCommandRunner};
pub use project::{GodotProject, ProjectError, find_project_dir};
pub use repository::{
    Asset, Checksum, ChecksumAlgorithm, EngineRepository, HttpClient, Release, RepositoryError,
};
pub use version::{GodotVersion, Stage, Variant, VersionParseError, VersionPattern};

/// Name of the application, shown in user facing output.
pub const APP_NAME: &str = "Godello";
