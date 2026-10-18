//! Cache single-flight, request batching, pools, scoped caches, and one
//! memoized run.
//!
//! One [`Cache`] lookup runs once no matter how many callers miss together:
//! the first caller for a key puts up a pending row and runs the lookup
//! without the map lock, and the rest wait on that row, so distinct keys
//! look up in parallel. An in-flight row is a visible row (`has`, `size`,
//! and `getSuccess` see it), and a completion touches the row only while it
//! still owns it, so a value written during the lookup survives
//! (Cache.test.ts:818, :936, :960, :978, :995). Failures are not cached
//! unless [`Cache::cache_errors`] opts in. A [`RequestResolver`] turns many
//! distinct keys into one downstream call. [`Pool`] reuses values and drops
//! idle extras per its [`IdlePolicy`]. [`ScopedCache`] ties a cache's
//! lifetime to a scope.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash as StdHash;
use std::sync::{Arc, Condvar, Mutex};

// Cache

/// What an entry owns, run once when it stops being the entry: the pin's
/// entry scope, closed when the entry is displaced, evicted, invalidated,
/// expired, or overwritten (ScopedCache.test.ts:400, :662, :783, :901, :1074,
/// :1202, :1660, :1976, :2058, :2093).
pub type Release = Box<dyn FnOnce() + Send + 'static>;

/// Releases collected while the map lock was held, run by `run` once it is
/// gone. A finalizer may call back into the cache (ScopedCache.test.ts:1088
/// refreshes from one), so firing under the lock would deadlock; and
/// `invalidate_all` drains every row before running any of them
/// (ScopedCache.test.ts:1126).
#[must_use = "collected releases must run, or an entry's resources leak"]
struct Releases(Vec<Release>);

impl Releases {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn take(&mut self, release: Option<Release>) {
        if let Some(release) = release {
            self.0.push(release);
        }
    }

    fn extend(&mut self, releases: Vec<Release>) {
        self.0.extend(releases);
    }

    fn fire(&mut self) {
        for release in self.0.drain(..) {
            release();
        }
    }

    fn run(mut self) {
        self.fire();
    }
}

impl Drop for Releases {
    /// Backstop for a path with an early return: the collector is declared
    /// before the map lock, so it drops after it, and a finalizer never runs
    /// holding the lock.
    fn drop(&mut self) {
        self.fire();
    }
}

/// A shared lookup function. Its second half is what the entry owns: the pin
/// runs a lookup inside the entry's scope, so `Effect.acquireRelease` there
/// is this `Option<Release>`, handed over even when the lookup failed (a
/// failed lookup's resource still needs releasing).
type LookupFn<K, V, E> = Arc<dyn Fn(&K) -> (Result<V, E>, Option<Release>) + Send + Sync>;

/// The pin's `makeWith` lifetime: a function of the lookup's exit and key, so
/// one cache can hold successes for an hour and drop failures at once.
type TtlFn<K, V, E> = Arc<dyn Fn(&Result<V, E>, &K) -> TimeToLive + Send + Sync>;

/// How long a stored entry stays valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeToLive {
    /// Entries never expire (the pin's default when `timeToLive` is absent).
    Infinite,
    /// Entries expire this long after they are stored. A zero duration
    /// expires at once, so a lookup is never served from the cache.
    For(elm_effect::Duration),
}

/// One lookup's outcome, awaited by its owner and by any waiters.
///
/// The map holds the cell from the moment a lookup starts, so an in-flight
/// entry is visible (`Cache.test.ts:978` proves `has` is true mid-refresh)
/// and completions compare it by identity: the pin compares entry objects,
/// this port compares the cell behind an `Arc`.
struct Cell<V, E> {
    outcome: Mutex<Option<Result<V, E>>>,
    ready: Condvar,
    /// What this entry owns until something fires it. It lives on the cell so
    /// a completion can hand its release to the row it installed, and a
    /// completion that no longer owns the row fires its own release instead
    /// of the replacement's (ScopedCache.test.ts:797, :1088).
    release: Mutex<Option<Release>>,
}

impl<V: Clone, E: Clone> Cell<V, E> {
    fn pending() -> Self {
        Self {
            outcome: Mutex::new(None),
            ready: Condvar::new(),
            release: Mutex::new(None),
        }
    }

    fn settled(outcome: Result<V, E>) -> Self {
        Self {
            outcome: Mutex::new(Some(outcome)),
            ready: Condvar::new(),
            release: Mutex::new(None),
        }
    }

    /// Give this entry what its lookup acquired.
    fn hold(&self, release: Release) {
        *self.release.lock().unwrap() = Some(release);
    }

    /// Take what this entry owns, if it still owns anything.
    fn take_release(&self) -> Option<Release> {
        self.release.lock().unwrap().take()
    }

    /// The outcome if the lookup has completed, `None` while it runs.
    fn poll(&self) -> Option<Result<V, E>> {
        self.outcome.lock().unwrap().clone()
    }

    /// Publish the outcome and wake every waiter.
    fn settle(&self, outcome: Result<V, E>) {
        *self.outcome.lock().unwrap() = Some(outcome);
        self.ready.notify_all();
    }

    /// Block until the lookup completes, then take its outcome. Callers must
    /// not hold the map lock: the lookup they wait for needs it to finish.
    fn wait(&self) -> Result<V, E> {
        let mut outcome = self.outcome.lock().unwrap();
        while outcome.is_none() {
            outcome = self.ready.wait(outcome).unwrap();
        }
        outcome.clone().unwrap()
    }
}

