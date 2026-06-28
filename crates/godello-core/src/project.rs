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

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use crate::vcs::DEFAULT_MAIN_BRANCH;
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
    /// The main branch an update pulls from, from the godello section. None means
    /// the default is used.
    pub main_branch: Option<String>,
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
            main_branch: parsed.main_branch,
        })
    }

    /// The branch an update pulls from and merges in. The godello section can set
    /// it with main_branch. When unset the common default is used.
    pub fn main_branch(&self) -> &str {
        self.main_branch.as_deref().unwrap_or(DEFAULT_MAIN_BRANCH)
    }

    /// True when every importable asset has its generated files present, so the
    /// project can run without a fresh import.
    ///
    /// Each importable asset keeps a sibling .import file that names the files
    /// Godot generates for it in the import cache (under .godot/imported for Godot
    /// 4, or .import for Godot 3). Those sidecars travel with the project but the
    /// generated files do not, so a fresh checkout has the sidecars and none of
    /// their outputs. The project is imported only when every output a sidecar
    /// declares is on disk. The presence of the cache folder alone is not enough,
    /// since it can exist while outputs are missing or were cleared. A project
    /// with no importable assets has nothing to import, so it counts as imported.
    pub fn is_imported(&self) -> bool {
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        queue.push_back((self.dir.clone(), 0));
        while let Some((dir, depth)) = queue.pop_front() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // The import cache and version control folders are hidden and
                    // hold no sidecars, so the scan skips them.
                    if depth < MAX_IMPORT_SCAN_DEPTH && !is_hidden(&path) {
                        queue.push_back((path, depth + 1));
                    }
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("import") {
                    continue;
                }
                // A sidecar we cannot read is treated as needing an import, since
                // we cannot prove its outputs are present.
                let Ok(text) = fs::read_to_string(&path) else {
                    return false;
                };
                for output in imported_outputs(&text) {
                    let relative = output.trim_start_matches("res://");
                    if !self.dir.join(relative).exists() {
                        return false;
                    }
                }
            }
        }
        true
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

/// How deep the downward search descends. A cloned repository keeps its project
/// near the top, so a shallow bound finds it without walking a deep tree.
const MAX_SEARCH_DEPTH: usize = 6;

/// How deep the import scan descends from the project root. Assets can sit well
/// below the top, for example under addons, so this is generous. It is only a
/// guard against an unusually deep tree or a symlink loop.
const MAX_IMPORT_SCAN_DEPTH: usize = 24;

/// Pull the generated import outputs declared in a .import sidecar. These are the
/// quoted res:// paths that point into the import cache, named by the path and
/// dest_files keys. The source asset and the uid are left out, since only the
/// generated files tell us whether an import has run.
fn imported_outputs(text: &str) -> Vec<String> {
    text.split('"')
        .filter(|token| {
            token.starts_with("res://.godot/imported/") || token.starts_with("res://.import/")
        })
        .map(|token| token.to_string())
        .collect()
}

/// Search a folder and the folders under it for the first one that holds
/// project.godot. The walk is breadth first, so the project nearest the top wins
/// when a repository nests it under a subfolder. Hidden folders such as .git are
/// skipped, and the depth is bounded so a large tree does not take long. Among
/// folders at the same depth the search is ordered by name, so the result is
/// stable.
pub fn find_project_dir_in_tree(root: impl AsRef<Path>) -> Option<PathBuf> {
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((root.as_ref().to_path_buf(), 0));
    while let Some((dir, depth)) = queue.pop_front() {
        if dir.join(PROJECT_FILE).is_file() {
            return Some(dir);
        }
        if depth >= MAX_SEARCH_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && !is_hidden(path))
            .collect();
        children.sort();
        for child in children {
            queue.push_back((child, depth + 1));
        }
    }
    None
}

/// True when a path's final component starts with a dot, such as .git or
/// .github. These never hold a Godot project, so the search skips them.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
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
    main_branch: Option<String>,
    has_dotnet_section: bool,
    has_mono_section: bool,
}

