//! Where engine versions come from.
//!
//! A repository is any source of Godot releases and their download assets. The
//! trait is the contract. A concrete source, such as the official GitHub source,
//! implements it. Keeping this generic means other sources can be added later
//! without changing the rest of the app.
//!
//! This module holds the trait and the shared data types. It does no network
//! work itself. The picking logic that turns a version requirement into a single
//! release lives here as a default method so every source shares it.

use std::fmt;

use crate::platform::Target;
use crate::version::{GodotVersion, Variant, VersionPattern};

/// One Godot release as seen by a repository. It carries the version and the
/// build flavors that release offers. Which exact file to download for a host is
/// resolved separately as an asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: GodotVersion,
    pub variants: Vec<Variant>,
}

impl Release {
    pub fn new(version: GodotVersion, variants: Vec<Variant>) -> Self {
        Release { version, variants }
    }

    /// True when this release offers the given build flavor.
    pub fn offers(&self, variant: Variant) -> bool {
        self.variants.contains(&variant)
    }
}

/// The hash algorithm used to check a downloaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Sha256,
    Sha512,
}

/// A checksum for a download, used to verify the file after it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: ChecksumAlgorithm,
    pub hex: String,
}

impl Checksum {
    pub fn new(algorithm: ChecksumAlgorithm, hex: impl Into<String>) -> Self {
        Checksum {
            algorithm,
            hex: hex.into(),
        }
    }
}

/// A single downloadable file for one version on one target. The checksum is
/// optional because not every source publishes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub file_name: String,
    pub url: String,
    pub checksum: Option<Checksum>,
}

/// The contract for a source of Godot releases.
///
/// A source implements id, list_releases, and asset. The resolve method has a
/// default body built on list_releases, so sources do not repeat the picking
/// logic.
#[allow(async_fn_in_trait)]
pub trait EngineRepository {
    /// A short stable id for this source, for example github.
    fn id(&self) -> &str;

    /// All releases this source knows about. When include_pre is false the
    /// result holds stable releases only.
    async fn list_releases(&self, include_pre: bool) -> Result<Vec<Release>, RepositoryError>;

    /// The download asset for a version on a target.
    async fn asset(&self, version: GodotVersion, target: Target) -> Result<Asset, RepositoryError>;

    /// Turn a version requirement into a single release. Picks the newest
    /// release that matches the pattern and offers the variant.
    ///
    /// Prereleases are skipped unless include_pre is set or the pattern names a
    /// stage. So a bare 4.3 finds the newest stable 4.3, while 4.3-rc1 finds that
    /// exact prerelease even with include_pre off.
    async fn resolve(
        &self,
        pattern: VersionPattern,
        variant: Variant,
        include_pre: bool,
    ) -> Result<Release, RepositoryError> {
        // Fetch prereleases when the caller asked, or when the pattern itself
        // names a prerelease stage. Otherwise a pattern like 4.4-rc1 could never
        // match, since the source would have filtered the rc out already.
        let pattern_wants_pre = pattern.stage.is_some_and(|stage| stage.is_prerelease());
        let releases = self.list_releases(include_pre || pattern_wants_pre).await?;
        let chosen = releases
            .into_iter()
            .filter(|release| pattern.matches(&release.version) && release.offers(variant))
            .filter(|release| {
                include_pre || pattern.stage.is_some() || !release.version.is_prerelease()
            })
            .max_by_key(|release| release.version);
        chosen.ok_or(RepositoryError::NoMatch { pattern, variant })
    }
}

/// A small fetch contract so a source can read remote text without depending on
/// a specific http library. The real client lives in the binary. Tests use a
/// fake client with canned responses.
#[allow(async_fn_in_trait)]
pub trait HttpClient {
    /// Fetch a url and return its body as text.
    async fn get_text(&self, url: &str) -> Result<String, RepositoryError>;
}

