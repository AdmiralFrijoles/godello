//! Running external programs.
//!
//! Several features shell out to other programs, for example the C# build and
//! the git integration. They share this small runner so the process call can be
//! faked in tests. The runner is neutral. Each caller maps the outcome and the
//! error into its own meaning.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

/// The result of running a program to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutcome {
    /// The output with leading and trailing space removed, preferring stdout and
    /// falling back to stderr. Useful for short values like a branch name.
    pub fn trimmed_stdout(&self) -> &str {
        self.stdout.trim()
    }

    /// Both streams joined for an error message, preferring whichever has text.
    pub fn combined(&self) -> String {
        let mut text = String::new();
        if !self.stdout.trim().is_empty() {
            text.push_str(self.stdout.trim());
        }
        if !self.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(self.stderr.trim());
        }
        text
    }
}

/// Runs an external program. The real runner uses the system process. Tests use
/// a fake runner that records the call and returns a set outcome.
pub trait CommandRunner {
    fn run(
        &self,
        program: &OsStr,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<CommandOutcome, ProcessError>;
}

/// Runs a program with the system process.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        program: &OsStr,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<CommandOutcome, ProcessError> {
        let output = Command::new(program).args(args).current_dir(cwd).output();
        match output {
            Ok(output) => Ok(CommandOutcome {
                success: output.status.success(),
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(ProcessError::ProgramNotFound(program.to_os_string()))
            }
            Err(err) => Err(ProcessError::Io(err)),
        }
    }
}

/// An error from starting a program. A program that runs but exits with a
/// failure code is not an error here. That is reported in the outcome so each
/// caller can decide what a failure means.
#[derive(Debug)]
pub enum ProcessError {
    /// The program could not be found, for example it is not installed.
    ProgramNotFound(OsString),
    /// The process could not be started or its output could not be read.
    Io(std::io::Error),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessError::ProgramNotFound(program) => {
                write!(f, "could not run {}", program.to_string_lossy())
            }
            ProcessError::Io(err) => write!(f, "process error: {err}"),
        }
    }
}

impl std::error::Error for ProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProcessError::Io(err) => Some(err),
            ProcessError::ProgramNotFound(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_runner_reports_a_missing_program() {
        let runner = SystemCommandRunner;
        let result = runner.run(
            OsStr::new("godello-no-such-program-xyz"),
            &[OsString::from("--version")],
            Path::new("."),
        );
        assert!(matches!(result, Err(ProcessError::ProgramNotFound(_))));
    }

    #[test]
    fn combined_prefers_both_streams() {
        let outcome = CommandOutcome {
            success: false,
            code: Some(1),
            stdout: "out".to_string(),
            stderr: "err".to_string(),
        };
        assert_eq!(outcome.combined(), "out\nerr");
        assert_eq!(outcome.trimmed_stdout(), "out");
    }

    #[test]
    fn combined_uses_only_what_has_content() {
        let outcome = CommandOutcome {
            success: true,
            code: Some(0),
            stdout: "  branch-name\n".to_string(),
            stderr: String::new(),
        };
        assert_eq!(outcome.combined(), "branch-name");
        assert_eq!(outcome.trimmed_stdout(), "branch-name");
    }
}
