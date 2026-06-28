//! App settings, resolved paths, and the list of added projects.
//!
//! Paths resolve to the normal per platform locations. Settings and the project
//! list are small toml files. Every setting has a default, so a missing or fresh
//! file still works. Engines are large, so on Windows they go under the local app
//! data path rather than the roaming one.

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::csharp::CsharpBuildTool;
use crate::version::Variant;

/// The resolved folders Godello uses. The config dir holds settings. The data dir
/// holds engines, the download cache, and the project list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl Paths {
    /// Resolve the paths from the system, for example ~/.config/godello and
    /// ~/.local/share/godello on Linux.
    pub fn discover() -> Result<Paths, ConfigError> {
        let dirs = ProjectDirs::from("", "", "godello").ok_or(ConfigError::NoHome)?;
        Ok(Paths {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_local_dir().to_path_buf(),
        })
    }

    /// Build paths from explicit folders. Used by tests and by any override.
    pub fn with_dirs(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Paths {
        Paths {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Where settings are stored.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    /// The default engines folder, used when no override is set.
    pub fn default_engines_dir(&self) -> PathBuf {
        self.data_dir.join("engines")
    }

    /// Where downloaded archives are cached.
    pub fn downloads_dir(&self) -> PathBuf {
        self.data_dir.join("cache").join("downloads")
    }

    /// Where the version list is cached.
    pub fn manifest_cache(&self) -> PathBuf {
        self.data_dir.join("cache").join("manifest.json")
    }

    /// Where the list of added projects is stored.
    pub fn projects_file(&self) -> PathBuf {
        self.data_dir.join("projects.toml")
    }
}

/// The user settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where engines are installed. None means use the default engines folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_install_dir: Option<PathBuf>,
    /// Build the C# solution before opening the editor.
    pub build_csharp_before_launch: bool,
    /// Which tool builds the C# solution.
    pub csharp_build_tool: CsharpBuildTool,
    /// Include rc, beta, and dev releases in remote listings by default.
    pub include_prereleases: bool,
    /// The variant used when a command does not say.
    pub default_variant: Variant,
    /// Start the editor or project detached so the command returns right away.
    /// When off, the command stays attached and waits for the editor to close.
    pub launch_detached: bool,
    /// The color theme the desktop app opens with. A short name the app maps to
    /// one of its themes. The command line ignores this.
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine_install_dir: None,
            build_csharp_before_launch: true,
            csharp_build_tool: CsharpBuildTool::Godot,
            include_prereleases: false,
            default_variant: Variant::Standard,
            launch_detached: false,
            theme: "dark".to_string(),
        }
    }
}

impl Settings {
    /// The names of every setting, in a stable order. Used to list them all and
    /// kept in step with get_field and set_field.
    pub const FIELD_NAMES: &'static [&'static str] = &[
        "engine_install_dir",
        "build_csharp_before_launch",
        "csharp_build_tool",
        "include_prereleases",
        "default_variant",
        "launch_detached",
        "theme",
    ];

    /// Load settings from a file. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Settings, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(err) => {
                return Err(ConfigError::Io(err));
            }
        };
        toml::from_str(&text).map_err(|err| ConfigError::Parse(err.to_string()))
    }

    /// Write settings to a file, creating parent folders as needed.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text =
            toml::to_string_pretty(self).map_err(|err| ConfigError::Parse(err.to_string()))?;
        fs::write(path, text)?;
        Ok(())
    }

    /// The engines folder in effect, the override if set or the default.
    pub fn effective_engines_dir(&self, paths: &Paths) -> PathBuf {
        self.engine_install_dir
            .clone()
            .unwrap_or_else(|| paths.default_engines_dir())
    }

    /// Read a setting by name as text. Returns None for an unknown name, or for
    /// engine_install_dir when it is not set.
    pub fn get_field(&self, key: &str) -> Option<String> {
        match key {
            "engine_install_dir" => self
                .engine_install_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            "build_csharp_before_launch" => Some(self.build_csharp_before_launch.to_string()),
            "csharp_build_tool" => Some(self.csharp_build_tool.to_string()),
            "include_prereleases" => Some(self.include_prereleases.to_string()),
            "default_variant" => Some(self.default_variant.to_string()),
            "launch_detached" => Some(self.launch_detached.to_string()),
            "theme" => Some(self.theme.clone()),
            _ => None,
        }
    }

    /// Change a setting by name from text. An empty value for engine_install_dir
    /// resets it to the default.
    pub fn set_field(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        match key {
            "engine_install_dir" => {
                self.engine_install_dir = if value.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value))
                };
            }
            "build_csharp_before_launch" => {
                self.build_csharp_before_launch = parse_bool(key, value)?;
            }
            "csharp_build_tool" => {
                self.csharp_build_tool = value.parse().map_err(|_| ConfigError::InvalidValue {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            "include_prereleases" => {
                self.include_prereleases = parse_bool(key, value)?;
            }
            "launch_detached" => {
                self.launch_detached = parse_bool(key, value)?;
            }
            "theme" => {
                let name = value.trim();
                if name.is_empty() {
                    return Err(ConfigError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    });
                }
                self.theme = name.to_ascii_lowercase();
            }
            "default_variant" => {
                self.default_variant = value.parse().map_err(|_| ConfigError::InvalidValue {
                    key: key.to_string(),
                    value: value.to_string(),
                })?;
            }
            _ => {
                return Err(ConfigError::UnknownSetting(key.to_string()));
            }
        }
        Ok(())
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(ConfigError::InvalidValue {
            key: key.to_string(),
            value: value.to_string(),
        }),
    }
}

