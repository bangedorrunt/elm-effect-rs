//! `RcMap` port (`RcMap.ts` at the pin): keyed reference-counted cache.
//!
//! One big lock linearizes retention before invalidation, which also makes
//! concurrent lookups share in-flight acquisitions (the second lookup waits
//! on the lock and then sees the cached value). Failures cache like values
//! and propagate to every waiter. Idle TTL, [`touch`](RcMap::touch),
//! [`capacity`](RcMap::set_capacity) eviction of unreferenced entries, and
//! [`close`](RcMap::close) (later lookups return `None`) mirror the tag.
//!
//! Residuals: per-key acquisition concurrency (entries acquire serially
//! here), background TTL sweep (expiry is lazy on lookup/touch), and
//! weak-value collectors.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

struct Entry<V, E> {
    value: Option<Result<V, E>>,
    refs: usize,
    idle_since: Option<Instant>,
    last_used: Instant,
}

/// Lookup constructor: key in, shared value or cached failure out.
type Lookup<K, V, E> = Box<dyn Fn(&K) -> Result<V, E> + Send + Sync>;
/// Release hook for an evicted value.
type Release<V> = Box<dyn Fn(V) + Send + Sync>;

/// Keyed cache with per-entry reference counts. `K` is unconstrained beyond
/// hashing: complex keys work by value.
pub struct RcMap<K, V, E> {
    lookup: Lookup<K, V, E>,
    release: Release<V>,
    idle_ttl: Option<Duration>,
    capacity: Option<usize>,
    closed: bool,
    entries: HashMap<K, Entry<V, E>>,
}

impl<K, V, E> RcMap<K, V, E>
where
    K: Hash + Eq + Clone,
{
    /// Build from lookup/release closures (`make`).
    pub fn make(
        lookup: impl Fn(&K) -> Result<V, E> + Send + Sync + 'static,
        release: impl Fn(V) + Send + Sync + 'static,
    ) -> Self {
        Self {
            lookup: Box::new(lookup),
            release: Box::new(release),
            idle_ttl: None,
            capacity: None,
            closed: false,
            entries: HashMap::new(),
        }
    }

    /// Set the idle TTL for unreferenced entries.
    pub fn set_idle_ttl(&mut self, ttl: Option<Duration>) {
        self.idle_ttl = ttl;
    }

    /// Cap live entries; over-cap inserts evict unreferenced oldest-idle
    /// entries first.
    pub fn set_capacity(&mut self, capacity: Option<usize>) {
        self.capacity = capacity;
        self.evict();
    }

    fn expired(&self, since: Instant) -> bool {
        match self.idle_ttl {
            None => true,
            Some(ttl) => since.elapsed() >= ttl,
        }
    }

    fn evict(&mut self) {
        let Some(cap) = self.capacity else {
            return;
        };
        while self.entries.len() > cap {
            let victim = self
                .entries
                .iter()
                .filter(|(_, e)| e.refs == 0)
                .min_by_key(|(_, e)| e.idle_since.or(e.last_used.into()))
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    if let Some(entry) = self.entries.remove(&k)
                        && let Some(Ok(v)) = entry.value
                    {
                        (self.release)(v);
                    }
                }
                None => break,
            }
        }
    }

    /// Shared value for `key`, acquiring once. `None` when closed. Cached
    /// failures propagate (the `Err` is cloned out like values).
    pub fn get(&mut self, key: &K) -> Option<Result<V, E>>
    where
        V: Clone,
        E: Clone,
    {
        if self.closed {
            return None;
        }
        let now = Instant::now();
        let stale = match self.entries.get(key) {
            None => true,
            Some(e) if e.refs == 0 => match (e.idle_since, &e.value) {
                (Some(since), Some(_)) => self.expired(since),
                (None, None) => true,
                _ => false,
            },
            Some(_) => false,
        };
        if stale
            && let Some(old) = self.entries.remove(key)
            && let Some(Ok(v)) = old.value
        {
            (self.release)(v);
        }
        let entry = self.entries.entry(key.clone()).or_insert_with(|| Entry {
            value: None,
            refs: 0,
            idle_since: None,
            last_used: now,
        });
        if entry.value.is_none() {
            entry.value = Some((self.lookup)(key));
            entry.last_used = Instant::now();
        }
        entry.refs += 1;
        entry.idle_since = None;
        let out = entry.value.clone();
        self.evict();
        out
    }

    /// Return one reference to `key`.
    pub fn release_ref(&mut self, key: &K) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.refs = entry.refs.saturating_sub(1);
            if entry.refs == 0 {
                match self.idle_ttl {
                    None => {
                        if let Some(Ok(v)) = entry.value.take() {
                            (self.release)(v);
                        }
                        entry.idle_since = None;
                    }
                    Some(_) => {
                        entry.idle_since = Some(Instant::now());
                    }
                }
            }
        }
    }

    /// Reference count held for `key`.
    pub fn ref_count(&self, key: &K) -> usize {
        self.entries.get(key).map(|e| e.refs).unwrap_or(0)
    }

    /// Reset the idle clock for `key`; `false` when absent.
    pub fn touch(&mut self, key: &K) -> bool {
        match self.entries.get_mut(key) {
            None => false,
            Some(entry) => {
                entry.idle_since = Some(Instant::now());
                entry.last_used = Instant::now();
                true
            }
        }
    }

    /// Release and forget `key`; `false` when absent.
    pub fn invalidate(&mut self, key: &K) -> bool {
        match self.entries.remove(key) {
            None => false,
            Some(entry) => {
                if let Some(Ok(v)) = entry.value {
                    (self.release)(v);
                }
                true
            }
        }
    }

    /// Live keys.
    pub fn keys(&self) -> Vec<K> {
        self.entries.keys().cloned().collect()
    }

    /// Cached entry count.
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// Release everything; later lookups return `None`.
    pub fn close(&mut self) {
        self.release_all();
        self.closed = true;
    }

    /// `true` after [`close`](RcMap::close).
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl<K, V, E> RcMap<K, V, E> {
    fn release_all(&mut self) {
        for (_, entry) in std::mem::take(&mut self.entries) {
            if let Some(Ok(v)) = entry.value {
                (self.release)(v);
            }
        }
    }
}

impl<K, V, E> Drop for RcMap<K, V, E> {
    fn drop(&mut self) {
        self.release_all();
    }
}