/// A bounded memo table with single-flight lookups and an optional TTL.
pub struct Cache<K, V, E> {
    map: Mutex<HashMap<K, Arc<Cell<V, E>>>>,
    order: Mutex<VecDeque<K>>,
    capacity: usize,
    lookup: LookupFn<K, V, E>,
    cache_errors: bool,
    ttl: TtlFn<K, V, E>,
    clock: Arc<dyn elm_effect::ClockImpl>,
    /// Each held key's expiry deadline in clock millis. An unbounded (or
    /// in-flight) row carries no deadline: absence reads as never-expiring.
    expiry: Mutex<HashMap<K, u64>>,
}

impl<K, V, E> Cache<K, V, E>
where
    K: Clone + Eq + StdHash + Send + 'static,
    V: Clone + Send + 'static,
    E: Clone + Send + 'static,
{
    /// A cache with `capacity` entries sharing one lookup. The first caller
    /// for a key puts up a pending row and runs the lookup without the map
    /// lock, so distinct keys look up in parallel while concurrent callers
    /// for one key wait on that row and the lookup runs once. Use
    /// [`RequestResolver`] when distinct keys must batch into one call.
    pub fn new(
        capacity: usize,
        lookup: impl Fn(&K) -> Result<V, E> + Send + Sync + 'static,
    ) -> Self {
        Self::new_owned(capacity, move |key| ((lookup)(key), None))
    }

    /// [`Cache::new`] for a lookup that acquires something its entry owns.
    /// The release runs when the entry stops being the entry, or at once when
    /// the row is never stored or is no longer the lookup's own.
    pub fn new_owned(
        capacity: usize,
        lookup: impl Fn(&K) -> (Result<V, E>, Option<Release>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            order: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            lookup: Arc::new(lookup),
            cache_errors: false,
            ttl: Arc::new(|_, _| TimeToLive::Infinite),
            clock: Arc::new(elm_effect::SystemClock),
            expiry: Mutex::new(HashMap::new()),
        }
    }

    /// Give entries a lifetime, measured on `clock`. An expired entry is a
    /// miss everywhere it is read: `get` runs the lookup again, `has` and
    /// `get_option` report absence, and the stale entry is dropped.
    pub fn with_ttl(mut self, ttl: TimeToLive, clock: Arc<dyn elm_effect::ClockImpl>) -> Self {
        self.ttl = Arc::new(move |_, _| ttl);
        self.clock = clock;
        self
    }

    /// The pin's `makeWith`: the lifetime depends on the lookup's exit and the
    /// key, so successes can live for an hour while failures drop at once
    /// (Cache.test.ts:995 builds exactly that shape).
    pub fn with_ttl_fn(
        mut self,
        ttl: impl Fn(&Result<V, E>, &K) -> TimeToLive + Send + Sync + 'static,
        clock: Arc<dyn elm_effect::ClockImpl>,
    ) -> Self {
        self.ttl = Arc::new(ttl);
        self.clock = clock;
        self
    }

    /// Configured entry capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Also cache failures, returning them again instead of re-running.
    pub fn cache_errors(mut self, cache: bool) -> Self {
        self.cache_errors = cache;
        self
    }

    /// The lifetime this outcome earns under the configured TTL.
    fn lifetime(&self, outcome: &Result<V, E>, key: &K) -> TimeToLive {
        (self.ttl)(outcome, key)
    }

    /// Record `key`'s expiry after a store: a deadline for a finite lifetime,
    /// nothing for an unbounded one. A zero lifetime never reaches here: the
    /// pin skips the store outright, so a lookup leaves no row behind rather
    /// than one that expired on arrival, and a later expiry still counts
    /// until it is touched (`Cache::size`).
    fn record_expiry(&self, key: &K, ttl: TimeToLive) {
        let mut expiry = self.expiry.lock().unwrap();
        match ttl {
            TimeToLive::Infinite => {
                expiry.remove(key);
            }
            TimeToLive::For(duration) => {
                if !duration.is_zero() {
                    expiry.insert(key.clone(), self.clock.now_millis() + duration.as_millis());
                }
            }
        }
    }

    /// The value for `key`, running the lookup at most once per miss.
    /// Concurrent callers for one key share the same in-flight cell; a caller
    /// that finds one waits on it instead of starting a second lookup.
    pub fn get(&self, key: &K) -> Result<V, E> {
        let mut install = Releases::new();
        let (cell, owner) = {
            let mut map = self.map.lock().unwrap();
            if self.expired_now(key) {
                install.take(self.forget(&mut map, key));
            }
            match map.get(key).cloned() {
                Some(cell) => {
                    // Access order: a hit refreshes recency, so the least
                    // recently used entry is the eviction victim. This is what
                    // distinguishes `get` from `has`, which must leave the
                    // order alone. A pending row moves too, as the pin's does.
                    self.order.lock().unwrap().retain(|held| held != key);
                    self.order.lock().unwrap().push_back(key.clone());
                    (cell, false)
                }
                None => {
                    // The row goes up before the lookup runs, so readers see
                    // the in-flight entry (Cache.test.ts:978) and waiters can
                    // join it; capacity runs exactly as the pin's install does.
                    let cell = Arc::new(Cell::pending());
                    map.insert(key.clone(), Arc::clone(&cell));
                    self.order.lock().unwrap().push_back(key.clone());
                    install.extend(self.trim(&mut map));
                    (cell, true)
                }
            }
        };
        // An eviction releases what it dropped BEFORE the lookup runs, so a
        // lookup that acquires sees the release already happen
        // (ScopedCache.test.ts:2033 orders `acquired-a`, `acquired-b`,
        // `released-a`, `acquired-c`).
        install.run();
        if !owner {
            return cell.wait();
        }
        let mut releases = Releases::new();
        // The lookup runs outside the map lock: that is the point of the cell.
        // Holding the lock here would make the pin's interleavings (a set that
        // lands mid-lookup, an interrupt that removes the pending row)
        // inexpressible.
        let (outcome, release) = (self.lookup)(key);
        {
            let mut map = self.map.lock().unwrap();
            let store = match &outcome {
                Ok(_) => true,
                Err(_) => self.cache_errors,
            };
            let ttl = self.lifetime(&outcome, key);
            let zero = matches!(ttl, TimeToLive::For(duration) if duration.is_zero());
            if zero || !store {
                // Nothing is stored: the row leaves with the completed lookup
                // unless a concurrent write already replaced it, and what the
                // lookup acquired goes with it.
                if Self::owns(&map, key, &cell) {
                    releases.take(self.forget(&mut map, key));
                }
                releases.take(release);
            } else if Self::owns(&map, key, &cell) {
                // The row is this lookup's, so it owns what the lookup took.
                if let Some(release) = release {
                    cell.hold(release);
                }
                self.record_expiry(key, ttl);
            } else {
                releases.take(release);
            }
        }
        releases.run();
        cell.settle(outcome.clone());
        outcome
    }

    /// Force `key`'s value (`Cache.refresh`): the lookup always runs, even
    /// on a hit.
    ///
    /// A live entry is replaced IN PLACE: its recency row does not move, so
    /// a refresh is not a use and does not rescue the key from eviction
    /// (the pin's plain map set keeps position, unlike the read path's
    /// move). The TTL restarts at the new value. A missing or expired key
    /// goes to the back of the queue and pays the capacity check. Under a
    /// zero TTL the row goes instead of staying expired-on-arrival.
    pub fn refresh(&self, key: &K) -> Result<V, E> {
        let mut install = Releases::new();
        let (cell, existing) = {
            let mut map = self.map.lock().unwrap();
            let existing = if self.expired_now(key) {
                install.take(self.forget(&mut map, key));
                false
            } else {
                map.contains_key(key)
            };
            let cell = Arc::new(Cell::pending());
            if !existing {
                // A refresh that is creating the entry puts it up right away,
                // so the row is visible while the lookup runs; a refresh of a
                // live entry leaves that entry in place and replaces it on
                // completion, keeping its position (a refresh is not a use).
                map.insert(key.clone(), Arc::clone(&cell));
                self.order.lock().unwrap().push_back(key.clone());
                install.extend(self.trim(&mut map));
            }
            (cell, existing)
        };
        install.run();
        let mut releases = Releases::new();
        let (outcome, release) = (self.lookup)(key);
        {
            let mut map = self.map.lock().unwrap();
            let store = match &outcome {
                Ok(_) => true,
                Err(_) => self.cache_errors,
            };
            let ttl = self.lifetime(&outcome, key);
            let zero = matches!(ttl, TimeToLive::For(duration) if duration.is_zero());
            if zero {
                // A zero-TTL refresh removes the row that is there now when it
                // was updating an existing entry, and only its own row when it
                // was creating one: a value written during the lookup survives
                // the latter (Cache.test.ts:995). Either way its own lookup's
                // resource has no entry left to belong to.
                if existing || Self::owns(&map, key, &cell) {
                    releases.take(self.forget(&mut map, key));
                }
                releases.take(release);
            } else if store {
                if existing {
                    // Replace the current row with this lookup's cell. If the
                    // key was evicted while the lookup ran it re-enters at the
                    // back and capacity runs again (Cache.test.ts:936). The
                    // row that was replaced hands over what it owned
                    // (ScopedCache.test.ts:1074).
                    let present = map.contains_key(key);
                    if let Some(old) = map.insert(key.clone(), Arc::clone(&cell)) {
                        releases.take(old.take_release());
                    }
                    if !present {
                        self.order.lock().unwrap().push_back(key.clone());
                        releases.extend(self.trim(&mut map));
                    }
                    if let Some(release) = release {
                        cell.hold(release);
                    }
                    self.record_expiry(key, ttl);
                } else if Self::owns(&map, key, &cell) {
                    // The row went up at the start; completing it is an expiry.
                    if let Some(release) = release {
                        cell.hold(release);
                    }
                    self.record_expiry(key, ttl);
                } else {
                    releases.take(release);
                }
            } else {
                // Failures are not cached unless asked for, so a failed
                // creating refresh drops the row it put up. A failed refresh
                // of a live entry keeps that entry, and the new lookup's own
                // resource goes with the lookup.
                if !existing && Self::owns(&map, key, &cell) {
                    releases.take(self.forget(&mut map, key));
                }
                releases.take(release);
            }
        }
        releases.run();
        cell.settle(outcome.clone());
        outcome
    }

    /// Whether the stamp on `key` says it has expired. A never-stored key
    /// carries no stamp and is not expired (it is simply absent).
    fn expired_now(&self, key: &K) -> bool {
        match self.expiry.lock().unwrap().get(key).copied() {
            Some(deadline) => self.clock.now_millis() >= deadline,
            None => false,
        }
    }

    /// Forget `key` everywhere: cell, recency queue, and expiry. Hands back
    /// whatever the row owned, for the caller to run once the lock is gone.
    fn forget(&self, map: &mut HashMap<K, Arc<Cell<V, E>>>, key: &K) -> Option<Release> {
        let release = map.remove(key).and_then(|cell| cell.take_release());
        self.order.lock().unwrap().retain(|held| held != key);
        self.expiry.lock().unwrap().remove(key);
        release
    }

    /// Drop the oldest rows until the map fits the capacity, handing back
    /// what they owned.
    fn trim(&self, map: &mut HashMap<K, Arc<Cell<V, E>>>) -> Vec<Release> {
        let mut releases = Vec::new();
        while map.len() > self.capacity {
            let oldest = self.order.lock().unwrap().pop_front();
            match oldest {
                Some(oldest) => {
                    if let Some(cell) = map.remove(&oldest)
                        && let Some(release) = cell.take_release()
                    {
                        releases.push(release);
                    }
                    self.expiry.lock().unwrap().remove(&oldest);
                }
                None => break,
            }
        }
        releases
    }

    /// Whether `key`'s current row is `cell`: the pin's entry identity.
    fn owns(map: &HashMap<K, Arc<Cell<V, E>>>, key: &K, cell: &Arc<Cell<V, E>>) -> bool {
        map.get(key).map_or(false, |held| Arc::ptr_eq(held, cell))
    }

    /// Whether `key` is held and unexpired; an expired entry is dropped.
    pub fn has(&self, key: &K) -> bool {
        let mut releases = Releases::new();
        let held = {
            let mut map = self.map.lock().unwrap();
            if map.contains_key(key) && self.expired_now(key) {
                releases.take(self.forget(&mut map, key));
            }
            map.contains_key(key)
        };
        releases.run();
        held
    }

    /// Entries currently held.
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    /// Entries currently held, under the pin's name for it (`Cache.size`).
    pub fn size(&self) -> usize {
        self.len()
    }

    /// Store `value` for `key` without running the lookup (`Cache.set`).
    ///
    /// This is a fresh write, not a refresh: the key moves to the back of
    /// the recency queue and its TTL starts now, so a `set` after an
    /// expired `get` cannot resurrect the old stamp. Capacity is enforced
    /// exactly as a lookup's store does, evicting the oldest entry.
    pub fn set(&self, key: K, value: V) {
        let mut releases = Releases::new();
        let outcome = Ok(value);
        let ttl = self.lifetime(&outcome, &key);
        {
            let mut map = self.map.lock().unwrap();
            // The overwritten row hands over what it owned
            // (ScopedCache.test.ts:662).
            releases.take(self.forget(&mut map, &key));
            if matches!(ttl, TimeToLive::For(duration) if duration.is_zero()) {
                // The pin removes any existing row and stores nothing under a
                // zero TTL, so `set` cannot leave a row expired on arrival.
            } else {
                map.insert(key.clone(), Arc::new(Cell::settled(outcome)));
                self.order.lock().unwrap().push_back(key.clone());
                releases.extend(self.trim(&mut map));
                self.record_expiry(&key, ttl);
            }
        }
        releases.run();
    }

    /// `Cache.getOption` at the pin: `Some(value)` for an entry already
    /// held and unexpired, `None` otherwise. Notably a MISS never runs the
    /// lookup — this reads the map and nothing else, which is what
    /// distinguishes it from `get`.
    pub fn get_option(&self, key: &K) -> Option<V> {
        if !self.has(key) {
            return None;
        }
        match self.map.lock().unwrap().get(key) {
            // A cached failure is not a value, and a pending lookup has no
            // outcome to report yet (the pin's getOption waits; this reader
            // reports absence and `get` is the waiting door).
            Some(cell) => match cell.poll() {
                Some(Ok(value)) => Some(value),
                _ => None,
            },
            None => None,
        }
    }

    /// Drop `key` if held (`Cache.invalidate`): value, recency row, and
    /// stamp. An unknown key is a no-op rather than an error.
    ///
    /// Taking the stamp is hygiene, not observable behavior: every read
    /// consults a stamp only for a key the map still holds and forgets a
    /// stale one before running the lookup, so a kept stamp could never
    /// change an answer. It is dropped anyway, because invalidation-only
    /// traffic would otherwise grow the stamp map without bound.
    pub fn invalidate(&self, key: &K) {
        let mut releases = Releases::new();
        {
            let mut map = self.map.lock().unwrap();
            releases.take(self.forget(&mut map, key));
        }
        releases.run();
    }

    /// `Cache.invalidateWhen`: drop `key` only when the predicate accepts
    /// its held success value, reporting whether it did.
    ///
    /// A missing key or a cached failure never calls the predicate: there is
    /// no value to judge, and the pin's failure rows record an empty verdict
    /// list. A miss keeps the row; only the predicate's `true` forgets it.
    ///
    /// The pin judges the value the invalidator saw. A settled row is judged
    /// and dropped under one lock, so nothing lands between the verdict and
    /// the removal. For an in-flight row the invalidator waits for the
    /// lookup's value and drops the row only while it is still that lookup's:
    /// Cache.test.ts:818 puts a `set` in between, and the newer value must
    /// survive.
    pub fn invalidate_when(&self, key: &K, predicate: impl FnOnce(V) -> bool) -> bool {
        let mut releases = Releases::new();
        let cell = {
            let mut map = self.map.lock().unwrap();
            if self.expired_now(key) {
                releases.take(self.forget(&mut map, key));
                return false;
            }
            match map.get(key).cloned() {
                None => return false,
                Some(cell) => match cell.poll() {
                    // Settled: the verdict and the removal are one locked step,
                    // so the entry cannot change between them.
                    Some(outcome) => {
                        return match outcome {
                            Ok(value) => {
                                if predicate(value) {
                                    releases.take(self.forget(&mut map, key));
                                    true
                                } else {
                                    false
                                }
                            }
                            Err(_) => false,
                        };
                    }
                    None => cell,
                },
            }
        };
        // In flight: wait for the value the invalidator will judge, then drop
        // the row only when it is STILL the same entry (Cache.test.ts:818 puts
        // a `set` in between, and the newer value must survive).
        match cell.wait() {
            Err(_) => false,
            Ok(value) => {
                if !predicate(value) {
                    return false;
                }
                let mut map = self.map.lock().unwrap();
                if Self::owns(&map, key, &cell) {
                    releases.take(self.forget(&mut map, key));
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Drop the in-flight lookup for `key`, if one is running: this port's
    /// form of interrupting a fiber (Cache.test.ts:978). The orphaned lookup
    /// keeps running, but its completion no longer touches the map, and a
    /// value written in the meantime is never removed (Cache.test.ts:960).
    pub fn cancel_pending(&self, key: &K) -> bool {
        let mut releases = Releases::new();
        let pending = {
            let mut map = self.map.lock().unwrap();
            let pending = map.get(key).map_or(false, |cell| cell.poll().is_none());
            if pending {
                releases.take(self.forget(&mut map, key));
            }
            pending
        };
        releases.run();
        pending
    }

    /// Active keys (`Cache.keys`): every held key whose entry has not
    /// expired. An expired row is REMOVED by the walk, not merely filtered
    /// out, so `size` falls with it. The pin hands back a lazy iterable over
    /// the live map; an eager snapshot is the honest Rust form here, since a
    /// lazy iterator would have to hold the map lock across yields.
    pub fn keys(&self) -> Vec<K> {
        let mut releases = Releases::new();
        let live = {
            let mut map = self.map.lock().unwrap();
            let mut live = Vec::new();
            let mut expired = Vec::new();
            for key in map.keys() {
                if self.expired_now(key) {
                    expired.push(key.clone());
                } else {
                    live.push(key.clone());
                }
            }
            for key in expired {
                releases.take(self.forget(&mut map, &key));
            }
            live
        };
        releases.run();
        live
    }

    /// Values of every successful, unexpired entry (`Cache.values`). A failed
    /// lookup is never a value, and expired rows go as a side effect of the
    /// walk (the pin defines `values` as a map over `entries`).
    pub fn values(&self) -> Vec<V> {
        self.entries().into_iter().map(|(_, value)| value).collect()
    }

    /// Successful, unexpired `(key, value)` pairs (`Cache.entries`). A cached
    /// failure is a stored row but never an entry, and expired rows are
    /// removed by the walk.
    pub fn entries(&self) -> Vec<(K, V)> {
        let mut releases = Releases::new();
        let live = {
            let mut map = self.map.lock().unwrap();
            let mut live = Vec::new();
            let mut expired = Vec::new();
            for (key, cell) in map.iter() {
                if self.expired_now(key) {
                    expired.push(key.clone());
                } else if let Some(Ok(value)) = cell.poll() {
                    live.push((key.clone(), value.clone()));
                }
            }
            for key in expired {
                releases.take(self.forget(&mut map, &key));
            }
            live
        };
        releases.run();
        live
    }

    /// Drop every entry (`Cache.invalidateAll`).
    pub fn invalidate_all(&self) {
        self.clear()
    }

    /// Drop every entry. Every row leaves the map before any finalizer runs,
    /// so a finalizer that re-enters the cache sees it empty and an entry it
    /// installs survives (ScopedCache.test.ts:1126, :1088).
    pub fn clear(&self) {
        let mut releases = Releases::new();
        {
            let mut map = self.map.lock().unwrap();
            for cell in map.values() {
                releases.take(cell.take_release());
            }
            map.clear();
        }
        self.order.lock().unwrap().clear();
        self.expiry.lock().unwrap().clear();
        releases.run();
    }
}

// Request

/// One data request: completed exactly once, waited on by anyone.
pub struct Request<A, E> {
    cell: Mutex<Option<Result<A, E>>>,
    ready: Condvar,
}

impl<A: Clone + Send + 'static, E: Clone + Send + 'static> Request<A, E> {
    /// An uncompleted request.
    pub fn new() -> Self {
        Self {
            cell: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    /// Complete with `outcome`. Returns false if already completed; the
    /// first outcome wins.
    pub fn complete(&self, outcome: Result<A, E>) -> bool {
        let mut guard = self.cell.lock().unwrap();
        if guard.is_some() {
            return false;
        }
        *guard = Some(outcome);
        self.ready.notify_all();
        true
    }

    /// Block until completed, then return the outcome.
    pub fn wait(&self) -> Result<A, E> {
        let mut guard = self.cell.lock().unwrap();
        while guard.is_none() {
            guard = self.ready.wait(guard).unwrap();
        }
        guard.clone().expect("checked")
    }
}

impl<A: Clone + Send + 'static, E: Clone + Send + 'static> Default for Request<A, E> {
    fn default() -> Self {
        Self::new()
    }
}

// RequestResolver

/// A shared batch function.
type BatchFn<K, V, E> = Arc<dyn Fn(Vec<K>) -> Vec<Result<V, E>> + Send + Sync>;

/// Turns many distinct keys into one downstream call.
pub struct RequestResolver<K, V, E> {
    run_batch: BatchFn<K, V, E>,
}

impl<K, V, E> RequestResolver<K, V, E>
where
    K: Send + 'static,
    V: Send + 'static,
    E: Send + 'static,
{
    /// Build from a batch function: one call handles every key.
    pub fn new(run_batch: impl Fn(Vec<K>) -> Vec<Result<V, E>> + Send + Sync + 'static) -> Self {
        Self {
            run_batch: Arc::new(run_batch),
        }
    }

    /// Run one downstream call for every key, in order.
    pub fn resolve_all(&self, keys: Vec<K>) -> Vec<Result<V, E>> {
        (self.run_batch)(keys)
    }

    /// Complete each request from a single downstream call over its keys,
    /// in order. Short outcomes leave the tail requests pending (never
    /// complete!); callers must return one outcome per key.
    pub fn resolve_requests(&self, requests: Vec<(K, Request<V, E>)>) -> usize
    where
        V: Clone,
        E: Clone,
    {
        let mut keys = Vec::with_capacity(requests.len());
        let mut handles = Vec::with_capacity(requests.len());
        for (key, request) in requests {
            keys.push(key);
            handles.push(request);
        }
        let outcomes = (self.run_batch)(keys);
        let mut completed = 0;
        for (request, outcome) in handles.iter().zip(outcomes) {
            if request.complete(outcome) {
                completed += 1;
            }
        }
        completed
    }
}

// Pool

/// What happens to values beyond the idle bound when released.
#[derive(Clone, Debug)]
pub struct IdlePolicy {
    /// Idle values kept at most.
    pub max_idle: usize,
    /// Idle values older than this are dropped on acquire.
    pub max_idle_time: Option<std::time::Duration>,
}

impl Default for IdlePolicy {
    fn default() -> Self {
        Self {
            max_idle: 8,
            max_idle_time: None,
        }
    }
}

/// A bounded pool of reusable values. `acquire` blocks while exhausted;
/// dropping the guard returns the value, or releases it when idle bounds
/// say so.
pub struct Pool<T> {
    inner: Mutex<PoolInner<T>>,
    available: Condvar,
    make: Arc<dyn Fn() -> T + Send + Sync>,
    max: usize,
    idle: IdlePolicy,
}

struct PoolInner<T> {
    idle: Vec<(T, std::time::Instant)>,
    total: usize,
}

impl<T> Pool<T>
where
    T: Send + 'static,
{
    /// A pool making values with `make`, holding at most `max` total.
    pub fn new(max: usize, idle: IdlePolicy, make: impl Fn() -> T + Send + Sync + 'static) -> Self {
        Self {
            inner: Mutex::new(PoolInner {
                idle: Vec::new(),
                total: 0,
            }),
            available: Condvar::new(),
            make: Arc::new(make),
            max: max.max(1),
            idle,
        }
    }

    /// Take a value, blocking while exhausted. Idle values older than the
    /// idle timeout are released instead of reused.
    pub fn acquire(&self) -> Pooled<'_, T> {
        let mut guard = self.inner.lock().unwrap();
        loop {
            while let Some((value, at)) = guard.idle.pop() {
                if let Some(timeout) = self.idle.max_idle_time
                    && at.elapsed() >= timeout
                {
                    guard.total -= 1;
                    continue;
                }
                return Pooled {
                    value: Some(value),
                    pool: self,
                    returning: true,
                };
            }
            if guard.total < self.max {
                guard.total += 1;
                drop(guard);
                let value = (self.make)();
                return Pooled {
                    value: Some(value),
                    pool: self,
                    returning: true,
                };
            }
            guard = self.available.wait(guard).unwrap();
        }
    }

    /// Take a value the caller makes, handing the slot back when making
    /// fails.
    ///
    /// A failed attempt must not shrink the pool: the slot is counted while
    /// the attempt runs and given back on `Err`, which is what lets a retry
    /// loop keep trying without starving itself. An idle value is reused
    /// first; the pool's own `make` is not used on this path.
    pub fn acquire_with<E>(
        &self,
        create: impl FnOnce() -> Result<T, E>,
    ) -> Result<Pooled<'_, T>, E> {
        let mut guard = self.inner.lock().unwrap();
        loop {
            while let Some((value, at)) = guard.idle.pop() {
                if let Some(timeout) = self.idle.max_idle_time
                    && at.elapsed() >= timeout
                {
                    guard.total -= 1;
                    continue;
                }
                return Ok(Pooled {
                    value: Some(value),
                    pool: self,
                    returning: true,
                });
            }
            if guard.total < self.max {
                guard.total += 1;
                drop(guard);
                return match create() {
                    Ok(value) => Ok(Pooled {
                        value: Some(value),
                        pool: self,
                        returning: true,
                    }),
                    Err(error) => {
                        let mut guard = self.inner.lock().unwrap();
                        guard.total -= 1;
                        self.available.notify_one();
                        Err(error)
                    }
                };
            }
            guard = self.available.wait(guard).unwrap();
        }
    }

    /// Values currently checked out.
    pub fn in_use(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.total - guard.idle.len()
    }

    /// Values currently idle.
    pub fn idle_count(&self) -> usize {
        self.inner.lock().unwrap().idle.len()
    }

    fn release(&self, value: T) {
        let mut guard = self.inner.lock().unwrap();
        if guard.idle.len() < self.idle.max_idle {
            guard.idle.push((value, std::time::Instant::now()));
        } else {
            guard.total -= 1;
        }
        self.available.notify_one();
    }

    /// Drop a value without returning it to circulation: the total
    /// shrinks and one waiter (if any) wakes to make a replacement.
    fn discard(&self, value: T) {
        drop(value);
        let mut guard = self.inner.lock().unwrap();
        guard.total -= 1;
        self.available.notify_one();
    }

    /// Fill idle storage up to `count` values (bounded by `max` total).
    /// Mirrors pool preallocation: later acquires reuse without making.
    pub fn preallocate(&self, count: usize) {
        loop {
            let room = {
                let guard = self.inner.lock().unwrap();
                guard.idle.len() < count && guard.total < self.max
            };
            if !room {
                return;
            }
            let value = (self.make)();
            let mut guard = self.inner.lock().unwrap();
            if guard.idle.len() < count && guard.total < self.max {
                guard.total += 1;
                guard.idle.push((value, std::time::Instant::now()));
            } else {
                drop(guard);
                drop(value);
                return;
            }
            self.available.notify_one();
        }
    }

    /// Take a value out of shared circulation permanently: dropping the
    /// guard shrinks the pool instead of returning the value.
    pub fn reserve(&self) -> Pooled<'_, T> {
        let mut guard = self.acquire();
        guard.returning = false;
        guard
    }
}

/// A checked-out pool value. Dropping returns it (or releases it when the
/// idle bound says so).
pub struct Pooled<'a, T: Send + 'static> {
    value: Option<T>,
    pool: &'a Pool<T>,
    returning: bool,
}

impl<T: Send + 'static> std::ops::Deref for Pooled<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value.as_ref().expect("held")
    }
}

impl<T: Send + 'static> std::ops::DerefMut for Pooled<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("held")
    }
}

