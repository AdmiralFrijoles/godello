//! The git implementation of the version control abstraction.
//!
//! Git runs through the shared command runner, so it is a clean failure when git
//! is not installed and is fully tested with a fake runner. Status reads are safe
//! and read only. The only changing actions are an update that advances cleanly
//! and an explicit reset, which loses local changes and is never done on its own.

use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::process::{CommandOutcome, CommandRunner};
use crate::vcs::{BlockReason, RepoStatus, SyncState, UpdateOutcome, VcsError, VersionControl};

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

    fn tracked_remote(&self, dir: &Path) -> Result<Option<String>, VcsError> {
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

    fn has_local_changes(&self, dir: &Path) -> Result<bool, VcsError> {
        let out = self.run(dir, &["status", "--porcelain"])?;
        Ok(!out.trimmed_stdout().is_empty())
    }

    /// The first configured remote, by convention the one to update from. None
    /// when the working copy has no remote at all.
    fn first_remote(&self, dir: &Path) -> Result<Option<String>, VcsError> {
        let out = self.run(dir, &["remote"])?;
        Ok(out
            .trimmed_stdout()
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string))
    }

    /// How many commits the target has that the base does not. Used to tell a real
    /// advance from a no op without parsing localized command output.
    fn ahead_of(&self, dir: &Path, base: &str, target: &str) -> Result<u32, VcsError> {
        let range = format!("{base}..{target}");
        let out = self.run(dir, &["rev-list", "--count", &range])?;
        if out.success {
            Ok(out.trimmed_stdout().parse().unwrap_or(0))
        } else {
            Ok(0)
        }
    }

    /// True when a merge is part way done, which is how we tell a conflict from a
    /// merge that never started.
    fn merge_in_progress(&self, dir: &Path) -> Result<bool, VcsError> {
        let out = self.run(dir, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"])?;
        Ok(out.success)
    }

    /// Fast forward the current branch onto a ref. Advances only when it is a
    /// clean fast forward, otherwise reports a divergence.
    fn fast_forward(&self, dir: &Path, onto: &str) -> Result<UpdateOutcome, VcsError> {
        if self.ahead_of(dir, "HEAD", onto)? == 0 {
            return Ok(UpdateOutcome::AlreadyUpToDate);
        }
        let out = self.run(dir, &["merge", "--ff-only", onto])?;
        if out.success {
            Ok(UpdateOutcome::Advanced)
        } else {
            Ok(UpdateOutcome::Blocked(BlockReason::Diverged))
        }
    }
}

impl<C: CommandRunner> VersionControl for Git<C> {
    fn id(&self) -> &str {
        "git"
    }

    fn is_repo(&self, dir: &Path) -> bool {
        self.ensure_repo(dir).is_ok()
    }

    fn status(&self, dir: &Path, contact_remote: bool) -> Result<RepoStatus, VcsError> {
        self.ensure_repo(dir)?;
        if contact_remote {
            // Best effort. A failed fetch, for example with no network, leaves
            // the answer based on the last known remote refs.
            self.run(dir, &["fetch"])?;
        }
        let branch = self.current_branch(dir)?;
        let tracked_remote = self.tracked_remote(dir)?;
        let sync = match &tracked_remote {
            Some(_) => {
                let (ahead, behind) = self.ahead_behind(dir)?;
                sync_state(ahead, behind)
            }
            None => SyncState::NoRemote,
        };
        let has_local_changes = self.has_local_changes(dir)?;
        Ok(RepoStatus {
            branch,
            tracked_remote,
            sync,
            has_local_changes,
        })
    }

