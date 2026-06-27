//! The official Godot GitHub source.
//!
//! This source is only for the official Godot project. It is not a generic
//! GitHub source. Version list comes from the Godot website versions.yml
//! manifest. Binaries come from the godotengine/godot-builds releases. The asset
//! for a host is chosen by matching the file name to the target, which is the
//! fiddly part because Godot file names have varied over the years and differ
//! between the standard and mono builds.
//!
//! Network work goes through the HttpClient trait, so the parsing and matching
//! logic here is tested with a fake client and no real requests.

use serde::Deserialize;

use crate::platform::{Arch, Os, Target};
use crate::repository::{
    Asset, Checksum, ChecksumAlgorithm, EngineRepository, HttpClient, Release, RepositoryError,
};
use crate::version::{GodotVersion, Variant};

const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/godotengine/godot-website/master/_data/versions.yml";
const RELEASES_BY_TAG: &str = "https://api.github.com/repos/godotengine/godot-builds/releases/tags";
const SHA_SUMS_NAME: &str = "SHA512-SUMS.txt";

/// The official Godot GitHub source, generic over the http client so the binary
/// can supply a real one and tests can supply a fake one.
pub struct GodotGitHubRepository<C> {
    client: C,
    manifest_url: String,
    releases_by_tag: String,
}

impl<C: HttpClient> GodotGitHubRepository<C> {
    /// A source pointed at the real Godot endpoints.
    pub fn new(client: C) -> Self {
        GodotGitHubRepository {
            client,
            manifest_url: MANIFEST_URL.to_string(),
            releases_by_tag: RELEASES_BY_TAG.to_string(),
        }
    }

    /// A source with custom endpoints, used by tests.
    pub fn with_endpoints(
        client: C,
        manifest_url: impl Into<String>,
        releases_by_tag: impl Into<String>,
    ) -> Self {
        GodotGitHubRepository {
            client,
            manifest_url: manifest_url.into(),
            releases_by_tag: releases_by_tag.into(),
        }
    }

    /// Best effort lookup of a checksum for a file from the release sums asset.
    /// Any failure here returns None so a download can still proceed unverified.
    async fn checksum_for(&self, assets: &[GhAsset], file_name: &str) -> Option<Checksum> {
        let sums = assets
            .iter()
            .find(|asset| asset.name.eq_ignore_ascii_case(SHA_SUMS_NAME))?;
        let text = self
            .client
            .get_text(&sums.browser_download_url)
            .await
            .ok()?;
        let hex = parse_sha512sums(&text, file_name)?;
        Some(Checksum::new(ChecksumAlgorithm::Sha512, hex))
    }
}

impl<C: HttpClient> EngineRepository for GodotGitHubRepository<C> {
    fn id(&self) -> &str {
        "github"
    }

    async fn list_releases(&self, include_pre: bool) -> Result<Vec<Release>, RepositoryError> {
        let yaml = self.client.get_text(&self.manifest_url).await?;
        let releases = parse_manifest(&yaml)?;
        Ok(releases
            .into_iter()
            .filter(|release| include_pre || !release.version.is_prerelease())
            .collect())
    }

    async fn asset(&self, version: GodotVersion, target: Target) -> Result<Asset, RepositoryError> {
        let url = format!("{}/{}", self.releases_by_tag, version.to_tag());
        let json = self.client.get_text(&url).await?;
        let release = parse_release_json(&json)?;
        let matched = match_asset(&release.assets, target)
            .ok_or(RepositoryError::AssetNotFound { version, target })?;
        let file_name = matched.name.clone();
        let download_url = matched.browser_download_url.clone();
        let checksum = self.checksum_for(&release.assets, &file_name).await;
        Ok(Asset {
            file_name,
            url: download_url,
            checksum,
        })
    }
}

// Manifest parsing.

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    name: String,
    flavor: String,
    #[serde(default)]
    releases: Vec<ManifestRelease>,
}

#[derive(Debug, Deserialize)]
struct ManifestRelease {
    name: String,
}

