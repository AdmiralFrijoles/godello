//! The messages that drive the GUI update loop.
//!
//! Every user action and every async result arrives as one of these. Async
//! results carry a String error rather than a core error type, because the core
//! errors are not Clone and an iced message must be Clone. We map the error to a
//! string at the task boundary.

use godello_core::{GodotVersion, Release, Variant};
use iced::Theme;

use crate::gui::progress::ProgressEvent;
use crate::gui::state::{Channel, EnginesTab, Screen};

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
}
