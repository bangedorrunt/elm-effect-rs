//! `LayerMap` port (`LayerMap.ts` at the pin): memoized keyed builds.
//!
//! One built value per key under an explicit idle policy: `Keep` is the pin's
//! `idleTimeToLive: 0` (nothing is kept once the last borrow ends, and a
//! preload pass skips the key), `Finite` is an idle window, `Never` is the
//! pin's infinity. Time is virtual because the pin's rows drive
//! `TestClock.adjust`: [`advance`](LayerMap::advance) is what releases an
//! expired entry, so nothing here waits on the wall clock.
//!
//! `None` for `idle_time_to_live` is the pin's default and behaves as `Keep`,
//! which is why `make` builds nothing until first use.
//!
//! Residuals: `Service` tag derivation beyond the named wrapper, layer-scoped
//! memo lifetimes, and the reference scope machinery.

use std::collections::HashMap;
use std::convert::Infallible;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The idle policy for one key: the pin's `idleTimeToLive` in its three
/// observable cases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdlePolicy {
    /// Zero: the entry is dropped as soon as its last borrow ends, and a
    /// preload pass skips it.
    Keep,
    /// A finite idle window: an entry idle for this long is released.
    Finite(Duration),
    /// Infinity: the entry never expires.
    Never,
}

impl IdlePolicy {
    /// `true` when the policy holds an entry long enough to be worth
    /// preloading. This is the rule behind "preloads only keys with non-zero
    /// idle TTL".
    pub fn is_cacheable(self) -> bool {
        !matches!(self, IdlePolicy::Keep)
    }

    fn expired(self, idle_for: Duration) -> bool {
        match self {
            IdlePolicy::Keep => true,
            IdlePolicy::Finite(ttl) => idle_for >= ttl,
            IdlePolicy::Never => false,
        }
    }
}

/// The per-key TTL function (`idleTimeToLive` as a function of the key).
pub type IdlePolicyFn<K> = Box<dyn Fn(&K) -> IdlePolicy + Send + Sync>;

type Build<K, V, E> = Box<dyn Fn(&K) -> Result<V, E> + Send + Sync>;
type Release<V> = Box<dyn Fn(&V) + Send + Sync>;
/// A record field's constructor: `LayerMap.fromRecord`'s fixed entries.
type LayerCtor<V> = Box<dyn Fn() -> V + Send + Sync>;

struct MapEntry<V> {
    value: V,
    refs: usize,
    /// When the entry last became idle. Only meaningful while `refs == 0`.
    idle_since: Duration,
    policy: IdlePolicy,
}

/// Memoized keyed builds with idle expiry and an explicit release hook.
pub struct LayerMap<K, V, E = Infallible> {
    build: Build<K, V, E>,
    release: Release<V>,
    idle: Option<IdlePolicyFn<K>>,
    /// Static TTL set through [`set_idle_ttl`](LayerMap::set_idle_ttl),
    /// overridden per key when `idle` answers for it.
    default_policy: Option<IdlePolicy>,
    /// `fromRecord`: the keys this map serves, and how to build each.
    record: Option<HashMap<K, LayerCtor<V>>>,
    now: Duration,
    entries: HashMap<K, MapEntry<V>>,
}

// The infallible constructors are their own impl block: with `E` still open,
// `LayerMap::make(..)` cannot infer it, and the default does not apply to
// inference.
impl<K, V> LayerMap<K, V, Infallible>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Build from a keyed constructor (`make`), with no release hook and the
    /// default policy: entries are built on first use and kept while borrowed.
    pub fn make(build: impl Fn(&K) -> V + Send + Sync + 'static) -> Self {
        Self::assemble(
            Box::new(move |k| Ok(build(k))),
            Box::new(|_| {}),
            None,
            None,
            None,
        )
    }

    /// `make` with `preloadKeys`: build the listed keys eagerly and hold one
    /// borrow on each, so the port's convenience form keeps what it preloads.
    /// Without preload nothing is built, which is what the row "make does not
    /// preload with the default idle TTL" observes.
    pub fn make_with_preload(
        build: impl Fn(&K) -> V + Send + Sync + 'static,
        preload: Option<Vec<K>>,
    ) -> Self {
        let mut this = Self::make(build);
        if let Some(keys) = preload {
            for key in keys {
                this.get(&key);
            }
        }
        this
    }

    /// `LayerMap.make(lookup, { idleTimeToLive })`: preload only the keys the
    /// policy would keep, and hold no borrow on them, so their idle window
    /// starts at construction the way the pin's timer does.
    pub fn make_idle(
        build: impl Fn(&K) -> V + Send + Sync + 'static,
        release: impl Fn(&V) + Send + Sync + 'static,
        preload_keys: Vec<K>,
        idle_time_to_live: Option<IdlePolicyFn<K>>,
    ) -> Self {
        Self::try_make_idle(
            move |k| Ok::<V, Infallible>(build(k)),
            release,
            preload_keys,
            idle_time_to_live,
        )
        .expect("an infallible build cannot fail")
    }

    /// `LayerMap.fromRecord`: one built value per record field. A key outside
    /// the record is not this map's to build, so it answers `None`.
    pub fn from_record(
        entries: Vec<(K, LayerCtor<V>)>,
        release: impl Fn(&V) + Send + Sync + 'static,
        idle_time_to_live: Option<IdlePolicyFn<K>>,
    ) -> Self {
        let record: HashMap<K, LayerCtor<V>> = entries.into_iter().collect();
        Self::assemble(
            // Never reached: a record map builds through its field instead,
            // and `context_effect` checks membership first.
            Box::new(|_| unreachable!("a record map builds from its fields")),
            Box::new(release),
            idle_time_to_live,
            Some(record),
            None,
        )
    }
}