/// Turn the versions.yml text into releases. Each top entry gives one tag from
/// its flavor plus one tag per prerelease in its releases list. Entries that do
/// not parse into a known tag are skipped rather than failing the whole list, so
/// one odd row cannot break installs.
pub fn parse_manifest(yaml: &str) -> Result<Vec<Release>, RepositoryError> {
    let entries: Vec<ManifestEntry> =
        serde_yaml::from_str(yaml).map_err(|err| RepositoryError::Parse(err.to_string()))?;

    let mut seen = std::collections::HashSet::new();
    let mut releases = Vec::new();
    for entry in entries {
        let mut tags = Vec::with_capacity(entry.releases.len() + 1);
        tags.push(format!("{}-{}", entry.name, entry.flavor));
        for release in &entry.releases {
            tags.push(format!("{}-{}", entry.name, release.name));
        }
        for tag in tags {
            if let Ok(version) = GodotVersion::parse_tag(&tag) {
                if seen.insert(version) {
                    // The manifest does not say which flavors exist, so both are
                    // offered here. The asset lookup is the real check and will
                    // report when a build is missing.
                    releases.push(Release::new(
                        version,
                        vec![Variant::Standard, Variant::Mono],
                    ));
                }
            }
        }
    }
    Ok(releases)
}

// Release json parsing.

#[derive(Debug, Deserialize)]
struct GhRelease {
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Parse the GitHub release json for a tag into its asset list.
fn parse_release_json(json: &str) -> Result<GhRelease, RepositoryError> {
    serde_json::from_str(json).map_err(|err| RepositoryError::Parse(err.to_string()))
}

// Asset matching.

/// Pick the editor asset that fits the target. Standard and mono use different
/// name shapes, so each target has its own ordered list of name fragments to
/// look for. The first asset that contains the highest priority fragment wins.
/// Standard never returns a mono file.
fn match_asset(assets: &[GhAsset], target: Target) -> Option<&GhAsset> {
    let fragments = asset_fragments(target);
    let want_mono = target.variant == Variant::Mono;
    for fragment in fragments {
        let found = assets.iter().find(|asset| {
            let name = asset.name.to_ascii_lowercase();
            let is_mono = name.contains("mono");
            is_mono == want_mono && name.contains(fragment)
        });
        if let Some(asset) = found {
            return Some(asset);
        }
    }
    None
}

/// The ordered name fragments to look for, most preferred first. Newer naming
/// comes first, older naming follows as a fallback. The lists cover the whole
/// history of Godot file names, not just current ones.
///
/// Linux dropped the old x11 token for linux and at times used an underscore
/// instead of a dot before the arch. Mac moved from osx and fat or 32 or 64 to a
/// single universal build. Windows mono drops the exe token. The standard lists
/// can safely include broad fragments because the matcher never returns a file
/// whose name contains mono for a standard request.
fn asset_fragments(target: Target) -> Vec<&'static str> {
    match (target.variant, target.os, target.arch) {
        // Standard Linux. Modern dot form, the old underscore form, then the
        // 3.x x11 and bare 64 or 32 forms.
        (Variant::Standard, Os::Linux, Arch::X86_64) => {
            vec!["linux.x86_64", "linux_x86_64", "x11.64", "linux.64"]
        }
        (Variant::Standard, Os::Linux, Arch::X86) => {
            vec!["linux.x86_32", "linux_x86_32", "x11.32", "linux.32"]
        }
        (Variant::Standard, Os::Linux, Arch::Arm64) => vec!["linux.arm64", "linux_arm64"],
        // Standard Windows.
        (Variant::Standard, Os::Windows, Arch::X86_64) => vec!["win64.exe", "win64"],
        (Variant::Standard, Os::Windows, Arch::X86) => vec!["win32.exe", "win32"],
        (Variant::Standard, Os::Windows, Arch::Arm64) => {
            vec!["windows_arm64.exe", "windows_arm64"]
        }
        // Standard Mac uses one fat or universal build for every arch. The 64
        // forms come before the 32 forms so a 32 build is only a last resort.
        (Variant::Standard, Os::Mac, _) => vec![
            "macos.universal",
            "osx.universal",
            "osx.fat",
            "osx.64",
            "osx64",
            "osx.32",
            "osx32",
        ],
        // Mono Linux. Modern form, then the 3.x x11 and bare 64 forms.
        (Variant::Mono, Os::Linux, Arch::X86_64) => {
            vec!["mono_linux_x86_64", "mono_x11_64", "mono_linux.64"]
        }
        (Variant::Mono, Os::Linux, Arch::X86) => {
            vec!["mono_linux_x86_32", "mono_x11_32", "mono_linux.32"]
        }
        (Variant::Mono, Os::Linux, Arch::Arm64) => vec!["mono_linux_arm64"],
        // Mono Windows. The mono Windows zip has no exe token in its name.
        (Variant::Mono, Os::Windows, Arch::X86_64) => vec!["mono_win64"],
        (Variant::Mono, Os::Windows, Arch::X86) => vec!["mono_win32"],
        (Variant::Mono, Os::Windows, Arch::Arm64) => vec!["mono_windows_arm64"],
        // Mono Mac universal or fat or 64 build.
        (Variant::Mono, Os::Mac, _) => vec![
            "mono_macos.universal",
            "mono_osx.universal",
            "mono_osx.fat",
            "mono_osx.64",
            "mono_osx64",
        ],
    }
}