impl<T: Send + 'static> Pooled<'_, T> {
    /// Reject a checked-out value: drops it without returning it to the
    /// pool (a broken item never circulates again).
    pub fn invalidate(mut self) {
        if let Some(value) = self.value.take() {
            self.pool.discard(value);
        }
    }
}

impl<T: Send + 'static> Drop for Pooled<'_, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            if self.returning {
                self.pool.release(value);
            } else {
                self.pool.discard(value);
            }
        }
    }
}

// ScopedCache

/// A cache tied to a scope: [`close`](ScopedCache::close) drops every entry
/// and closes the scope, releasing anything the lookups acquired there.
pub struct ScopedCache<K, V, E> {
    cache: Cache<K, V, E>,
    scope: std::sync::Arc<elm_effect::Scope>,
}

impl<K, V, E> ScopedCache<K, V, E>
where
    K: Clone + Eq + StdHash + Send + 'static,
    V: Clone + Send + 'static,
    E: Clone + Send + 'static,
{
    /// Wrap a cache with its construction scope.
    pub fn new(cache: Cache<K, V, E>, scope: std::sync::Arc<elm_effect::Scope>) -> Self {
        Self { cache, scope }
    }

    /// `ScopedCache.make`: one step from capacity, lifetime, clock, scope,
    /// and lookup to a scoped cache.
    pub fn make(
        capacity: usize,
        ttl: TimeToLive,
        clock: Arc<dyn elm_effect::ClockImpl>,
        scope: std::sync::Arc<elm_effect::Scope>,
        lookup: impl Fn(&K) -> Result<V, E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            cache: Cache::new(capacity, lookup).with_ttl(ttl, clock),
            scope,
        }
    }

    /// `ScopedCache.make` for a lookup that acquires something its entry
    /// owns: the resource rides the entry, so it is released when the entry is
    /// displaced, evicted, invalidated, expired, overwritten, or when the
    /// cache closes, and a lookup that no longer owns its row releases its
    /// own rather than a replacement's (ScopedCache.test.ts:400, :662, :783,
    /// :797, :901, :1074, :1088, :1126, :1202, :1660, :1976, :2033, :2058,
    /// :2093).
    pub fn make_resource(
        capacity: usize,
        ttl: TimeToLive,
        clock: Arc<dyn elm_effect::ClockImpl>,
        scope: std::sync::Arc<elm_effect::Scope>,
        lookup: impl Fn(&K) -> (Result<V, E>, Option<Release>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            // The pin's ScopedCache keeps failed entries as rows, so their
            // resources stay alive until the entry goes and an eviction
            // releases them (ScopedCache.test.ts:1976).
            cache: Cache::new_owned(capacity, lookup)
                .with_ttl(ttl, clock)
                .cache_errors(true),
            scope,
        }
    }

    /// Configured entry capacity.
    pub fn capacity(&self) -> usize {
        self.cache.capacity()
    }

    /// Whether `key` is held and unexpired.
    pub fn has(&self, key: &K) -> bool {
        self.cache.has(key)
    }

    /// `ScopedCache.refresh`.
    pub fn refresh(&self, key: &K) -> Result<V, E> {
        self.cache.refresh(key)
    }

    /// `ScopedCache.invalidateAll`.
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all()
    }

    /// `ScopedCache.keys`.
    pub fn keys(&self) -> Vec<K> {
        self.cache.keys()
    }

    /// `ScopedCache.values`.
    pub fn values(&self) -> Vec<V> {
        self.cache.values()
    }

    /// `ScopedCache.entries`.
    pub fn entries(&self) -> Vec<(K, V)> {
        self.cache.entries()
    }

    /// `ScopedCache.getOption`: the held value, never running the lookup.
    pub fn get_option(&self, key: &K) -> Option<V> {
        self.cache.get_option(key)
    }

    /// The value for `key`, running the lookup at most once per miss.
    pub fn get(&self, key: &K) -> Result<V, E> {
        self.cache.get(key)
    }

    /// `ScopedCache.set`: store a value without running the lookup.
    pub fn set(&self, key: K, value: V) {
        self.cache.set(key, value)
    }

    /// `ScopedCache.invalidate`: drop one entry; unknown keys are a no-op.
    pub fn invalidate(&self, key: &K) {
        self.cache.invalidate(key)
    }

    /// `ScopedCache.invalidateWhen`: the plain cache's predicate rule, one
    /// entry at a time (ScopedCache.test.ts:848, :862, :876, :888).
    pub fn invalidate_when(&self, key: &K, predicate: impl FnOnce(V) -> bool) -> bool {
        self.cache.invalidate_when(key, predicate)
    }

    /// Entries currently held.
    pub fn size(&self) -> usize {
        self.cache.size()
    }

    /// Drop every entry and close the scope.
    pub fn close(&self) {
        self.cache.clear();
        self.scope.close();
    }
}

