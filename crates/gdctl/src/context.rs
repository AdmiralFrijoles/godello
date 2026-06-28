//! Shared state for a single command run.
//!
//! This resolves the application folders, loads the settings, and builds the one
//! network client. Commands take a context and ask it for an install manager or a
//! repository rather than wiring those up themselves.

use anyhow::{Context as _, Result, anyhow};
use godello_core::{GodotGitHubRepository, InstallManager, Paths, Settings, resolve_tools};

use crate::interaction::Interaction;
use crate::net::WebClient;

/// The official Godot source backed by the real network client.
pub type Repository = GodotGitHubRepository<WebClient>;

/// Everything a command needs that is resolved once at startup.
///
/// This is cheap to clone. The settings and paths are small and the network
/// client shares one connection pool across clones, so the GUI can hand a copy
/// to its startup hook without rebuilding anything.
#[derive(Clone)]
pub struct Context {
    pub paths: Paths,
    pub settings: Settings,
    /// True when prompts should be skipped in favor of safe defaults.
    pub yes: bool,
    /// True when normal output should be suppressed. Errors still show.
    pub silent: bool,
    client: WebClient,
}

impl Context {
    /// Resolve folders, load settings, and build the network client.
    pub fn load(yes: bool, silent: bool) -> Result<Self> {
        let paths = Paths::discover().context("could not resolve the application folders")?;
        let settings_file = paths.settings_file();
        let mut settings = if settings_file.exists() {
            Settings::load(&settings_file).context("could not load the settings")?
        } else {
            // First run. Pick sensible defaults from what the system has, such as
            // using dotnet for C# builds when it is installed, and save them so
            // the choice is stable and visible. A save problem is not fatal, the
            // same defaults still apply in memory for this run.
            let settings = Settings::initial();
            let _ = settings.save(&settings_file);
            settings
        };
        // Find the external tools on startup. A stored path that still exists is
        // kept, otherwise we search for the tool again, otherwise it is left blank.
        // Save only when something changed so an unchanged start does no writes.
        if resolve_tools(&mut settings) {
            let _ = settings.save(&settings_file);
        }
        let client =
            WebClient::new().map_err(|err| anyhow!("could not start the network client: {err}"))?;
        Ok(Context {
            paths,
            settings,
            yes,
            silent,
            client,
        })
    }

    /// The install manager pointed at the engines folder in effect.
    pub fn install_manager(&self) -> InstallManager {
        InstallManager::new(
            self.settings.effective_engines_dir(&self.paths),
            self.paths.downloads_dir(),
        )
    }

    /// The engine source. The client is cheap to clone and shares connections.
    pub fn repository(&self) -> Repository {
        GodotGitHubRepository::new(self.client.clone())
    }

    /// The network client used as a downloader.
    pub fn client(&self) -> &WebClient {
        &self.client
    }

    /// Ask a yes or no question, honoring the non interactive flag. A silent run
    /// never prompts either, since a prompt would print and wait for input, so it
    /// takes the safe default.
    pub fn confirm(&self, question: &str, default_yes: bool) -> bool {
        Interaction::new(self.yes || self.silent).confirm(question, default_yes)
    }
}
