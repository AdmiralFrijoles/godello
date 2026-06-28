//! Building C# solutions before launching.
//!
//! A C# project must have its solution built before the editor opens, so the
//! editor has the assemblies ready. Two tools can do this. The default is the
//! Godot editor itself, run for the project version with --path, then
//! --build-solutions, then --quit. The other is the dotnet command, run as
//! dotnet build on the solution or project file. The choice is a setting.
//!
//! The process call goes through the CommandRunner trait so the build logic is
//! tested with a fake runner. A real runner using the system process is also
//! provided. Whether to build at all is decided by the caller from the project
//! and the settings.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::process::{CommandOutcome, CommandRunner, ProcessError};

/// Which tool builds the C# solutions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CsharpBuildTool {
    /// Build with the Godot editor using --build-solutions. The default.
    #[default]
    Godot,
    /// Build with the dotnet command.
    Dotnet,
}

impl CsharpBuildTool {
    /// The short token used in settings.
    pub fn as_str(self) -> &'static str {
        match self {
            CsharpBuildTool::Godot => "godot",
            CsharpBuildTool::Dotnet => "dotnet",
        }
    }
}

impl std::fmt::Display for CsharpBuildTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CsharpBuildTool {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "godot" | "editor" => Ok(CsharpBuildTool::Godot),
            "dotnet" => Ok(CsharpBuildTool::Dotnet),
            _ => Err(()),
        }
    }
}

/// The build tool to start with on a fresh install, picked from what the system
/// has. When dotnet is on the PATH it is the default, since a developer with
/// dotnet usually prefers it. Otherwise the Godot editor builds the solutions,
/// which needs nothing extra installed.
pub fn default_build_tool() -> CsharpBuildTool {
    if dotnet_on_path() {
        CsharpBuildTool::Dotnet
    } else {
        CsharpBuildTool::Godot
    }
}

/// True when a dotnet executable can be found on the PATH.
pub fn dotnet_on_path() -> bool {
    match std::env::var_os("PATH") {
        Some(path) => dotnet_in(&path, |candidate| candidate.is_file()),
        None => false,
    }
}

/// The PATH scan behind dotnet_on_path, with the file check passed in so it can
/// be tested without touching the real filesystem. True when any folder on the
/// path holds a dotnet executable.
fn dotnet_in(path: &OsStr, is_exe: impl Fn(&Path) -> bool) -> bool {
    let names: &[&str] = if cfg!(windows) {
        &["dotnet.exe"]
    } else {
        &["dotnet"]
    };
    std::env::split_paths(path).any(|dir| names.iter().any(|name| is_exe(&dir.join(name))))
}

/// Build the C# solutions for a project with the chosen tool. The editor should
/// be the Mono build for the project's version. It is used by the Godot tool and
/// ignored by the dotnet tool. Blocks until the build finishes.
pub fn build_solutions(
    tool: CsharpBuildTool,
    editor: &Path,
    project_dir: &Path,
    runner: &impl CommandRunner,
) -> Result<(), CsharpError> {
    match tool {
        CsharpBuildTool::Godot => run_godot_build(editor, project_dir, runner),
        CsharpBuildTool::Dotnet => run_dotnet_build(project_dir, runner),
    }
}

/// Build with the Godot editor. The build flag implies editor mode and needs a
/// valid project, so quit is added so it exits.
fn run_godot_build(
    editor: &Path,
    project_dir: &Path,
    runner: &impl CommandRunner,
) -> Result<(), CsharpError> {
    let args = vec![
        OsString::from("--path"),
        project_dir.as_os_str().to_os_string(),
        OsString::from("--build-solutions"),
        OsString::from("--quit"),
        OsString::from("--quiet"),
        OsString::from("--no-header"),
        OsString::from("--headless"),
    ];
    let outcome = runner.run(editor.as_os_str(), &args, project_dir)?;
    finish(outcome)
}

/// Build with the dotnet command. The solution or project file is passed so the
/// command is unambiguous. When there is no such file there is nothing to build.
fn run_dotnet_build(project_dir: &Path, runner: &impl CommandRunner) -> Result<(), CsharpError> {
    let Some(target) = dotnet_target(project_dir) else {
        return Ok(());
    };
    let args = vec![
        OsString::from("build"),
        target.into_os_string(),
        OsString::from("--property"),
        OsString::from("WarningLevel=0"),
    ];
    let outcome = runner.run(OsStr::new("dotnet"), &args, project_dir)?;
    finish(outcome)
}

