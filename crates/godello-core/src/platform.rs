//! Host detection and platform specific paths.
//!
//! This module knows the host operating system and cpu arch, builds the target
//! used to pick a download, and resolves the engine executable inside an
//! installed version. The executable layout differs per system, so each case is
//! handled here in one place.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::version::Variant;

/// A supported operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Linux,
    Windows,
    Mac,
}

impl Os {
    /// The token used in asset file names and platform keys.
    pub fn tag(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Windows => "windows",
            Os::Mac => "macos",
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// A supported cpu arch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    X86,
    Arm64,
}

impl Arch {
    /// The token used in asset file names.
    pub fn tag(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::X86 => "x86_32",
            Arch::Arm64 => "arm64",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// The operating system this build is running on.
pub fn current_os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

/// The cpu arch this build is running on.
pub fn current_arch() -> Arch {
    if cfg!(target_arch = "x86_64") {
        Arch::X86_64
    } else if cfg!(any(target_arch = "aarch64", target_arch = "arm")) {
        Arch::Arm64
    } else {
        Arch::X86
    }
}

/// What to download for. The operating system, the cpu arch, and the build
/// flavor together decide which release asset to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Target {
    pub os: Os,
    pub arch: Arch,
    pub variant: Variant,
}

impl Target {
    pub fn new(os: Os, arch: Arch, variant: Variant) -> Self {
        Target { os, arch, variant }
    }

