//! Tracking external tools and opening projects in them.
//!
//! The app keeps the path to a handful of tools it can use, such as git, the
//! dotnet command, and a few code editors. Paths are found by scanning the PATH
//! and a small set of well known install locations, so a tool the user already
//! has is picked up without any setup. A found path is saved, and on each start
//! it is checked and found again if it moved. The editors can open a project,
//! which is how the project menu offers to open a project in them.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Settings;
use crate::launch::{LaunchError, Launcher};

/// A known external tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    /// The git version control command.
    Git,
    /// The dotnet command line tool.
    Dotnet,
    /// The VS Code editor.
    VsCode,
    /// The Visual Studio IDE.
    VisualStudio,
    /// The JetBrains Rider IDE.
    Rider,
}

impl Tool {
    /// Every tool, in the order the settings screen shows them.
    pub const ALL: [Tool; 5] = [
        Tool::Git,
        Tool::Dotnet,
        Tool::VsCode,
        Tool::VisualStudio,
        Tool::Rider,
    ];

    /// The editors, in the order the project menu offers them.
    pub const EDITORS: [Tool; 3] = [Tool::VsCode, Tool::VisualStudio, Tool::Rider];

    /// The short stable key used to store the tool path in settings.
    pub fn key(self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Dotnet => "dotnet",
            Tool::VsCode => "vscode",
            Tool::VisualStudio => "visual_studio",
            Tool::Rider => "rider",
        }
    }

    /// Map a stored key back to a tool, for reading settings.
    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.key() == key)
    }

    /// The name shown in the settings screen and the menus.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Git => "Git",
            Tool::Dotnet => "dotnet",
            Tool::VsCode => "VS Code",
            Tool::VisualStudio => "Visual Studio",
            Tool::Rider => "Rider",
        }
    }

    /// True when the tool is a code editor a project can be opened in.
    pub fn is_editor(self) -> bool {
        matches!(self, Tool::VsCode | Tool::VisualStudio | Tool::Rider)
    }

    /// True when the tool only makes sense for a project that uses C#, so it is
    /// hidden for a project with no solution.
    pub fn needs_csharp(self) -> bool {
        matches!(self, Tool::VisualStudio | Tool::Rider)
    }

    /// The executable names to look for on the PATH, in order of preference, for
    /// the current platform. An empty list means the tool is not expected here,
    /// for example Visual Studio away from Windows.
    fn executable_names(self) -> &'static [&'static str] {
        match self {
            Tool::Git => {
                if cfg!(windows) {
                    &["git.exe"]
                } else {
                    &["git"]
                }
            }
            Tool::Dotnet => {
                if cfg!(windows) {
                    &["dotnet.exe"]
                } else {
                    &["dotnet"]
                }
            }
            Tool::VsCode => {
                if cfg!(windows) {
                    &["code.cmd", "code.exe", "code"]
                } else {
                    &["code"]
                }
            }
            Tool::VisualStudio => {
                if cfg!(windows) {
                    &["devenv.exe"]
                } else {
                    &[]
                }
            }
            Tool::Rider => {
                if cfg!(windows) {
                    &["rider64.exe", "rider.exe"]
                } else {
                    &["rider", "rider.sh"]
                }
            }
        }
    }

    /// Well known absolute locations to check when the tool is not on the PATH,
    /// for the current platform. These cover common installs that do not add a
    /// command to the PATH, such as macOS app bundles and the JetBrains Toolbox.
    fn extra_locations(self) -> Vec<PathBuf> {
        match self {
            Tool::VsCode if cfg!(target_os = "macos") => vec![PathBuf::from(
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
            )],
            Tool::Rider if cfg!(target_os = "macos") => {
                vec![PathBuf::from(
                    "/Applications/Rider.app/Contents/MacOS/rider",
                )]
            }
            Tool::Rider if cfg!(target_os = "linux") => match std::env::var_os("HOME") {
                Some(home) => {
                    vec![PathBuf::from(&home).join(".local/share/JetBrains/Toolbox/scripts/rider")]
                }
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

/// Find a tool's executable, returning the first match. The PATH is searched
/// first, then a few well known locations for tools that do not add a command to
/// the PATH. Returns None when nothing is found.
pub fn find_tool(tool: Tool) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    locate(
        tool.executable_names(),
        &dirs,
        &tool.extra_locations(),
        |candidate| candidate.is_file(),
    )
}

/// The search behind find_tool, with the directories, extra locations, and the
/// file check passed in so it can be tested without the real environment. It
/// checks each name in each directory, then each extra location, and returns the
/// first that the check accepts.
fn locate(
    names: &[&str],
    dirs: &[PathBuf],
    extras: &[PathBuf],
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if is_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    for extra in extras {
        if is_file(extra) {
            return Some(extra.clone());
        }
    }
    None
}

/// Resolve a tool's path. A path that is set and still exists is kept. Otherwise
/// the tool is searched for. When nothing is found the result is None, so the
/// path is left blank. The current argument is the stored path, if any.
pub fn resolve_tool(tool: Tool, current: Option<&Path>) -> Option<PathBuf> {
    resolve_with(current, |path| path.exists(), || find_tool(tool))
}

/// The resolve logic with the existence check and the search passed in, so it can
/// be tested without the real environment.
fn resolve_with(
    current: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
    search: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = current {
        if exists(path) {
            return Some(path.to_path_buf());
        }
    }
    search()
}

/// Resolve every tool path against the system: keep a stored path that still
/// exists, otherwise search, otherwise leave it unset. Returns true when any path
/// changed, so the caller can save the settings.
pub fn resolve_tools(settings: &mut Settings) -> bool {
    let mut changed = false;
    for tool in Tool::ALL {
        let current = settings.tool_path(tool).map(Path::to_path_buf);
        let resolved = resolve_tool(tool, current.as_deref());
        if resolved != current {
            settings.set_tool_path(tool, resolved);
            changed = true;
        }
    }
    changed
}

/// Open a project in an editor, detached so the launcher does not wait on it. The
/// program is the editor executable found earlier and stored in settings. The
/// path opened is chosen by open_target.
pub fn open_in_tool(
    tool: Tool,
    program: &Path,
    project_dir: &Path,
    launcher: &impl Launcher,
) -> Result<(), LaunchError> {
    let target = open_target(tool, project_dir);
    launcher.launch(program.as_os_str(), &[target.into_os_string()], true)
}

/// The path to open in an editor for a project. Visual Studio and Rider open a
/// solution when there is one, so the user lands on the solution rather than a
/// bare folder. The others open the project folder.
pub fn open_target(tool: Tool, project_dir: &Path) -> PathBuf {
    if tool.needs_csharp() {
        if let Some(solution) = find_solution(project_dir) {
            return solution;
        }
    }
    project_dir.to_path_buf()
}

/// Find a Visual Studio solution in a folder, the lowest named one when there is
/// more than one, so the choice is stable.
fn find_solution(dir: &Path) -> Option<PathBuf> {
    let mut solutions: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_sln = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("sln"))
                .unwrap_or(false);
            if is_sln {
                solutions.push(path);
            }
        }
    }
    solutions.sort();
    solutions.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("godello-tools-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Tool metadata.

    #[test]
    fn keys_round_trip_through_from_key() {
        for tool in Tool::ALL {
            assert_eq!(Tool::from_key(tool.key()), Some(tool));
        }
        assert_eq!(Tool::from_key("nope"), None);
    }

    #[test]
    fn editors_and_csharp_flags_are_right() {
        assert!(Tool::VsCode.is_editor());
        assert!(Tool::VisualStudio.is_editor());
        assert!(Tool::Rider.is_editor());
        assert!(!Tool::Git.is_editor());
        assert!(!Tool::Dotnet.is_editor());
        // Only the .NET focused IDEs need a solution.
        assert!(Tool::VisualStudio.needs_csharp());
        assert!(Tool::Rider.needs_csharp());
        assert!(!Tool::VsCode.needs_csharp());
    }

    // Locating an executable.

    #[test]
    fn locate_finds_a_name_in_a_listed_dir() {
        let dirs = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let found = locate(&["code"], &dirs, &[], |c| c == Path::new("/b/code"));
        assert_eq!(found, Some(PathBuf::from("/b/code")));
    }

    #[test]
    fn locate_prefers_the_earlier_name() {
        // Both names exist, so the first listed name wins.
        let dirs = vec![PathBuf::from("/x")];
        let found = locate(&["rider64.exe", "rider.exe"], &dirs, &[], |_| true);
        assert_eq!(found, Some(PathBuf::from("/x/rider64.exe")));
    }

    #[test]
    fn locate_falls_back_to_an_extra_location() {
        let extra = PathBuf::from("/opt/app/bin/code");
        let found = locate(
            &["code"],
            &[PathBuf::from("/usr/bin")],
            std::slice::from_ref(&extra),
            |c| c == extra,
        );
        assert_eq!(found, Some(extra));
    }

    #[test]
    fn locate_returns_none_when_nothing_matches() {
        let dirs = vec![PathBuf::from("/a")];
        assert_eq!(locate(&["git"], &dirs, &[], |_| false), None);
    }

    #[test]
    fn locate_finds_nothing_with_no_names() {
        // Visual Studio has no names away from Windows, so the scan finds nothing.
        let dirs = vec![PathBuf::from("/a")];
        assert_eq!(locate(&[], &dirs, &[], |_| true), None);
    }

    // Resolving a stored path.

    #[test]
    fn resolve_keeps_a_path_that_still_exists() {
        let kept = resolve_with(
            Some(Path::new("/here/git")),
            |_| true,
            || panic!("must not search when the path still exists"),
        );
        assert_eq!(kept, Some(PathBuf::from("/here/git")));
    }

    #[test]
    fn resolve_searches_when_the_path_is_gone() {
        let found = resolve_with(
            Some(Path::new("/gone/git")),
            |_| false,
            || Some(PathBuf::from("/usr/bin/git")),
        );
        assert_eq!(found, Some(PathBuf::from("/usr/bin/git")));
    }

    #[test]
    fn resolve_searches_when_nothing_is_stored() {
        let found = resolve_with(None, |_| true, || Some(PathBuf::from("/usr/bin/git")));
        assert_eq!(found, Some(PathBuf::from("/usr/bin/git")));
    }

    #[test]
    fn resolve_is_none_when_the_search_finds_nothing() {
        let found = resolve_with(None, |_| true, || None);
        assert_eq!(found, None);
    }

    // Opening a project in an editor.

    #[test]
    fn open_target_uses_the_solution_for_dotnet_ides() {
        let dir = scratch("open-target-sln");
        fs::write(dir.join("Game.sln"), b"").unwrap();
        assert_eq!(open_target(Tool::Rider, &dir), dir.join("Game.sln"));
        assert_eq!(open_target(Tool::VisualStudio, &dir), dir.join("Game.sln"));
    }

    #[test]
    fn open_target_uses_the_folder_without_a_solution() {
        let dir = scratch("open-target-nosln");
        assert_eq!(open_target(Tool::Rider, &dir), dir);
    }

    #[test]
    fn open_target_uses_the_folder_for_plain_editors() {
        // VS Code opens the folder even when a solution is present.
        let dir = scratch("open-target-vscode");
        fs::write(dir.join("Game.sln"), b"").unwrap();
        assert_eq!(open_target(Tool::VsCode, &dir), dir);
    }

    #[test]
    fn find_solution_picks_the_lowest_named_one() {
        let dir = scratch("find-sln");
        fs::write(dir.join("Beta.sln"), b"").unwrap();
        fs::write(dir.join("Alpha.sln"), b"").unwrap();
        fs::write(dir.join("notes.txt"), b"").unwrap();
        assert_eq!(find_solution(&dir), Some(dir.join("Alpha.sln")));
    }

    #[test]
    fn find_solution_is_none_without_one() {
        let dir = scratch("find-sln-none");
        fs::write(dir.join("readme.md"), b"").unwrap();
        assert_eq!(find_solution(&dir), None);
    }
}
