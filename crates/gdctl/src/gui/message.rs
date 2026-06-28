//! The messages that drive the GUI update loop.
//!
//! Every user action and every async result arrives as one of these. Async
//! results carry a String error rather than a core error type, because the core
//! errors are not Clone and an iced message must be Clone. We map the error to a
//! string at the task boundary.

use std::path::PathBuf;

use godello_core::{
    CsharpBuildTool, GodotVersion, ProjectEntry, Release, RepoStatus, UpdateOutcome, Variant,
};
use iced::Theme;

use crate::gui::progress::ProgressEvent;
use crate::gui::state::{Channel, EnginesTab, PinChoice, Screen};

/// Why a launch failed, so the GUI can treat a C# build failure specially. The
/// editor can still open without building, but other failures cannot be worked
/// around here.
#[derive(Debug, Clone)]
pub enum LaunchFailure {
    /// The C# build failed before the editor or project could start.
    Compile(String),
    /// Any other failure.
    Other(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Switch the visible screen.
    Navigate(Screen),
    /// Pick a theme.
    SetTheme(Theme),
    /// Copy a path to the clipboard.
    CopyPath(String),

    /// Dismiss a toast by id, for a click.
    DismissToast(u64),
    /// A periodic tick that ages and dismisses toasts.
    ToastTick,
    /// The pointer entered or left a toast, which pauses or resumes auto dismiss.
    HoverToasts(bool),

    /// Re read the installed engines from disk. This is a fast local scan, so it
    /// runs inline in the update step rather than as a task.
    RefreshInstalled,
    /// Fetch the list of versions available to install. When force is set it
    /// skips the cache and always hits the network, for an explicit refresh.
    LoadRemote { force: bool },
    /// The remote list finished loading.
    RemoteLoaded(Result<Vec<Release>, String>),
    /// Clear the cached version list.
    ClearCache,

    // Settings.
    /// Open a folder picker to choose where engines are installed.
    ChooseEngineDir,
    /// The engine install folder was picked, or the picker was cancelled.
    EngineDirPicked(Option<PathBuf>),
    /// Reset the engine install folder to the default.
    ResetEngineDir,
    /// Open a folder picker to choose the default project folder.
    ChooseProjectDir,
    /// The default project folder was picked, or the picker was cancelled.
    ProjectDirPicked(Option<PathBuf>),
    /// Clear the default project folder.
    ResetProjectDir,
    /// Set the variant used when nothing else says.
    SetDefaultVariant(Variant),
    /// Set whether prereleases are included by default.
    SetIncludePrereleases(bool),
    /// Set whether the C# solution is built before a launch.
    SetBuildCsharp(bool),
    /// Set which tool builds the C# solution.
    SetCsharpBuildTool(CsharpBuildTool),

    /// Switch between the installed and available lists.
    SetEnginesTab(EnginesTab),
    /// Switch the available list between released and prerelease channels.
    SetChannel(Channel),
    /// Type into the engines search box.
    FilterChanged(String),

    /// Open or close the row menu for an installed engine.
    ToggleEngineMenu {
        variant: Variant,
        version: GodotVersion,
    },
    /// Close any open row menu, for example on an outside click.
    CloseEngineMenu,
    /// Open the install folder in the system file manager.
    OpenFolder {
        variant: Variant,
        version: GodotVersion,
    },
    /// Open the editor for a version with no project, the project manager window.
    OpenProjectManager {
        variant: Variant,
        version: GodotVersion,
    },
    /// Ask to remove an installed engine. Opens the confirmation dialog.
    RequestRemove {
        variant: Variant,
        version: GodotVersion,
    },
    /// Dismiss the remove confirmation without removing.
    CancelRemove,
    /// Remove an installed engine. This is a local delete, so it runs inline. It
    /// is the confirmed action from the dialog.
    Remove {
        variant: Variant,
        version: GodotVersion,
    },

    /// Start installing the given version and variant.
    Install {
        variant: Variant,
        version: GodotVersion,
    },
    /// Cancel an install in flight.
    CancelInstall {
        variant: Variant,
        version: GodotVersion,
    },
    /// One progress event from an install in flight.
    InstallProgress {
        variant: Variant,
        version: GodotVersion,
        event: ProgressEvent,
    },
    /// An install finished, with success or an error. This is the single source
    /// of truth for an install being over. Progress events are only cosmetic.
    Installed {
        variant: Variant,
        version: GodotVersion,
        result: Result<(), String>,
    },

    // Projects.
    /// Open the native file picker to add a project by its project.godot file.
    AddProject,
    /// The file picker returned, with the chosen project.godot file or nothing.
    ProjectFilePicked(Option<PathBuf>),
    /// Forget a project, removing it from the list but not from disk.
    RemoveProject(PathBuf),
    /// Open a project folder in the file manager.
    OpenProjectFolder(PathBuf),
    /// Open or close the row menu for a project.
    ToggleProjectMenu(PathBuf),
    /// Close any open project row menu.
    CloseProjectMenu,
    /// Open the editor (run false) or run the project (run true).
    LaunchProject { dir: PathBuf, run: bool },
    /// Any C# build is done and the editor or project is about to start. Moves the
    /// project row from compiling to starting.
    LaunchStarting { dir: PathBuf, run: bool },
    /// A launch finished, with success or a failure. Carries the project and what
    /// was being done so the row can stop working and a C# build failure on an
    /// edit can offer to open the editor anyway.
    LaunchFinished {
        dir: PathBuf,
        run: bool,
        result: Result<(), LaunchFailure>,
    },
    /// Open the editor without building the C# solution first, after the user
    /// chose to edit anyway past a build failure.
    EditAnyway,
    /// Dismiss the build failure dialog without opening the editor.
    CancelCompileWarning,
    /// The resolve for an offered install finished. On success it raises the
    /// install offer dialog, carrying the launch to resume.
    OfferResolved {
        dir: PathBuf,
        run: bool,
        variant: Variant,
        result: Result<GodotVersion, String>,
    },
    /// Accept the offered install.
    AcceptOffer,
    /// Dismiss the offered install.
    DismissOffer,

    // Pinning.
    /// Open the pin editor for a project.
    OpenPinEditor(PathBuf),
    /// Choose a version in the pin dropdown.
    PinSelected(PinChoice),
    /// Save the pin.
    SavePin,
    /// Close the pin editor without saving.
    CancelPin,

    // Version control.
    /// Re check every project on a timer, so changes show up without the user
    /// doing anything. This re reads each project.godot for its version, re scans
    /// the installed engines so the engine pill is current, and rechecks the git
    /// status of each project.
    RecheckProjects,
    /// The git status of a project loaded.
    GitStatusLoaded {
        dir: PathBuf,
        status: Option<RepoStatus>,
    },
    /// Bring a project up to date with its remote. When there are local changes
    /// this raises a warning to confirm first, otherwise it updates right away.
    UpdateProject(PathBuf),
    /// Go ahead with the update that raised a local changes warning.
    ConfirmUpdate,
    /// Dismiss the update warning without updating.
    CancelUpdate,
    /// A project update finished.
    ProjectUpdated {
        dir: PathBuf,
        result: Result<UpdateOutcome, String>,
    },

    // Cloning.
    /// Open the clone dialog.
    OpenCloneDialog,
    /// Type into the clone url field.
    CloneUrlChanged(String),
    /// Dismiss the clone dialog.
    CancelClone,
    /// Open the folder picker to choose where the clone goes.
    ChooseCloneDir,
    /// The folder picker returned, with the chosen folder or nothing.
    CloneDirPicked(Option<PathBuf>),
    /// Clone into the chosen folder using the entered url.
    StartClone,
    /// A clone finished, with the added project entry or an error.
    Cloned(Result<Option<ProjectEntry>, String>),
}