    /// The target for the current host with the given variant.
    pub fn current(variant: Variant) -> Self {
        Target {
            os: current_os(),
            arch: current_arch(),
            variant,
        }
    }
}

/// Find the engine executable inside an installed version on the current host.
/// For an editor launch pass console as false. Pass true only when the console
/// build is wanted on Windows.
pub fn find_executable(install_dir: &Path, console: bool) -> Result<PathBuf, PlatformError> {
    find_executable_for(install_dir, current_os(), console)
}

/// The same as find_executable but the operating system is given. This keeps the
/// selection rules testable on any host.
pub fn find_executable_for(
    install_dir: &Path,
    os: Os,
    console: bool,
) -> Result<PathBuf, PlatformError> {
    match os {
        Os::Windows => find_windows_executable(install_dir, console),
        Os::Mac => find_mac_executable(install_dir),
        Os::Linux => find_linux_executable(install_dir),
    }
}

/// Read the direct entries of a directory, sorted by file name so the result is
/// stable across runs.
fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, PlatformError> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(PlatformError::Io)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();
    Ok(entries)
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

/// On Windows the editor is an exe. There is a normal build and a console build
/// whose name contains console. Pick the one that matches what was asked for.
fn find_windows_executable(dir: &Path, console: bool) -> Result<PathBuf, PlatformError> {
    let entries = sorted_entries(dir)?;
    let exes: Vec<PathBuf> = entries
        .into_iter()
        .filter(|path| path.is_file() && file_name_of(path).to_ascii_lowercase().ends_with(".exe"))
        .collect();
    let is_console = |path: &Path| file_name_of(path).to_ascii_lowercase().contains("console");
    let chosen = if console {
        exes.iter().find(|path| is_console(path))
    } else {
        exes.iter().find(|path| !is_console(path))
    };
    chosen
        .or_else(|| exes.first())
        .cloned()
        .ok_or_else(|| PlatformError::NotFound(dir.to_path_buf()))
}

/// On Mac the build is an app bundle. The runnable binary lives inside it under
/// Contents/MacOS. Prefer a file whose name starts with Godot.
fn find_mac_executable(dir: &Path) -> Result<PathBuf, PlatformError> {
    let app = sorted_entries(dir)?
        .into_iter()
        .find(|path| path.is_dir() && file_name_of(path).to_ascii_lowercase().ends_with(".app"))
        .ok_or_else(|| PlatformError::NotFound(dir.to_path_buf()))?;
    let macos_dir = app.join("Contents").join("MacOS");
    if !macos_dir.is_dir() {
        return Err(PlatformError::NotFound(macos_dir));
    }
    let binaries = sorted_entries(&macos_dir)?;
    binaries
        .iter()
        .find(|path| path.is_file() && file_name_of(path).starts_with("Godot"))
        .or_else(|| binaries.iter().find(|path| path.is_file()))
        .cloned()
        .ok_or(PlatformError::NotFound(macos_dir))
}

/// On Linux the build is a single binary whose name starts with Godot and ends
/// with an arch suffix. Prefer a name that matches the host arch.
fn find_linux_executable(dir: &Path) -> Result<PathBuf, PlatformError> {
    let candidates: Vec<PathBuf> = sorted_entries(dir)?
        .into_iter()
        .filter(|path| path.is_file() && file_name_of(path).starts_with("Godot"))
        .collect();
    let arch = current_arch().tag();
    candidates
        .iter()
        .find(|path| file_name_of(path).contains(arch))
        .or_else(|| candidates.first())
        .cloned()
        .ok_or_else(|| PlatformError::NotFound(dir.to_path_buf()))
}

/// An error from resolving an executable.
#[derive(Debug)]
pub enum PlatformError {
    /// No engine executable was found under the given path.
    NotFound(PathBuf),
    /// A filesystem call failed.
    Io(std::io::Error),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::NotFound(path) => {
                write!(f, "no engine executable found under {}", path.display())
            }
            PlatformError::Io(err) => write!(f, "filesystem error: {err}"),
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PlatformError::Io(err) => Some(err),
            PlatformError::NotFound(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Make a unique scratch directory for a test and remove any old copy.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("godello-platform-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn current_host_is_known() {
        // These just need to resolve to a value without panic.
        let _ = current_os();
        let _ = current_arch();
        let target = Target::current(Variant::Standard);
        assert_eq!(target.variant, Variant::Standard);
    }

    #[test]
    fn windows_prefers_the_normal_exe_then_console() {
        let dir = scratch("windows");
        touch(&dir.join("Godot_v4.3-stable_win64.exe"));
        touch(&dir.join("Godot_v4.3-stable_win64_console.exe"));

        let editor = find_executable_for(&dir, Os::Windows, false).unwrap();
        assert_eq!(file_name_of(&editor), "Godot_v4.3-stable_win64.exe");

        let console = find_executable_for(&dir, Os::Windows, true).unwrap();
        assert_eq!(
            file_name_of(&console),
            "Godot_v4.3-stable_win64_console.exe"
        );
    }

    #[test]
    fn mac_looks_inside_the_app_bundle() {
        let dir = scratch("mac");
        touch(&dir.join("Godot.app/Contents/MacOS/Godot"));
        touch(&dir.join("Godot.app/Contents/Info.plist"));

        let exe = find_executable_for(&dir, Os::Mac, false).unwrap();
        assert!(exe.ends_with("Godot.app/Contents/MacOS/Godot"));
    }

    #[test]
    fn linux_finds_the_godot_binary() {
        let dir = scratch("linux");
        touch(&dir.join(format!("Godot_v4.3-stable_linux.{}", current_arch().tag())));
        touch(&dir.join("README.txt"));

        let exe = find_executable_for(&dir, Os::Linux, false).unwrap();
        assert!(file_name_of(&exe).starts_with("Godot_v4.3-stable_linux"));
    }

    #[test]
    fn missing_executable_is_an_error() {
        let dir = scratch("empty");
        let result = find_executable_for(&dir, Os::Linux, false);
        assert!(matches!(result, Err(PlatformError::NotFound(_))));
    }

    /// An arch tag that is not the host arch, used to test arch preference in a
    /// way that does not depend on the machine running the test.
    fn non_host_arch_tag() -> &'static str {
        match current_arch() {
            Arch::X86_64 => "arm64",
            Arch::X86 => "x86_64",
            Arch::Arm64 => "x86_64",
        }
    }

