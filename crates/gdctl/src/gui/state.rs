//! The GUI application state.
//!
//! This holds the shared context reused from the CLI plus the cached data each
//! screen renders. It is the state type the GUI framework owns and threads
//! through update and view.

use std::collections::HashMap;
use std::path::PathBuf;

use godello_core::{
    GodotProject, GodotVersion, InstalledEngine, ProjectEntry, Release, RepoStatus, Variant,
    VersionPattern,
};
use iced::Theme;
use iced::task::Handle;

use crate::context::Context;
use crate::gui::theme;

/// Which top level screen is showing. Engines is the landing screen. Projects is
/// a placeholder until a later pass. Settings holds the theme picker for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Engines,
    Projects,
    Settings,
}

/// Which list the engines screen is showing, the engines on disk or the ones
/// available to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginesTab {
    Installed,
    Available,
}

/// Which release channel the available list is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Released,
    Prerelease,
}

/// The kind of a toast, which sets its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// A short message that floats at the bottom of the window for a while, then
/// fades on its own. It does not push content around.
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    /// Seconds left before it dismisses itself.
    pub remaining: f32,
    /// The starting time, so the view can show how much is left as a fraction.
    pub total: f32,
}

impl Toast {
    /// How much time is left, from 1.0 when fresh down to 0.0 when it expires.
    pub fn fraction(&self) -> f32 {
        if self.total > 0.0 {
            (self.remaining / self.total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// The state of something loaded from disk or the network. The view renders a
/// hint, a spinner stand in, an error, or the data, without extra flags.
#[derive(Debug, Clone, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Loaded(T),
    Failed(String),
}

/// One install in flight. Progress fills in as the download reports bytes. The
/// abort handle lets the user cancel it.
#[derive(Debug, Clone)]
pub struct InstallJob {
    pub variant: Variant,
    pub version: GodotVersion,
    /// The total size once the server reports it, or None while unknown.
    pub total: Option<u64>,
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// True once the download finished and the install (verify and extract) is
    /// running. That phase has no progress, so the view shows it as busy.
    pub installing: bool,
    /// Aborts the async work (the download) when the user cancels.
    pub abort: Handle,
    /// Asks the blocking extract to stop. The download is stopped by the abort
    /// handle, but the extract runs off the executor and watches this flag.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl InstallJob {
    /// The download fraction from 0.0 to 1.0, or None while the total is unknown.
    pub fn fraction(&self) -> Option<f32> {
        match self.total {
            Some(total) if total > 0 => Some(self.downloaded as f32 / total as f32),
            _ => None,
        }
    }
}

/// The whole GUI state.
pub struct App {
    /// Shared wiring reused from the CLI: paths, settings, and the network client.
    pub ctx: Context,
    pub screen: Screen,
    pub theme: Theme,

    /// Toasts floating at the bottom of the window, newest last.
    pub toasts: Vec<Toast>,
    /// The id for the next toast.
    pub next_toast_id: u64,
    /// True while the pointer is over a toast, which pauses the auto dismiss.
    pub toast_paused: bool,

    /// Engines found on disk.
    pub installed: Load<Vec<InstalledEngine>>,
    /// Versions available to install.
    pub remote: Load<Vec<Release>>,
    /// Which engines list is showing.
    pub engines_tab: EnginesTab,
    /// Which release channel the available list shows.
    pub channel: Channel,
    /// The filter typed into the engines search box.
    pub filter: String,
    /// The installed engine whose row menu is open, if any.
    pub menu_open: Option<(Variant, GodotVersion)>,
    /// The installed engine waiting on a remove confirmation, if any.
    pub confirm_remove: Option<(Variant, GodotVersion)>,

    /// Installs in flight, usually zero or one.
    pub jobs: Vec<InstallJob>,

    /// The added projects.
    pub projects: Load<Vec<ProjectEntry>>,
    /// The parsed project.godot per project, for the row badges. Loaded with the
    /// list. A project missing from here failed to load.
    pub project_info: HashMap<PathBuf, GodotProject>,
    /// The project whose row menu is open, if any.
    pub project_menu_open: Option<PathBuf>,
    /// The version control status per project, filled in as it loads.
    pub git_status: HashMap<PathBuf, RepoStatus>,
    /// The open pin editor, if any.
    pub pin_editor: Option<PinEditor>,
    /// A raised offer to install the engine a launch needs, if any.
    pub install_offer: Option<InstallOffer>,
    /// The open clone dialog, if any.
    pub clone_dialog: Option<CloneDialog>,
    /// The url of a clone in progress, if any, so the projects list can show it
    /// is working until the clone finishes.
    pub cloning: Option<String>,
    /// A launch to resume once an offered install finishes.
    pub pending_launch: Option<PendingLaunch>,
    /// A raised warning that an update would touch local changes, if any.
    pub update_warning: Option<UpdateWarning>,
    /// What each project row is busy doing, so it can show that work is under way
    /// instead of its launch buttons. Empty when a project is idle.
    pub project_activity: HashMap<PathBuf, ProjectActivity>,
    /// A raised offer to open the editor anyway after a C# build failed, if any.
    pub compile_warning: Option<CompileWarning>,
}

/// What a project is busy doing, so its row can show a spinner and a short label
/// in place of the launch buttons. These follow the order a launch goes through:
/// install the engine if needed, build the C# solution if the project uses it,
/// then start the editor or the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectActivity {
    /// Installing the engine the project needs before the launch can go ahead.
    InstallingEngine,
    /// Building the C# solution before the launch.
    Compiling,
    /// Starting the editor or the project, after any install and build.
    Launching { run: bool },
}

impl ProjectActivity {
    /// A short label for the row while this work is under way.
    pub fn label(self) -> &'static str {
        match self {
            ProjectActivity::InstallingEngine => "Installing engine...",
            ProjectActivity::Compiling => "Compiling C#...",
            ProjectActivity::Launching { run: true } => "Starting the project...",
            ProjectActivity::Launching { run: false } => "Opening the editor...",
        }
    }
}

/// One option in the pin dropdown: a version to pin and how to label it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinChoice {
    pub pattern: VersionPattern,
    pub label: String,
}

