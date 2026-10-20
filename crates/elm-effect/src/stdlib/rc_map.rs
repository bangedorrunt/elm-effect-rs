//! `RcMap` port (`RcMap.ts` at the pin): keyed reference-counted cache.
//!
//! One big lock linearizes retention before invalidation, which also makes
//! concurrent lookups share in-flight acquisitions (the second lookup waits on
//! the lock and then sees the cached value). Failures cache like values and
//! propagate to every waiter.
//!
//! Capacity is the pin's rule: a hard limit that refuses a new key with
//! [`RcMapError::ExceededCapacity`], never an eviction to make room. Idle TTL is
//! read per key and counted on an injected [`RcClock`], so a test advances
//! virtual milliseconds instead of sleeping. [`expire_idle`](RcMap::expire_idle)
//! is the sweep the pin runs from a timer.
//!
//! Invalidation defers: an entry that is still referenced leaves the map but
//! keeps its value until the last borrower returns, so a replacement under the
//! same key has its own lifecycle (row 281; 54 and 103 additionally depend on
//! the pin's per-entry idle fiber, which this port does not model).
//!
//! Residuals: in-flight sharing with a deferred, and a background TTL sweep
//! followed by concurrent close (expiry is lazy on access, or on
//! `expire_idle`). A lookup that dies is no longer a residual: it reports a
//! [`LookupFailure::Defect`], which caches and occupies capacity exactly like a
//! typed failure, so a later borrower sees the same defect.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Millisecond time source for idle TTL.
///
/// Shaped like the crate's `ClockImpl` (`now_millis`), but declared here on
/// purpose: this module is compiled through `#[path]` includes from the test
/// binaries, where `crate::time` is not in scope.
pub trait RcClock: Send + Sync + 'static {
    /// Current time in milliseconds. Virtual for tests, wall for the default.
    fn now_millis(&self) -> u64;
}

/// Wall clock: the default when no clock is injected.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRcClock;

impl RcClock for SystemRcClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// An unexpected failure from a lookup: the pin's throw or die.
///
/// Message-only on purpose. This module is compiled through `#[path]` includes
/// from test binaries, where `crate::cause::Defect` is out of reach (the same
/// reason [`RcClock`] exists here); a cached defect only has to compare equal
/// to the one it was built from, and that is its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defect {
    /// What the lookup threw or died with.
    pub message: String,
}

impl Defect {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// What a lookup may report: a value, a typed failure, or a defect.
///
/// Kept apart from `E` because the pin never collapses the three: a defect is
/// not a typed error, and a caller may recover from one without the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupFailure<E> {
    /// The expected failure; cached like a value.
    Fail(E),
    /// The lookup died; cached like a value, and replayed as a defect.
    Defect(Defect),
}

/// A lookup's own error, or the capacity refusal (`E | ExceededCapacityError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RcMapError<E> {
    /// The lookup failed; the failure is cached like a value.
    Lookup(E),
    /// The lookup died; the defect is cached like a value and replayed.
    Defect(Defect),
    /// The map already holds `capacity` entries.
    ExceededCapacity(usize),
}

struct Entry<V, E> {
    value: Option<Result<V, LookupFailure<E>>>,
    refs: usize,
    idle_since_ms: Option<u64>,
    last_used_ms: u64,
}

/// Lookup constructor: key in, shared value or cached failure out.
type Lookup<K, V, E> = Box<dyn Fn(&K) -> Result<V, LookupFailure<E>> + Send + Sync>;
/// Release hook for an evicted value.
type Release<V> = Box<dyn Fn(V) + Send + Sync>;
/// Per-key idle time-to-live (`idleTimeToLive` as a function in the pin).
type IdleTtlFn<K> = Box<dyn Fn(&K) -> Option<Duration> + Send + Sync>;

/// Keyed cache with per-entry reference counts. `K` is unconstrained beyond
/// hashing: complex keys work by value.
pub struct RcMap<K, V, E> {
    lookup: Lookup<K, V, E>,
    release: Release<V>,
    idle_ttl: Option<Duration>,
    idle_ttl_fn: Option<IdleTtlFn<K>>,
    capacity: Option<usize>,
    closed: bool,
    clock: Arc<dyn RcClock>,
    entries: HashMap<K, Entry<V, E>>,
    /// Invalidated entries still held by a borrower. Not reachable through the
    /// map: `has`, `size` and capacity only see `entries`.
    retired: Vec<(K, Entry<V, E>)>,
}

