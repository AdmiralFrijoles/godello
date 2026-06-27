//! Installing and managing engine versions on disk.
//!
//! This module owns the layout of installed engines, the download and verify and
//! extract flow, and the list and remove operations. Engines live under
//! engines_root/variant/version. The byte download goes through the Downloader
//! trait so the real http client lives in the binary, while verify, extract,
//! list, and remove are local and tested here with no network.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256, Sha512};

use crate::platform::{self, PlatformError};
use crate::repository::{Asset, Checksum, ChecksumAlgorithm};
use crate::version::{GodotVersion, Variant};

/// One engine version found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledEngine {
    pub variant: Variant,
    pub version: GodotVersion,
    pub path: PathBuf,
}

/// Receives download progress as bytes arrive. The total is optional because a
/// server may not send a content length. Every method takes a shared reference
/// so one sink can be passed through a download by reference.
///
/// Sinks must be safe to share across threads. A download future may run on a
/// multi thread executor and hold the sink by reference across an await, so the
/// reference has to be Send, which needs the sink to be Sync.
pub trait DownloadProgress: Send + Sync {
    /// Called once before any bytes, with the total size when it is known.
    fn start(&self, total: Option<u64>);
    /// Called as bytes arrive, with the running total downloaded so far.
    fn update(&self, downloaded: u64);
    /// Called once when the download ends, whether it finished or failed.
    fn finish(&self);
}

/// A progress sink that ignores every update. Used when no display is wanted,
/// for example in tests or a quiet run.
pub struct NoProgress;

impl DownloadProgress for NoProgress {
    fn start(&self, _total: Option<u64>) {}
    fn update(&self, _downloaded: u64) {}
    fn finish(&self) {}
}

/// Fetches a url to a local path, reporting progress as it goes. The binary
/// supplies a real client. Tests supply a fake one. Kept separate from the text
/// HttpClient because engine downloads are binary and large.
#[allow(async_fn_in_trait)]
pub trait Downloader {
    async fn download_to(
        &self,
        url: &str,
        dest: &Path,
        progress: &dyn DownloadProgress,
    ) -> Result<(), InstallError>;
}

/// Manages the on disk set of installed engines.
pub struct InstallManager {
    engines_root: PathBuf,
    downloads_dir: PathBuf,
}

impl InstallManager {
    pub fn new(engines_root: impl Into<PathBuf>, downloads_dir: impl Into<PathBuf>) -> Self {
        InstallManager {
            engines_root: engines_root.into(),
            downloads_dir: downloads_dir.into(),
        }
    }

    /// Where a given version is or would be installed.
    pub fn install_dir(&self, variant: Variant, version: GodotVersion) -> PathBuf {
        self.engines_root
            .join(variant.as_str())
            .join(version.to_tag())
    }

    /// True when this version is already on disk.
    pub fn is_installed(&self, variant: Variant, version: GodotVersion) -> bool {
        self.install_dir(variant, version).is_dir()
    }

    /// Every installed engine found under the engines root. Folders that do not
    /// name a known variant and version are skipped so stray files cannot break
    /// the listing.
    pub fn list_installed(&self) -> Result<Vec<InstalledEngine>, InstallError> {
        let mut found = Vec::new();
        if !self.engines_root.is_dir() {
            return Ok(found);
        }
        for variant_entry in read_dir_sorted(&self.engines_root)? {
            let Some(variant) = dir_name(&variant_entry).and_then(|n| n.parse::<Variant>().ok())
            else {
                continue;
            };
            if !variant_entry.is_dir() {
                continue;
            }
            for version_entry in read_dir_sorted(&variant_entry)? {
                let Some(version) =
                    dir_name(&version_entry).and_then(|n| GodotVersion::parse_tag(n).ok())
                else {
                    continue;
                };
                if version_entry.is_dir() {
                    found.push(InstalledEngine {
                        variant,
                        version,
                        path: version_entry,
                    });
                }
            }
        }
        Ok(found)
    }

