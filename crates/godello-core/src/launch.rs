//! Launching the editor and running projects.
//!
//! This ties the other modules together. It resolves the engine a project needs
//! to an installed version, builds the C# solution first when needed, and then
//! starts the editor. It can also open the editor for a version with no project,
//! which shows the project manager.
//!
//! Starting the editor goes through the Launcher trait so the logic is tested
//! with a fake. By default the command stays attached and waits for the editor
//! to close. The launch_detached setting can turn that on so a launch returns
//! right away instead.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use crate::Settings;
use crate::csharp::{self, CsharpError};
use crate::install::{InstallError, InstallManager};
use crate::process::CommandRunner;
use crate::project::GodotProject;
use crate::version::{GodotVersion, Variant, VersionPattern};

/// Starts a program. When detached the call returns right away. When not, it
/// waits for the program to exit.
pub trait Launcher {
    fn launch(&self, program: &OsStr, args: &[OsString], detached: bool)
    -> Result<(), LaunchError>;
}

/// Starts a program with the system process.
pub struct SystemLauncher;

impl Launcher for SystemLauncher {
    fn launch(
        &self,
        program: &OsStr,
        args: &[OsString],
        detached: bool,
    ) -> Result<(), LaunchError> {
        let mut command = Command::new(program);
        command.args(args);
        // Detached spawns and returns. Attached waits for the editor to close.
        let result = if detached {
            command.spawn().map(|_child| ())
        } else {
            command.status().map(|_status| ())
        };
        match result {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(LaunchError::ProgramNotFound(program.to_os_string()))
            }
            Err(err) => Err(LaunchError::Spawn(err)),
        }
    }
}

/// The engine a project needs, resolved against what is installed. The variant
/// comes from whether the project uses C#. The version comes from the pin, then
/// the feature hint, and falls back to the newest installed build of that
/// variant when the project names nothing.
pub fn engine_for_project(
    manager: &InstallManager,
    project: &GodotProject,
) -> Result<(GodotVersion, Variant), LaunchError> {
    let variant = project_variant(project);
    let installed: Vec<GodotVersion> = manager
        .list_installed()?
        .into_iter()
        .filter(|engine| engine.variant == variant)
        .map(|engine| engine.version)
        .collect();

    let wanted = project.pinned_version.or(project.feature_version);
    let chosen = match wanted {
        Some(pattern) => pattern.best_match(&installed),
        None => installed.iter().copied().max(),
    };
    let version = chosen.ok_or(LaunchError::NotInstalled {
        pattern: wanted,
        variant,
    })?;
    Ok((version, variant))
}

fn project_variant(project: &GodotProject) -> Variant {
    if project.uses_csharp {
        Variant::Mono
    } else {
        Variant::Standard
    }
}

/// Open the editor for a project. Builds the C# solution first when the project
/// uses C# and the setting is on.
pub fn open_editor(
    manager: &InstallManager,
    settings: &Settings,
    project: &GodotProject,
    runner: &impl CommandRunner,
    launcher: &impl Launcher,
) -> Result<(), LaunchError> {
    let (version, variant) = engine_for_project(manager, project)?;
    let editor = manager.executable(variant, version, false)?;
    maybe_build_csharp(settings, project, &editor, runner)?;
    let args = vec![
        OsString::from("--path"),
        project.dir.as_os_str().to_os_string(),
        OsString::from("--editor"),
    ];
    launcher.launch(editor.as_os_str(), &args, settings.launch_detached)
}

/// Run a project without opening the editor. A C# project is still built first so
/// its assemblies are ready.
pub fn run_project(
    manager: &InstallManager,
    settings: &Settings,
    project: &GodotProject,
    runner: &impl CommandRunner,
    launcher: &impl Launcher,
) -> Result<(), LaunchError> {
    let (version, variant) = engine_for_project(manager, project)?;
    let editor = manager.executable(variant, version, false)?;
    maybe_build_csharp(settings, project, &editor, runner)?;
    let args = vec![
        OsString::from("--path"),
        project.dir.as_os_str().to_os_string(),
    ];
    launcher.launch(editor.as_os_str(), &args, settings.launch_detached)
}

/// Open the editor for a version with no project. This shows the project manager
/// for that engine.
pub fn open_version(
    manager: &InstallManager,
    version: GodotVersion,
    variant: Variant,
    detached: bool,
    launcher: &impl Launcher,
) -> Result<(), LaunchError> {
    let editor = manager.executable(variant, version, false)?;
    let args = vec![OsString::from("--project-manager")];
    launcher.launch(editor.as_os_str(), &args, detached)
}

fn maybe_build_csharp(
    settings: &Settings,
    project: &GodotProject,
    editor: &Path,
    runner: &impl CommandRunner,
) -> Result<(), LaunchError> {
    if settings.build_csharp_before_launch && project.uses_csharp {
        csharp::build_solutions(settings.csharp_build_tool, editor, &project.dir, runner)?;
    }
    Ok(())
}