// Checksum file parsing.

/// Find the hash for a file in a SHA512-SUMS.txt body. Lines look like a hash,
/// some spaces, then the file name. The name can carry a leading star or dot
/// slash. Returns the hash only when it looks like a sha512 hex string.
pub fn parse_sha512sums(text: &str, file_name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((hash, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim().trim_start_matches('*').trim_start_matches("./");
        if name == file_name && is_sha512_hex(hash) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

fn is_sha512_hex(value: &str) -> bool {
    value.len() == 128 && value.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Stage;
    use std::cell::RefCell;
    use std::collections::HashMap;

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

    /// A client that returns canned text per url, or an error for unknown urls.
    struct FakeClient {
        responses: HashMap<String, String>,
        requested: RefCell<Vec<String>>,
    }

    impl FakeClient {
        fn new() -> Self {
            FakeClient {
                responses: HashMap::new(),
                requested: RefCell::new(Vec::new()),
            }
        }

        fn with(mut self, url: &str, body: &str) -> Self {
            self.responses.insert(url.to_string(), body.to_string());
            self
        }
    }

    impl HttpClient for FakeClient {
        async fn get_text(&self, url: &str) -> Result<String, RepositoryError> {
            self.requested.borrow_mut().push(url.to_string());
            self.responses
                .get(url)
                .cloned()
                .ok_or_else(|| RepositoryError::Network(format!("no canned response for {url}")))
        }
    }

    fn asset(name: &str) -> GhAsset {
        GhAsset {
            name: name.to_string(),
            browser_download_url: format!("https://dl.test/{name}"),
        }
    }

    /// The real 4.3 asset names, used so the matcher is tested against reality.
    fn real_assets() -> Vec<GhAsset> {
        [
            "godot-4.3-stable.tar.xz",
            "Godot_v4.3-stable_export_templates.tpz",
            "Godot_v4.3-stable_linux.arm32.zip",
            "Godot_v4.3-stable_linux.arm64.zip",
            "Godot_v4.3-stable_linux.x86_32.zip",
            "Godot_v4.3-stable_linux.x86_64.zip",
            "Godot_v4.3-stable_macos.universal.zip",
            "Godot_v4.3-stable_mono_export_templates.tpz",
            "Godot_v4.3-stable_mono_linux_arm64.zip",
            "Godot_v4.3-stable_mono_linux_x86_32.zip",
            "Godot_v4.3-stable_mono_linux_x86_64.zip",
            "Godot_v4.3-stable_mono_macos.universal.zip",
            "Godot_v4.3-stable_mono_win32.zip",
            "Godot_v4.3-stable_mono_win64.zip",
            "Godot_v4.3-stable_mono_windows_arm64.zip",
            "Godot_v4.3-stable_web_editor.zip",
            "Godot_v4.3-stable_win32.exe.zip",
            "Godot_v4.3-stable_win64.exe.zip",
            "Godot_v4.3-stable_windows_arm64.exe.zip",
            "SHA512-SUMS.txt",
        ]
        .iter()
        .map(|name| asset(name))
        .collect()
    }

    fn matched_name(assets: &[GhAsset], os: Os, arch: Arch, variant: Variant) -> Option<String> {
        match_asset(assets, Target::new(os, arch, variant)).map(|a| a.name.clone())
    }

    // Manifest parsing.

    const SAMPLE_MANIFEST: &str = r#"
- name: "4.3"
  flavor: "stable"
  release_date: "15 August 2024"
  releases:
    - name: "rc1"
      release_date: "1 August 2024"
- name: "4.2.2"
  flavor: "stable"
  releases: []
- name: "4.0"
  flavor: "stable"
  releases:
    - name: "rc6"
    - name: "beta17"
    - name: "alpha1"
"#;

    #[test]
    fn manifest_expands_flavor_and_prereleases() {
        let releases = parse_manifest(SAMPLE_MANIFEST).unwrap();
        let tags: Vec<String> = releases.iter().map(|r| r.version.to_tag()).collect();
        assert!(tags.contains(&"4.3-stable".to_string()));
        assert!(tags.contains(&"4.3-rc1".to_string()));
        assert!(tags.contains(&"4.2.2-stable".to_string()));
        assert!(tags.contains(&"4.0-stable".to_string()));
        assert!(tags.contains(&"4.0-rc6".to_string()));
        assert!(tags.contains(&"4.0-beta17".to_string()));
        assert!(tags.contains(&"4.0-alpha1".to_string()));
    }

    #[test]
    fn manifest_offers_both_variants() {
        let releases = parse_manifest(SAMPLE_MANIFEST).unwrap();
        assert!(
            releases
                .iter()
                .all(|r| r.offers(Variant::Standard) && r.offers(Variant::Mono))
        );
    }

    #[test]
    fn manifest_skips_unparseable_rows_without_failing() {
        // The middle entry has a flavor that is not a real stage. It should be
        // dropped while the good entries still load.
        let yaml = r#"
- name: "4.3"
  flavor: "stable"
  releases: []
- name: "weird"
  flavor: "nonsense"
  releases: []
- name: "4.2"
  flavor: "stable"
  releases: []
"#;
        let releases = parse_manifest(yaml).unwrap();
        let tags: Vec<String> = releases.iter().map(|r| r.version.to_tag()).collect();
        assert!(tags.contains(&"4.3-stable".to_string()));
        assert!(tags.contains(&"4.2-stable".to_string()));
        assert!(
            !tags
                .iter()
                .any(|t| t.contains("weird") || t.contains("nonsense"))
        );
    }

    #[test]
    fn manifest_dedups_repeated_tags() {
        let yaml = r#"
- name: "4.3"
  flavor: "stable"
  releases:
    - name: "stable"
"#;
        let releases = parse_manifest(yaml).unwrap();
        let count = releases
            .iter()
            .filter(|r| r.version.to_tag() == "4.3-stable")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn manifest_handles_empty_input() {
        assert!(parse_manifest("[]").unwrap().is_empty());
    }

    #[test]
    fn manifest_reports_bad_yaml() {
        let result = parse_manifest("this: : : not valid");
        assert!(matches!(result, Err(RepositoryError::Parse(_))));
    }

    // Asset matching against the real 4.3 names.

    #[test]
    fn matches_standard_linux_x86_64() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Standard).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_linux.x86_64.zip");
    }

    #[test]
    fn matches_mono_linux_x86_64() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Mono).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_mono_linux_x86_64.zip");
    }

    #[test]
    fn matches_standard_windows_x86_64_not_mono() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Windows, Arch::X86_64, Variant::Standard).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_win64.exe.zip");
    }

    #[test]
    fn matches_mono_windows_x86_64_not_standard() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Windows, Arch::X86_64, Variant::Mono).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_mono_win64.zip");
    }

    #[test]
    fn matches_standard_windows_arm64() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Windows, Arch::Arm64, Variant::Standard).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_windows_arm64.exe.zip");
    }

    #[test]
    fn matches_standard_mac_universal_not_mono() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Mac, Arch::Arm64, Variant::Standard).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_macos.universal.zip");
    }

    #[test]
    fn matches_mono_mac_universal() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Mac, Arch::X86_64, Variant::Mono).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_mono_macos.universal.zip");
    }

    #[test]
    fn matches_the_right_arch_for_32_bit() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Linux, Arch::X86, Variant::Standard).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_linux.x86_32.zip");
    }

    #[test]
    fn matches_linux_arm64() {
        let assets = real_assets();
        let name = matched_name(&assets, Os::Linux, Arch::Arm64, Variant::Standard).unwrap();
        assert_eq!(name, "Godot_v4.3-stable_linux.arm64.zip");
    }

    #[test]
    fn never_picks_a_non_editor_asset() {
        let assets = real_assets();
        for variant in [Variant::Standard, Variant::Mono] {
            for os in [Os::Linux, Os::Windows, Os::Mac] {
                for arch in [Arch::X86_64, Arch::X86, Arch::Arm64] {
                    if let Some(name) = matched_name(&assets, os, arch, variant) {
                        let lower = name.to_ascii_lowercase();
                        assert!(!lower.contains("export_templates"), "{name}");
                        assert!(!lower.contains("web_editor"), "{name}");
                        assert!(!lower.contains("android"), "{name}");
                        assert!(!lower.contains(".tar.xz"), "{name}");
                        assert!(lower.ends_with(".zip"), "{name}");
                    }
                }
            }
        }
    }

    #[test]
    fn standard_never_returns_a_mono_file() {
        // Only mono files are present, so a standard request must find nothing.
        let assets = vec![
            asset("Godot_v4.3-stable_mono_linux_x86_64.zip"),
            asset("Godot_v4.3-stable_mono_win64.zip"),
        ];
        assert!(matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Standard).is_none());
    }

    #[test]
    fn mono_arm64_is_missing_when_absent() {
        // The real 4.3 mono list here has no linux arm64, so it should be none.
        let assets: Vec<GhAsset> = real_assets()
            .into_iter()
            .filter(|a| a.name != "Godot_v4.3-stable_mono_linux_arm64.zip")
            .collect();
        assert!(matched_name(&assets, Os::Linux, Arch::Arm64, Variant::Mono).is_none());
    }

    #[test]
    fn matches_old_x11_and_osx_names() {
        let assets = vec![
            asset("Godot_v3.2.3-stable_x11.64.zip"),
            asset("Godot_v3.2.3-stable_x11.32.zip"),
            asset("Godot_v3.2.3-stable_osx.universal.zip"),
            asset("Godot_v3.2.3-stable_mono_x11_64.zip"),
        ];
        assert_eq!(
            matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Standard).unwrap(),
            "Godot_v3.2.3-stable_x11.64.zip"
        );
        assert_eq!(
            matched_name(&assets, Os::Mac, Arch::X86_64, Variant::Standard).unwrap(),
            "Godot_v3.2.3-stable_osx.universal.zip"
        );
        assert_eq!(
            matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Mono).unwrap(),
            "Godot_v3.2.3-stable_mono_x11_64.zip"
        );
    }

    #[test]
    fn matches_underscore_linux_standard_form() {
        // Some Godot Linux zips used an underscore before the arch instead of a
        // dot. The standard request must still find it and not a mono file.
        let assets = vec![
            asset("Godot_v4.1-stable_linux_x86_64.zip"),
            asset("Godot_v4.1-stable_mono_linux_x86_64.zip"),
        ];
        assert_eq!(
            matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Standard).unwrap(),
            "Godot_v4.1-stable_linux_x86_64.zip"
        );
        assert_eq!(
            matched_name(&assets, Os::Linux, Arch::X86_64, Variant::Mono).unwrap(),
            "Godot_v4.1-stable_mono_linux_x86_64.zip"
        );
    }

    #[test]
    fn matches_windows_zip_without_exe_token() {
        let assets = vec![asset("Godot_v3.0-stable_win64.zip")];
        assert_eq!(
            matched_name(&assets, Os::Windows, Arch::X86_64, Variant::Standard).unwrap(),
            "Godot_v3.0-stable_win64.zip"
        );
    }

    #[test]
    fn matches_3x_mono_x11_and_osx_names() {
        let assets = vec![
            asset("Godot_v3.2.3-stable_mono_x11_64.zip"),
            asset("Godot_v3.2.3-stable_mono_x11_32.zip"),
            asset("Godot_v3.2.3-stable_mono_osx.64.zip"),
            asset("Godot_v3.2.3-stable_mono_win64.zip"),
        ];
        assert_eq!(
            matched_name(&assets, Os::Linux, Arch::X86, Variant::Mono).unwrap(),
            "Godot_v3.2.3-stable_mono_x11_32.zip"
        );
        assert_eq!(
            matched_name(&assets, Os::Mac, Arch::X86_64, Variant::Mono).unwrap(),
            "Godot_v3.2.3-stable_mono_osx.64.zip"
        );
    }

    #[test]
    fn mac_prefers_64_over_32_for_old_builds() {
        let assets = vec![
            asset("Godot_v2.1.6-stable_osx32.zip"),
            asset("Godot_v2.1.6-stable_osx64.zip"),
        ];
        assert_eq!(
            matched_name(&assets, Os::Mac, Arch::X86_64, Variant::Standard).unwrap(),
            "Godot_v2.1.6-stable_osx64.zip"
        );
    }

    #[test]
    fn mac_falls_back_to_32_only_when_that_is_all_there_is() {
        let assets = vec![asset("Godot_v2.1.6-stable_osx32.zip")];
        assert_eq!(
            matched_name(&assets, Os::Mac, Arch::X86_64, Variant::Standard).unwrap(),
            "Godot_v2.1.6-stable_osx32.zip"
        );
    }

    #[test]
    fn mac_prefers_universal_over_fat_and_old_forms() {
        let assets = vec![
            asset("Godot_v3.5-stable_osx.fat.zip"),
            asset("Godot_v3.5-stable_macos.universal.zip"),
            asset("Godot_v3.5-stable_osx64.zip"),
        ];
        assert_eq!(
            matched_name(&assets, Os::Mac, Arch::Arm64, Variant::Standard).unwrap(),
            "Godot_v3.5-stable_macos.universal.zip"
        );
    }

    #[test]
    fn empty_asset_list_matches_nothing() {
        assert!(matched_name(&[], Os::Linux, Arch::X86_64, Variant::Standard).is_none());
    }

    // Release json parsing.

    #[test]
    fn parses_release_assets() {
        let json = r#"{ "assets": [
            { "name": "a.zip", "browser_download_url": "https://dl.test/a.zip" },
            { "name": "b.zip", "browser_download_url": "https://dl.test/b.zip" }
        ] }"#;
        let release = parse_release_json(json).unwrap();
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[0].name, "a.zip");
    }

    #[test]
    fn release_json_without_assets_is_empty_not_an_error() {
        let release = parse_release_json(r#"{ "tag_name": "4.3-stable" }"#).unwrap();
        assert!(release.assets.is_empty());
    }

    #[test]
    fn bad_release_json_is_a_parse_error() {
        assert!(matches!(
            parse_release_json("not json"),
            Err(RepositoryError::Parse(_))
        ));
    }

    // Checksum parsing.

    #[test]
    fn finds_a_sha512_for_a_file() {
        let hash = "a".repeat(128);
        let text = format!(
            "{hash}  Godot_v4.3-stable_linux.x86_64.zip\n{}  other.zip\n",
            "b".repeat(128)
        );
        let found = parse_sha512sums(&text, "Godot_v4.3-stable_linux.x86_64.zip").unwrap();
        assert_eq!(found, hash);
    }

    #[test]
    fn handles_star_and_dot_slash_prefixes() {
        let hash = "c".repeat(128);
        let star = format!("{hash} *file.zip");
        assert_eq!(parse_sha512sums(&star, "file.zip").unwrap(), hash);
        let dot = format!("{hash}  ./file.zip");
        assert_eq!(parse_sha512sums(&dot, "file.zip").unwrap(), hash);
    }

    #[test]
    fn missing_file_in_sums_is_none() {
        let text = format!("{}  other.zip\n", "d".repeat(128));
        assert!(parse_sha512sums(&text, "wanted.zip").is_none());
    }

    #[test]
    fn rejects_a_hash_of_the_wrong_length() {
        let text = "abc123  file.zip";
        assert!(parse_sha512sums(text, "file.zip").is_none());
    }

    #[test]
    fn ignores_blank_lines() {
        let hash = "e".repeat(128);
        let text = format!("\n\n{hash}  file.zip\n\n");
        assert_eq!(parse_sha512sums(&text, "file.zip").unwrap(), hash);
    }

    // End to end through the trait with a fake client.

    fn endpoints() -> (&'static str, &'static str) {
        ("https://test/versions.yml", "https://test/tags")
    }

    #[test]
    fn list_releases_filters_prereleases() {
        let (manifest, tags) = endpoints();
        let client = FakeClient::new().with(manifest, SAMPLE_MANIFEST);
        let repo = GodotGitHubRepository::with_endpoints(client, manifest, tags);

        let stable_only = block_on(repo.list_releases(false)).unwrap();
        assert!(stable_only.iter().all(|r| !r.version.is_prerelease()));
        let with_pre = block_on(repo.list_releases(true)).unwrap();
        assert!(with_pre.iter().any(|r| r.version.is_prerelease()));
    }

    #[test]
    fn asset_resolves_with_a_checksum() {
        let (manifest, tags) = endpoints();
        let tag_url = format!("{tags}/4.3-stable");
        let hash = "f".repeat(128);
        let release_json = format!(
            r#"{{ "assets": [
                {{ "name": "Godot_v4.3-stable_linux.x86_64.zip", "browser_download_url": "https://dl.test/editor.zip" }},
                {{ "name": "SHA512-SUMS.txt", "browser_download_url": "https://dl.test/sums" }}
            ] }}"#
        );
        let sums = format!("{hash}  Godot_v4.3-stable_linux.x86_64.zip\n");
        let client = FakeClient::new()
            .with(manifest, SAMPLE_MANIFEST)
            .with(&tag_url, &release_json)
            .with("https://dl.test/sums", &sums);
        let repo = GodotGitHubRepository::with_endpoints(client, manifest, tags);

        let version = GodotVersion::new(4, 3, 0, Stage::Stable);
        let target = Target::new(Os::Linux, Arch::X86_64, Variant::Standard);
        let asset = block_on(repo.asset(version, target)).unwrap();
        assert_eq!(asset.url, "https://dl.test/editor.zip");
        let checksum = asset.checksum.unwrap();
        assert_eq!(checksum.algorithm, ChecksumAlgorithm::Sha512);
        assert_eq!(checksum.hex, hash);
    }

    #[test]
    fn asset_resolves_without_a_checksum_when_sums_missing() {
        let (manifest, tags) = endpoints();
        let tag_url = format!("{tags}/4.3-stable");
        let release_json = r#"{ "assets": [
            { "name": "Godot_v4.3-stable_linux.x86_64.zip", "browser_download_url": "https://dl.test/editor.zip" }
        ] }"#;
        let client = FakeClient::new().with(&tag_url, release_json);
        let repo = GodotGitHubRepository::with_endpoints(client, manifest, tags);

        let version = GodotVersion::new(4, 3, 0, Stage::Stable);
        let target = Target::new(Os::Linux, Arch::X86_64, Variant::Standard);
        let asset = block_on(repo.asset(version, target)).unwrap();
        assert!(asset.checksum.is_none());
    }

    #[test]
    fn asset_for_a_missing_target_is_not_found() {
        let (manifest, tags) = endpoints();
        let tag_url = format!("{tags}/4.3-stable");
        // Only a linux build is present, so a windows request should not match.
        let release_json = r#"{ "assets": [
            { "name": "Godot_v4.3-stable_linux.x86_64.zip", "browser_download_url": "https://dl.test/editor.zip" }
        ] }"#;
        let client = FakeClient::new().with(&tag_url, release_json);
        let repo = GodotGitHubRepository::with_endpoints(client, manifest, tags);

        let version = GodotVersion::new(4, 3, 0, Stage::Stable);
        let target = Target::new(Os::Windows, Arch::X86_64, Variant::Standard);
        let result = block_on(repo.asset(version, target));
        assert!(matches!(result, Err(RepositoryError::AssetNotFound { .. })));
    }

    #[test]
    fn network_failure_propagates() {
        let (manifest, tags) = endpoints();
        // No canned manifest response, so the client errors.
        let client = FakeClient::new();
        let repo = GodotGitHubRepository::with_endpoints(client, manifest, tags);
        let result = block_on(repo.list_releases(true));
        assert!(matches!(result, Err(RepositoryError::Network(_))));
    }
}
