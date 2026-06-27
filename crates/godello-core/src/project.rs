//! Reading and writing a Godot project.
//!
//! A project is a folder with a project.godot file. That file is an ini style
//! list of keys, with some values written in Godot's own syntax such as
//! PackedStringArray. This module reads the parts we care about, decides if the
//! project uses C#, and reads or writes the godello version pin.
//!
//! The pin lives in a godello section inside project.godot so it travels with the
//! project. Writing the pin is a surgical edit that changes only that one value
//! and leaves the rest of the file untouched, so a user's project.godot is never
//! reformatted.

use std::fs;
use std::path::{Path, PathBuf};

use crate::version::{Variant, VersionPattern};

/// The file name that marks a Godot project folder.
pub const PROJECT_FILE: &str = "project.godot";

/// What was read from a project.godot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotProject {
    /// The project folder, which holds project.godot.
    pub dir: PathBuf,
    /// The project name, if it had one.
    pub name: Option<String>,
    /// The raw config_version value. 5 means Godot 4, 4 means Godot 3.
    pub config_version: Option<u32>,
    /// True when the project uses C#.
    pub uses_csharp: bool,
    /// The version pin from the godello section, if set.
    pub pinned_version: Option<VersionPattern>,
    /// A version read from the project features, used as a fallback hint.
    pub feature_version: Option<VersionPattern>,
}

impl GodotProject {
    /// The path to the project.godot file inside a project folder.
    pub fn project_file(dir: &Path) -> PathBuf {
        dir.join(PROJECT_FILE)
    }

    /// Load and parse the project in the given folder.
    pub fn load(dir: impl AsRef<Path>) -> Result<GodotProject, ProjectError> {
        let dir = dir.as_ref();
        let file = Self::project_file(dir);
        let content = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(ProjectError::NotAProject(dir.to_path_buf()));
            }
            Err(err) => {
                return Err(ProjectError::Io(err));
            }
        };

        let parsed = parse(&content);
        let uses_csharp = parsed.has_dotnet_section
            || parsed.has_mono_section
            || parsed.features.iter().any(|f| f.eq_ignore_ascii_case("C#"))
            || has_csharp_files(dir);

        Ok(GodotProject {
            dir: dir.to_path_buf(),
            name: parsed.name,
            config_version: parsed.config_version,
            uses_csharp,
            pinned_version: parsed.pinned_version,
            feature_version: parsed
                .features
                .iter()
                .find_map(|f| f.parse::<VersionPattern>().ok()),
        })
    }

    /// The engine this project needs, as a version requirement and a variant.
    /// The pin wins when set, otherwise the feature hint is used. The variant is
    /// Mono when the project uses C#, otherwise Standard. Returns None when there
    /// is nothing to go on.
    pub fn required_engine(&self) -> Option<(VersionPattern, Variant)> {
        let pattern = self.pinned_version.or(self.feature_version)?;
        let variant = if self.uses_csharp {
            Variant::Mono
        } else {
            Variant::Standard
        };
        Some((pattern, variant))
    }

    /// Write the version pin into the godello section of project.godot. Only that
    /// one value is changed. Every other line is left exactly as it was.
    pub fn set_pin(dir: &Path, pattern: VersionPattern) -> Result<(), ProjectError> {
        let file = Self::project_file(dir);
        if !file.is_file() {
            return Err(ProjectError::NotAProject(dir.to_path_buf()));
        }
        let content = fs::read_to_string(&file)?;
        let updated = write_pin(&content, pattern);
        fs::write(&file, updated)?;
        Ok(())
    }
}