/// An error from launching.
#[derive(Debug)]
pub enum LaunchError {
    /// No installed engine matches what the project needs. The caller can offer
    /// to install it. The pattern is None when the project named no version.
    NotInstalled {
        pattern: Option<VersionPattern>,
        variant: Variant,
    },
    /// A problem reading installs or finding the engine executable.
    Engine(InstallError),
    /// The C# build failed.
    Csharp(CsharpError),
    /// The editor program could not be found.
    ProgramNotFound(OsString),
    /// The editor process could not be started.
    Spawn(std::io::Error),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::NotInstalled { pattern, variant } => match pattern {
                Some(pattern) => write!(f, "no installed {variant} engine matches {pattern}"),
                None => write!(f, "no {variant} engine is installed"),
            },
            LaunchError::Engine(err) => write!(f, "{err}"),
            LaunchError::Csharp(err) => write!(f, "{err}"),
            LaunchError::ProgramNotFound(program) => {
                write!(f, "could not run the editor {}", program.to_string_lossy())
            }
            LaunchError::Spawn(err) => write!(f, "could not start the editor: {err}"),
        }
    }
}

impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LaunchError::Engine(err) => Some(err),
            LaunchError::Csharp(err) => Some(err),
            LaunchError::Spawn(err) => Some(err),
            _ => None,
        }
    }
}

impl From<InstallError> for LaunchError {
    fn from(err: InstallError) -> Self {
        LaunchError::Engine(err)
    }
}

