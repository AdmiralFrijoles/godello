//! Godot version types.
//!
//! Godot release tags are not plain semver. They look like 4.3-stable,
//! 4.2.1-stable, or 4.0-rc1. Trailing zero patches are dropped in the tag, so
//! 4.3-stable means 4.3.0. This module models that.
//!
//! Two views exist. A resolved version where every part is known, parsed from a
//! real tag. And a pattern with optional parts, used to match a requirement like
//! 4.3 against the installed builds. The build flavor is tracked separately as a
//! variant.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// The engine build flavor. Kept as its own value, not a bool, so more flavors
/// can be added later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Variant {
    /// The standard build.
    #[default]
    Standard,
    /// The C# build, also called Mono.
    Mono,
}

impl Variant {
    /// The short token used in paths and pins.
    pub fn as_str(self) -> &'static str {
        match self {
            Variant::Standard => "standard",
            Variant::Mono => "mono",
        }
    }
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Variant {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "standard" | "stable-build" | "default" => Ok(Variant::Standard),
            "mono" | "csharp" | "c#" => Ok(Variant::Mono),
            _ => Err(VersionParseError::Variant(s.to_string())),
        }
    }
}

/// The release stage of a version. Ordered from least to most stable as alpha,
/// dev, beta, rc, then stable. Within a stage the higher number is newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Alpha(u32),
    Dev(u32),
    Beta(u32),
    Rc(u32),
    Stable,
}

impl Stage {
    /// True for any stage that is not stable.
    pub fn is_prerelease(self) -> bool {
        !matches!(self, Stage::Stable)
    }

    /// A sort key. The first part ranks the stage. The second is the number
    /// within the stage. Stable has no number, so it uses zero.
    fn sort_key(self) -> (u8, u32) {
        match self {
            Stage::Alpha(n) => (0, n),
            Stage::Dev(n) => (1, n),
            Stage::Beta(n) => (2, n),
            Stage::Rc(n) => (3, n),
            Stage::Stable => (4, 0),
        }
    }
}

impl PartialOrd for Stage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Stage {
    fn cmp(&self, other: &Self) -> Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stage::Alpha(n) => write!(f, "alpha{n}"),
            Stage::Dev(n) => write!(f, "dev{n}"),
            Stage::Beta(n) => write!(f, "beta{n}"),
            Stage::Rc(n) => write!(f, "rc{n}"),
            Stage::Stable => f.write_str("stable"),
        }
    }
}

impl FromStr for Stage {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        if lower == "stable" {
            return Ok(Stage::Stable);
        }
        // Split the leading letters from the trailing number, for example rc1.
        let split = lower
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(lower.len());
        let (name, number_part) = lower.split_at(split);
        let number: u32 = if number_part.is_empty() {
            0
        } else {
            number_part
                .parse()
                .map_err(|_| VersionParseError::Stage(s.to_string()))?
        };
        match name {
            "alpha" => Ok(Stage::Alpha(number)),
            "dev" => Ok(Stage::Dev(number)),
            "beta" => Ok(Stage::Beta(number)),
            "rc" => Ok(Stage::Rc(number)),
            _ => Err(VersionParseError::Stage(s.to_string())),
        }
    }
}

/// A fully known Godot version, parsed from a real release tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GodotVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub stage: Stage,
}

impl GodotVersion {
    pub fn new(major: u32, minor: u32, patch: u32, stage: Stage) -> Self {
        GodotVersion {
            major,
            minor,
            patch,
            stage,
        }
    }

    /// True when this is not a stable release.
    pub fn is_prerelease(self) -> bool {
        self.stage.is_prerelease()
    }

    /// The release tag string, the way the tags are written. A zero patch is
    /// dropped, for example 4.3.0-stable becomes 4.3-stable.
    pub fn to_tag(self) -> String {
        if self.patch == 0 {
            format!("{}.{}-{}", self.major, self.minor, self.stage)
        } else {
            format!(
                "{}.{}.{}-{}",
                self.major, self.minor, self.patch, self.stage
            )
        }
    }

    /// Parse a release tag such as 4.3-stable, 4.2.1-stable, or 4.0-rc1.
    pub fn parse_tag(s: &str) -> Result<Self, VersionParseError> {
        let (number_part, stage_part) = s
            .split_once('-')
            .ok_or_else(|| VersionParseError::MissingStage(s.to_string()))?;
        let (major, minor, patch) = parse_number_part(number_part)?;
        let stage = stage_part.parse()?;
        Ok(GodotVersion {
            major,
            minor,
            patch,
            stage,
        })
    }
}

impl PartialOrd for GodotVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GodotVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(self.stage.cmp(&other.stage))
    }
}

impl fmt::Display for GodotVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_tag())
    }
}

impl FromStr for GodotVersion {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        GodotVersion::parse_tag(s)
    }
}

