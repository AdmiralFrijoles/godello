//! Version control integration for projects.
//!
//! The VersionControl trait is the contract. Git is the first implementation.
//! Keeping this generic means other version control systems can be added later
//! without changing the callers.
//!
//! Git runs through the shared command runner, so it is a clean failure when git
//! is not installed and is fully tested with a fake runner. Status reads are safe
//! and read only. The only changing actions are a fast forward update and an
//! explicit reset, which loses local changes and is never done on its own.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::process::{CommandOutcome, CommandRunner, ProcessError};

/// The state of a project's repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    /// The checked out branch, or None when the head is detached.
    pub branch: Option<String>,
    /// The upstream tracking branch, or None when there is none.
    pub upstream: Option<String>,
    /// Commits the branch is ahead of its upstream.
    pub ahead: u32,
    /// Commits the branch is behind its upstream.
    pub behind: u32,
    /// True when the working tree has uncommitted changes.
    pub dirty: bool,
}

impl RepoStatus {
    /// True when the branch tracks an upstream and matches it.
    pub fn is_up_to_date(&self) -> bool {
        self.upstream.is_some() && self.ahead == 0 && self.behind == 0
    }
}

/// The result of trying to bring a branch up to date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Nothing to do, the branch was not behind its upstream.
    AlreadyUpToDate,
    /// The branch was fast forwarded to its upstream.
    FastForwarded,
    /// The update was not done. The caller can explain and offer a reset.
    Blocked(BlockReason),
}

/// Why an update did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The working tree has uncommitted changes.
    Dirty,
    /// The branch and its upstream have diverged, so it is not a fast forward.
    NotFastForward,
    /// The branch has no upstream to update from.
    NoUpstream,
}

/// The contract for a version control system.
pub trait VersionControl {
    /// A short stable id, for example git.
    fn id(&self) -> &str;

    /// True when the folder is a working copy of this system.
    fn is_repo(&self, dir: &Path) -> bool;

    /// Read the status of the repository in a folder. When fetch is true the
    /// remote refs are updated first so the ahead and behind counts are current.
    fn status(&self, dir: &Path, fetch: bool) -> Result<RepoStatus, VcsError>;

    /// Bring the branch up to date with its upstream by fast forward only. Does
    /// nothing and reports a block when the tree is dirty, there is no upstream,
    /// or the history has diverged.
    fn update(&self, dir: &Path) -> Result<UpdateOutcome, VcsError>;

    /// Hard reset the branch to its upstream. This loses local changes and local
    /// commits. It is a separate explicit action, never part of an update.
    fn reset_to_upstream(&self, dir: &Path) -> Result<(), VcsError>;

    /// Clone a repository url into a destination folder.
    fn clone_repo(&self, url: &str, dest: &Path) -> Result<(), VcsError>;
}

/// Git, backed by the system git command through a command runner.
pub struct Git<C> {
    runner: C,
}

impl<C: CommandRunner> Git<C> {
    pub fn new(runner: C) -> Self {
        Git { runner }
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Result<CommandOutcome, VcsError> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        self.runner
            .run(OsStr::new("git"), &args, cwd)
            .map_err(VcsError::from)
    }

    fn ensure_repo(&self, dir: &Path) -> Result<(), VcsError> {
        let out = self.run(dir, &["rev-parse", "--is-inside-work-tree"])?;
        if out.success && out.trimmed_stdout() == "true" {
            Ok(())
        } else {
            Err(VcsError::NotARepo(dir.to_path_buf()))
        }
    }

    fn current_branch(&self, dir: &Path) -> Result<Option<String>, VcsError> {
        let out = self.run(dir, &["branch", "--show-current"])?;
        let name = out.trimmed_stdout();
        if out.success && !name.is_empty() {
            Ok(Some(name.to_string()))
        } else {
            Ok(None)
        }
    }

    fn upstream(&self, dir: &Path) -> Result<Option<String>, VcsError> {
        let out = self.run(
            dir,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )?;
        let name = out.trimmed_stdout();
        if out.success && !name.is_empty() {
            Ok(Some(name.to_string()))
        } else {
            Ok(None)
        }
    }