/// An error from a repository.
#[derive(Debug)]
pub enum RepositoryError {
    /// No release matched the requirement.
    NoMatch {
        pattern: VersionPattern,
        variant: Variant,
    },
    /// The version exists but has no asset for this target.
    AssetNotFound {
        version: GodotVersion,
        target: Target,
    },
    /// A network call failed.
    Network(String),
    /// Source data could not be read.
    Parse(String),
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepositoryError::NoMatch { pattern, variant } => {
                write!(f, "no {variant} release matches {pattern}")
            }
            RepositoryError::AssetNotFound { version, target } => {
                write!(
                    f,
                    "no {} {} download for {} on {}",
                    target.variant,
                    version.to_tag(),
                    target.arch,
                    target.os
                )
            }
            RepositoryError::Network(msg) => write!(f, "network error: {msg}"),
            RepositoryError::Parse(msg) => write!(f, "could not read source data: {msg}"),
        }
    }
}

impl std::error::Error for RepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Arch, Os};
    use crate::version::Stage;

    /// Drive a future to completion without a runtime. The futures here finish on
    /// the first poll because the fake source does no real waiting.
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

    /// A source backed by an in memory list, used to test the contract.
    struct FakeRepo {
        releases: Vec<Release>,
    }

    impl EngineRepository for FakeRepo {
        fn id(&self) -> &str {
            "fake"
        }

        async fn list_releases(&self, include_pre: bool) -> Result<Vec<Release>, RepositoryError> {
            Ok(self
                .releases
                .iter()
                .filter(|release| include_pre || !release.version.is_prerelease())
                .cloned()
                .collect())
        }

        async fn asset(
            &self,
            version: GodotVersion,
            target: Target,
        ) -> Result<Asset, RepositoryError> {
            let known = self
                .releases
                .iter()
                .any(|release| release.version == version && release.offers(target.variant));
            if known {
                Ok(Asset {
                    file_name: format!("Godot_{}_{}.zip", version.to_tag(), target.os.tag()),
                    url: format!("https://example.test/{}", version.to_tag()),
                    checksum: None,
                })
            } else {
                Err(RepositoryError::AssetNotFound { version, target })
            }
        }
    }

    fn stable(major: u32, minor: u32, patch: u32, variants: Vec<Variant>) -> Release {
        Release::new(
            GodotVersion::new(major, minor, patch, Stage::Stable),
            variants,
        )
    }

    fn sample() -> FakeRepo {
        FakeRepo {
            releases: vec![
                stable(3, 5, 0, vec![Variant::Standard, Variant::Mono]),
                stable(4, 2, 0, vec![Variant::Standard, Variant::Mono]),
                stable(4, 3, 0, vec![Variant::Standard]),
                stable(4, 3, 1, vec![Variant::Standard]),
                Release::new(
                    GodotVersion::new(4, 4, 0, Stage::Rc(1)),
                    vec![Variant::Standard],
                ),
            ],
        }
    }

    fn pattern(text: &str) -> VersionPattern {
        text.parse().unwrap()
    }

    #[test]
    fn list_releases_hides_prereleases_by_default() {
        let repo = sample();
        let stable_only = block_on(repo.list_releases(false)).unwrap();
        assert!(stable_only.iter().all(|r| !r.version.is_prerelease()));
        let with_pre = block_on(repo.list_releases(true)).unwrap();
        assert!(with_pre.iter().any(|r| r.version.is_prerelease()));
    }

    #[test]
    fn resolve_picks_newest_stable_in_a_line() {
        let repo = sample();
        let release = block_on(repo.resolve(pattern("4.3"), Variant::Standard, false)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 3, 1, Stage::Stable));
    }

    #[test]
    fn resolve_skips_prereleases_unless_asked() {
        let repo = sample();
        // Without pre, a bare 4.4 has no stable match.
        let none = block_on(repo.resolve(pattern("4.4"), Variant::Standard, false));
        assert!(matches!(none, Err(RepositoryError::NoMatch { .. })));
        // With pre, the rc is accepted.
        let release = block_on(repo.resolve(pattern("4.4"), Variant::Standard, true)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 4, 0, Stage::Rc(1)));
    }

    #[test]
    fn resolve_accepts_an_explicit_prerelease_pattern_without_the_flag() {
        let repo = sample();
        let release = block_on(repo.resolve(pattern("4.4-rc1"), Variant::Standard, false)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 4, 0, Stage::Rc(1)));
    }

    #[test]
    fn resolve_honors_variant_availability() {
        let repo = sample();
        // 4.3 has no mono build, so the newest mono in the 4 line is 4.2.
        let release = block_on(repo.resolve(pattern("4"), Variant::Mono, false)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 2, 0, Stage::Stable));
    }

    #[test]
    fn resolve_with_no_mono_at_all_is_no_match() {
        let repo = sample();
        let result = block_on(repo.resolve(pattern("4.3"), Variant::Mono, false));
        assert!(matches!(result, Err(RepositoryError::NoMatch { .. })));
    }

    #[test]
    fn resolve_unknown_major_is_no_match() {
        let repo = sample();
        let result = block_on(repo.resolve(pattern("5"), Variant::Standard, true));
        assert!(matches!(result, Err(RepositoryError::NoMatch { .. })));
    }

    #[test]
    fn asset_is_returned_for_a_known_build() {
        let repo = sample();
        let version = GodotVersion::new(4, 3, 0, Stage::Stable);
        let target = Target::new(Os::Linux, Arch::X86_64, Variant::Standard);
        let asset = block_on(repo.asset(version, target)).unwrap();
        assert!(asset.file_name.contains("4.3-stable"));
        assert!(asset.url.ends_with("4.3-stable"));
    }

    #[test]
    fn asset_for_a_missing_variant_is_not_found() {
        let repo = sample();
        let version = GodotVersion::new(4, 3, 0, Stage::Stable);
        let target = Target::new(Os::Linux, Arch::X86_64, Variant::Mono);
        let result = block_on(repo.asset(version, target));
        assert!(matches!(result, Err(RepositoryError::AssetNotFound { .. })));
    }

    fn rc(major: u32, minor: u32, patch: u32, n: u32, variants: Vec<Variant>) -> Release {
        Release::new(
            GodotVersion::new(major, minor, patch, Stage::Rc(n)),
            variants,
        )
    }

    fn repo_with(releases: Vec<Release>) -> FakeRepo {
        FakeRepo { releases }
    }

    #[test]
    fn release_offers_reports_each_variant() {
        let release = stable(4, 2, 0, vec![Variant::Standard, Variant::Mono]);
        assert!(release.offers(Variant::Standard));
        assert!(release.offers(Variant::Mono));
        let standard_only = stable(4, 3, 0, vec![Variant::Standard]);
        assert!(standard_only.offers(Variant::Standard));
        assert!(!standard_only.offers(Variant::Mono));
        let empty = stable(4, 4, 0, vec![]);
        assert!(!empty.offers(Variant::Standard));
    }

    #[test]
    fn resolve_prefers_stable_over_rc_at_the_same_version() {
        // Both share the version number 4.3.0. Stable must win even with pre on.
        let repo = repo_with(vec![
            rc(4, 3, 0, 2, vec![Variant::Standard]),
            stable(4, 3, 0, vec![Variant::Standard]),
        ]);
        let release = block_on(repo.resolve(pattern("4.3"), Variant::Standard, true)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 3, 0, Stage::Stable));
    }

    #[test]
    fn resolve_picks_the_highest_rc_number() {
        let repo = repo_with(vec![
            rc(4, 4, 0, 1, vec![Variant::Standard]),
            rc(4, 4, 0, 3, vec![Variant::Standard]),
            rc(4, 4, 0, 2, vec![Variant::Standard]),
        ]);
        let release = block_on(repo.resolve(pattern("4.4"), Variant::Standard, true)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 4, 0, Stage::Rc(3)));
    }

    #[test]
    fn resolve_with_pre_lets_a_newer_rc_beat_an_older_stable() {
        // A bare pattern with pre on takes the newest by version number, so a
        // later rc outranks an earlier stable.
        let repo = repo_with(vec![
            stable(4, 3, 1, vec![Variant::Standard]),
            rc(4, 3, 2, 1, vec![Variant::Standard]),
        ]);
        let release = block_on(repo.resolve(pattern("4.3"), Variant::Standard, true)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 3, 2, Stage::Rc(1)));
    }

    #[test]
    fn resolve_exact_patch_matches_only_that_patch() {
        let repo = repo_with(vec![
            stable(4, 3, 0, vec![Variant::Standard]),
            stable(4, 3, 1, vec![Variant::Standard]),
            stable(4, 3, 2, vec![Variant::Standard]),
        ]);
        let release = block_on(repo.resolve(pattern("4.3.1"), Variant::Standard, false)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 3, 1, Stage::Stable));
    }

    #[test]
    fn resolve_exact_patch_with_no_such_patch_is_no_match() {
        let repo = repo_with(vec![
            stable(4, 3, 0, vec![Variant::Standard]),
            stable(4, 3, 2, vec![Variant::Standard]),
        ]);
        let result = block_on(repo.resolve(pattern("4.3.1"), Variant::Standard, false));
        assert!(matches!(result, Err(RepositoryError::NoMatch { .. })));
    }

    #[test]
    fn resolve_explicit_stable_pattern_rejects_a_prerelease() {
        let repo = repo_with(vec![rc(4, 4, 0, 1, vec![Variant::Standard])]);
        let result = block_on(repo.resolve(pattern("4.4-stable"), Variant::Standard, true));
        assert!(matches!(result, Err(RepositoryError::NoMatch { .. })));
    }

    #[test]
    fn resolve_exact_rc_pattern_matches_that_rc_only() {
        let repo = repo_with(vec![
            rc(4, 4, 0, 1, vec![Variant::Standard]),
            rc(4, 4, 0, 2, vec![Variant::Standard]),
        ]);
        let release = block_on(repo.resolve(pattern("4.4-rc1"), Variant::Standard, false)).unwrap();
        assert_eq!(release.version, GodotVersion::new(4, 4, 0, Stage::Rc(1)));
    }

    #[test]
    fn resolve_finds_a_variant_offered_only_on_a_prerelease() {
        // Mono ships first on an rc here. A bare pattern without pre should miss
        // it, while pre on or an explicit stage finds it.
        let repo = repo_with(vec![
            stable(4, 3, 0, vec![Variant::Standard]),
            rc(4, 4, 0, 1, vec![Variant::Standard, Variant::Mono]),
        ]);
        let missing = block_on(repo.resolve(pattern("4"), Variant::Mono, false));
        assert!(matches!(missing, Err(RepositoryError::NoMatch { .. })));
        let found = block_on(repo.resolve(pattern("4"), Variant::Mono, true)).unwrap();
        assert_eq!(found.version, GodotVersion::new(4, 4, 0, Stage::Rc(1)));
    }

    #[test]
    fn resolve_on_an_empty_source_is_no_match() {
        let repo = repo_with(vec![]);
        let result = block_on(repo.resolve(pattern("4"), Variant::Standard, true));
        assert!(matches!(result, Err(RepositoryError::NoMatch { .. })));
    }

    #[test]
    fn resolve_returns_the_full_release_with_its_variants() {
        let repo = repo_with(vec![stable(
            4,
            2,
            0,
            vec![Variant::Standard, Variant::Mono],
        )]);
        let release = block_on(repo.resolve(pattern("4.2"), Variant::Mono, false)).unwrap();
        assert!(release.offers(Variant::Standard));
        assert!(release.offers(Variant::Mono));
    }

    #[test]
    fn asset_for_an_unknown_version_is_not_found() {
        let repo = sample();
        let version = GodotVersion::new(9, 9, 9, Stage::Stable);
        let target = Target::new(Os::Windows, Arch::Arm64, Variant::Standard);
        let result = block_on(repo.asset(version, target));
        assert!(matches!(result, Err(RepositoryError::AssetNotFound { .. })));
    }

    #[test]
    fn error_messages_name_the_request() {
        let no_match = RepositoryError::NoMatch {
            pattern: pattern("4.3"),
            variant: Variant::Mono,
        };
        let text = no_match.to_string();
        assert!(text.contains("4.3"));
        assert!(text.contains("mono"));

        let missing = RepositoryError::AssetNotFound {
            version: GodotVersion::new(4, 3, 0, Stage::Stable),
            target: Target::new(Os::Linux, Arch::X86_64, Variant::Standard),
        };
        let text = missing.to_string();
        assert!(text.contains("4.3-stable"));
        assert!(text.contains("linux"));
    }
}