/// A version requirement with optional parts. A missing part is a wildcard.
/// Used to match a pin like 4.3 against installed builds. Minor, patch, and
/// stage may each be left open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VersionPattern {
    pub major: u32,
    pub minor: Option<u32>,
    pub patch: Option<u32>,
    pub stage: Option<Stage>,
}

impl VersionPattern {
    /// True when the given resolved version satisfies this pattern. Each part
    /// that is set must be equal. A part left open matches anything.
    pub fn matches(&self, version: &GodotVersion) -> bool {
        if self.major != version.major {
            return false;
        }
        if let Some(minor) = self.minor {
            if minor != version.minor {
                return false;
            }
        }
        if let Some(patch) = self.patch {
            if patch != version.patch {
                return false;
            }
        }
        if let Some(stage) = self.stage {
            if stage != version.stage {
                return false;
            }
        }
        true
    }

    /// Pick the newest version in the list that matches this pattern.
    pub fn best_match<'a, I>(&self, versions: I) -> Option<GodotVersion>
    where
        I: IntoIterator<Item = &'a GodotVersion>,
    {
        versions
            .into_iter()
            .filter(|v| self.matches(v))
            .max()
            .copied()
    }
}

impl fmt::Display for VersionPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.major)?;
        if let Some(minor) = self.minor {
            write!(f, ".{minor}")?;
        }
        if let Some(patch) = self.patch {
            write!(f, ".{patch}")?;
        }
        if let Some(stage) = self.stage {
            write!(f, "-{stage}")?;
        }
        Ok(())
    }
}

impl FromStr for VersionPattern {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (number_part, stage) = match s.split_once('-') {
            Some((number_part, stage_part)) => (number_part, Some(stage_part.parse()?)),
            None => (s, None),
        };
        let mut parts = number_part.split('.');
        let major = parts
            .next()
            .ok_or_else(|| VersionParseError::Number(s.to_string()))?;
        let major = parse_component(major, s)?;
        let minor = parts.next().map(|p| parse_component(p, s)).transpose()?;
        let patch = parts.next().map(|p| parse_component(p, s)).transpose()?;
        if parts.next().is_some() {
            return Err(VersionParseError::Number(s.to_string()));
        }
        Ok(VersionPattern {
            major,
            minor,
            patch,
            stage,
        })
    }
}

/// Parse the number part of a tag into major, minor, and patch. The patch
/// defaults to zero when it is not present. Older four part versions like
/// 2.0.4.1 are not handled yet.
fn parse_number_part(s: &str) -> Result<(u32, u32, u32), VersionParseError> {
    let mut parts = s.split('.');
    let major = parts
        .next()
        .ok_or_else(|| VersionParseError::Number(s.to_string()))?;
    let minor = parts
        .next()
        .ok_or_else(|| VersionParseError::Number(s.to_string()))?;
    let major = parse_component(major, s)?;
    let minor = parse_component(minor, s)?;
    let patch = match parts.next() {
        Some(p) => parse_component(p, s)?,
        None => 0,
    };
    if parts.next().is_some() {
        return Err(VersionParseError::Number(s.to_string()));
    }
    Ok((major, minor, patch))
}

fn parse_component(part: &str, whole: &str) -> Result<u32, VersionParseError> {
    part.parse()
        .map_err(|_| VersionParseError::Number(whole.to_string()))
}

/// An error from parsing a version, pattern, stage, or variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionParseError {
    /// The number part could not be read.
    Number(String),
    /// The stage word could not be read.
    Stage(String),
    /// A tag was missing its stage, for example 4.3 with no stage.
    MissingStage(String),
    /// The variant word could not be read.
    Variant(String),
}

impl fmt::Display for VersionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionParseError::Number(s) => write!(f, "invalid version number in {s:?}"),
            VersionParseError::Stage(s) => write!(f, "invalid release stage in {s:?}"),
            VersionParseError::MissingStage(s) => write!(f, "version tag {s:?} has no stage"),
            VersionParseError::Variant(s) => write!(f, "unknown variant {s:?}"),
        }
    }
}