    fn ahead_behind(&self, dir: &Path) -> Result<(u32, u32), VcsError> {
        let out = self.run(
            dir,
            &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
        )?;
        if out.success {
            Ok(parse_ahead_behind(out.trimmed_stdout()))
        } else {
            Ok((0, 0))
        }
    }

    fn is_dirty(&self, dir: &Path) -> Result<bool, VcsError> {
        let out = self.run(dir, &["status", "--porcelain"])?;
        Ok(!out.trimmed_stdout().is_empty())
    }
}

impl<C: CommandRunner> VersionControl for Git<C> {
    fn id(&self) -> &str {
        "git"
    }

    fn is_repo(&self, dir: &Path) -> bool {
        self.ensure_repo(dir).is_ok()
    }

    fn status(&self, dir: &Path, fetch: bool) -> Result<RepoStatus, VcsError> {
        self.ensure_repo(dir)?;
        if fetch {
            // Best effort. A failed fetch, for example with no network, leaves
            // the answer based on the last known remote refs.
            self.run(dir, &["fetch"])?;
        }
        let branch = self.current_branch(dir)?;
        let upstream = self.upstream(dir)?;
        let (ahead, behind) = if upstream.is_some() {
            self.ahead_behind(dir)?
        } else {
            (0, 0)
        };
        let dirty = self.is_dirty(dir)?;
        Ok(RepoStatus {
            branch,
            upstream,
            ahead,
            behind,
            dirty,
        })
    }

    fn update(&self, dir: &Path) -> Result<UpdateOutcome, VcsError> {
        let status = self.status(dir, true)?;
        if status.upstream.is_none() {
            return Ok(UpdateOutcome::Blocked(BlockReason::NoUpstream));
        }
        if status.dirty {
            return Ok(UpdateOutcome::Blocked(BlockReason::Dirty));
        }
        if status.behind == 0 {
            return Ok(UpdateOutcome::AlreadyUpToDate);
        }
        if status.ahead > 0 {
            // Behind and ahead means the history diverged.
            return Ok(UpdateOutcome::Blocked(BlockReason::NotFastForward));
        }
        let out = self.run(dir, &["merge", "--ff-only", "@{upstream}"])?;
        if out.success {
            Ok(UpdateOutcome::FastForwarded)
        } else {
            Ok(UpdateOutcome::Blocked(BlockReason::NotFastForward))
        }
    }

    fn reset_to_upstream(&self, dir: &Path) -> Result<(), VcsError> {
        self.ensure_repo(dir)?;
        self.run(dir, &["fetch"])?;
        if self.upstream(dir)?.is_none() {
            return Err(VcsError::NoUpstream);
        }
        let out = self.run(dir, &["reset", "--hard", "@{upstream}"])?;
        if out.success {
            Ok(())
        } else {
            Err(VcsError::Command {
                what: "reset".to_string(),
                output: out.combined(),
            })
        }
    }

    fn clone_repo(&self, url: &str, dest: &Path) -> Result<(), VcsError> {
        let args = vec![
            OsString::from("clone"),
            OsString::from(url),
            dest.as_os_str().to_os_string(),
        ];
        let out = self
            .runner
            .run(OsStr::new("git"), &args, Path::new("."))
            .map_err(VcsError::from)?;
        if out.success {
            Ok(())
        } else {
            Err(VcsError::Command {
                what: "clone".to_string(),
                output: out.combined(),
            })
        }
    }
}

/// Parse the output of a left right count. Git prints the upstream only count
/// first, which is how far behind the branch is, then the head only count, which
/// is how far ahead it is. Returns ahead then behind.
fn parse_ahead_behind(text: &str) -> (u32, u32) {
    let mut parts = text.split_whitespace();
    let behind = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let ahead = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    (ahead, behind)
}