impl<K, V, E> RcMap<K, V, E>
where
    K: Hash + Eq + Clone,
{
    /// Build from lookup/release closures (`make`).
    ///
    /// A lookup that has a defect to report uses
    /// [`make_with_defects`](Self::make_with_defects) instead; this spelling
    /// only answers typed failures, so every existing map keeps its shape.
    pub fn make(
        lookup: impl Fn(&K) -> Result<V, E> + Send + Sync + 'static,
        release: impl Fn(V) + Send + Sync + 'static,
    ) -> Self {
        Self::make_with_defects(move |key| lookup(key).map_err(LookupFailure::Fail), release)
    }

    /// Build from a lookup that may report a defect as well as a typed
    /// failure. Both are cached and replayed to later borrowers.
    pub fn make_with_defects(
        lookup: impl Fn(&K) -> Result<V, LookupFailure<E>> + Send + Sync + 'static,
        release: impl Fn(V) + Send + Sync + 'static,
    ) -> Self {
        Self {
            lookup: Box::new(lookup),
            release: Box::new(release),
            idle_ttl: None,
            idle_ttl_fn: None,
            capacity: None,
            closed: false,
            clock: Arc::new(SystemRcClock),
            entries: HashMap::new(),
            retired: Vec::new(),
        }
    }

    /// Read idle time from `clock` instead of the wall (`TestClock` wiring).
    pub fn with_clock(mut self, clock: Arc<dyn RcClock>) -> Self {
        self.clock = clock;
        self
    }

    /// Set one idle TTL for every key.
    pub fn set_idle_ttl(&mut self, ttl: Option<Duration>) {
        self.idle_ttl = ttl;
    }

    /// Read the idle TTL per key (`idleTimeToLive: (key) => ...`).
    pub fn set_idle_ttl_fn(
        &mut self,
        ttl: impl Fn(&K) -> Option<Duration> + Send + Sync + 'static,
    ) {
        self.idle_ttl_fn = Some(Box::new(ttl));
    }

    /// Cap live entries; a new key past the cap is refused, never evicted for.
    pub fn set_capacity(&mut self, capacity: Option<usize>) {
        self.capacity = capacity;
    }

    fn now(&self) -> u64 {
        self.clock.now_millis()
    }

    fn ttl_configured(&self) -> bool {
        self.idle_ttl_fn.is_some() || self.idle_ttl.is_some_and(|ttl| !ttl.is_zero())
    }

    /// The key's idle TTL: the per-key function wins, then the fixed value.
    /// `None` and zero both mean "release on the last reference".
    fn ttl_for(&self, key: &K) -> Option<Duration> {
        match &self.idle_ttl_fn {
            Some(f) => f(key),
            None => self.idle_ttl,
        }
    }

    fn expired(&self, key: &K, idle_since_ms: u64, now: u64) -> bool {
        match self.ttl_for(key) {
            None => true,
            Some(ttl) if ttl.is_zero() => true,
            Some(ttl) => now.saturating_sub(idle_since_ms) >= ttl.as_millis() as u64,
        }
    }

    /// Whether `key` is retained (idle counts, per the pin's `has`).
    pub fn has(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Shared value for `key`, acquiring once. `None` when closed.
    ///
    /// A new key on a full map fails with
    /// [`RcMapError::ExceededCapacity`], which is the pin's rule: capacity is a
    /// limit, not an eviction pressure. Cached failures propagate.
    pub fn get(&mut self, key: &K) -> Option<Result<V, RcMapError<E>>>
    where
        V: Clone,
        E: Clone,
    {
        if self.closed {
            return None;
        }
        let now = self.now();
        self.sweep(now);

        if self.entries.contains_key(key) {
            let entry = self.entries.get_mut(key).expect("just checked");
            entry.refs += 1;
            entry.idle_since_ms = None;
            entry.last_used_ms = now;
            return self.cached(key);
        }

        if let Some(cap) = self.capacity
            && self.entries.len() >= cap
        {
            return Some(Err(RcMapError::ExceededCapacity(cap)));
        }

        let value = (self.lookup)(key);
        self.entries.insert(
            key.clone(),
            Entry {
                value: Some(value),
                refs: 1,
                idle_since_ms: None,
                last_used_ms: now,
            },
        );
        self.cached(key)
    }

    /// Retain and return a cached entry without invoking the lookup
    /// (`getOption`): `None` when the key is absent or the map is closed.
    ///
    /// This is the pin's non-acquiring read, so a miss costs neither a lookup
    /// nor a capacity slot.
    pub fn get_option(&mut self, key: &K) -> Option<Result<V, RcMapError<E>>>
    where
        V: Clone,
        E: Clone,
    {
        if self.closed || !self.entries.contains_key(key) {
            return None;
        }
        let now = self.now();
        let entry = self.entries.get_mut(key).expect("just checked");
        entry.refs += 1;
        entry.idle_since_ms = None;
        entry.last_used_ms = now;
        self.cached(key)
    }

    /// The cached success or failure at `key`, cloned out.
    fn cached(&self, key: &K) -> Option<Result<V, RcMapError<E>>>
    where
        V: Clone,
        E: Clone,
    {
        match self.entries.get(key)?.value.as_ref()? {
            Ok(v) => Some(Ok(v.clone())),
            Err(LookupFailure::Fail(e)) => Some(Err(RcMapError::Lookup(e.clone()))),
            Err(LookupFailure::Defect(d)) => Some(Err(RcMapError::Defect(d.clone()))),
        }
    }

    /// Return one reference to `key`.
    ///
    /// The last reference removes the entry when the key's TTL is zero or
    /// unset, and arms the idle clock otherwise.
    ///
    /// A retired generation is drained first: after an invalidate plus a fresh
    /// `get`, the outstanding reference is the invalidated borrower's, and the
    /// pin closes that entry rather than the replacement. The pin tells the two
    /// apart with one scope finalizer per acquisition; this port releases by
    /// key, so it assumes the older generation's holder returns first. Two
    /// holders on the same key across an invalidate therefore cannot be told
    /// apart, which is the lease handle row 103 wants.
    pub fn release_ref(&mut self, key: &K) {
        if self.release_retired(key) {
            return;
        }
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        entry.refs = entry.refs.saturating_sub(1);
        if entry.refs > 0 {
            return;
        }
        let now = self.now();
        let ttl = self.ttl_for(key);
        if ttl.is_none_or(|ttl| ttl.is_zero()) {
            self.drop_entry(key);
            return;
        }
        if let Some(entry) = self.entries.get_mut(key) {
            entry.idle_since_ms = Some(now);
            entry.last_used_ms = now;
        }
    }

    /// Reference count held for `key`.
    pub fn ref_count(&self, key: &K) -> usize {
        self.entries.get(key).map(|e| e.refs).unwrap_or(0)
    }

    /// Reset the idle clock for `key`; `false` when absent.
    pub fn touch(&mut self, key: &K) -> bool {
        let now = self.now();
        match self.entries.get_mut(key) {
            None => false,
            Some(entry) => {
                entry.idle_since_ms = Some(now);
                entry.last_used_ms = now;
                true
            }
        }
    }

    /// Remove `key` from the map. A value someone still holds is released when
    /// that last borrower returns, not here: the pin's invalidation retires the
    /// entry instead of tearing it down under a live reference.
    pub fn invalidate(&mut self, key: &K) -> bool {
        let Some(entry) = self.entries.remove(key) else {
            return false;
        };
        if entry.refs > 0 {
            self.retired.push((key.clone(), entry));
        } else {
            self.release_entry(entry);
        }
        true
    }

    /// Give back one reference to a retired entry; the last one releases it.
    /// `true` when a retired entry consumed the release.
    fn release_retired(&mut self, key: &K) -> bool {
        let Some(index) = self.retired.iter().rposition(|(k, _)| k == key) else {
            return false;
        };
        self.retired[index].1.refs = self.retired[index].1.refs.saturating_sub(1);
        if self.retired[index].1.refs == 0 {
            let (_, entry) = self.retired.remove(index);
            self.release_entry(entry);
        }
        true
    }

    /// Release every idle entry whose TTL elapsed; the pin's timer, made
    /// explicit so tests advance virtual time instead of sleeping.
    pub fn expire_idle(&mut self) -> usize {
        let now = self.now();
        self.sweep(now)
    }

    // ponytail: one linear scan per get while a TTL is configured; a deadline
    // queue (or the pin's per-entry timer) if the map ever gets large.
    fn sweep(&mut self, now: u64) -> usize {
        if !self.ttl_configured() {
            return 0;
        }
        let victims: Vec<K> = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                entry.refs == 0
                    && entry
                        .idle_since_ms
                        .is_some_and(|since| self.expired(key, since, now))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &victims {
            self.drop_entry(key);
        }
        victims.len()
    }

    fn drop_entry(&mut self, key: &K) {
        if let Some(entry) = self.entries.remove(key) {
            self.release_entry(entry);
        }
    }

    /// Live keys.
    pub fn keys(&self) -> Vec<K> {
        self.entries.keys().cloned().collect()
    }

    /// Retained entry count (idle entries count, per the pin's capacity rule).
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
    /// Run the release hook for a value that left the map. No key bounds: a
    /// retired entry hands its value over without naming the key again.
    fn release_entry(&mut self, entry: Entry<V, E>) {
        if let Some(Ok(value)) = entry.value {
            (self.release)(value);
        }
    }

    /// Release every retained value. No key bounds: `drain` needs none, which
    /// is what lets `Drop` call it for any `K`.
    fn release_all(&mut self) {
        let live: Vec<Entry<V, E>> = self.entries.drain().map(|(_, entry)| entry).collect();
        for entry in live {
            self.release_entry(entry);
        }
        let retired: Vec<Entry<V, E>> = self.retired.drain(..).map(|(_, entry)| entry).collect();
        for entry in retired {
            self.release_entry(entry);
        }
    }
}

impl<K, V, E> Drop for RcMap<K, V, E> {
    fn drop(&mut self) {
        self.release_all();
    }
}
