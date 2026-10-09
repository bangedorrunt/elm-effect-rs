//! `LayerMap` port (`LayerMap.ts` at the pin): memoized keyed builds.
//!
//! A `LayerMap` memoizes one built value per key with an explicit idle
//! policy, like [`RcMap`](super::rc_map::RcMap) without reference counting:
//! values build once, refresh on [`invalidate`](LayerMap::invalidate), and
//! go stale past the TTL. The tag types this over `Layer`; the Rust spelling
//! builds over plain closures (layers reduce to their constructor here).
//!
//! Residuals: `Service` tag derivation, layer-scoped memo lifetimes, and
//! dynamic per-key TTL updates.

use super::rc_map::RcMap;
use std::hash::Hash;
use std::time::Duration;

/// Memoized keyed builds with idle expiry.
pub struct LayerMap<K, V> {
    inner: RcMap<K, V, String>,
}

impl<K, V> LayerMap<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Build from a keyed constructor (`make`).
    pub fn make(build: impl Fn(&K) -> V + Send + Sync + 'static) -> Self {
        Self {
            inner: RcMap::make(move |k| Ok::<V, String>(build(k)), |_| {}),
        }
    }

    /// `LayerMap.make` with `preloadKeys`: build the listed keys eagerly.
    ///
    /// Without preload, `make` asserts nothing about which keys exist yet --
    /// the default TTL means nothing is built until first use, which is what
    /// the row "make does not preload with the default idle TTL" observes.
    /// Callers that want eager construction pass `Some(keys)`; `None` keeps
    /// the existing lazy behavior.
    pub fn make_with_preload(
        build: impl Fn(&K) -> V + Send + Sync + 'static,
        preload: Option<Vec<K>>,
    ) -> Self {
        let mut this = Self::make(build);
        if let Some(keys) = preload {
            for key in keys {
                // Build through get(), which memoizes on first use.
                this.get(&key);
            }
        }
        this
    }

    /// Set the idle TTL (`idleTimeToLive` shape, static form).
    pub fn set_idle_ttl(&mut self, ttl: Option<Duration>) {
        self.inner.set_idle_ttl(ttl);
    }

    /// Memoized value for `key`, building once. The memo holds its
    /// reference until [`invalidate`](LayerMap::invalidate): reads never
    /// release.
    pub fn get(&mut self, key: &K) -> Option<V> {
        match self.inner.get(key) {
            Some(Ok(v)) => Some(v),
            _ => None,
        }
    }

    /// Forget `key` so the next lookup rebuilds.
    pub fn invalidate(&mut self, key: &K) -> bool {
        self.inner.invalidate(key)
    }

    /// Reset the idle clock; `false` when absent (`touch` shape).
    pub fn touch(&mut self, key: &K) -> bool {
        self.inner.touch(key)
    }

    /// Live keys.
    pub fn keys(&self) -> Vec<K> {
        self.inner.keys()
    }
}
