//! Shared state for a single command run.
//!
//! This resolves the application folders, loads the settings, and builds the one
//! network client. Commands take a context and ask it for an install manager or a
//! repository rather than wiring those up themselves.

use anyhow::{Context as _, Result, anyhow};
use godello_core::{GodotGitHubRepository, InstallManager, Paths, Settings};

use crate::interaction::Interaction;
use crate::net::WebClient;

/// The official Godot source backed by the real network client.
pub type Repository = GodotGitHubRepository<WebClient>;

/// Everything a command needs that is resolved once at startup.
pub struct Context {
    pub paths: Paths,
    pub settings: Settings,
    /// True when prompts should be skipped in favor of safe defaults.
    pub yes: bool,
    client: WebClient,
}

impl Context {
    /// Resolve folders, load settings, and build the network client.
    pub fn load(yes: bool) -> Result<Self> {
        let paths = Paths::discover().context("could not resolve the application folders")?;
        let settings =
            Settings::load(&paths.settings_file()).context("could not load the settings")?;
        let client =
            WebClient::new().map_err(|err| anyhow!("could not start the network client: {err}"))?;
        Ok(Context {
            paths,
            settings,
            yes,
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

    /// Ask a yes or no question, honoring the non interactive flag.
    pub fn confirm(&self, question: &str, default_yes: bool) -> bool {
        Interaction::new(self.yes).confirm(question, default_yes)
    }
}