/// Walk up from a starting folder looking for one that holds project.godot.
pub fn find_project_dir(start: impl AsRef<Path>) -> Option<PathBuf> {
    let mut current = Some(start.as_ref());
    while let Some(dir) = current {
        if dir.join(PROJECT_FILE).is_file() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// True when the folder has a C# solution or project file next to project.godot.
fn has_csharp_files(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        if matches!(extension.as_deref(), Some("sln") | Some("csproj")) {
            return true;
        }
    }
    false
}

/// The raw facts pulled from a project.godot body.
struct Parsed {
    name: Option<String>,
    config_version: Option<u32>,
    features: Vec<String>,
    pinned_version: Option<VersionPattern>,
    has_dotnet_section: bool,
    has_mono_section: bool,
}

fn parse(content: &str) -> Parsed {
    let mut parsed = Parsed {
        name: None,
        config_version: None,
        features: Vec::new(),
        pinned_version: None,
        has_dotnet_section: false,
        has_mono_section: false,
    };
    let mut section: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = section_header(trimmed) {
            match name {
                "dotnet" => parsed.has_dotnet_section = true,
                "mono" => parsed.has_mono_section = true,
                _ => {}
            }
            section = Some(name.to_string());
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match (section.as_deref(), key) {
            (None, "config_version") => {
                parsed.config_version = value.parse().ok();
            }
            (Some("application"), "config/name") => {
                parsed.name = Some(unquote(value));
            }
            (Some("application"), "config/features") => {
                parsed.features = parse_packed_string_array(value);
            }
            (Some("godello"), "version") => {
                parsed.pinned_version = unquote(value).parse().ok();
            }
            _ => {}
        }
    }
    parsed
}

/// Return the section name when a line is a header like [application].
fn section_header(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    Some(inner.trim())
}

/// Remove one pair of surrounding double quotes if present.
fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Pull the strings out of a value like PackedStringArray("4.3", "C#").
fn parse_packed_string_array(value: &str) -> Vec<String> {
    let Some(open) = value.find('(') else {
        return Vec::new();
    };
    let Some(close) = value.rfind(')') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    value[open + 1..close]
        .split(',')
        .map(unquote)
        .filter(|item| !item.is_empty())
        .collect()
}

/// Produce the new file body with the pin written into the godello section.
fn write_pin(content: &str, pattern: VersionPattern) -> String {
    let mut lines: Vec<String> = content.lines().map(|line| line.to_string()).collect();
    let version_line = format!("version=\"{pattern}\"");

    let header = lines
        .iter()
        .position(|line| section_header(line.trim()) == Some("godello"));
    match header {
        Some(start) => {
            let end = lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, line)| section_header(line.trim()).is_some())
                .map(|(index, _)| index)
                .unwrap_or(lines.len());
            let existing = (start + 1..end).find(|&index| {
                lines[index]
                    .trim()
                    .split_once('=')
                    .map(|(key, _)| key.trim() == "version")
                    .unwrap_or(false)
            });
            match existing {
                Some(index) => lines[index] = version_line,
                None => lines.insert(start + 1, version_line),
            }
        }
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[godello]".to_string());
            lines.push(version_line);
        }
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// An error from loading or pinning a project.
#[derive(Debug)]
pub enum ProjectError {
    /// The folder has no project.godot file.
    NotAProject(PathBuf),
    /// A filesystem call failed.
    Io(std::io::Error),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::NotAProject(path) => {
                write!(f, "{} is not a Godot project", path.display())
            }
            ProjectError::Io(err) => write!(f, "filesystem error: {err}"),
        }
    }
}

impl std::error::Error for ProjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProjectError::Io(err) => Some(err),
            ProjectError::NotAProject(_) => None,
        }
    }
}

impl From<std::io::Error> for ProjectError {
    fn from(err: std::io::Error) -> Self {
        ProjectError::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("godello-project-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_project(dir: &Path, body: &str) {
        fs::write(dir.join(PROJECT_FILE), body).unwrap();
    }

    const GODOT4: &str = r#"; Engine configuration file.
config_version=5

[application]

config/name="My Game"
config/features=PackedStringArray("4.3", "Forward Plus")
run/main_scene="res://main.tscn"
"#;

    #[test]
    fn reads_name_and_config_version() {
        let dir = scratch("read-basic");
        write_project(&dir, GODOT4);
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.name.as_deref(), Some("My Game"));
        assert_eq!(project.config_version, Some(5));
        assert!(!project.uses_csharp);
    }

    #[test]
    fn reads_feature_version_as_a_hint() {
        let dir = scratch("read-feature");
        write_project(&dir, GODOT4);
        let project = GodotProject::load(&dir).unwrap();
        let pattern: VersionPattern = "4.3".parse().unwrap();
        assert_eq!(project.feature_version, Some(pattern));
    }