    fn update(&self, dir: &Path, main_branch: &str) -> Result<UpdateOutcome, VcsError> {
        self.ensure_repo(dir)?;

        let remote = match self.first_remote(dir)? {
            Some(remote) => remote,
            None => return Ok(UpdateOutcome::Blocked(BlockReason::NoRemote)),
        };

        // Fetch the main branch from the remote so the comparison is current. A
        // failed fetch, for example with no network, is a real error, not a block.
        let fetched = self.run(dir, &["fetch", &remote, main_branch])?;
        if !fetched.success {
            return Err(VcsError::Command {
                what: format!("git fetch {remote} {main_branch}"),
                output: fetched.combined(),
            });
        }

        let remote_main = format!("{remote}/{main_branch}");

        // When the working branch is the main branch there is nothing to merge
        // into. Just fast forward it to the freshly fetched remote main.
        if self.current_branch(dir)?.as_deref() == Some(main_branch) {
            return self.fast_forward(dir, &remote_main);
        }

        // Fast forward the local main branch to the remote main without checking
        // it out. This updates, or creates, the local main ref, but only when it
        // is a clean advance. A non fast forward means local main has its own
        // commits, so there is no safe automatic update.
        let refspec = format!("{remote_main}:{main_branch}");
        let advanced = self.run(dir, &["fetch", ".", &refspec])?;
        if !advanced.success {
            return Ok(UpdateOutcome::Blocked(BlockReason::Diverged));
        }

        // Merge the advanced main branch into the working branch. If the working
        // branch already contains it there is nothing to do.
        if self.ahead_of(dir, "HEAD", main_branch)? == 0 {
            return Ok(UpdateOutcome::AlreadyUpToDate);
        }
        let merged = self.run(dir, &["merge", main_branch])?;
        if merged.success {
            Ok(UpdateOutcome::Advanced)
        } else if self.merge_in_progress(dir)? {
            // Conflicts. Roll the merge back so the working copy is left as it was.
            self.run(dir, &["merge", "--abort"])?;
            Ok(UpdateOutcome::Blocked(BlockReason::Conflict))
        } else {
            // The merge never started, for example local changes would be
            // overwritten. Nothing changed.
            Ok(UpdateOutcome::Blocked(BlockReason::LocalChanges))
        }
    }

    fn reset_to_remote(&self, dir: &Path) -> Result<(), VcsError> {
        self.ensure_repo(dir)?;
        self.run(dir, &["fetch"])?;
        if self.tracked_remote(dir)?.is_none() {
            return Err(VcsError::NoRemote);
        }
        let out = self.run(dir, &["reset", "--hard", "@{upstream}"])?;
        if out.success {
            Ok(())
        } else {
            Err(VcsError::Command {
                what: "git reset".to_string(),
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
                what: "git clone".to_string(),
                output: out.combined(),
            })
        }
    }
}

/// Turn ahead and behind counts into a sync state.
fn sync_state(ahead: u32, behind: u32) -> SyncState {
    match (ahead, behind) {
        (0, 0) => SyncState::UpToDate,
        (0, behind) => SyncState::Behind {
            commits: Some(behind),
        },
        (ahead, 0) => SyncState::Ahead {
            commits: Some(ahead),
        },
        _ => SyncState::Diverged,
    }
}

