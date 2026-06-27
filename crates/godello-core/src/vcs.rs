//! The version control abstraction.
//!
//! This module holds the contract and the shared types, with no ties to any one
//! system. The names avoid git only jargon so a centralized system or a newer
//! system can implement the trait without strain. Concrete systems, such as git,
//! live in their own modules and implement VersionControl.

use std::path::{Path, PathBuf};

use crate::process::ProcessError;

/// The branch an update pulls from when a project does not name its own. Most
/// repositories use this name for the line of work everyone shares.
pub const DEFAULT_MAIN_BRANCH: &str = "main";

/// How a working copy compares to its tracked remote. Some systems give exact
/// commit counts and some only give a coarse state, so the counts are optional.
/// A centralized system can never be Ahead, since its commits go straight to the
/// server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// The working copy matches its tracked remote.
    UpToDate,
    /// The remote has changes the working copy does not, by this many if known.
    Behind { commits: Option<u32> },
    /// The working copy has changes the remote does not, by this many if known.
    Ahead { commits: Option<u32> },
    /// Both sides have changes the other does not.
    Diverged,
    /// There is no tracked remote to compare against.
    NoRemote,
    /// The comparison could not be worked out.
    Unknown,
}

/// The state of a project's working copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    /// The branch, bookmark, stream, or named position the working copy is on.
    /// None when there is no such name, for example a detached or centralized
    /// working copy.
    pub branch: Option<String>,
    /// The remote the working copy tracks, such as a remote branch or a server
    /// location. None when there is none.
    pub tracked_remote: Option<String>,
    /// How the working copy compares to its tracked remote.
    pub sync: SyncState,
    /// True when the working copy has local changes that are not committed. For
    /// some systems this means edited files, for others it means files opened
    /// for edit in a pending change.
    pub has_local_changes: bool,
}

impl RepoStatus {
    /// True when the working copy tracks a remote and matches it.
    pub fn is_up_to_date(&self) -> bool {
        matches!(self.sync, SyncState::UpToDate)
    }
}

/// The result of trying to bring a working copy up to date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Nothing to do, the working copy was not behind its remote.
    AlreadyUpToDate,
    /// The working copy advanced cleanly to the remote state.
    Advanced,
    /// The update was not done. The caller can explain and offer a reset.
    Blocked(BlockReason),
}

/// Why an update did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The working copy has local changes that the update would overwrite.
    LocalChanges,
    /// The local and remote histories diverged, so there is no clean advance.
    Diverged,
    /// There is no tracked remote to update from.
    NoRemote,
    /// Merging the main branch in would cause conflicts, so the update was rolled
    /// back and nothing changed.
    Conflict,
}

/// The contract for a version control system.
pub trait VersionControl {
    /// A short stable id, for example git.
    fn id(&self) -> &str;

    /// True when the folder is a working copy of this system.
    fn is_repo(&self, dir: &Path) -> bool;

    /// Read the status of the working copy. When contact_remote is true the
    /// remote is contacted first so the comparison is current. When false the
    /// answer is based on what is already known locally.
    fn status(&self, dir: &Path, contact_remote: bool) -> Result<RepoStatus, VcsError>;

    /// Bring the working copy up to date with the main branch. The main branch is
    /// fetched from the remote and fast forwarded, then merged into the working
    /// branch. A merge that would conflict is rolled back so nothing changes.
    /// Reports a block when there is no remote, the main branch cannot fast
    /// forward, the merge would conflict, or local changes would be overwritten.
    fn update(&self, dir: &Path, main_branch: &str) -> Result<UpdateOutcome, VcsError>;

    /// Force the working copy to match its tracked remote. This loses local
    /// changes and local commits. It is a separate explicit action, never part
    /// of an update.
    fn reset_to_remote(&self, dir: &Path) -> Result<(), VcsError>;

    /// Get a fresh local copy of a remote repository into a destination folder.
    fn clone_repo(&self, url: &str, dest: &Path) -> Result<(), VcsError>;
}

/// An error from a version control action.
#[derive(Debug)]
pub enum VcsError {
    /// The version control tool is not installed.
    NotInstalled,
    /// The folder is not a working copy.
    NotARepo(PathBuf),
    /// The working copy has no tracked remote, so the action cannot run.
    NoRemote,
    /// A command failed in a way that was not expected.
    Command { what: String, output: String },
    /// The underlying process could not be started or read.
    Process(ProcessError),
}

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcsError::NotInstalled => write!(f, "the version control tool is not installed"),
            VcsError::NotARepo(path) => write!(f, "{} is not a working copy", path.display()),
            VcsError::NoRemote => write!(f, "the working copy has no tracked remote"),
            VcsError::Command { what, output } => {
                if output.is_empty() {
                    write!(f, "{what} failed")
                } else {
                    write!(f, "{what} failed:\n{output}")
                }
            }
            VcsError::Process(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for VcsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VcsError::Process(err) => Some(err),
            _ => None,
        }
    }
}

impl From<ProcessError> for VcsError {
    fn from(err: ProcessError) -> Self {
        match err {
            ProcessError::ProgramNotFound(_) => VcsError::NotInstalled,
            other => VcsError::Process(other),
        }
    }
}