    /// Remove an installed version. Errors when it is not installed.
    pub fn remove(&self, variant: Variant, version: GodotVersion) -> Result<(), InstallError> {
        let dir = self.install_dir(variant, version);
        if !dir.is_dir() {
            return Err(InstallError::NotInstalled { variant, version });
        }
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// The engine executable for an installed version. Pass console true only to
    /// get the Windows console build.
    pub fn executable(
        &self,
        variant: Variant,
        version: GodotVersion,
        console: bool,
    ) -> Result<PathBuf, InstallError> {
        let dir = self.install_dir(variant, version);
        if !dir.is_dir() {
            return Err(InstallError::NotInstalled { variant, version });
        }
        platform::find_executable(&dir, console).map_err(InstallError::Executable)
    }

    /// Download and verify a version, leaving the archive ready to install.
    ///
    /// This is the async half of an install. Returns the archive path on disk.
    /// The extract step is install_archive, kept separate so a caller can run the
    /// blocking extract off the async executor and cancel it.
    pub async fn fetch<D: Downloader>(
        &self,
        asset: &Asset,
        variant: Variant,
        version: GodotVersion,
        downloader: &D,
        progress: &dyn DownloadProgress,
    ) -> Result<PathBuf, InstallError> {
        let target_dir = self.install_dir(variant, version);
        if target_dir.exists() {
            return Err(InstallError::AlreadyInstalled { variant, version });
        }

        fs::create_dir_all(&self.downloads_dir)?;
        let archive_path = self.downloads_dir.join(&asset.file_name);
        let partial_path = with_added_extension(&archive_path, "partial");

        downloader
            .download_to(&asset.url, &partial_path, progress)
            .await?;

        // The verify is deliberately not here. It is in install_archive so the
        // whole blocking tail (verify and extract) runs off the async executor.
        fs::rename(&partial_path, &archive_path)?;
        Ok(archive_path)
    }

    /// Extract a fetched archive into place. The work happens in a temp folder
    /// renamed into the final path at the end, so a failed or cancelled install
    /// never leaves a half written version behind.
    ///
    /// This is synchronous and blocking. The cancelled flag is checked between
    /// archive entries, so setting it stops the extract promptly and cleans up.
    pub fn install_archive(
        &self,
        archive_path: &Path,
        variant: Variant,
        version: GodotVersion,
        checksum: Option<&Checksum>,
        cancelled: &AtomicBool,
    ) -> Result<InstalledEngine, InstallError> {
        // Verify here, in the blocking phase, so hashing a large file does not
        // stall the async executor. A bad archive is deleted so a retry refetches.
        if let Some(checksum) = checksum {
            if let Err(err) = verify_file(archive_path, checksum) {
                let _ = fs::remove_file(archive_path);
                return Err(err);
            }
        }

        let target_dir = self.install_dir(variant, version);
        let variant_dir = self.engines_root.join(variant.as_str());
        fs::create_dir_all(&variant_dir)?;
        let temp_dir = variant_dir.join(format!(".{}.tmp", version.to_tag()));
        let _ = fs::remove_dir_all(&temp_dir);

        if let Err(err) = extract_zip_cancelable(archive_path, &temp_dir, cancelled) {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(err);
        }
        fs::rename(&temp_dir, &target_dir)?;

        Ok(InstalledEngine {
            variant,
            version,
            path: target_dir,
        })
    }

    /// Download, verify, and extract a version into place, in one call. This
    /// cannot be cancelled, which suits the command line. The desktop app uses
    /// fetch and install_archive so it can cancel and stay responsive.
    pub async fn install<D: Downloader>(
        &self,
        asset: &Asset,
        variant: Variant,
        version: GodotVersion,
        downloader: &D,
        progress: &dyn DownloadProgress,
    ) -> Result<InstalledEngine, InstallError> {
        let archive_path = self
            .fetch(asset, variant, version, downloader, progress)
            .await?;
        self.install_archive(
            &archive_path,
            variant,
            version,
            asset.checksum.as_ref(),
            &AtomicBool::new(false),
        )
    }
}

/// Check a file against a checksum. The algorithm is taken from the checksum.
pub fn verify_file(path: &Path, checksum: &Checksum) -> Result<(), InstallError> {
    let mut file = File::open(path)?;
    let actual = match checksum.algorithm {
        ChecksumAlgorithm::Sha256 => hash_file::<Sha256>(&mut file)?,
        ChecksumAlgorithm::Sha512 => hash_file::<Sha512>(&mut file)?,
    };
    if actual.eq_ignore_ascii_case(&checksum.hex) {
        Ok(())
    } else {
        Err(InstallError::ChecksumMismatch {
            expected: checksum.hex.clone(),
            actual,
        })
    }
}

fn hash_file<D: Digest>(file: &mut File) -> io::Result<String> {
    let mut hasher = D::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Extract a zip into a destination folder.
///
/// Godot zips are not uniform. A standard Linux build is a single file. A mono
/// build is a folder with the binary and the GodotSharp data. A Mac build is an
/// app bundle. So when every entry sits under one common folder, and that folder
/// is not an app bundle, the folder is stripped so the install dir holds the
/// engine directly. App bundles are kept as is. Unsafe paths are rejected and
/// Unix mode bits are preserved so the engine binary stays runnable. Godot
/// archives do not contain symlinks, so they are not handled.
pub fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<(), InstallError> {
    extract_zip_cancelable(zip_path, dest_dir, &AtomicBool::new(false))
}

/// Extract a zip, checking the cancelled flag between entries so a long extract
/// can be stopped. The caller cleans up the destination on a cancel.
fn extract_zip_cancelable(
    zip_path: &Path,
    dest_dir: &Path,
    cancelled: &AtomicBool,
) -> Result<(), InstallError> {
    let file = File::open(zip_path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|err| InstallError::Extract(err.to_string()))?;

    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| InstallError::Extract(err.to_string()))?;
        match entry.enclosed_name() {
            Some(name) => names.push(name.to_path_buf()),
            None => {
                return Err(InstallError::UnsafePath(entry.name().to_string()));
            }
        }
    }
    let strip = common_strip_prefix(&names);

