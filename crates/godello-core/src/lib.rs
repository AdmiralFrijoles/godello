//! Core library for Godello.
//!
//! Holds the shared logic used by the gdctl CLI and the iced GUI. That covers
//! discovering and downloading Godot engine versions, managing installed
//! versions on disk, and tracking projects with their bound engine version.

pub mod config;
pub mod csharp;
pub mod git;
pub mod godot_github;
pub mod install;
pub mod launch;
pub mod platform;
pub mod process;
pub mod project;
pub mod repository;
pub mod vcs;
pub mod version;

pub use config::{ConfigError, Paths, ProjectEntry, ProjectList, Settings};
pub use csharp::{CsharpBuildTool, CsharpError, build_solutions};
pub use git::Git;
pub use godot_github::GodotGitHubRepository;
pub use install::{
    DownloadProgress, Downloader, InstallError, InstallManager, InstalledEngine, NoProgress,
};
pub use launch::{
    LaunchError, Launcher, SystemLauncher, engine_for_project, file_manager_program, open_editor,
    open_path, open_version, run_project,
};
pub use platform::{Arch, Os, PlatformError, Target};
pub use process::{CommandOutcome, CommandRunner, ProcessError, SystemCommandRunner};
pub use project::{GodotProject, ProjectError, find_project_dir};
pub use repository::{
    Asset, Checksum, ChecksumAlgorithm, EngineRepository, HttpClient, Release, RepositoryError,
};
pub use vcs::{
    BlockReason, DEFAULT_MAIN_BRANCH, RepoStatus, SyncState, UpdateOutcome, VcsError,
    VersionControl,
};
pub use version::{GodotVersion, Stage, Variant, VersionParseError, VersionPattern};

/// Name of the application, shown in user facing output.
pub const APP_NAME: &str = "Godello";
