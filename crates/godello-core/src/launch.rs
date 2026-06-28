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
use crate::platform::{Os, current_os};
use crate::process::{CommandRunner, ProcessError};
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
            // Fully cut the launched program loose so it lives on its own. Without
            // this it shares our process group and standard streams, so when the
            // launcher exits a terminal hangup or a group signal can also close the
            // editor. See detach for the per platform details.
            detach(&mut command);
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

/// Set a command up so the spawned program is independent of this process.
///
/// On Unix it goes into its own process group, so a signal aimed at our group,
/// such as a terminal hangup when the launcher exits, does not reach it. On
/// Windows it gets its own process group and no console. On both, its standard
/// streams are detached from ours so closing the launcher cannot disturb it and
/// its output does not mix into ours.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    command.process_group(0);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    // DETACHED_PROCESS detaches from our console, CREATE_NEW_PROCESS_GROUP gives
    // it its own group so a group signal to us does not reach it.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut Command) {}

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

/// A step a launch passes through after any C# build, so a caller can show what
/// is happening. Importing only happens for a run whose resources are not ready.
/// Starting fires just before the editor or project starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPhase {
    /// Importing the project's resources before it can run.
    Importing,
    /// About to start the editor or the project, after any build and import.
    Starting,
}

/// Open the editor for a project. Builds the C# solution first when the project
/// uses C# and the setting is on.
///
/// The on_phase hook reports each step after any build, so a caller can show the
/// build and the launch as separate steps. It does not run when the build fails.
/// Opening the editor imports on its own, so this only reports the starting step.
pub fn open_editor(
    manager: &InstallManager,
    settings: &Settings,
    project: &GodotProject,
    runner: &impl CommandRunner,
    launcher: &impl Launcher,
    on_phase: impl Fn(LaunchPhase),
) -> Result<(), LaunchError> {
    let (version, variant) = engine_for_project(manager, project)?;
    let editor = manager.executable(variant, version, false)?;
    maybe_build_csharp(settings, project, &editor, runner)?;
    on_phase(LaunchPhase::Starting);
    let args = vec![
        OsString::from("--path"),
        project.dir.as_os_str().to_os_string(),
        OsString::from("--editor"),
    ];
    launcher.launch(editor.as_os_str(), &args, settings.launch_detached)
}

/// Run a project without opening the editor. A C# project is still built first so
/// its assemblies are ready.
///
/// The on_phase hook reports each step after any build. A project whose resources
/// are not imported yet reports the importing step while that runs, then the
/// starting step just before the project starts, the same as in open_editor.
pub fn run_project(
    manager: &InstallManager,
    settings: &Settings,
    project: &GodotProject,
    runner: &impl CommandRunner,
    launcher: &impl Launcher,
    on_phase: impl Fn(LaunchPhase),
) -> Result<(), LaunchError> {
    let (version, variant) = engine_for_project(manager, project)?;
    let editor = manager.executable(variant, version, false)?;
    maybe_build_csharp(settings, project, &editor, runner)?;
    // A project that was never opened in the editor has no imported resources, so
    // running it would fail. Import them first and wait for that to finish. This
    // runs after any C# build, which opens the editor and so imports on its own,
    // in which case there is nothing left to do here.
    if !project.is_imported() {
        on_phase(LaunchPhase::Importing);
        import_project(project, &editor, runner)?;
    }
    on_phase(LaunchPhase::Starting);
    let args = vec![
        OsString::from("--path"),
        project.dir.as_os_str().to_os_string(),
    ];
    launcher.launch(editor.as_os_str(), &args, settings.launch_detached)
}

/// Import a project's resources. This opens the editor headless with the import
/// flag, which imports and then exits on its own, and waits for it through the
/// command runner so the run that follows has its resources ready. The caller
/// imports only when the project needs it.
fn import_project(
    project: &GodotProject,
    editor: &Path,
    runner: &impl CommandRunner,
) -> Result<(), LaunchError> {
    let args = vec![
        OsString::from("--path"),
        project.dir.as_os_str().to_os_string(),
        OsString::from("--import"),
        OsString::from("--headless"),
    ];
    let outcome = runner
        .run(editor.as_os_str(), &args, &project.dir)
        .map_err(LaunchError::Import)?;
    if outcome.success {
        Ok(())
    } else {
        Err(LaunchError::ImportFailed {
            code: outcome.code,
            output: outcome.combined(),
        })
    }
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

/// The system program that opens a path in the file manager, per host.
pub fn file_manager_program(os: Os) -> &'static str {
    match os {
        Os::Linux => "xdg-open",
        Os::Mac => "open",
        Os::Windows => "explorer",
    }
}