/// Parse the output of a left right count. Git prints the upstream only count
/// first, which is how far behind the working copy is, then the head only count,
/// which is how far ahead it is. Returns ahead then behind.
fn parse_ahead_behind(text: &str) -> (u32, u32) {
    let mut parts = text.split_whitespace();
    let behind = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let ahead = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    (ahead, behind)
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

        /// A working repo with a clean tree, so a test only sets what it cares
        /// about.
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
        ) -> Result<CommandOutcome, crate::process::ProcessError> {
            if self.git_missing {
                return Err(crate::process::ProcessError::ProgramNotFound(
                    program.to_os_string(),
                ));
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

    #[test]
    fn sync_state_maps_counts() {
        assert_eq!(sync_state(0, 0), SyncState::UpToDate);
        assert_eq!(sync_state(0, 3), SyncState::Behind { commits: Some(3) });
        assert_eq!(sync_state(2, 0), SyncState::Ahead { commits: Some(2) });
        assert_eq!(sync_state(2, 3), SyncState::Diverged);
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
        assert_eq!(status.tracked_remote.as_deref(), Some("origin/main"));
        assert_eq!(status.sync, SyncState::UpToDate);
        assert!(!status.has_local_changes);
        assert!(status.is_up_to_date());
    }

    #[test]
    fn status_reports_behind_only() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "3\t0");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.sync, SyncState::Behind { commits: Some(3) });
        assert!(!status.is_up_to_date());
    }

    #[test]
    fn status_reports_ahead_only() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "0\t2");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.sync, SyncState::Ahead { commits: Some(2) });
    }

    #[test]
    fn status_reports_diverged() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "3\t2");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.sync, SyncState::Diverged);
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
    fn status_reports_no_remote() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let status = git(runner).status(dir(), false).unwrap();
        assert_eq!(status.tracked_remote, None);
        assert_eq!(status.sync, SyncState::NoRemote);
        assert!(!status.is_up_to_date());
    }

    #[test]
    fn status_reports_local_changes() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("status --porcelain", true, " M src/main.rs")
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on(COUNT_ARGS, true, "0\t0");
        let status = git(runner).status(dir(), false).unwrap();
        assert!(status.has_local_changes);
    }

    #[test]
    fn status_contacts_remote_when_asked() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let git = git(runner);
        git.status(dir(), true).unwrap();
        assert!(git.runner.called("fetch"));
    }

    #[test]
    fn status_does_not_contact_remote_when_not_asked() {
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

    /// A repo on a working branch other than main, with one remote, where both
    /// fetches succeed. A test then sets the merge result and the commit counts.
    fn working_branch_repo() -> ScriptedRunner {
        ScriptedRunner::new()
            .repo()
            .on("branch --show-current", true, "feature")
            .on("remote", true, "origin")
            .on("fetch origin main", true, "")
            .on("fetch . origin/main:main", true, "")
    }

    const HEAD_AHEAD_OF_MAIN: &str = "rev-list --count HEAD..main";
    const HEAD_AHEAD_OF_REMOTE_MAIN: &str = "rev-list --count HEAD..origin/main";
    const MERGE_HEAD: &str = "rev-parse --verify --quiet MERGE_HEAD";

    #[test]
    fn update_merges_main_into_the_working_branch() {
        let runner = working_branch_repo().on(HEAD_AHEAD_OF_MAIN, true, "2").on(
            "merge main",
            true,
            "Merge made",
        );
        let git = git(runner);
        assert_eq!(git.update(dir(), "main").unwrap(), UpdateOutcome::Advanced);
        assert!(git.runner.called("fetch origin main"));
        assert!(git.runner.called("fetch . origin/main:main"));
        assert!(git.runner.called("merge main"));
    }

    #[test]
    fn update_does_nothing_when_the_working_branch_has_main() {
        let runner = working_branch_repo().on(HEAD_AHEAD_OF_MAIN, true, "0");
        let git = git(runner);
        assert_eq!(
            git.update(dir(), "main").unwrap(),
            UpdateOutcome::AlreadyUpToDate
        );
        // Nothing to merge, so no merge runs.
        assert!(!git.runner.called("merge main"));
    }

    #[test]
    fn update_aborts_and_reports_a_conflict() {
        let runner = working_branch_repo()
            .on(HEAD_AHEAD_OF_MAIN, true, "2")
            .on("merge main", false, "CONFLICT (content)")
            // A merge is part way done, so this was a real conflict.
            .on(MERGE_HEAD, true, "abc123");
        let git = git(runner);
        assert_eq!(
            git.update(dir(), "main").unwrap(),
            UpdateOutcome::Blocked(BlockReason::Conflict)
        );
        // The conflicted merge is rolled back.
        assert!(git.runner.called("merge --abort"));
    }

    #[test]
    fn update_blocks_when_local_changes_would_be_overwritten() {
        let runner = working_branch_repo()
            .on(HEAD_AHEAD_OF_MAIN, true, "2")
            .on(
                "merge main",
                false,
                "Your local changes would be overwritten",
            )
            // No merge started, so this was not a conflict.
            .on(MERGE_HEAD, false, "");
        let git = git(runner);
        assert_eq!(
            git.update(dir(), "main").unwrap(),
            UpdateOutcome::Blocked(BlockReason::LocalChanges)
        );
        // Nothing to roll back when the merge never started.
        assert!(!git.runner.called("merge --abort"));
    }

    #[test]
    fn update_blocks_when_local_main_diverged() {
        // The local main branch cannot fast forward to the remote main.
        let runner = working_branch_repo().on("fetch . origin/main:main", false, "rejected");
        let git = git(runner);
        assert_eq!(
            git.update(dir(), "main").unwrap(),
            UpdateOutcome::Blocked(BlockReason::Diverged)
        );
        assert!(!git.runner.called("merge main"));
    }

    #[test]
    fn update_fast_forwards_when_on_the_main_branch() {
        // The working branch is main, so the update fast forwards it in place.
        let runner = ScriptedRunner::new()
            .repo()
            .on("remote", true, "origin")
            .on("fetch origin main", true, "")
            .on(HEAD_AHEAD_OF_REMOTE_MAIN, true, "3")
            .on("merge --ff-only origin/main", true, "Updated");
        let git = git(runner);
        assert_eq!(git.update(dir(), "main").unwrap(), UpdateOutcome::Advanced);
        assert!(git.runner.called("merge --ff-only origin/main"));
        // On main there is no separate working branch to merge into.
        assert!(!git.runner.called("fetch . origin/main:main"));
    }

    #[test]
    fn update_on_main_does_nothing_when_up_to_date() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("remote", true, "origin")
            .on("fetch origin main", true, "")
            .on(HEAD_AHEAD_OF_REMOTE_MAIN, true, "0");
        let git = git(runner);
        assert_eq!(
            git.update(dir(), "main").unwrap(),
            UpdateOutcome::AlreadyUpToDate
        );
        assert!(!git.runner.called("merge --ff-only origin/main"));
    }

    #[test]
    fn update_on_main_blocks_when_not_a_fast_forward() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("remote", true, "origin")
            .on("fetch origin main", true, "")
            .on(HEAD_AHEAD_OF_REMOTE_MAIN, true, "3")
            .on(
                "merge --ff-only origin/main",
                false,
                "not possible to fast-forward",
            );
        let git = git(runner);
        assert_eq!(
            git.update(dir(), "main").unwrap(),
            UpdateOutcome::Blocked(BlockReason::Diverged)
        );
    }

    #[test]
    fn update_blocks_without_a_remote() {
        let runner = ScriptedRunner::new().repo().on("remote", true, "");
        assert_eq!(
            git(runner).update(dir(), "main").unwrap(),
            UpdateOutcome::Blocked(BlockReason::NoRemote)
        );
    }

    #[test]
    fn update_errors_when_the_fetch_fails() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("branch --show-current", true, "feature")
            .on("remote", true, "origin")
            .on("fetch origin main", false, "could not resolve host");
        let result = git(runner).update(dir(), "main");
        assert!(matches!(result, Err(VcsError::Command { .. })));
    }

    #[test]
    fn update_uses_the_configured_main_branch() {
        let runner = ScriptedRunner::new()
            .repo()
            .on("branch --show-current", true, "feature")
            .on("remote", true, "origin")
            .on("fetch origin trunk", true, "")
            .on("fetch . origin/trunk:trunk", true, "")
            .on("rev-list --count HEAD..trunk", true, "1")
            .on("merge trunk", true, "Merge made");
        let git = git(runner);
        assert_eq!(git.update(dir(), "trunk").unwrap(), UpdateOutcome::Advanced);
        assert!(git.runner.called("fetch origin trunk"));
        assert!(git.runner.called("merge trunk"));
    }

    // Reset.

    #[test]
    fn reset_matches_the_remote() {
        let runner = ScriptedRunner::new()
            .repo()
            .on(UPSTREAM_ARGS, true, "origin/main")
            .on("reset --hard @{upstream}", true, "");
        let git = git(runner);
        git.reset_to_remote(dir()).unwrap();
        assert!(git.runner.called("reset --hard @{upstream}"));
    }

    #[test]
    fn reset_without_a_remote_errors() {
        let runner = ScriptedRunner::new().repo().on(UPSTREAM_ARGS, false, "");
        let git = git(runner);
        let result = git.reset_to_remote(dir());
        assert!(matches!(result, Err(VcsError::NoRemote)));
        assert!(!git.runner.called("reset --hard @{upstream}"));
    }

    #[test]
    fn reset_on_a_non_repo_errors() {
        let runner = ScriptedRunner::new().on("rev-parse --is-inside-work-tree", false, "");
        let result = git(runner).reset_to_remote(dir());
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
                assert_eq!(what, "git clone");
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