    fs::create_dir_all(dest_dir)?;
    for index in 0..archive.len() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled);
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|err| InstallError::Extract(err.to_string()))?;
        let Some(name) = entry.enclosed_name() else {
            return Err(InstallError::UnsafePath(entry.name().to_string()));
        };
        let enclosed = name.to_path_buf();
        let relative = match &strip {
            Some(prefix) => enclosed.strip_prefix(prefix).unwrap_or(&enclosed),
            None => enclosed.as_path(),
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out_path = dest_dir.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut out_file = File::create(&out_path)?;
        io::copy(&mut entry, &mut out_file)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
            }
        }
    }
    Ok(())
}

/// If every entry sits under a single top folder that is not an app bundle,
/// return that folder so it can be stripped. Otherwise return None.
fn common_strip_prefix(names: &[PathBuf]) -> Option<PathBuf> {
    let mut tops = HashSet::new();
    for name in names {
        let first = name.components().next()?;
        tops.insert(first.as_os_str().to_os_string());
    }
    if tops.len() != 1 {
        return None;
    }
    let top = PathBuf::from(tops.into_iter().next()?);
    let is_dir_prefix = names
        .iter()
        .any(|name| name != &top && name.starts_with(&top));
    let looks_like_bundle = top.to_string_lossy().to_ascii_lowercase().ends_with(".app");
    if is_dir_prefix && !looks_like_bundle {
        Some(top)
    } else {
        None
    }
}

/// Read a directory and return its entries sorted, for a stable order.
fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>, InstallError> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();
    Ok(entries)
}

fn dir_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

/// Append an extra extension to a path, keeping the existing name intact. So a
/// file named Godot.zip becomes Godot.zip.partial.
fn with_added_extension(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(extension);
    path.with_file_name(name)
}

