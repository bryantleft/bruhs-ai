//! In-memory cache for in-flight OAuth attempts. Maps `state` →
//! `PendingPkce { verifier, created_at_ms }` with a 5-minute TTL.
//!
//! Single-use: `take` removes the entry whether or not it's expired,
//! returning `None` if the state is unknown or stale.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

const TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct PendingPkce {
    pub verifier: String,
    pub created_at_ms: i64,
}

pub struct PendingPkceCache {
    inner: Mutex<HashMap<String, PendingPkce>>,
}

impl PendingPkceCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, state: String, verifier: String, now_ms: i64) {
        // Opportunistic sweep on every insert — cheap and bounds the map.
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, v| (now_ms - v.created_at_ms) < TTL_MS);
        map.insert(
            state,
            PendingPkce {
                verifier,
                created_at_ms: now_ms,
            },
        );
    }

    /// Remove the entry for `state` and return it if still valid.
    /// Stale entries (older than TTL) return `None` AND are removed.
    pub fn take(&self, state: &str, now_ms: i64) -> Option<PendingPkce> {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = map.remove(state)?;
        drop(map);
        if (now_ms - entry.created_at_ms) >= TTL_MS {
            None
        } else {
            Some(entry)
        }
    }
}

impl Default for PendingPkceCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_take_returns_entry() {
        let cache = PendingPkceCache::new();
        cache.insert("state-1".into(), "verifier-1".into(), 1_000);
        let got = cache.take("state-1", 2_000).unwrap();
        assert_eq!(got.verifier, "verifier-1");
        assert_eq!(got.created_at_ms, 1_000);
    }

    #[test]
    fn take_is_single_use() {
        let cache = PendingPkceCache::new();
        cache.insert("state-1".into(), "v".into(), 0);
        assert!(cache.take("state-1", 1_000).is_some());
        assert!(cache.take("state-1", 1_000).is_none());
    }

    #[test]
    fn stale_entry_returns_none() {
        let cache = PendingPkceCache::new();
        cache.insert("state-1".into(), "v".into(), 0);
        // 5min + 1ms later → stale
        assert!(cache.take("state-1", 5 * 60 * 1000 + 1).is_none());
    }

    #[test]
    fn insert_sweeps_stale_entries() {
        let cache = PendingPkceCache::new();
        cache.insert("old".into(), "v".into(), 0);
        // Insert later than the TTL — this should sweep "old".
        cache.insert("new".into(), "v2".into(), 5 * 60 * 1000 + 1);
        assert!(cache.take("old", 5 * 60 * 1000 + 1).is_none());
        assert!(cache.take("new", 5 * 60 * 1000 + 1).is_some());
    }

    #[test]
    fn unknown_state_returns_none() {
        let cache = PendingPkceCache::new();
        assert!(cache.take("never-inserted", 0).is_none());
    }
}