/// Open a path in the system file manager. Used to reveal an install on disk. It
/// always runs detached, since the file manager is its own window.
pub fn open_path(path: &Path, launcher: &impl Launcher) -> Result<(), LaunchError> {
    let program = file_manager_program(current_os());
    let args = vec![path.as_os_str().to_os_string()];
    launcher.launch(OsStr::new(program), &args, true)
}

/// Build the C# solution when the project needs it and the setting is on. The
/// build always runs through the command runner, which waits for it to finish,
/// so it is never detached. The detached setting only applies to the launch that
/// follows, so a build always completes before a detached editor starts.
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
    /// The import step could not be started.
    Import(ProcessError),
    /// The import step ran but failed.
    ImportFailed { code: Option<i32>, output: String },
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
            LaunchError::Import(err) => write!(f, "could not import the project: {err}"),
            LaunchError::ImportFailed { code, output } => {
                let code = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                if output.is_empty() {
                    write!(f, "the project import failed with exit code {code}")
                } else {
                    write!(
                        f,
                        "the project import failed with exit code {code}:\n{output}"
                    )
                }
            }
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
            LaunchError::Import(err) => Some(err),
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

    /// A build runner that records the args of each call and can be set to fail.
    struct FakeRunner {
        calls: RefCell<Vec<Vec<OsString>>>,
        fail: bool,
    }

    impl FakeRunner {
        fn new() -> Self {
            FakeRunner {
                calls: RefCell::new(Vec::new()),
                fail: false,
            }
        }

        fn failing() -> Self {
            FakeRunner {
                calls: RefCell::new(Vec::new()),
                fail: true,
            }
        }

        fn ran(&self) -> bool {
            !self.calls.borrow().is_empty()
        }

        fn last_args(&self) -> Vec<OsString> {
            self.calls.borrow().last().cloned().unwrap()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            _program: &OsStr,
            args: &[OsString],
            _cwd: &Path,
        ) -> Result<crate::process::CommandOutcome, crate::process::ProcessError> {
            self.calls.borrow_mut().push(args.to_vec());
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
        let project = write_project(
            &proj_dir,
            "config_version=5\n[godello]\npin_version=\"4.3\"\n",
        );
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
        let project = write_project(
            &proj_dir,
            "config_version=5\n[godello]\npin_version=\"4.3\"\n",
        );
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
        open_editor(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |_| {},
        )
        .unwrap();

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
        run_project(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |_| {},
        )
        .unwrap();

        let (_program, args, _detached) = launcher.last();
        assert!(args.contains(&OsString::from("--path")));
        assert!(!args.contains(&OsString::from("--editor")));
    }

    /// Write a texture import sidecar declaring one generated output, and create
    /// that output when present is true. This sets up a project that either needs
    /// an import or does not.
    fn write_import_state(dir: &Path, present: bool) {
        let output = "res://.godot/imported/icon.svg-abc123.ctex";
        let body = format!("[remap]\n\npath=\"{output}\"\n\n[deps]\n\ndest_files=[\"{output}\"]\n");
        fs::write(dir.join("icon.svg.import"), body).unwrap();
        if present {
            let imported = dir.join(".godot").join("imported");
            fs::create_dir_all(&imported).unwrap();
            fs::write(imported.join("icon.svg-abc123.ctex"), b"").unwrap();
        }
    }

    #[test]
    fn run_imports_an_unimported_project_first() {
        let root = scratch("import-needed-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("import-needed-proj");
        let project = write_project(&proj_dir, PLAIN);
        // An asset whose imported output is missing, so a run must import first.
        write_import_state(&proj_dir, false);
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        run_project(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |_| {},
        )
        .unwrap();

        // The import ran headless with the import flag pointed at the project.
        assert!(
            runner.ran(),
            "an unimported project should be imported first"
        );
        let args = runner.last_args();
        assert!(args.contains(&OsString::from("--import")));
        assert!(args.contains(&OsString::from("--headless")));
        let path_index = args.iter().position(|a| a == "--path").unwrap();
        assert_eq!(args[path_index + 1], proj_dir.as_os_str());
        // The project still launched after the import.
        assert_eq!(launcher.count(), 1);
    }

    #[test]
    fn run_skips_import_when_already_imported() {
        let root = scratch("import-skip-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("import-skip-proj");
        let project = write_project(&proj_dir, PLAIN);
        // The asset's imported output is present, so no import is needed.
        write_import_state(&proj_dir, true);
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        run_project(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |_| {},
        )
        .unwrap();

        // No import was needed, so the runner never ran.
        assert!(
            !runner.ran(),
            "an imported project should not be imported again"
        );
        assert_eq!(launcher.count(), 1);
    }

    #[test]
    fn an_import_failure_stops_the_run() {
        let root = scratch("import-fail-root");
        install_engine(&root, Variant::Standard, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("import-fail-proj");
        let project = write_project(&proj_dir, PLAIN);
        // A missing output makes the run attempt an import, which then fails.
        write_import_state(&proj_dir, false);
        let runner = FakeRunner::failing();
        let launcher = FakeLauncher::new();
        let phases = RefCell::new(Vec::new());
        let result = run_project(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |phase| {
                phases.borrow_mut().push(phase);
            },
        );
        assert!(matches!(result, Err(LaunchError::ImportFailed { .. })));
        // The project must not run when the import failed.
        assert_eq!(launcher.count(), 0);
        // The importing phase is reported, but the starting phase must not fire
        // once the import has failed.
        assert!(phases.borrow().contains(&LaunchPhase::Importing));
        assert!(!phases.borrow().contains(&LaunchPhase::Starting));
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

        let settings = Settings {
            launch_detached: true,
            ..Settings::default()
        };
        let launcher = FakeLauncher::new();
        open_editor(&manager, &settings, &project, &runner, &launcher, |_| {}).unwrap();
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
        // The hook captures whether the build had already run when it fired, so
        // this checks the order is build first, then the launch step.
        let built_before_hook = RefCell::new(false);
        open_editor(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |_| {
                *built_before_hook.borrow_mut() = runner.ran();
            },
        )
        .unwrap();
        assert!(runner.ran(), "the C# build should have run");
        assert!(
            *built_before_hook.borrow(),
            "the before launch hook should run after the build"
        );
        assert_eq!(launcher.count(), 1, "the editor should still launch");
    }

    #[test]
    fn build_disabled_setting_skips_the_build() {
        let root = scratch("cs-nobuild-root");
        install_engine(&root, Variant::Mono, stable(4, 3, 0));
        let manager = InstallManager::new(&root, root.join("dl"));
        let proj_dir = scratch("cs-nobuild-proj");
        let project = write_project(&proj_dir, CSHARP);
        let settings = Settings {
            build_csharp_before_launch: false,
            ..Settings::default()
        };
        let runner = FakeRunner::new();
        let launcher = FakeLauncher::new();
        open_editor(&manager, &settings, &project, &runner, &launcher, |_| {}).unwrap();
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
        let hook_ran = RefCell::new(false);
        let result = open_editor(
            &manager,
            &Settings::default(),
            &project,
            &runner,
            &launcher,
            |_| {
                *hook_ran.borrow_mut() = true;
            },
        );
        assert!(matches!(result, Err(LaunchError::Csharp(_))));
        // The editor must not start when the build failed.
        assert_eq!(launcher.count(), 0);
        // No phase fires when the build failed.
        assert!(!*hook_ran.borrow());
    }

    #[test]
    fn file_manager_program_matches_the_host() {
        assert_eq!(file_manager_program(Os::Linux), "xdg-open");
        assert_eq!(file_manager_program(Os::Mac), "open");
        assert_eq!(file_manager_program(Os::Windows), "explorer");
    }

    #[test]
    fn open_path_launches_the_file_manager_detached_with_the_path() {
        let launcher = FakeLauncher::new();
        let path = Path::new("/tmp/godello/engines/standard/4.3-stable");
        open_path(path, &launcher).unwrap();
        let (program, args, detached) = launcher.last();
        // The program is the host file manager, the one argument is the path, and
        // it always runs detached.
        assert_eq!(program, OsString::from(file_manager_program(current_os())));
        assert_eq!(args, vec![path.as_os_str().to_os_string()]);
        assert!(detached);
    }
}