impl std::error::Error for VersionParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_tag() {
        let v = GodotVersion::parse_tag("4.3-stable").unwrap();
        assert_eq!(v, GodotVersion::new(4, 3, 0, Stage::Stable));
    }

    #[test]
    fn parses_a_tag_with_patch() {
        let v = GodotVersion::parse_tag("4.2.1-stable").unwrap();
        assert_eq!(v, GodotVersion::new(4, 2, 1, Stage::Stable));
    }

    #[test]
    fn parses_a_prerelease_tag() {
        let v = GodotVersion::parse_tag("4.0-rc1").unwrap();
        assert_eq!(v, GodotVersion::new(4, 0, 0, Stage::Rc(1)));
        assert!(v.is_prerelease());
    }

    #[test]
    fn tag_drops_zero_patch() {
        assert_eq!(
            GodotVersion::new(4, 3, 0, Stage::Stable).to_tag(),
            "4.3-stable"
        );
        assert_eq!(
            GodotVersion::new(4, 2, 1, Stage::Stable).to_tag(),
            "4.2.1-stable"
        );
        assert_eq!(GodotVersion::new(4, 0, 0, Stage::Rc(2)).to_tag(), "4.0-rc2");
    }

    #[test]
    fn tag_round_trips() {
        for tag in [
            "4.3-stable",
            "4.2.1-stable",
            "4.0-rc1",
            "3.5-beta3",
            "4.4-dev2",
        ] {
            let v = GodotVersion::parse_tag(tag).unwrap();
            assert_eq!(v.to_tag(), tag);
        }
    }

    #[test]
    fn missing_stage_is_an_error() {
        assert!(GodotVersion::parse_tag("4.3").is_err());
    }

    #[test]
    fn stage_orders_stable_highest() {
        assert!(Stage::Stable > Stage::Rc(9));
        assert!(Stage::Rc(1) > Stage::Beta(9));
        assert!(Stage::Beta(1) > Stage::Dev(9));
        assert!(Stage::Dev(1) > Stage::Alpha(9));
        assert!(Stage::Rc(2) > Stage::Rc(1));
    }

    #[test]
    fn versions_sort_newest_last() {
        let mut versions = vec![
            GodotVersion::new(4, 3, 0, Stage::Stable),
            GodotVersion::new(4, 0, 0, Stage::Rc(1)),
            GodotVersion::new(4, 0, 0, Stage::Stable),
            GodotVersion::new(4, 2, 1, Stage::Stable),
            GodotVersion::new(3, 5, 0, Stage::Stable),
        ];
        versions.sort();
        assert_eq!(
            versions,
            vec![
                GodotVersion::new(3, 5, 0, Stage::Stable),
                GodotVersion::new(4, 0, 0, Stage::Rc(1)),
                GodotVersion::new(4, 0, 0, Stage::Stable),
                GodotVersion::new(4, 2, 1, Stage::Stable),
                GodotVersion::new(4, 3, 0, Stage::Stable),
            ]
        );
    }

    #[test]
    fn pattern_with_open_parts_is_a_wildcard() {
        let pattern: VersionPattern = "4.3".parse().unwrap();
        assert!(pattern.matches(&GodotVersion::new(4, 3, 0, Stage::Stable)));
        assert!(pattern.matches(&GodotVersion::new(4, 3, 0, Stage::Rc(1))));
        assert!(pattern.matches(&GodotVersion::new(4, 3, 2, Stage::Stable)));
        assert!(!pattern.matches(&GodotVersion::new(4, 2, 0, Stage::Stable)));
    }

    #[test]
    fn pattern_with_stage_is_exact() {
        let pattern: VersionPattern = "4.3-stable".parse().unwrap();
        assert!(pattern.matches(&GodotVersion::new(4, 3, 0, Stage::Stable)));
        assert!(!pattern.matches(&GodotVersion::new(4, 3, 0, Stage::Rc(1))));
    }

    #[test]
    fn major_only_pattern_matches_the_whole_line() {
        let pattern: VersionPattern = "4".parse().unwrap();
        assert!(pattern.matches(&GodotVersion::new(4, 0, 0, Stage::Stable)));
        assert!(pattern.matches(&GodotVersion::new(4, 3, 1, Stage::Beta(2))));
        assert!(!pattern.matches(&GodotVersion::new(3, 5, 0, Stage::Stable)));
    }

    #[test]
    fn best_match_picks_the_newest() {
        let installed = vec![
            GodotVersion::new(4, 3, 0, Stage::Stable),
            GodotVersion::new(4, 3, 1, Stage::Stable),
            GodotVersion::new(4, 3, 2, Stage::Rc(1)),
            GodotVersion::new(4, 2, 0, Stage::Stable),
        ];
        let pattern: VersionPattern = "4.3".parse().unwrap();
        assert_eq!(
            pattern.best_match(&installed),
            Some(GodotVersion::new(4, 3, 2, Stage::Rc(1)))
        );
        let stable_only: VersionPattern = "4.3-stable".parse().unwrap();
        assert_eq!(
            stable_only.best_match(&installed),
            Some(GodotVersion::new(4, 3, 1, Stage::Stable))
        );
    }

    #[test]
    fn parses_variants() {
        assert_eq!("mono".parse::<Variant>().unwrap(), Variant::Mono);
        assert_eq!("csharp".parse::<Variant>().unwrap(), Variant::Mono);
        assert_eq!("standard".parse::<Variant>().unwrap(), Variant::Standard);
        assert!("unknown".parse::<Variant>().is_err());
    }
}