// One memoized run: `Effect.cached` / `Effect.cachedWithTTL`
// (Effect.test.ts :4292-:4412). The pin memoizes an effect value —
// concurrent callers share one run and every later caller replays the
// recorded exit. This port's descriptions are sync-only and
// interruption is observed by polling, so the memoized unit is the same
// `Body` a fiber runs and the memo is an imperative handle beside it.
//
// The caller that finds the memo empty is the owner: it runs the body
// inline and publishes the exit. Every other caller waits on the
// recorded exit and replays it — success, failure, or the owner's own
// interruption. A waiter whose fiber is interrupted gives up WITHOUT
// publishing, so interrupting a waiter never poisons the owner's run or
// later callers.
//
// With a TTL, the expiry is computed from the clock when the exit lands
// — the value's creation, never the caller's request — and an expired
// exit is a miss: the next caller claims and runs again. A zero TTL
// skips the store, so an exit the ttl function declines to keep (the
// pin's failing exits) is never replayed.

/// What one `get` should do next.
enum Next {
    Replay,
    Claim,
    Wait,
}

/// The memo's state: nothing recorded, a run in flight, or an exit with
/// its expiry (milliseconds on the configured clock; `None` is
/// unbounded).
enum MemoState<A, E> {
    Empty,
    Running,
    Done {
        exit: elm_effect::Exit<A, E>,
        expires_at: Option<u64>,
    },
}

