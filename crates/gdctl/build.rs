//! Build script.
//!
//! It works out the version to compile in and hands it to the crate through an
//! environment variable, so a local build reports a real version without anyone
//! editing the source. A release passes GODELLO_VERSION and that wins. Otherwise
//! the version is read from git. On Windows it also writes the version into the
//! binary resource so the file properties dialog shows it.

fn main() {
    // A release sets this, and a changed value must force a rebuild so the new
    // version is compiled in even when an earlier build is cached.
    println!("cargo:rerun-if-env-changed=GODELLO_VERSION");
    watch_git();

    let version = resolve_version();
    // Expose the resolved version to the crate. The crate reads it with
    // option_env, so this is what its version flag and the GUI report.
    println!("cargo:rustc-env=GODELLO_VERSION={version}");

    embed_windows_version(&version);
}

/// Pick the version to compile in. A release version passed through the
/// environment wins. Otherwise we ask git. With no git at all we fall back to the
/// crate version so the build still has something to report.
fn resolve_version() -> String {
    if let Ok(version) = std::env::var("GODELLO_VERSION") {
        let version = version.trim();
        if !version.is_empty() {
            return version.to_string();
        }
    }
    if let Some(version) = git_version() {
        return version;
    }
    std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string())
}

/// Describe the checkout as a version. A tagged commit gives the tag, a later
/// commit gives the tag plus how far past it, and a dirty tree is marked. With no
/// tags yet we pair the short commit with the crate version for a readable base.
fn git_version() -> Option<String> {
    // Asking for a tag without the always fallback fails cleanly when there are
    // no tags, which is how we tell a real tag from a bare commit.
    if let Some(tag) = git(&["describe", "--tags", "--dirty"]) {
        return Some(tag.trim_start_matches('v').to_string());
    }
    let commit = git(&["describe", "--always", "--dirty"])?;
    let base = std::env::var("CARGO_PKG_VERSION").ok()?;
    Some(format!("{base}+{commit}"))
}

/// Rebuild when the checkout moves to a new commit or stages a change, so the git
/// version stays current. Unstaged edits are not watched, so the dirty mark can
/// lag until the next commit or stage.
fn watch_git() {
    if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/index");
    }
}

/// Run a git command and return its trimmed output, or None when git is missing,
/// fails, or says nothing. This keeps a build outside a git checkout working.
fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(text)
}

/// Write the product and file version into the Windows binary resource so the file
/// properties dialog shows it. This matches what the binary reports through its
/// version flag.
#[cfg(windows)]
fn embed_windows_version(version: &str) {
    let packed = packed_version(version);
    let mut resource = winresource::WindowsResource::new();
    resource.set("ProductName", "Godello");
    resource.set(
        "FileDescription",
        "Godello: a Godot engine and project launcher",
    );
    resource.set("ProductVersion", version);
    resource.set("FileVersion", version);
    resource.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
    resource.set_version_info(winresource::VersionInfo::FILEVERSION, packed);

    if let Err(err) = resource.compile() {
        // A missing resource compiler should not fail the whole build. The binary
        // still reports its version through the version flag.
        println!("cargo:warning=could not embed the Windows version resource: {err}");
    }
}

/// On other platforms there is no binary resource to write, so this does nothing.
#[cfg(not(windows))]
fn embed_windows_version(_version: &str) {}

/// Pack a version string into the four part number the Windows resource needs.
/// Any prerelease or build suffix is dropped and a missing part becomes zero, so
/// a value like 1.2.3 or 1.2.3-rc1 both pack to the same numeric version.
#[cfg(windows)]
fn packed_version(version: &str) -> u64 {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major << 48) | (minor << 32) | (patch << 16)
}