/// One project the user has added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The stored list of added projects. The version a project needs is always read
/// from its project.godot, so only the path and a cached name are kept here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectList {
    #[serde(default, rename = "project")]
    projects: Vec<ProjectEntry>,
}

impl ProjectList {
    /// Load the list from a file. A missing file gives an empty list.
    pub fn load(path: &Path) -> Result<ProjectList, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProjectList::default());
            }
            Err(err) => {
                return Err(ConfigError::Io(err));
            }
        };
        toml::from_str(&text).map_err(|err| ConfigError::Parse(err.to_string()))
    }

    /// Write the list to a file, creating parent folders as needed.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text =
            toml::to_string_pretty(self).map_err(|err| ConfigError::Parse(err.to_string()))?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn entries(&self) -> &[ProjectEntry] {
        &self.projects
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    /// Add a project, or update the cached name when the path is already known.
    /// Returns true when a new project was added.
    pub fn add(&mut self, path: impl Into<PathBuf>, name: Option<String>) -> bool {
        let path = path.into();
        if let Some(entry) = self.projects.iter_mut().find(|entry| entry.path == path) {
            entry.name = name;
            false
        } else {
            self.projects.push(ProjectEntry { path, name });
            true
        }
    }

    /// Forget a project by path. Returns true when one was removed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.projects.len();
        self.projects.retain(|entry| entry.path != path);
        self.projects.len() != before
    }

    /// True when a project with this path is in the list.
    pub fn contains(&self, path: &Path) -> bool {
        self.projects.iter().any(|entry| entry.path == path)
    }
}