pub struct Cached<A: Clone + Send + 'static, E: Clone + Send + 'static> {
    body: Box<dyn Fn(&elm_effect::Ctx) -> elm_effect::Exit<A, E> + Send + Sync>,
    state: Mutex<MemoState<A, E>>,
    ready: Condvar,
    ttl: Box<dyn Fn(&elm_effect::Exit<A, E>) -> elm_effect::Duration + Send + Sync>,
    clock: Arc<dyn elm_effect::ClockImpl>,
}

impl<A: Clone + Send + 'static, E: Clone + Send + 'static> Cached<A, E> {
    /// Memoize `body`. Without [`with_ttl`](Self::with_ttl) the recorded
    /// exit never expires.
    pub fn new(
        body: impl Fn(&elm_effect::Ctx) -> elm_effect::Exit<A, E> + Send + Sync + 'static,
    ) -> Self {
        Self {
            body: Box::new(body),
            state: Mutex::new(MemoState::Empty),
            ready: Condvar::new(),
            ttl: Box::new(|_| elm_effect::Duration::from_millis(u64::MAX)),
            clock: Arc::new(elm_effect::SystemClock),
        }
    }

    /// Give recorded exits a lifetime selected per exit, measured on
    /// `clock` from the moment the exit lands. A zero duration stores
    /// nothing: the exit lands already expired, so a failure the ttl
    /// function declines to keep is never replayed.
    pub fn with_ttl(
        mut self,
        ttl: impl Fn(&elm_effect::Exit<A, E>) -> elm_effect::Duration + Send + Sync + 'static,
        clock: Arc<dyn elm_effect::ClockImpl>,
    ) -> Self {
        self.ttl = Box::new(ttl);
        self.clock = clock;
        self
    }