impl<K, V, E> LayerMap<K, V, E>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// The fallible form. A preload that fails fails construction, which is
    /// the pin's row "make fails construction when a non-zero TTL preload
    /// fails".
    pub fn try_make_idle(
        build: impl Fn(&K) -> Result<V, E> + Send + Sync + 'static,
        release: impl Fn(&V) + Send + Sync + 'static,
        preload_keys: Vec<K>,
        idle_time_to_live: Option<IdlePolicyFn<K>>,
    ) -> Result<Self, E> {
        let mut this = Self::assemble(
            Box::new(build),
            Box::new(release),
            idle_time_to_live,
            None,
            None,
        );
        for key in preload_keys {
            let policy = this.policy_for(&key);
            if !policy.is_cacheable() {
                continue;
            }
            let value = (this.build)(&key)?;
            this.entries.insert(
                key,
                MapEntry {
                    value,
                    refs: 0,
                    idle_since: this.now,
                    policy,
                },
            );
        }
        Ok(this)
    }

    fn assemble(
        build: Build<K, V, E>,
        release: Release<V>,
        idle: Option<IdlePolicyFn<K>>,
        record: Option<HashMap<K, LayerCtor<V>>>,
        default_policy: Option<IdlePolicy>,
    ) -> Self {
        Self {
            build,
            release,
            idle,
            default_policy,
            record,
            now: Duration::ZERO,
            entries: HashMap::new(),
        }
    }

    /// The policy in force for `key`.
    pub fn policy_for(&self, key: &K) -> IdlePolicy {
        match &self.idle {
            Some(idle) => idle(key),
            None => self.default_policy.unwrap_or(IdlePolicy::Keep),
        }
    }

    /// Set a static idle TTL for every key (`idleTimeToLive` as a duration).
    pub fn set_idle_ttl(&mut self, ttl: Option<Duration>) {
        self.default_policy = ttl.map(IdlePolicy::Finite);
    }

    /// The virtual clock this map measures idle windows against.
    pub fn now(&self) -> Duration {
        self.now
    }

    fn is_expired(&self, entry: &MapEntry<V>) -> bool {
        if entry.refs > 0 {
            return false;
        }
        entry
            .policy
            .expired(self.now.saturating_sub(entry.idle_since))
    }

    fn release_value(&self, entry: MapEntry<V>) {
        (self.release)(&entry.value);
    }

    /// Build `key` if it is missing or expired, then hold one borrow on it.
    /// This is `contextEffect`; the borrow ends at [`release`](Self::release).
    pub fn context_effect(&mut self, key: &K) -> Option<V> {
        if let Some(record) = &self.record {
            if !record.contains_key(key) {
                return None;
            }
        }
        let stale = match self.entries.get(key) {
            None => true,
            Some(entry) => self.is_expired(entry),
        };
        if stale
            && let Some(entry) = self.entries.remove(key)
        {
            self.release_value(entry);
        }
        if !self.entries.contains_key(key) {
            let policy = self.policy_for(key);
            let built = match &self.record {
                Some(record) => record
                    .get(key)
                    .map(|ctor| Ok::<V, E>(ctor()))
                    .unwrap_or_else(|| (self.build)(key)),
                None => (self.build)(key),
            };
            match built {
                Ok(value) => {
                    self.entries.insert(
                        key.clone(),
                        MapEntry {
                            value,
                            refs: 0,
                            idle_since: self.now,
                            policy,
                        },
                    );
                }
                // A failed read builds nothing; a cached failure is not among
                // the carried rows, so it is not invented here.
                Err(_) => return None,
            }
        }
        let entry = self.entries.get_mut(key)?;
        entry.refs += 1;
        Some(entry.value.clone())
    }

    /// A borrow that ends immediately: the pin's `Effect.scoped(contextEffect)`.
    pub fn scoped(&mut self, key: &K) -> Option<V> {
        let value = self.context_effect(key);
        self.release(key);
        value
    }

    /// Drop one borrow. The last one starts the idle window, and under `Keep`
    /// the entry goes with it.
    pub fn release(&mut self, key: &K) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.refs > 0 {
            entry.refs -= 1;
        }
        if entry.refs > 0 {
            return true;
        }
        entry.idle_since = self.now;
        if entry.policy == IdlePolicy::Keep {
            let entry = self.entries.remove(key).expect("just read");
            self.release_value(entry);
        }
        true
    }

    /// `contextEffectOption`: an existing, unexpired entry, without building
    /// one and without holding a borrow. This is the row's "does not build
    /// missing entries and returns existing entries".
    pub fn context_effect_option(&mut self, key: &K) -> Option<V> {
        if let Some(record) = &self.record {
            if !record.contains_key(key) {
                return None;
            }
        }
        let entry = self.entries.get(key)?;
        if self.is_expired(entry) {
            return None;
        }
        Some(entry.value.clone())
    }

    /// Advance the virtual clock and release every entry whose idle window
    /// closed. The pin gets this from `TestClock.adjust` firing its timers.
    pub fn advance(&mut self, delta: Duration) {
        self.now = self.now.saturating_add(delta);
        let expired: Vec<K> = self
            .entries
            .iter()
            .filter(|(_, entry)| self.is_expired(entry))
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            if let Some(entry) = self.entries.remove(&key) {
                self.release_value(entry);
            }
        }
    }

    /// Memoized value for `key`, building once and holding the borrow.
    pub fn get(&mut self, key: &K) -> Option<V> {
        self.context_effect(key)
    }

    /// Forget `key` so the next lookup rebuilds.
    pub fn invalidate(&mut self, key: &K) -> bool {
        match self.entries.remove(key) {
            None => false,
            Some(entry) => {
                self.release_value(entry);
                true
            }
        }
    }

    /// Restart the idle clock for `key`; `false` when absent.
    pub fn touch(&mut self, key: &K) -> bool {
        match self.entries.get_mut(key) {
            None => false,
            Some(entry) => {
                entry.idle_since = self.now;
                true
            }
        }
    }

    /// Live (unexpired) keys.
    pub fn keys(&self) -> Vec<K> {
        self.entries
            .iter()
            .filter(|(_, entry)| !self.is_expired(entry))
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Cached entry count.
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// Borrows currently held for `key`.
    pub fn ref_count(&self, key: &K) -> usize {
        self.entries.get(key).map(|entry| entry.refs).unwrap_or(0)
    }
}