/// An error from installing or managing engines.
#[derive(Debug)]
pub enum InstallError {
    Io(io::Error),
    /// The downloader failed to fetch the file.
    Download(String),
    /// The downloaded file did not match its checksum.
    ChecksumMismatch {
        expected: String,
        actual: String,
    },
    /// The version is already installed.
    AlreadyInstalled {
        variant: Variant,
        version: GodotVersion,
    },
    /// The version is not installed.
    NotInstalled {
        variant: Variant,
        version: GodotVersion,
    },
    /// The archive could not be extracted.
    Extract(String),
    /// A zip entry had an unsafe path.
    UnsafePath(String),
    /// The install was cancelled by the user.
    Cancelled,
    /// The engine executable could not be found.
    Executable(PlatformError),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Io(err) => write!(f, "filesystem error: {err}"),
            InstallError::Download(msg) => write!(f, "download failed: {msg}"),
            InstallError::ChecksumMismatch { expected, actual } => write!(
                f,
                "checksum did not match, expected {expected} but got {actual}"
            ),
            InstallError::AlreadyInstalled { variant, version } => {
                write!(f, "{} {variant} is already installed", version.to_tag())
            }
            InstallError::NotInstalled { variant, version } => {
                write!(f, "{} {variant} is not installed", version.to_tag())
            }
            InstallError::Extract(msg) => write!(f, "could not extract archive: {msg}"),
            InstallError::UnsafePath(name) => write!(f, "archive has an unsafe path: {name}"),
            InstallError::Cancelled => write!(f, "the install was cancelled"),
            InstallError::Executable(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InstallError::Io(err) => Some(err),
            InstallError::Executable(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(err: io::Error) -> Self {
        InstallError::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Stage;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

        fn noop_raw_waker() -> RawWaker {
            fn no_op(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                noop_raw_waker()
            }
            let vtable = &RawWakerVTable::new(clone, no_op, no_op, no_op);
            RawWaker::new(std::ptr::null(), vtable)
        }

        let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
        let mut cx = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(value) => {
                    return value;
                }
                Poll::Pending => {}
            }
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("godello-install-tests")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stable(major: u32, minor: u32, patch: u32) -> GodotVersion {
        GodotVersion::new(major, minor, patch, Stage::Stable)
    }

    /// Build a zip in memory from a list of entries. Each entry is a name and an
    /// optional body. A name ending in a slash is a directory.
    fn make_zip(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(io::Cursor::new(&mut buffer));
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            for (name, body) in entries {
                if let Some(bytes) = body {
                    writer.start_file(*name, options).unwrap();
                    writer.write_all(bytes).unwrap();
                } else {
                    writer.add_directory(*name, options).unwrap();
                }
            }
            writer.finish().unwrap();
        }
        buffer
    }

    fn sha512_hex(bytes: &[u8]) -> String {
        to_hex(&Sha512::digest(bytes))
    }

    struct FakeDownloader {
        data: Vec<u8>,
        fail: bool,
    }

    impl Downloader for FakeDownloader {
        async fn download_to(
            &self,
            _url: &str,
            dest: &Path,
            progress: &dyn DownloadProgress,
        ) -> Result<(), InstallError> {
            if self.fail {
                return Err(InstallError::Download("boom".to_string()));
            }
            let total = self.data.len() as u64;
            progress.start(Some(total));
            fs::write(dest, &self.data)?;
            progress.update(total);
            progress.finish();
            Ok(())
        }
    }

    /// A progress sink that records the calls it received, to check forwarding.
    /// It uses atomics so it stays Send and Sync, which the trait now requires.
    #[derive(Default)]
    struct RecordingProgress {
        started: std::sync::atomic::AtomicBool,
        total: std::sync::atomic::AtomicU64,
        last: std::sync::atomic::AtomicU64,
        finished: std::sync::atomic::AtomicBool,
    }

    impl DownloadProgress for RecordingProgress {
        fn start(&self, total: Option<u64>) {
            self.started
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.total
                .store(total.unwrap_or(0), std::sync::atomic::Ordering::SeqCst);
        }
        fn update(&self, downloaded: u64) {
            self.last
                .store(downloaded, std::sync::atomic::Ordering::SeqCst);
        }
        fn finish(&self) {
            self.finished
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    // Checksum verification.

    #[test]
    fn verify_passes_for_a_correct_sha512() {
        let dir = scratch("verify-512");
        let path = dir.join("file.bin");
        fs::write(&path, b"hello godello").unwrap();
        let checksum = Checksum::new(ChecksumAlgorithm::Sha512, sha512_hex(b"hello godello"));
        assert!(verify_file(&path, &checksum).is_ok());
    }

    #[test]
    fn verify_passes_for_a_correct_sha256() {
        let dir = scratch("verify-256");
        let path = dir.join("file.bin");
        fs::write(&path, b"data").unwrap();
        let hex = to_hex(&Sha256::digest(b"data"));
        let checksum = Checksum::new(ChecksumAlgorithm::Sha256, hex);
        assert!(verify_file(&path, &checksum).is_ok());
    }

    #[test]
    fn verify_is_case_insensitive() {
        let dir = scratch("verify-case");
        let path = dir.join("file.bin");
        fs::write(&path, b"x").unwrap();
        let upper = sha512_hex(b"x").to_ascii_uppercase();
        let checksum = Checksum::new(ChecksumAlgorithm::Sha512, upper);
        assert!(verify_file(&path, &checksum).is_ok());
    }

    #[test]
    fn verify_rejects_a_wrong_hash() {
        let dir = scratch("verify-wrong");
        let path = dir.join("file.bin");
        fs::write(&path, b"hello").unwrap();
        let checksum = Checksum::new(ChecksumAlgorithm::Sha512, "deadbeef");
        assert!(matches!(
            verify_file(&path, &checksum),
            Err(InstallError::ChecksumMismatch { .. })
        ));
    }

    // Extraction.

    #[test]
    fn extracts_a_single_file_at_top() {
        let dir = scratch("extract-single");
        let zip = dir.join("a.zip");
        fs::write(
            &zip,
            make_zip(&[("Godot_v4.3-stable_linux.x86_64", Some(b"binary"))]),
        )
        .unwrap();
        let dest = dir.join("out");
        extract_zip(&zip, &dest).unwrap();
        assert!(dest.join("Godot_v4.3-stable_linux.x86_64").is_file());
    }

    #[test]
    fn strips_a_single_top_folder() {
        let dir = scratch("extract-strip");
        let zip = dir.join("a.zip");
        fs::write(
            &zip,
            make_zip(&[
                ("Godot_v4.3-stable_mono_linux_x86_64/", None),
                (
                    "Godot_v4.3-stable_mono_linux_x86_64/Godot_v4.3-stable_mono_linux.x86_64",
                    Some(b"bin"),
                ),
                (
                    "Godot_v4.3-stable_mono_linux_x86_64/GodotSharp/Api.dll",
                    Some(b"dll"),
                ),
            ]),
        )
        .unwrap();
        let dest = dir.join("out");
        extract_zip(&zip, &dest).unwrap();
        // The top folder is gone, the contents are directly under dest.
        assert!(dest.join("Godot_v4.3-stable_mono_linux.x86_64").is_file());
        assert!(dest.join("GodotSharp/Api.dll").is_file());
    }

    #[test]
    fn keeps_an_app_bundle_folder() {
        let dir = scratch("extract-app");
        let zip = dir.join("a.zip");
        fs::write(
            &zip,
            make_zip(&[
                ("Godot.app/", None),
                ("Godot.app/Contents/MacOS/Godot", Some(b"bin")),
            ]),
        )
        .unwrap();
        let dest = dir.join("out");
        extract_zip(&zip, &dest).unwrap();
        // The bundle is preserved, not stripped.
        assert!(dest.join("Godot.app/Contents/MacOS/Godot").is_file());
    }

    #[test]
    fn extracts_multiple_top_entries_as_is() {
        let dir = scratch("extract-multi");
        let zip = dir.join("a.zip");
        fs::write(
            &zip,
            make_zip(&[
                ("Godot_v4.3-stable_win64.exe", Some(b"a")),
                ("Godot_v4.3-stable_win64_console.exe", Some(b"b")),
            ]),
        )
        .unwrap();
        let dest = dir.join("out");
        extract_zip(&zip, &dest).unwrap();
        assert!(dest.join("Godot_v4.3-stable_win64.exe").is_file());
        assert!(dest.join("Godot_v4.3-stable_win64_console.exe").is_file());
    }

    #[test]
    fn rejects_a_path_traversal_entry() {
        let dir = scratch("extract-evil");
        let zip = dir.join("a.zip");
        fs::write(&zip, make_zip(&[("../escape.txt", Some(b"x"))])).unwrap();
        let dest = dir.join("out");
        let result = extract_zip(&zip, &dest);
        assert!(matches!(result, Err(InstallError::UnsafePath(_))));
        assert!(!dir.join("escape.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("extract-mode");
        let zip = dir.join("a.zip");
        fs::write(&zip, make_zip(&[("Godot", Some(b"bin"))])).unwrap();
        let dest = dir.join("out");
        extract_zip(&zip, &dest).unwrap();
        let mode = fs::metadata(dest.join("Godot"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "expected the executable bits to be set"
        );
    }

    // Layout, list, remove.

    #[test]
    fn install_dir_is_variant_then_version() {
        let manager = InstallManager::new("/engines", "/downloads");
        let dir = manager.install_dir(Variant::Mono, stable(4, 3, 0));
        assert!(dir.ends_with("mono/4.3-stable"));
    }

    #[test]
    fn lists_installed_and_skips_junk() {
        let root = scratch("list-root");
        let downloads = scratch("list-dl");
        fs::create_dir_all(root.join("standard/4.3-stable")).unwrap();
        fs::create_dir_all(root.join("mono/4.2.1-stable")).unwrap();
        // Junk that must be ignored.
        fs::create_dir_all(root.join("standard/not-a-version")).unwrap();
        fs::create_dir_all(root.join("bogus-variant/4.0-stable")).unwrap();
        fs::write(root.join("standard/loose-file"), b"x").unwrap();

        let manager = InstallManager::new(&root, &downloads);
        let mut found = manager.list_installed().unwrap();
        found.sort_by_key(|engine| engine.version);
        let tags: Vec<String> = found.iter().map(|e| e.version.to_tag()).collect();
        assert_eq!(tags, vec!["4.2.1-stable", "4.3-stable"]);
        assert!(found.iter().any(|e| e.variant == Variant::Mono));
        assert!(found.iter().any(|e| e.variant == Variant::Standard));
    }

    #[test]
    fn list_on_a_missing_root_is_empty() {
        let manager = InstallManager::new("/no/such/engines/root/here", "/tmp/x");
        assert!(manager.list_installed().unwrap().is_empty());
    }

    #[test]
    fn remove_deletes_and_reports_when_absent() {
        let root = scratch("remove-root");
        let downloads = scratch("remove-dl");
        let manager = InstallManager::new(&root, &downloads);
        let version = stable(4, 3, 0);
        fs::create_dir_all(manager.install_dir(Variant::Standard, version)).unwrap();
        assert!(manager.is_installed(Variant::Standard, version));
        manager.remove(Variant::Standard, version).unwrap();
        assert!(!manager.is_installed(Variant::Standard, version));
        assert!(matches!(
            manager.remove(Variant::Standard, version),
            Err(InstallError::NotInstalled { .. })
        ));
    }

    // Full install flow.

    fn linux_zip_bytes() -> Vec<u8> {
        make_zip(&[("Godot_v4.3-stable_linux.x86_64", Some(b"the engine"))])
    }

    #[test]
    fn install_downloads_verifies_and_places() {
        let root = scratch("install-root");
        let downloads = scratch("install-dl");
        let manager = InstallManager::new(&root, &downloads);
        let bytes = linux_zip_bytes();
        let asset = Asset {
            file_name: "Godot_v4.3-stable_linux.x86_64.zip".to_string(),
            url: "https://dl.test/godot.zip".to_string(),
            checksum: Some(Checksum::new(ChecksumAlgorithm::Sha512, sha512_hex(&bytes))),
        };
        let downloader = FakeDownloader {
            data: bytes,
            fail: false,
        };
        let version = stable(4, 3, 0);
        let engine =
            block_on(manager.install(&asset, Variant::Standard, version, &downloader, &NoProgress))
                .unwrap();

        assert_eq!(engine.version, version);
        assert!(engine.path.join("Godot_v4.3-stable_linux.x86_64").is_file());
        assert!(manager.is_installed(Variant::Standard, version));
        assert_eq!(manager.list_installed().unwrap().len(), 1);
        // The executable resolves through the platform module.
        let exe = manager
            .executable(Variant::Standard, version, false)
            .unwrap();
        assert!(exe.ends_with("Godot_v4.3-stable_linux.x86_64"));
    }

    #[test]
    fn install_refuses_when_already_present() {
        let root = scratch("install-dup-root");
        let downloads = scratch("install-dup-dl");
        let manager = InstallManager::new(&root, &downloads);
        let version = stable(4, 3, 0);
        fs::create_dir_all(manager.install_dir(Variant::Standard, version)).unwrap();
        let asset = Asset {
            file_name: "godot.zip".to_string(),
            url: "https://dl.test/godot.zip".to_string(),
            checksum: None,
        };
        let downloader = FakeDownloader {
            data: linux_zip_bytes(),
            fail: false,
        };
        let result =
            block_on(manager.install(&asset, Variant::Standard, version, &downloader, &NoProgress));
        assert!(matches!(result, Err(InstallError::AlreadyInstalled { .. })));
    }

    #[test]
    fn install_fails_on_checksum_mismatch_and_leaves_no_install() {
        let root = scratch("install-bad-root");
        let downloads = scratch("install-bad-dl");
        let manager = InstallManager::new(&root, &downloads);
        let asset = Asset {
            file_name: "godot.zip".to_string(),
            url: "https://dl.test/godot.zip".to_string(),
            checksum: Some(Checksum::new(ChecksumAlgorithm::Sha512, "00".repeat(64))),
        };
        let downloader = FakeDownloader {
            data: linux_zip_bytes(),
            fail: false,
        };
        let version = stable(4, 3, 0);
        let result =
            block_on(manager.install(&asset, Variant::Standard, version, &downloader, &NoProgress));
        assert!(matches!(result, Err(InstallError::ChecksumMismatch { .. })));
        assert!(!manager.is_installed(Variant::Standard, version));
    }

    #[test]
    fn install_forwards_progress_to_the_sink() {
        let root = scratch("install-progress-root");
        let downloads = scratch("install-progress-dl");
        let manager = InstallManager::new(&root, &downloads);
        let bytes = linux_zip_bytes();
        let total = bytes.len() as u64;
        let asset = Asset {
            file_name: "Godot.zip".to_string(),
            url: "https://dl.test/godot.zip".to_string(),
            checksum: None,
        };
        let downloader = FakeDownloader {
            data: bytes,
            fail: false,
        };
        let progress = RecordingProgress::default();
        let version = stable(4, 3, 0);
        block_on(manager.install(&asset, Variant::Standard, version, &downloader, &progress))
            .unwrap();
        use std::sync::atomic::Ordering::SeqCst;
        assert!(progress.started.load(SeqCst));
        assert_eq!(progress.total.load(SeqCst), total);
        assert_eq!(progress.last.load(SeqCst), total);
        assert!(progress.finished.load(SeqCst));
    }

    #[test]
    fn install_propagates_a_download_failure() {
        let root = scratch("install-net-root");
        let downloads = scratch("install-net-dl");
        let manager = InstallManager::new(&root, &downloads);
        let asset = Asset {
            file_name: "godot.zip".to_string(),
            url: "https://dl.test/godot.zip".to_string(),
            checksum: None,
        };
        let downloader = FakeDownloader {
            data: Vec::new(),
            fail: true,
        };
        let version = stable(4, 3, 0);
        let result =
            block_on(manager.install(&asset, Variant::Standard, version, &downloader, &NoProgress));
        assert!(matches!(result, Err(InstallError::Download(_))));
        assert!(!manager.is_installed(Variant::Standard, version));
    }
}