/// Find the build target for dotnet, preferring a solution over a project file.
fn dotnet_target(project_dir: &Path) -> Option<PathBuf> {
    let mut solutions = Vec::new();
    let mut projects = Vec::new();
    if let Ok(entries) = fs::read_dir(project_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase());
            match extension.as_deref() {
                Some("sln") => solutions.push(path),
                Some("csproj") => projects.push(path),
                _ => {}
            }
        }
    }
    solutions.sort();
    projects.sort();
    solutions
        .into_iter()
        .next()
        .or_else(|| projects.into_iter().next())
}

fn finish(outcome: CommandOutcome) -> Result<(), CsharpError> {
    if outcome.success {
        Ok(())
    } else {
        Err(CsharpError::BuildFailed {
            code: outcome.code,
            output: outcome.combined(),
        })
    }
}

/// An error from building C# solutions.
#[derive(Debug)]
pub enum CsharpError {
    /// The build program could not be run, for example dotnet is not installed.
    ProgramNotFound(OsString),
    /// The build process could not be started or read.
    Io(std::io::Error),
    /// The build ran but failed.
    BuildFailed { code: Option<i32>, output: String },
}

impl From<ProcessError> for CsharpError {
    fn from(err: ProcessError) -> Self {
        match err {
            ProcessError::ProgramNotFound(program) => CsharpError::ProgramNotFound(program),
            ProcessError::Io(err) => CsharpError::Io(err),
        }
    }
}

impl std::fmt::Display for CsharpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CsharpError::ProgramNotFound(program) => {
                write!(f, "could not run {}", program.to_string_lossy())
            }
            CsharpError::Io(err) => write!(f, "build process error: {err}"),
            CsharpError::BuildFailed { code, output } => {
                let code = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                if output.is_empty() {
                    write!(f, "C# build failed with exit code {code}")
                } else {
                    write!(f, "C# build failed with exit code {code}:\n{output}")
                }
            }
        }
    }
}