fn parse(content: &str) -> Parsed {
    let mut parsed = Parsed {
        name: None,
        config_version: None,
        features: Vec::new(),
        pinned_version: None,
        main_branch: None,
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
            (Some("godello"), "pin_version") => {
                parsed.pinned_version = unquote(value).parse().ok();
            }
            (Some("godello"), "main_branch") => {
                let name = unquote(value);
                if !name.is_empty() {
                    parsed.main_branch = Some(name);
                }
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
    let pin_line = format!("pin_version=\"{pattern}\"");

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
                    .map(|(key, _)| key.trim() == "pin_version")
                    .unwrap_or(false)
            });
            match existing {
                Some(index) => lines[index] = pin_line,
                None => lines.insert(start + 1, pin_line),
            }
        }
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[godello]".to_string());
            lines.push(pin_line);
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
    fn main_branch_defaults_when_unset() {
        let dir = scratch("read-no-branch");
        write_project(&dir, GODOT4);
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.main_branch, None);
        assert_eq!(project.main_branch(), "main");
    }

    #[test]
    fn reads_the_main_branch_override() {
        let dir = scratch("read-branch");
        write_project(&dir, "config_version=5\n[godello]\nmain_branch=\"trunk\"\n");
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.main_branch.as_deref(), Some("trunk"));
        assert_eq!(project.main_branch(), "trunk");
    }

    #[test]
    fn an_empty_main_branch_override_is_ignored() {
        let dir = scratch("read-branch-empty");
        write_project(&dir, "config_version=5\n[godello]\nmain_branch=\"\"\n");
        let project = GodotProject::load(&dir).unwrap();
        assert_eq!(project.main_branch, None);
        assert_eq!(project.main_branch(), "main");
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
            "config_version=5\n[application]\nconfig/features=PackedStringArray(\"4.3\")\n[godello]\npin_version=\"4.2-stable\"\n",
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
            "config_version=5\n[godello]\npin_version=\"4.1-stable\"\n",
        );
        GodotProject::set_pin(&dir, "4.4-stable".parse().unwrap()).unwrap();
        let body = fs::read_to_string(GodotProject::project_file(&dir)).unwrap();
        // Only one pin line, so an update did not leave the old value behind.
        assert_eq!(body.matches("pin_version=\"").count(), 1);
        assert!(body.contains("pin_version=\"4.4-stable\""));
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

    #[test]
    fn tree_search_finds_a_project_at_the_top() {
        let root = scratch("tree-top");
        write_project(&root, GODOT4);
        assert_eq!(find_project_dir_in_tree(&root), Some(root));
    }

    #[test]
    fn tree_search_finds_a_nested_project() {
        let root = scratch("tree-nested");
        let nested = root.join("game").join("client");
        fs::create_dir_all(&nested).unwrap();
        write_project(&nested, GODOT4);
        assert_eq!(find_project_dir_in_tree(&root), Some(nested));
    }

    #[test]
    fn tree_search_prefers_the_shallowest_project() {
        // A repository may hold more than one project.godot. The one nearest the
        // top is the project, so a deeper one must not win.
        let root = scratch("tree-shallow");
        let shallow = root.join("main");
        let deep = root.join("addons").join("sample").join("demo");
        fs::create_dir_all(&shallow).unwrap();
        fs::create_dir_all(&deep).unwrap();
        write_project(&shallow, GODOT4);
        write_project(&deep, GODOT4);
        assert_eq!(find_project_dir_in_tree(&root), Some(shallow));
    }

    #[test]
    fn tree_search_skips_hidden_folders() {
        // A project.godot inside a hidden folder such as .git is not a real
        // project and must be ignored.
        let root = scratch("tree-hidden");
        let hidden = root.join(".git").join("templates");
        fs::create_dir_all(&hidden).unwrap();
        write_project(&hidden, GODOT4);
        assert_eq!(find_project_dir_in_tree(&root), None);
    }

    #[test]
    fn tree_search_returns_none_when_nothing_is_found() {
        let root = scratch("tree-none");
        fs::create_dir_all(root.join("src").join("assets")).unwrap();
        assert_eq!(find_project_dir_in_tree(&root), None);
    }

    // Import detection.

    /// Write a texture import sidecar that declares one generated output, the way
    /// Godot 4 does, so a test can then choose to create the output or not.
    fn write_sidecar(dir: &Path, asset: &str, output: &str) {
        let body = format!(
            "[remap]\n\npath=\"{output}\"\n\n[deps]\n\nsource_file=\"res://{asset}\"\ndest_files=[\"{output}\"]\n"
        );
        fs::write(dir.join(format!("{asset}.import")), body).unwrap();
    }

    #[test]
    fn a_project_with_no_assets_needs_no_import() {
        // Nothing to import, so the project is ready to run as is.
        let dir = scratch("import-no-assets");
        write_project(&dir, GODOT4);
        let project = GodotProject::load(&dir).unwrap();
        assert!(project.is_imported());
    }

    #[test]
    fn a_missing_generated_output_means_not_imported() {
        // The sidecar travels with the checkout but its output does not, which is
        // exactly the fresh clone case.
        let dir = scratch("import-missing-output");
        write_project(&dir, GODOT4);
        write_sidecar(
            &dir,
            "icon.svg",
            "res://.godot/imported/icon.svg-abc123.ctex",
        );
        let project = GodotProject::load(&dir).unwrap();
        assert!(!project.is_imported());
    }

    #[test]
    fn a_present_generated_output_means_imported() {
        let dir = scratch("import-present-output");
        write_project(&dir, GODOT4);
        write_sidecar(
            &dir,
            "icon.svg",
            "res://.godot/imported/icon.svg-abc123.ctex",
        );
        fs::create_dir_all(dir.join(".godot").join("imported")).unwrap();
        fs::write(
            dir.join(".godot")
                .join("imported")
                .join("icon.svg-abc123.ctex"),
            b"",
        )
        .unwrap();
        let project = GodotProject::load(&dir).unwrap();
        assert!(project.is_imported());
    }

    #[test]
    fn the_godot_folder_alone_is_not_enough() {
        // A .godot folder can exist while an output is still missing. The old
        // check was fooled by this, so guard against a regression.
        let dir = scratch("import-godot-folder-only");
        write_project(&dir, GODOT4);
        write_sidecar(
            &dir,
            "icon.svg",
            "res://.godot/imported/icon.svg-abc123.ctex",
        );
        fs::create_dir_all(dir.join(".godot").join("imported")).unwrap();
        let project = GodotProject::load(&dir).unwrap();
        assert!(!project.is_imported());
    }

    #[test]
    fn a_nested_asset_output_is_checked() {
        // Assets sit below the top, so the scan must reach a sidecar in a subfolder.
        let dir = scratch("import-nested");
        write_project(&dir, GODOT4);
        let assets = dir.join("assets").join("sprites");
        fs::create_dir_all(&assets).unwrap();
        write_sidecar(
            &assets,
            "hero.png",
            "res://.godot/imported/hero.png-deadbeef.ctex",
        );
        let project = GodotProject::load(&dir).unwrap();
        assert!(!project.is_imported());
    }

    #[test]
    fn a_godot_3_import_cache_is_recognized() {
        let dir = scratch("import-godot3");
        write_project(&dir, "config_version=4\n[application]\nconfig/name=\"X\"\n");
        write_sidecar(&dir, "icon.png", "res://.import/icon.png-feed.stex");
        let project = GodotProject::load(&dir).unwrap();
        assert!(!project.is_imported());
        fs::create_dir_all(dir.join(".import")).unwrap();
        fs::write(dir.join(".import").join("icon.png-feed.stex"), b"").unwrap();
        let imported = GodotProject::load(&dir).unwrap();
        assert!(imported.is_imported());
    }

    #[test]
    fn an_unreadable_sidecar_is_treated_as_needing_import() {
        // imported_outputs sees no declared outputs in junk, so a sidecar with no
        // parseable output does not falsely count as imported on its own. A real
        // sidecar always names at least one output.
        let dir = scratch("import-empty-sidecar");
        write_project(&dir, GODOT4);
        // A sidecar that declares an output the project never generated.
        write_sidecar(
            &dir,
            "music.ogg",
            "res://.godot/imported/music.ogg-1.oggvorbisstr",
        );
        let project = GodotProject::load(&dir).unwrap();
        assert!(!project.is_imported());
    }

    #[test]
    fn imported_outputs_pulls_only_cache_paths() {
        let sidecar = "[remap]\n\npath=\"res://.godot/imported/icon.svg-abc.ctex\"\nuid=\"uid://xyz\"\n\n[deps]\n\nsource_file=\"res://icon.svg\"\ndest_files=[\"res://.godot/imported/icon.svg-abc.ctex\"]\n";
        let outputs = imported_outputs(sidecar);
        // Both the path and the dest_files entry are caught, and the source and uid
        // are left out.
        assert_eq!(
            outputs,
            vec![
                "res://.godot/imported/icon.svg-abc.ctex".to_string(),
                "res://.godot/imported/icon.svg-abc.ctex".to_string(),
            ]
        );
    }

    #[test]
    fn tree_search_stops_at_the_depth_bound() {
        // A project buried below the search bound is not found, so a deeply nested
        // tree cannot make the search run long.
        let root = scratch("tree-deep");
        let mut deep = root.clone();
        for level in 0..(MAX_SEARCH_DEPTH + 2) {
            deep = deep.join(format!("d{level}"));
        }
        fs::create_dir_all(&deep).unwrap();
        write_project(&deep, GODOT4);
        assert_eq!(find_project_dir_in_tree(&root), None);
    }
}