/// An error from a version control action.
#[derive(Debug)]
pub enum VcsError {
    /// The version control program is not installed.
    NotInstalled,
    /// The folder is not a working copy.
    NotARepo(PathBuf),
    /// The branch has no upstream, so the action cannot run.
    NoUpstream,
    /// A command failed in a way that was not expected.
    Command { what: String, output: String },
    /// The process could not be started or read.
    Process(ProcessError),
}

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcsError::NotInstalled => write!(f, "git is not installed"),
            VcsError::NotARepo(path) => write!(f, "{} is not a git repository", path.display()),
            VcsError::NoUpstream => write!(f, "the branch has no upstream"),
            VcsError::Command { what, output } => {
                if output.is_empty() {
                    write!(f, "git {what} failed")
                } else {
                    write!(f, "git {what} failed:\n{output}")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A runner that returns a set outcome per git subcommand. The key is the
    /// arguments joined with spaces. Unset keys return a clean success.
    struct ScriptedRunner {
        responses: HashMap<String, CommandOutcome>,
        calls: RefCell<Vec<String>>,
        git_missing: bool,
    }

    impl ScriptedRunner {
        fn new() -> Self {
            ScriptedRunner {
                responses: HashMap::new(),
                calls: RefCell::new(Vec::new()),
                git_missing: false,
            }
        }

        fn missing() -> Self {
            ScriptedRunner {
                responses: HashMap::new(),
                calls: RefCell::new(Vec::new()),
                git_missing: true,
            }
        }

        /// Set a response for a subcommand, matched by its joined arguments.
        fn on(mut self, key: &str, success: bool, stdout: &str) -> Self {
            self.responses.insert(
                key.to_string(),
                CommandOutcome {
                    success,
                    code: Some(if success { 0 } else { 1 }),
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                },
            );
            self
        }

        /// The default response, a working repo with a clean tree, so a test only
        /// has to set what it cares about.
        fn repo(self) -> Self {
            self.on("rev-parse --is-inside-work-tree", true, "true")
                .on("branch --show-current", true, "main")
                .on("status --porcelain", true, "")
        }

        fn called(&self, key: &str) -> bool {
            self.calls.borrow().iter().any(|c| c == key)
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(
            &self,
            program: &OsStr,
            args: &[OsString],
            _cwd: &Path,
        ) -> Result<CommandOutcome, ProcessError> {
            if self.git_missing {
                return Err(ProcessError::ProgramNotFound(program.to_os_string()));
            }
            let key = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            self.calls.borrow_mut().push(key.clone());
            Ok(self.responses.get(&key).cloned().unwrap_or(CommandOutcome {
                success: true,
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            }))
        }
    }

    fn git(runner: ScriptedRunner) -> Git<ScriptedRunner> {
        Git::new(runner)
    }

    const UPSTREAM_ARGS: &str = "rev-parse --abbrev-ref --symbolic-full-name @{upstream}";
    const COUNT_ARGS: &str = "rev-list --left-right --count @{upstream}...HEAD";

    fn dir() -> &'static Path {
        Path::new("/some/project")
    }

    // Status.

    #[test]
    fn status_reports_up_to_date() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "0\t0");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (0, 0));
        assert!(!status.dirty);
        assert!(status.is_up_to_date());
    }

    #[test]
    fn status_parses_ahead_and_behind() {
        // Git prints behind first, then ahead.
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "3\t2");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.behind, 3);
        assert_eq!(status.ahead, 2);
        assert!(!status.is_up_to_date());
    }

    #[test]
    fn status_reports_a_detached_head() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("branch --show-current", true, "")
            .on(UPSTREAM_ARGS, false, "");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.branch, None);
    }

    #[test]
    fn status_reports_no_upstream() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.upstream, None);
        assert!(!status.is_up_to_date());
    }

    #[test]
    fn status_reports_a_dirty_tree() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("status --porcelain", true, " M src/main.rs")
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "0\t0");
        let status = git(runner).status(dir(), false).unwrap();
        assert!(status.dirty);
    }

    #[test]
    fn status_fetches_when_asked() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let git = git(runner);
        git.status(dir(), true).unwrap();
        assert!(git.runner.called("fetch"));
    }

    #[test]
    fn status_does_not_fetch_when_not_asked() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let git = git(runner);
        git.status(dir(), false).unwrap();
        assert!(!git.runner.called("fetch"));
    }

    #[test]
    fn status_on_a_non_repo_errors() {
        let runner = ScriptedRunner::new().on("rev-parse --is-inside-work-tree", false, "");
        let result = git(runner).status(dir(), false);
        assert!(matches!(result, Err(VcsError::NotARepo(_))));
    }

    #[test]
    fn status_with_git_missing_is_not_installed() {
        let result = git(ScriptedRunner::missing()).status(dir(), false);
        assert!(matches!(result, Err(VcsError::NotInstalled)));
    }

    // Update.

    #[test]
    fn update_does_nothing_when_not_behind() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "0\t0");
        let outcome = git(runner).update(dir()).unwrap();
        assert_eq!(outcome, UpdateOutcome::AlreadyUpToDate);
    }

    #[test]
    fn update_fast_forwards_when_behind_only() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "2\t0")
            .on("merge --ff-only @{upstream}", true, "Updated");
        let git = git(runner);
        let outcome = git.update(dir()).unwrap();
        assert_eq!(outcome, UpdateOutcome::FastForwarded);
        assert!(git.runner.called("merge --ff-only @{upstream}"));
    }

    #[test]
    fn update_blocks_a_dirty_tree() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("status --porcelain", true, " M file")
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "2\t0");
        let outcome = git(runner).update(dir()).unwrap();
        assert_eq!(outcome, UpdateOutcome::Blocked(BlockReason::Dirty));
    }

    #[test]
    fn update_blocks_on_divergence() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "2\t3");
        let outcome = git(runner).update(dir()).unwrap();
        assert_eq!(outcome, UpdateOutcome::Blocked(BlockReason::NotFastForward));
    }

    #[test]
    fn update_blocks_without_an_upstream() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let outcome = git(runner).update(dir()).unwrap();
        assert_eq!(outcome, UpdateOutcome::Blocked(BlockReason::NoUpstream));
    }

    #[test]
    fn update_does_not_merge_when_dirty() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("status --porcelain", true, " M file")
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "2\t0");
        let git = git(runner);
        git.update(dir()).unwrap();
        assert!(!git.runner.called("merge --ff-only @{upstream}"));
    }

    // Reset.

    #[test]
    fn reset_hard_resets_to_upstream() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on("reset --hard @{upstream}", true, "");
        let git = git(runner);
        git.reset_to_upstream(dir()).unwrap();
        assert!(git.runner.called("reset --hard @{upstream}"));
    }

    #[test]
    fn reset_without_an_upstream_errors() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let git = git(runner);
        let result = git.reset_to_upstream(dir());
        assert!(matches!(result, Err(VcsError::NoUpstream)));
        // It must not reset when there is no upstream.
        assert!(!git.runner.called("reset --hard @{upstream}"));
    }

    #[test]
    fn reset_on_a_non_repo_errors() {
        let runner = ScriptedRunner::new().on("rev-parse --is-inside-work-tree", false, "");
        let result = git(runner).reset_to_upstream(dir());
        assert!(matches!(result, Err(VcsError::NotARepo(_))));
    }

    // Clone.

    #[test]
    fn clone_runs_git_clone_with_url_and_dest() {
        let runner = ScriptedRunner::new();
        let git = git(runner);
        git.clone_repo("https://example.test/game.git", Path::new("/games/game"))
            .unwrap();
        assert!(
            git.runner
                .called("clone https://example.test/game.git /games/game")
        );
    }

    #[test]
    fn clone_failure_is_reported() {
        let runner = ScriptedRunner::new().on(
            "clone https://example.test/game.git /games/game",
            false,
            "fatal: repository not found",
        );
        let result =
            git(runner).clone_repo("https://example.test/game.git", Path::new("/games/game"));
        match result {
            Err(VcsError::Command { what, output }) => {
                assert_eq!(what, "clone");
                assert!(output.contains("not found"));
            }
            other => panic!("expected a command error, got {other:?}"),
        }
    }

    #[test]
    fn clone_with_git_missing_is_not_installed() {
        let result = git(ScriptedRunner::missing())
            .clone_repo("https://example.test/game.git", Path::new("/games/game"));
        assert!(matches!(result, Err(VcsError::NotInstalled)));
    }

    #[test]
    fn id_is_git() {
        assert_eq!(git(ScriptedRunner::new()).id(), "git");
    }

    #[test]
    fn parse_ahead_behind_reads_behind_then_ahead() {
        assert_eq!(parse_ahead_behind("3\t2"), (2, 3));
        assert_eq!(parse_ahead_behind("0 0"), (0, 0));
        assert_eq!(parse_ahead_behind("garbage"), (0, 0));
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }
}