/// `LayerMap.Service`: the same map behind a name, so a caller reaches it by
/// lookup the way the pin reaches a service tag. [`install`](Self::install)
/// is the port's `layer`.
pub struct LayerMapService<K, V, E = Infallible> {
    name: String,
    map: Arc<Mutex<LayerMap<K, V, E>>>,
}

// Same split as `LayerMap`: the record service has no failure type to infer.
impl<K, V> LayerMapService<K, V, Infallible>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// A service over a record of layers (`LayerMap.Service` with `layers`).
    pub fn install_record(
        name: impl Into<String>,
        entries: Vec<(K, LayerCtor<V>)>,
        release: impl Fn(&V) + Send + Sync + 'static,
        idle_time_to_live: Option<IdlePolicyFn<K>>,
    ) -> Self {
        Self {
            name: name.into(),
            map: Arc::new(Mutex::new(LayerMap::from_record(
                entries,
                release,
                idle_time_to_live,
            ))),
        }
    }
}

impl<K, V, E> LayerMapService<K, V, E>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// A service over a lookup function (`LayerMap.Service` with `lookup`).
    pub fn install(
        name: impl Into<String>,
        build: impl Fn(&K) -> Result<V, E> + Send + Sync + 'static,
        release: impl Fn(&V) + Send + Sync + 'static,
        preload_keys: Vec<K>,
        idle_time_to_live: Option<IdlePolicyFn<K>>,
    ) -> Result<Self, E> {
        Ok(Self {
            name: name.into(),
            map: Arc::new(Mutex::new(LayerMap::try_make_idle(
                build,
                release,
                preload_keys,
                idle_time_to_live,
            )?)),
        })
    }

    /// The service's name, which is its identity in place of a tag.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn with<R>(&self, f: impl FnOnce(&mut LayerMap<K, V, E>) -> R) -> R {
        let mut map = self.map.lock().expect("layer map lock");
        f(&mut map)
    }

    /// `contextEffect`: build if missing, then hold the borrow.
    pub fn context_effect(&self, key: &K) -> Option<V> {
        self.with(|map| map.context_effect(key))
    }

    /// `contextEffectOption`: existing entries only.
    pub fn context_effect_option(&self, key: &K) -> Option<V> {
        self.with(|map| map.context_effect_option(key))
    }

    /// A borrow that ends immediately.
    pub fn scoped(&self, key: &K) -> Option<V> {
        self.with(|map| map.scoped(key))
    }

    /// Drop one borrow.
    pub fn release(&self, key: &K) -> bool {
        self.with(|map| map.release(key))
    }

    /// Advance the shared virtual clock.
    pub fn advance(&self, delta: Duration) {
        self.with(|map| map.advance(delta));
    }
}