    #[test]
    fn reading_a_missing_directory_is_an_error() {
        let dir = scratch("gone").join("does-not-exist");
        let result = find_executable_for(&dir, Os::Linux, false);
        assert!(result.is_err());
    }

    // Windows cases.

    #[test]
    fn windows_falls_back_to_console_when_only_console_exists() {
        let dir = scratch("win-only-console");
        touch(&dir.join("Godot_v4.3-stable_win64_console.exe"));
        let exe = find_executable_for(&dir, Os::Windows, false).unwrap();
        assert_eq!(file_name_of(&exe), "Godot_v4.3-stable_win64_console.exe");
    }

    #[test]
    fn windows_falls_back_to_normal_when_console_requested_but_absent() {
        let dir = scratch("win-only-normal");
        touch(&dir.join("Godot_v4.3-stable_win64.exe"));
        let exe = find_executable_for(&dir, Os::Windows, true).unwrap();
        assert_eq!(file_name_of(&exe), "Godot_v4.3-stable_win64.exe");
    }

    #[test]
    fn windows_ignores_non_exe_files() {
        let dir = scratch("win-noise");
        touch(&dir.join("Godot_v4.3-stable_win64.exe"));
        touch(&dir.join("Godot_v4.3-stable_win64.exe.pck"));
        touch(&dir.join("vulkan-1.dll"));
        touch(&dir.join("README.txt"));
        let exe = find_executable_for(&dir, Os::Windows, false).unwrap();
        assert_eq!(file_name_of(&exe), "Godot_v4.3-stable_win64.exe");
    }

    #[test]
    fn windows_matches_extension_case_insensitively() {
        let dir = scratch("win-upper");
        touch(&dir.join("Godot_v4.3-stable_win64.EXE"));
        let exe = find_executable_for(&dir, Os::Windows, false).unwrap();
        assert_eq!(file_name_of(&exe), "Godot_v4.3-stable_win64.EXE");
    }

    #[test]
    fn windows_skips_a_directory_named_like_an_exe() {
        let dir = scratch("win-dir-trap");
        std::fs::create_dir_all(dir.join("not_a_real.exe")).unwrap();
        touch(&dir.join("Godot_v4.3-stable_win64.exe"));
        let exe = find_executable_for(&dir, Os::Windows, false).unwrap();
        assert_eq!(file_name_of(&exe), "Godot_v4.3-stable_win64.exe");
    }

    #[test]
    fn windows_with_no_exe_is_an_error() {
        let dir = scratch("win-empty");
        touch(&dir.join("GodotSharp.dll"));
        let result = find_executable_for(&dir, Os::Windows, false);
        assert!(matches!(result, Err(PlatformError::NotFound(_))));
    }

    // Mac cases.

    #[test]
    fn mac_finds_the_mono_binary() {
        let dir = scratch("mac-mono");
        touch(&dir.join("Godot_mono.app/Contents/MacOS/Godot_mono"));
        let exe = find_executable_for(&dir, Os::Mac, false).unwrap();
        assert!(exe.ends_with("Godot_mono.app/Contents/MacOS/Godot_mono"));
    }

    #[test]
    fn mac_prefers_a_godot_named_binary_over_others() {
        let dir = scratch("mac-many");
        touch(&dir.join("Godot.app/Contents/MacOS/helper"));
        touch(&dir.join("Godot.app/Contents/MacOS/Godot"));
        let exe = find_executable_for(&dir, Os::Mac, false).unwrap();
        assert_eq!(file_name_of(&exe), "Godot");
    }

    #[test]
    fn mac_falls_back_to_any_binary_when_none_start_with_godot() {
        let dir = scratch("mac-fallback");
        touch(&dir.join("Godot.app/Contents/MacOS/launcher"));
        let exe = find_executable_for(&dir, Os::Mac, false).unwrap();
        assert_eq!(file_name_of(&exe), "launcher");
    }

