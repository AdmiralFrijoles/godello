//! A disk cache for the version list, used by the GUI only.
//!
//! Fetching the version list hits the network every time, which is wasteful when
//! the list rarely changes. The GUI caches the parsed list to a file with the
//! time it was fetched, and reuses it while it is still fresh. The command line
//! never reads this cache, so a script always sees the live list. The GUI can
//! clear the cache from settings, and a refresh always fetches and rewrites it.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use godello_core::Release;
use serde::{Deserialize, Serialize};

/// How long a cached list stays fresh, in seconds. Six hours is plenty for a
/// list that changes a few times a year, and a manual refresh always overrides
/// it.
pub const TTL_SECS: u64 = 6 * 60 * 60;

/// The on disk shape: the list and when it was fetched.
#[derive(Serialize, Deserialize)]
struct CacheFile {
    fetched_at: u64,
    releases: Vec<Release>,
}

/// Seconds since the unix epoch, or zero if the clock is before it.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Read the cached list when it exists and is still within the given lifetime.
/// Returns None when there is no cache, it cannot be read, or it has expired.
pub fn load(path: &Path, ttl_secs: u64) -> Option<Vec<Release>> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: CacheFile = serde_json::from_str(&text).ok()?;
    let age = now_secs().saturating_sub(cache.fetched_at);
    if age <= ttl_secs {
        Some(cache.releases)
    } else {
        None
    }
}

/// Write the list to the cache with the current time. Errors are ignored, since
/// a cache write failure should never fail the fetch that produced the list.
pub fn store(path: &Path, releases: &[Release]) {
    let cache = CacheFile {
        fetched_at: now_secs(),
        releases: releases.to_vec(),
    };
    let Ok(text) = serde_json::to_string(&cache) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}

/// Delete the cache file, if it exists.
pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use godello_core::{GodotVersion, Stage, Variant};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("godello-cache-tests");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    fn sample() -> Vec<Release> {
        vec![Release::new(
            GodotVersion::new(4, 3, 0, Stage::Stable),
            vec![Variant::Standard, Variant::Mono],
        )]
    }

    #[test]
    fn store_then_load_returns_the_list_when_fresh() {
        let path = scratch("fresh.json");
        store(&path, &sample());
        let loaded = load(&path, TTL_SECS).unwrap();
        assert_eq!(loaded, sample());
    }

    #[test]
    fn a_missing_cache_loads_as_none() {
        let path = scratch("does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        assert!(load(&path, TTL_SECS).is_none());
    }

    #[test]
    fn an_expired_cache_loads_as_none() {
        let path = scratch("expired.json");
        // A cache stamped far in the past is older than any sane lifetime.
        std::fs::write(&path, r#"{"fetched_at":1,"releases":[]}"#).unwrap();
        assert!(load(&path, 60).is_none());
    }

    #[test]
    fn garbage_in_the_file_loads_as_none() {
        let path = scratch("garbage.json");
        std::fs::write(&path, "not json at all").unwrap();
        assert!(load(&path, TTL_SECS).is_none());
    }

    #[test]
    fn clear_removes_the_cache() {
        let path = scratch("clear.json");
        store(&path, &sample());
        assert!(load(&path, TTL_SECS).is_some());
        clear(&path);
        assert!(load(&path, TTL_SECS).is_none());
    }
}