impl std::fmt::Display for PinChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// The editor for a project's pinned engine version. It offers the installed
/// versions to choose from, with the project's detected version suggested.
#[derive(Debug, Clone)]
pub struct PinEditor {
    pub dir: PathBuf,
    pub choices: Vec<PinChoice>,
    pub selected: Option<PinChoice>,
}

/// An offer to install the engine a launch needs, carrying enough to resume the
/// launch once the install finishes.
#[derive(Debug, Clone)]
pub struct InstallOffer {
    pub variant: Variant,
    pub version: GodotVersion,
    pub label: String,
    pub dir: PathBuf,
    pub run: bool,
}

/// The clone a repository dialog. It holds the url and the folder to clone into,
/// both chosen before the clone starts.
#[derive(Debug, Clone, Default)]
pub struct CloneDialog {
    pub url: String,
    pub dest: Option<PathBuf>,
}

/// A launch waiting on an install to finish.
#[derive(Debug, Clone)]
pub struct PendingLaunch {
    pub dir: PathBuf,
    pub run: bool,
}

/// A pending update that the user must confirm because the working copy has local
/// changes. It carries what the update needs so it can run once confirmed.
#[derive(Debug, Clone)]
pub struct UpdateWarning {
    pub dir: PathBuf,
    pub main_branch: String,
}

/// A raised offer to open the editor anyway after the C# build failed. Only edits
/// raise this, since running needs the build. It carries the project to open and
/// the build error to show.
#[derive(Debug, Clone)]
pub struct CompileWarning {
    pub dir: PathBuf,
    pub error: String,
}

impl App {
    /// Build the starting state from the shared context. The theme comes from the
    /// saved settings and the available list opens on the released channel.
    pub fn new(ctx: Context) -> Self {
        let theme = theme::by_name(&ctx.settings.theme);
        App {
            ctx,
            screen: Screen::Projects,
            theme,
            toasts: Vec::new(),
            next_toast_id: 0,
            toast_paused: false,
            installed: Load::Idle,
            remote: Load::Idle,
            engines_tab: EnginesTab::Installed,
            channel: Channel::Released,
            filter: String::new(),
            menu_open: None,
            confirm_remove: None,
            jobs: Vec::new(),
            projects: Load::Idle,
            project_info: HashMap::new(),
            project_menu_open: None,
            git_status: HashMap::new(),
            pin_editor: None,
            install_offer: None,
            clone_dialog: None,
            cloning: None,
            pending_launch: None,
            update_warning: None,
            project_activity: HashMap::new(),
            compile_warning: None,
        }
    }

    /// Find the in flight job for a version and variant, if any.
    pub fn job_mut(&mut self, variant: Variant, version: GodotVersion) -> Option<&mut InstallJob> {
        self.jobs
            .iter_mut()
            .find(|job| job.variant == variant && job.version == version)
    }

    /// Read the in flight job for a version and variant, if any.
    pub fn job(&self, variant: Variant, version: GodotVersion) -> Option<&InstallJob> {
        self.jobs
            .iter()
            .find(|job| job.variant == variant && job.version == version)
    }

    /// True when a version and variant has an install in flight.
    pub fn is_installing(&self, variant: Variant, version: GodotVersion) -> bool {
        self.job(variant, version).is_some()
    }

    /// True when a version and variant is already installed.
    pub fn is_installed(&self, variant: Variant, version: GodotVersion) -> bool {
        match &self.installed {
            Load::Loaded(engines) => engines
                .iter()
                .any(|engine| engine.variant == variant && engine.version == version),
            _ => false,
        }
    }

    /// Show a toast. An error stays a little longer than an info.
    pub fn toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        let remaining = match kind {
            ToastKind::Info => 4.0,
            ToastKind::Error => 7.0,
        };
        self.toasts.push(Toast {
            id: self.next_toast_id,
            kind,
            message: message.into(),
            remaining,
            total: remaining,
        });
        self.next_toast_id += 1;
    }

    /// Remove a toast by id, for a click to dismiss.
    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|toast| toast.id != id);
    }

    /// Age the toasts by the elapsed seconds and drop any that have run out. The
    /// aging pauses while the pointer is over a toast.
    pub fn tick_toasts(&mut self, elapsed: f32) {
        if self.toast_paused {
            return;
        }
        for toast in &mut self.toasts {
            toast.remaining -= elapsed;
        }
        self.toasts.retain(|toast| toast.remaining > 0.0);
    }
}