    #[test]
    fn mac_without_an_app_bundle_is_an_error() {
        let dir = scratch("mac-noapp");
        touch(&dir.join("Godot_v4.3-stable_macos.universal"));
        let result = find_executable_for(&dir, Os::Mac, false);
        assert!(matches!(result, Err(PlatformError::NotFound(_))));
    }

    #[test]
    fn mac_app_without_macos_dir_is_not_found_not_io() {
        let dir = scratch("mac-broken");
        std::fs::create_dir_all(dir.join("Godot.app/Contents")).unwrap();
        let result = find_executable_for(&dir, Os::Mac, false);
        assert!(matches!(result, Err(PlatformError::NotFound(_))));
    }

    #[test]
    fn mac_app_with_empty_macos_dir_is_an_error() {
        let dir = scratch("mac-empty-macos");
        std::fs::create_dir_all(dir.join("Godot.app/Contents/MacOS")).unwrap();
        let result = find_executable_for(&dir, Os::Mac, false);
        assert!(matches!(result, Err(PlatformError::NotFound(_))));
    }

    // Linux cases.

    #[test]
    fn linux_prefers_the_host_arch_binary() {
        let dir = scratch("linux-arch");
        touch(&dir.join(format!("Godot_v4.3-stable_linux.{}", non_host_arch_tag())));
        touch(&dir.join(format!("Godot_v4.3-stable_linux.{}", current_arch().tag())));
        let exe = find_executable_for(&dir, Os::Linux, false).unwrap();
        assert!(file_name_of(&exe).ends_with(current_arch().tag()));
    }

    #[test]
    fn linux_mono_build_skips_the_sharp_dir_and_data() {
        let dir = scratch("linux-mono");
        // A mono build extracts to a folder with the binary, a GodotSharp dir,
        // and shared object data. Only the binary should be chosen.
        std::fs::create_dir_all(dir.join("GodotSharp")).unwrap();
        touch(&dir.join("libgodot.so"));
        touch(&dir.join(format!(
            "Godot_v4.3-stable_mono_linux.{}",
            current_arch().tag()
        )));
        let exe = find_executable_for(&dir, Os::Linux, false).unwrap();
        assert!(file_name_of(&exe).starts_with("Godot_v4.3-stable_mono_linux"));
    }

    #[test]
    fn linux_falls_back_when_no_arch_matches() {
        let dir = scratch("linux-noarch");
        touch(&dir.join(format!("Godot_v4.3-stable_linux.{}", non_host_arch_tag())));
        let exe = find_executable_for(&dir, Os::Linux, false).unwrap();
        assert!(file_name_of(&exe).starts_with("Godot_v4.3-stable_linux"));
    }

    #[test]
    fn linux_x86_64_and_x86_32_do_not_collide() {
        // The 64 bit lookup must not match the 32 bit file, and the reverse.
        let dir = scratch("linux-bits");
        touch(&dir.join("Godot_v4.3-stable_linux.x86_32"));
        touch(&dir.join("Godot_v4.3-stable_linux.x86_64"));
        let exe = find_executable_for(&dir, Os::Linux, false).unwrap();
        let name = file_name_of(&exe);
        if current_arch() == Arch::X86_64 {
            assert!(name.ends_with("x86_64"));
        } else if current_arch() == Arch::X86 {
            assert!(name.ends_with("x86_32"));
        }
    }

    #[test]
    fn linux_skips_directories_starting_with_godot() {
        let dir = scratch("linux-dir-trap");
        std::fs::create_dir_all(dir.join("GodotData")).unwrap();
        touch(&dir.join(format!("Godot_v4.3-stable_linux.{}", current_arch().tag())));
        let exe = find_executable_for(&dir, Os::Linux, false).unwrap();
        assert!(exe.is_file());
    }
}