impl From<CsharpError> for LaunchError {
    fn from(err: CsharpError) -> Self {
        LaunchError::Csharp(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Stage;
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("godello-launch-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stable(major: u32, minor: u32, patch: u32) -> GodotVersion {
        GodotVersion::new(major, minor, patch, Stage::Stable)
    }

    /// Create an installed engine on disk with a fake binary, so executable
    /// resolution works. Returns the engines root for an InstallManager.
    fn install_engine(root: &Path, variant: Variant, version: GodotVersion) {
        let dir = root.join(variant.as_str()).join(version.to_tag());
        fs::create_dir_all(&dir).unwrap();
        // A name the platform resolver will accept on any host.
        let binary = if cfg!(windows) {
            "Godot.exe"
        } else if cfg!(target_os = "macos") {
            // Build a minimal app bundle.
            let macos = dir.join("Godot.app").join("Contents").join("MacOS");
            fs::create_dir_all(&macos).unwrap();
            fs::write(macos.join("Godot"), b"").unwrap();
            return;
        } else {
            "Godot_v4.3-stable_linux.x86_64"
        };
        fs::write(dir.join(binary), b"").unwrap();
    }

    fn write_project(dir: &Path, body: &str) -> GodotProject {
        fs::write(dir.join("project.godot"), body).unwrap();
        GodotProject::load(dir).unwrap()
    }

    /// A launcher that records what was started and whether it was detached.
    struct FakeLauncher {
        calls: RefCell<Vec<(OsString, Vec<OsString>, bool)>>,
    }

    impl FakeLauncher {
        fn new() -> Self {
            FakeLauncher {
                calls: RefCell::new(Vec::new()),
            }
        }

        fn last(&self) -> (OsString, Vec<OsString>, bool) {
            self.calls.borrow().last().cloned().unwrap()
        }

        fn count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(
            &self,
            program: &OsStr,
            args: &[OsString],
            detached: bool,
        ) -> Result<(), LaunchError> {
            self.calls
                .borrow_mut()
                .push((program.to_os_string(), args.to_vec(), detached));
            Ok(())
        }
    }

    /// A build runner that records whether it ran and can be set to fail.
    struct FakeRunner {
        ran: RefCell<bool>,
        fail: bool,
    }

    impl FakeRunner {
        fn new() -> Self {
            FakeRunner {
                ran: RefCell::new(false),
                fail: false,
            }
        }

        fn failing() -> Self {
            FakeRunner {
                ran: RefCell::new(false),
                fail: true,
            }
        }

        fn ran(&self) -> bool {
            *self.ran.borrow()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            _program: &OsStr,
            _args: &[OsString],
            _cwd: &Path,
        ) -> Result<crate::process::CommandOutcome, crate::process::ProcessError> {
            *self.ran.borrow_mut() = true;
            Ok(crate::process::CommandOutcome {
                success: !self.fail,
                code: Some(if self.fail { 1 } else { 0 }),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    const PLAIN: &str = "config_version=5\n[application]\nconfig/name=\"X\"\nconfig/features=PackedStringArray(\"4.3\")\n";
    const CSHARP: &str = "config_version=5\n[application]\nconfig/name=\"X\"\nconfig/features=PackedStringArray(\"4.3\", \"C#\")\n";

    #[test]
    fn resolves_the_best_installed_match_for_a_pin() {
        let root = scratch("resolve-pin");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        install_engine(&root, Variant::Standard, stable(4, 3, 1));
        install_engine(&root, Variant::Standard, stable(4, 2, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("resolve-pin-proj");
        let project = write_project(&proj_dir, "config_version=5\n[godello]\nversion=\"4.3\"\n");
        let (version, variant) = engine_for_project(&manager, &project).unwrap();
        assert_eq!(version, stable(4, 3, 1));
        assert_eq!(variant, Variant::Standard);
    }

    #[test]
    fn falls_back_to_newest_when_project_names_nothing() {
        let root = scratch("resolve-newest");
        install_engine(&root, Variant::Standard, stable(4, 2, 0));
        install_engine(&root, Variant::Standard, stable(4, 4, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("resolve-newest-proj");
        let project = write_project(
            &proj_dir,
            "config_version=5\n[application]\nconfig/name=\"X\"\n",
        );
        let (version, _) = engine_for_project(&manager, &project).unwrap();
        assert_eq!(version, stable(4, 4, 0));
    }

    #[test]
    fn csharp_project_resolves_only_mono_installs() {
        let root = scratch("resolve-mono");
        // A standard 4.3 is installed, but the project needs mono.
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("resolve-mono-proj");
        let project = write_project(&proj_dir, CSHARP);
        let result = engine_for_project(&manager, &project);
        match result {
            Err(LaunchError::NotInstalled { variant, .. }) => {
                assert_eq!(variant, Variant::Mono);
            }
            other => panic!("expected not installed, got {other:?}"),
        }
    }

    #[test]
    fn not_installed_carries_the_request() {
        let root = scratch("resolve-missing");
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("resolve-missing-proj");
        let project = write_project(&proj_dir, "config_version=5\n[godello]\nversion=\"4.3\"\n");
        match engine_for_project(&manager, &project) {
            Err(LaunchError::NotInstalled { pattern, variant }) => {
                assert_eq!(pattern, Some("4.3".parse().unwrap()));
                assert_eq!(variant, Variant::Standard);
            }
            other => panic!("expected not installed, got {other:?}"),
        }
    }

    #[test]
    fn open_editor_spawns_with_path_and_editor() {
        let root = scratch("open-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("open-proj");
        let project = write_project(&proj_dir, PLAIN);
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        open_editor(&manager, &Settings::default(), &project, &runner, &launcher).unwrap();

        let (_program, args, detached) = launcher.last();
        assert!(args.contains(&OsString::from("--editor")));
        let path_index = args.iter().position(|a| a == "--path").unwrap();
        assert_eq!(args[path_index + 1], proj_dir.as_os_str());
        // Attached is the default.
        assert!(!detached);
        // A plain project does not trigger a build.
        assert!(!runner.ran());
    }

    #[test]
    fn run_project_spawns_without_editor() {
        let root = scratch("run-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("run-proj");
        let project = write_project(&proj_dir, PLAIN);
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        run_project(&manager, &Settings::default(), &project, &runner, &launcher).unwrap();

        let (_program, args, _detached) = launcher.last();
        assert!(args.contains(&OsString::from("--path")));
        assert!(!args.contains(&OsString::from("--editor")));
    }

    #[test]
    fn open_version_spawns_the_project_manager() {
        let root = scratch("pm-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let launcher = FakeLauncher::new();
        open_version(
            &manager,
            stable(4, 3, 0),
            Variant::Standard,
            true,
            &launcher,
        )
        .unwrap();
        let (_program, args, detached) = launcher.last();
        assert_eq!(args, vec![OsString::from("--project-manager")]);
        assert!(detached);
    }

    #[test]
    fn launch_respects_the_detached_setting() {
        let root = scratch("detached-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("detached-proj");
        let project = write_project(&proj_dir, PLAIN);
        let runner = FakeRunner::new();

        let mut settings = Settings::default();
        settings.launch_detached = true;
        let launcher = FakeLauncher::new();
        open_editor(&manager, &settings, &project, &runner, &launcher).unwrap();
        let (_program, _args, detached) = launcher.last();
        assert!(detached, "the setting should make the launch detached");
    }

    #[test]
    fn csharp_project_builds_before_opening() {
        let root = scratch("cs-build-root");
        install_engine(&root, Variant::Mono, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("cs-build-proj");
        let project = write_project(&proj_dir, CSHARP);
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        open_editor(&manager, &Settings::default(), &project, &runner, &launcher).unwrap();
        assert!(runner.ran(), "the C# build should have run");
        assert_eq!(launcher.count(), 1, "the editor should still launch");
    }

    #[test]
    fn build_disabled_setting_skips_the_build() {
        let root = scratch("cs-nobuild-root");
        install_engine(&root, Variant::Mono, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("cs-nobuild-proj");
        let project = write_project(&proj_dir, CSHARP);
        let mut settings = Settings::default();
        settings.build_csharp_before_launch = false;
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        open_editor(&manager, &settings, &project, &runner, &launcher).unwrap();
        assert!(!runner.ran(), "the build should be skipped");
        assert_eq!(launcher.count(), 1);
    }

    #[test]
    fn a_build_failure_stops_the_launch() {
        let root = scratch("cs-fail-root");
        install_engine(&root, Variant::Mono, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("cs-fail-proj");
        let project = write_project(&proj_dir, CSHARP);
        let runner = FakeRunner::failing();
        let launcher = FakeLauncher::new();
        let result = open_editor(&manager, &Settings::default(), &project, &runner, &launcher);
        assert!(matches!(result, Err(LaunchError::Csharp(_))));
        // The editor must not start when the build failed.
        assert_eq!(launcher.count(), 0);
    }
}