    #[test]
    fn missing_project_file_is_not_a_project() {
        let dir = scratch("read-missing");
        let result = GodotProject::load(&dir);
        assert!(matches!(result, Err(ProjectError::NotAProject(_))));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let dir = scratch("read-comments");
        write_project(
            &dir,
            "; a comment\n\n# another\nconfig_version=5\n\n[application]\nconfig/name=\"X\"\n",
        );
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.name.as_deref(), Some("X"));
        assert_eq!(project.config_version, Some(5));
    }

    #[test]
    fn non_numeric_config_version_is_none() {
        let dir = scratch("read-badver");
        write_project(&dir, "config_version=oops\n");
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.config_version, None);
    }

    #[test]
    fn name_with_spaces_and_punctuation_is_read() {
        let dir = scratch("read-name");
        write_project(
            &dir,
            "config_version=5\n[application]\nconfig/name=\"My Cool Game (2026)\"\n",
        );
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.name.as_deref(), Some("My Cool Game (2026)"));
    }

    // C# detection.

    #[test]
    fn detects_csharp_from_the_csharp_feature() {
        let dir = scratch("cs-feature");
        write_project(
            &dir,
            "config_version=5\n[application]\nconfig/features=PackedStringArray(\"4.3\", \"C#\")\n",
        );
        assert!(GodotProject::load(&dir).unwrap().uses_csharp);
    }

    #[test]
    fn detects_csharp_from_a_dotnet_section() {
        let dir = scratch("cs-dotnet");
        write_project(
            &dir,
            "config_version=5\n[application]\nconfig/name=\"X\"\n[dotnet]\nproject/assembly_name=\"X\"\n",
        );
        assert!(GodotProject::load(&dir).unwrap().uses_csharp);
    }

    #[test]
    fn detects_csharp_from_a_mono_section() {
        let dir = scratch("cs-mono");
        write_project(&dir, "config_version=4\n[application]\n[mono]\n");
        assert!(GodotProject::load(&dir).unwrap().uses_csharp);
    }

    #[test]
    fn detects_csharp_from_a_solution_file() {
        let dir = scratch("cs-sln");
        write_project(&dir, GODOT4);
        fs::write(dir.join("MyGame.sln"), b"").unwrap();
        assert!(GodotProject::load(&dir).unwrap().uses_csharp);
    }

    #[test]
    fn detects_csharp_from_a_csproj_file() {
        let dir = scratch("cs-csproj");
        write_project(&dir, GODOT4);
        fs::write(dir.join("MyGame.csproj"), b"").unwrap();
        assert!(GodotProject::load(&dir).unwrap().uses_csharp);
    }

    #[test]
    fn plain_project_is_not_csharp() {
        let dir = scratch("cs-none");
        write_project(&dir, GODOT4);
        fs::write(dir.join("readme.txt"), b"").unwrap();
        assert!(!GodotProject::load(&dir).unwrap().uses_csharp);
    }

    // Features parsing.

    #[test]
    fn parses_packed_string_array_forms() {
        assert_eq!(
            parse_packed_string_array("PackedStringArray(\"4.3\", \"C#\")"),
            vec!["4.3".to_string(), "C#".to_string()]
        );
        assert_eq!(
            parse_packed_string_array("PackedStringArray( \"4.2\" )"),
            vec!["4.2".to_string()]
        );
        assert!(parse_packed_string_array("PackedStringArray()").is_empty());
        assert!(parse_packed_string_array("not an array").is_empty());
    }

    // Required engine.

    #[test]
    fn required_engine_prefers_the_pin_over_the_feature() {
        let dir = scratch("req-pin");
        write_project(
            &dir,
            "config_version=5\n[application]\nconfig/features=PackedStringArray(\"4.3\")\n[godello]\nversion=\"4.2-stable\"\n",
        );
        let project = GodotProject::load(&dir).unwrap();
        let (pattern, variant) = project.required_engine().unwrap();
        assert_eq!(pattern, "4.2-stable".parse().unwrap());
        assert_eq!(variant, Variant::Standard);
    }

    #[test]
    fn required_engine_falls_back_to_the_feature_hint() {
        let dir = scratch("req-feature");
        write_project(&dir, GODOT4);
        let project = GodotProject::load(&dir).unwrap();
        let (pattern, variant) = project.required_engine().unwrap();
        assert_eq!(pattern, "4.3".parse().unwrap());
        assert_eq!(variant, Variant::Standard);
    }

    #[test]
    fn required_engine_uses_mono_for_csharp() {
        let dir = scratch("req-mono");
        write_project(
            &dir,
            "config_version=5\n[application]\nconfig/features=PackedStringArray(\"4.3\", \"C#\")\n",
        );
        let project = GodotProject::load(&dir).unwrap();
        let (_, variant) = project.required_engine().unwrap();
        assert_eq!(variant, Variant::Mono);
    }

    #[test]
    fn required_engine_is_none_without_a_hint() {
        let dir = scratch("req-none");
        write_project(&dir, "config_version=5\n[application]\nconfig/name=\"X\"\n");
        let project = GodotProject::load(&dir).unwrap();
        assert!(project.required_engine().is_none());
    }

    // Pin writing.

    #[test]
    fn set_pin_adds_a_godello_section_when_absent() {
        let dir = scratch("pin-add");
        write_project(&dir, GODOT4);
        GodotProject::set_pin(&dir, "4.3-stable".parse().unwrap()).unwrap();
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.pinned_version, Some("4.3-stable".parse().unwrap()));
        // The original content is still there.
        let body = fs::read_to_string(GodotProject::project_file(&dir)).unwrap();
        assert!(body.contains("config/name=\"My Game\""));
        assert!(body.contains("[godello]"));
    }

    #[test]
    fn set_pin_updates_without_duplicating() {
        let dir = scratch("pin-update");
        write_project(
            &dir,
            "config_version=5\n[godello]\nversion=\"4.1-stable\"\n",
        );
        GodotProject::set_pin(&dir, "4.4-stable".parse().unwrap()).unwrap();
        let body = fs::read_to_string(GodotProject::project_file(&dir)).unwrap();
        // Count the quoted pin form so config_version is not also matched.
        assert_eq!(body.matches("version=\"").count(), 1);
        assert!(body.contains("version=\"4.4-stable\""));
        assert!(!body.contains("4.1-stable"));
    }

    #[test]
    fn set_pin_round_trips() {
        let dir = scratch("pin-round");
        write_project(&dir, GODOT4);
        let pattern: VersionPattern = "4.3-rc1".parse().unwrap();
        GodotProject::set_pin(&dir, pattern).unwrap();
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.pinned_version, Some(pattern));
    }

    #[test]
    fn set_pin_preserves_other_sections_exactly() {
        let dir = scratch("pin-preserve");
        let body = "config_version=5\n\n[application]\nconfig/name=\"Keep Me\"\nrun/main_scene=\"res://main.tscn\"\n\n[input]\nui_accept={\n\"x\": 1\n}\n";
        write_project(&dir, body);
        GodotProject::set_pin(&dir, "4.3-stable".parse().unwrap()).unwrap();
        let updated = fs::read_to_string(GodotProject::project_file(&dir)).unwrap();
        assert!(updated.contains("config/name=\"Keep Me\""));
        assert!(updated.contains("run/main_scene=\"res://main.tscn\""));
        assert!(updated.contains("[input]"));
        assert!(updated.contains("ui_accept={"));
    }

    #[test]
    fn set_pin_on_a_non_project_errors() {
        let dir = scratch("pin-noproject");
        let result = GodotProject::set_pin(&dir, "4.3-stable".parse().unwrap());
        assert!(matches!(result, Err(ProjectError::NotAProject(_))));
    }

    // Finding a project by walking up.

    #[test]
    fn finds_a_project_in_a_parent_folder() {
        let root = scratch("find-root");
        write_project(&root, GODOT4);
        let nested = root.join("scenes").join("levels");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_project_dir(&nested), Some(root.clone()));
        assert_eq!(find_project_dir(&root), Some(root));
    }

    #[test]
    fn find_returns_none_when_there_is_no_project() {
        let dir = scratch("find-none");
        assert_eq!(find_project_dir(&dir), None);
    }
}