impl std::error::Error for CsharpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CsharpError::Io(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A runner that returns a set outcome and records the call.
    struct FakeRunner {
        outcome: Result<CommandOutcome, ()>,
        seen: RefCell<Option<(OsString, Vec<OsString>, PathBuf)>>,
    }

    impl FakeRunner {
        fn with(success: bool, code: Option<i32>, stdout: &str, stderr: &str) -> Self {
            FakeRunner {
                outcome: Ok(CommandOutcome {
                    success,
                    code,
                    stdout: stdout.to_string(),
                    stderr: stderr.to_string(),
                }),
                seen: RefCell::new(None),
            }
        }

        fn succeeding() -> Self {
            Self::with(true, Some(0), "Build succeeded", "")
        }

        fn failing() -> Self {
            Self::with(false, Some(1), "", "error CS1002: ; expected")
        }

        fn not_found() -> Self {
            FakeRunner {
                outcome: Err(()),
                seen: RefCell::new(None),
            }
        }

        fn call(&self) -> Option<(OsString, Vec<OsString>, PathBuf)> {
            self.seen.borrow().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            program: &OsStr,
            args: &[OsString],
            cwd: &Path,
        ) -> Result<CommandOutcome, ProcessError> {
            *self.seen.borrow_mut() =
                Some((program.to_os_string(), args.to_vec(), cwd.to_path_buf()));
            match &self.outcome {
                Ok(outcome) => Ok(outcome.clone()),
                Err(()) => Err(ProcessError::ProgramNotFound(program.to_os_string())),
            }
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("godello-csharp-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Build tool value.

    #[test]
    fn build_tool_parses_and_renders() {
        assert_eq!(
            "godot".parse::<CsharpBuildTool>().unwrap(),
            CsharpBuildTool::Godot
        );
        assert_eq!(
            "dotnet".parse::<CsharpBuildTool>().unwrap(),
            CsharpBuildTool::Dotnet
        );
        assert_eq!(CsharpBuildTool::Dotnet.to_string(), "dotnet");
        assert!("msbuild".parse::<CsharpBuildTool>().is_err());
        assert_eq!(CsharpBuildTool::default(), CsharpBuildTool::Godot);
    }

    // Godot tool.

    #[test]
    fn godot_build_passes_the_right_command() {
        let runner = FakeRunner::succeeding();
        let editor = Path::new("/engines/mono/4.3-stable/Godot_mono");
        let project = Path::new("/games/my-game");
        build_solutions(CsharpBuildTool::Godot, editor, project, &runner).unwrap();

        let (program, args, cwd) = runner.call().unwrap();
        assert_eq!(program, editor.as_os_str());
        assert_eq!(cwd, project);
        assert!(args.contains(&OsString::from("--build-solutions")));
        assert!(args.contains(&OsString::from("--quit")));
        let path_index = args.iter().position(|a| a == "--path").unwrap();
        assert_eq!(args[path_index + 1], project.as_os_str());
    }

    #[test]
    fn godot_build_failure_reports_code_and_output() {
        let runner = FakeRunner::failing();
        let result = build_solutions(
            CsharpBuildTool::Godot,
            Path::new("/editor"),
            Path::new("/proj"),
            &runner,
        );
        match result {
            Err(CsharpError::BuildFailed { code, output }) => {
                assert_eq!(code, Some(1));
                assert!(output.contains("CS1002"));
            }
            other => panic!("expected a build failure, got {other:?}"),
        }
    }

    #[test]
    fn godot_editor_not_found_propagates() {
        let runner = FakeRunner::not_found();
        let result = build_solutions(
            CsharpBuildTool::Godot,
            Path::new("/missing"),
            Path::new("/proj"),
            &runner,
        );
        assert!(matches!(result, Err(CsharpError::ProgramNotFound(_))));
    }

    // Dotnet tool.

    #[test]
    fn dotnet_build_uses_the_solution_file() {
        let dir = scratch("dotnet-sln");
        fs::write(dir.join("Game.sln"), b"").unwrap();
        fs::write(dir.join("Game.csproj"), b"").unwrap();
        let runner = FakeRunner::succeeding();
        build_solutions(CsharpBuildTool::Dotnet, Path::new("/unused"), &dir, &runner).unwrap();

        let (program, args, cwd) = runner.call().unwrap();
        assert_eq!(program, OsStr::new("dotnet"));
        assert_eq!(cwd, dir);
        assert_eq!(args[0], OsString::from("build"));
        // The solution wins over the project file.
        assert_eq!(args[1], dir.join("Game.sln").into_os_string());
    }

    #[test]
    fn dotnet_build_falls_back_to_the_csproj() {
        let dir = scratch("dotnet-csproj");
        fs::write(dir.join("Game.csproj"), b"").unwrap();
        let runner = FakeRunner::succeeding();
        build_solutions(CsharpBuildTool::Dotnet, Path::new("/unused"), &dir, &runner).unwrap();

        let (_, args, _) = runner.call().unwrap();
        assert_eq!(args[1], dir.join("Game.csproj").into_os_string());
    }

    #[test]
    fn dotnet_with_no_target_does_nothing() {
        let dir = scratch("dotnet-empty");
        let runner = FakeRunner::succeeding();
        let result = build_solutions(CsharpBuildTool::Dotnet, Path::new("/unused"), &dir, &runner);
        assert!(result.is_ok());
        // The runner was never called because there was nothing to build.
        assert!(runner.call().is_none());
    }

    #[test]
    fn dotnet_build_failure_reports_output() {
        let dir = scratch("dotnet-fail");
        fs::write(dir.join("Game.csproj"), b"").unwrap();
        let runner = FakeRunner::failing();
        let result = build_solutions(CsharpBuildTool::Dotnet, Path::new("/unused"), &dir, &runner);
        assert!(matches!(result, Err(CsharpError::BuildFailed { .. })));
    }

    #[test]
    fn dotnet_not_installed_propagates() {
        let dir = scratch("dotnet-missing");
        fs::write(dir.join("Game.csproj"), b"").unwrap();
        let runner = FakeRunner::not_found();
        let result = build_solutions(CsharpBuildTool::Dotnet, Path::new("/unused"), &dir, &runner);
        assert!(matches!(result, Err(CsharpError::ProgramNotFound(_))));
    }

    // Messages.

    #[test]
    fn build_failed_message_includes_output() {
        let err = CsharpError::BuildFailed {
            code: Some(1),
            output: "error CS1002".to_string(),
        };
        let text = err.to_string();
        assert!(text.contains("exit code 1"));
        assert!(text.contains("CS1002"));
    }

    // Dotnet detection.

    fn exe_name() -> &'static str {
        if cfg!(windows) { "dotnet.exe" } else { "dotnet" }
    }

    #[test]
    fn dotnet_in_finds_it_in_a_listed_folder() {
        let path = std::env::join_paths(["/opt/bin", "/usr/share/dotnet"]).unwrap();
        let wanted = PathBuf::from("/usr/share/dotnet").join(exe_name());
        assert!(dotnet_in(&path, |candidate| candidate == wanted));
    }

    #[test]
    fn dotnet_in_is_false_when_no_folder_has_it() {
        // The file check never matches, so no folder on the path qualifies.
        let path = std::env::join_paths(["/opt/bin", "/usr/bin"]).unwrap();
        assert!(!dotnet_in(&path, |_| false));
    }

    #[test]
    fn dotnet_in_checks_only_the_dotnet_name() {
        // A folder is on the path, but it holds some other tool, not dotnet.
        let path = std::env::join_paths(["/opt/bin"]).unwrap();
        let other = PathBuf::from("/opt/bin").join("node");
        assert!(!dotnet_in(&path, |candidate| candidate == other));
    }
}