    /// The memoized exit: claim and run when the memo is empty, wait for
    /// the owner's exit while one is in flight, or replay the recorded
    /// one. A waiter interrupted while the owner runs gives up with an
    /// interrupt exit and records nothing.
    pub fn get(&self, ctx: &elm_effect::Ctx) -> elm_effect::Exit<A, E> {
        let mut guard = self.state.lock().unwrap();
        loop {
            let next = match &*guard {
                MemoState::Done { expires_at, .. } => {
                    let live = expires_at.map_or(true, |at| self.clock.now_millis() < at);
                    if live {
                        Next::Replay
                    } else {
                        *guard = MemoState::Empty;
                        Next::Claim
                    }
                }
                MemoState::Empty => Next::Claim,
                MemoState::Running => {
                    if ctx.is_interrupted() {
                        return elm_effect::Exit::Failure(elm_effect::Cause::interrupt(None));
                    }
                    Next::Wait
                }
            };
            match next {
                Next::Replay => {
                    let MemoState::Done { exit, .. } = &*guard else {
                        unreachable!("replay only from a done state")
                    };
                    return exit.clone();
                }
                Next::Claim => {
                    *guard = MemoState::Running;
                    drop(guard);
                    let exit = (self.body)(ctx);
                    let expires_at = Some(
                        self.clock
                            .now_millis()
                            .saturating_add((self.ttl)(&exit).as_millis() as u64),
                    );
                    let mut published = self.state.lock().unwrap();
                    *published = MemoState::Done {
                        exit: exit.clone(),
                        expires_at,
                    };
                    self.ready.notify_all();
                    return exit;
                }
                Next::Wait => {
                    // Interruptible by polling in a 1ms quantum, the
                    // port's interruptible-blocking precedent.
                    let (next_guard, _) = self
                        .ready
                        .wait_timeout(guard, std::time::Duration::from_millis(1))
                        .unwrap();
                    guard = next_guard;
                }
            }
        }
    }
}