/// An error from settings or paths.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    /// A toml file could not be read or written.
    Parse(String),
    /// No home folder could be found.
    NoHome,
    /// The setting name is not known.
    UnknownSetting(String),
    /// The value was not valid for the setting.
    InvalidValue {
        key: String,
        value: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "filesystem error: {err}"),
            ConfigError::Parse(msg) => write!(f, "could not read settings: {msg}"),
            ConfigError::NoHome => write!(f, "could not find a home folder"),
            ConfigError::UnknownSetting(key) => write!(f, "unknown setting {key}"),
            ConfigError::InvalidValue { key, value } => {
                write!(f, "{value} is not a valid value for {key}")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("godello-config-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Paths.

    #[test]
    fn paths_derive_from_the_base_dirs() {
        let paths = Paths::with_dirs("/cfg", "/data");
        assert_eq!(paths.settings_file(), PathBuf::from("/cfg/settings.toml"));
        assert_eq!(paths.default_engines_dir(), PathBuf::from("/data/engines"));
        assert_eq!(
            paths.downloads_dir(),
            PathBuf::from("/data/cache/downloads")
        );
        assert_eq!(paths.projects_file(), PathBuf::from("/data/projects.toml"));
    }

    #[test]
    fn discover_resolves_on_this_platform() {
        // Just confirm it produces some paths without error here.
        let paths = Paths::discover().unwrap();
        assert!(!paths.config_dir().as_os_str().is_empty());
        assert!(!paths.data_dir().as_os_str().is_empty());
    }

    // Settings defaults and round trip.

    #[test]
    fn defaults_are_sensible() {
        let settings = Settings::default();
        assert!(settings.build_csharp_before_launch);
        assert_eq!(settings.csharp_build_tool, CsharpBuildTool::Godot);
        assert!(!settings.include_prereleases);
        assert_eq!(settings.default_variant, Variant::Standard);
        assert_eq!(settings.engine_install_dir, None);
        assert!(!settings.launch_detached);
        assert_eq!(settings.theme, "dark");
    }

    #[test]
    fn csharp_build_tool_round_trips_and_sets() {
        let dir = scratch("settings-tool");
        let path = dir.join("settings.toml");
        let mut settings = Settings::default();
        settings.set_field("csharp_build_tool", "dotnet").unwrap();
        assert_eq!(settings.csharp_build_tool, CsharpBuildTool::Dotnet);
        settings.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("csharp_build_tool = \"dotnet\""));
        assert_eq!(Settings::load(&path).unwrap(), settings);
        assert_eq!(
            settings.get_field("csharp_build_tool").as_deref(),
            Some("dotnet")
        );
    }

    #[test]
    fn csharp_build_tool_rejects_a_bad_value() {
        let mut settings = Settings::default();
        assert!(matches!(
            settings.set_field("csharp_build_tool", "msbuild"),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn missing_settings_file_gives_defaults() {
        let dir = scratch("settings-missing");
        let settings = Settings::load(&dir.join("settings.toml")).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let dir = scratch("settings-round");
        let path = dir.join("settings.toml");
        let settings = Settings {
            include_prereleases: true,
            default_variant: Variant::Mono,
            engine_install_dir: Some(PathBuf::from("/opt/godot-engines")),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn partial_settings_file_fills_in_defaults() {
        let dir = scratch("settings-partial");
        let path = dir.join("settings.toml");
        fs::write(&path, "include_prereleases = true\n").unwrap();
        let settings = Settings::load(&path).unwrap();
        assert!(settings.include_prereleases);
        // Everything else is still the default.
        assert!(settings.build_csharp_before_launch);
        assert_eq!(settings.default_variant, Variant::Standard);
    }

    #[test]
    fn invalid_settings_file_is_a_parse_error() {
        let dir = scratch("settings-bad");
        let path = dir.join("settings.toml");
        fs::write(&path, "build_csharp_before_launch = not_a_bool\n").unwrap();
        assert!(matches!(Settings::load(&path), Err(ConfigError::Parse(_))));
    }

    #[test]
    fn default_variant_serializes_as_a_word() {
        let dir = scratch("settings-variant-word");
        let path = dir.join("settings.toml");
        let settings = Settings {
            default_variant: Variant::Mono,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("default_variant = \"mono\""));
    }

    // Effective engines dir.

    #[test]
    fn effective_engines_dir_uses_default_when_unset() {
        let paths = Paths::with_dirs("/cfg", "/data");
        let settings = Settings::default();
        assert_eq!(
            settings.effective_engines_dir(&paths),
            PathBuf::from("/data/engines")
        );
    }

    #[test]
    fn effective_engines_dir_uses_override_when_set() {
        let paths = Paths::with_dirs("/cfg", "/data");
        let settings = Settings {
            engine_install_dir: Some(PathBuf::from("/elsewhere")),
            ..Settings::default()
        };
        assert_eq!(
            settings.effective_engines_dir(&paths),
            PathBuf::from("/elsewhere")
        );
    }

    // Get and set by name.

    #[test]
    fn every_listed_field_is_a_real_setting() {
        // With the engine dir set, every listed name must read back a value. A
        // stray or misspelled name would return None and fail here.
        let settings = Settings {
            engine_install_dir: Some(PathBuf::from("/x")),
            ..Settings::default()
        };
        for key in Settings::FIELD_NAMES {
            assert!(
                settings.get_field(key).is_some(),
                "{key} should be a readable setting"
            );
        }
    }

    #[test]
    fn get_field_reads_each_setting() {
        let settings = Settings::default();
        assert_eq!(
            settings.get_field("build_csharp_before_launch").as_deref(),
            Some("true")
        );
        assert_eq!(
            settings.get_field("include_prereleases").as_deref(),
            Some("false")
        );
        assert_eq!(
            settings.get_field("default_variant").as_deref(),
            Some("standard")
        );
        assert_eq!(settings.get_field("engine_install_dir"), None);
        assert_eq!(settings.get_field("nope"), None);
    }

    #[test]
    fn set_field_changes_bools() {
        let mut settings = Settings::default();
        settings
            .set_field("build_csharp_before_launch", "false")
            .unwrap();
        assert!(!settings.build_csharp_before_launch);
        settings.set_field("include_prereleases", "on").unwrap();
        assert!(settings.include_prereleases);
    }

    #[test]
    fn set_field_rejects_a_bad_bool() {
        let mut settings = Settings::default();
        let result = settings.set_field("build_csharp_before_launch", "maybe");
        assert!(matches!(result, Err(ConfigError::InvalidValue { .. })));
    }

    #[test]
    fn set_field_changes_variant() {
        let mut settings = Settings::default();
        settings.set_field("default_variant", "mono").unwrap();
        assert_eq!(settings.default_variant, Variant::Mono);
    }

    #[test]
    fn set_field_rejects_a_bad_variant() {
        let mut settings = Settings::default();
        assert!(matches!(
            settings.set_field("default_variant", "csharp_maybe"),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn set_field_sets_and_resets_engine_dir() {
        let mut settings = Settings::default();
        settings
            .set_field("engine_install_dir", "/opt/engines")
            .unwrap();
        assert_eq!(
            settings.engine_install_dir,
            Some(PathBuf::from("/opt/engines"))
        );
        settings.set_field("engine_install_dir", "").unwrap();
        assert_eq!(settings.engine_install_dir, None);
    }

    #[test]
    fn set_field_sets_theme_and_lowercases() {
        let mut settings = Settings::default();
        settings.set_field("theme", "Light").unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.get_field("theme").as_deref(), Some("light"));
    }

    #[test]
    fn set_field_rejects_an_empty_theme() {
        let mut settings = Settings::default();
        assert!(matches!(
            settings.set_field("theme", "   "),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn theme_round_trips_and_unknown_name_is_kept() {
        let dir = scratch("settings-theme");
        let path = dir.join("settings.toml");
        // An unknown name is stored as is. The desktop app decides how to map it,
        // so the config layer does not police the value beyond it being present.
        let settings = Settings {
            theme: "midnight".to_string(),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.theme, "midnight");
    }

    #[test]
    fn set_field_rejects_unknown_key() {
        let mut settings = Settings::default();
        assert!(matches!(
            settings.set_field("color", "blue"),
            Err(ConfigError::UnknownSetting(_))
        ));
    }

    // Project list.

    #[test]
    fn missing_project_list_is_empty() {
        let dir = scratch("projects-missing");
        let list = ProjectList::load(&dir.join("projects.toml")).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn add_then_round_trip() {
        let dir = scratch("projects-round");
        let path = dir.join("projects.toml");
        let mut list = ProjectList::default();
        assert!(list.add("/games/one", Some("One".to_string())));
        assert!(list.add("/games/two", None));
        list.save(&path).unwrap();
        let loaded = ProjectList::load(&path).unwrap();
        assert_eq!(loaded.entries().len(), 2);
        assert_eq!(loaded.entries()[0].name.as_deref(), Some("One"));
        assert_eq!(loaded.entries()[1].name, None);
    }

    #[test]
    fn add_dedupes_by_path_and_updates_name() {
        let mut list = ProjectList::default();
        assert!(list.add("/games/one", None));
        // Adding the same path again updates instead of duplicating.
        assert!(!list.add("/games/one", Some("Renamed".to_string())));
        assert_eq!(list.entries().len(), 1);
        assert_eq!(list.entries()[0].name.as_deref(), Some("Renamed"));
    }

    #[test]
    fn remove_reports_whether_anything_changed() {
        let mut list = ProjectList::default();
        list.add("/games/one", None);
        assert!(list.contains(Path::new("/games/one")));
        assert!(list.remove(Path::new("/games/one")));
        assert!(!list.contains(Path::new("/games/one")));
        // Removing again changes nothing.
        assert!(!list.remove(Path::new("/games/one")));
    }
}
