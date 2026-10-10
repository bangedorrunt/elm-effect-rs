//! Cache single-flight, request batching, pools, and scoped caches.
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

/// A shared lookup function.
type LookupFn<K, V, E> = Arc<dyn Fn(&K) -> Result<V, E> + Send + Sync>;

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
}

impl<V: Clone, E: Clone> Cell<V, E> {
    fn pending() -> Self {
        Self { outcome: Mutex::new(None), ready: Condvar::new() }
    }

    fn settled(outcome: Result<V, E>) -> Self {
        Self { outcome: Mutex::new(Some(outcome)), ready: Condvar::new() }
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
        let (cell, owner) = {
            let mut map = self.map.lock().unwrap();
            if self.expired_now(key) {
                self.forget(&mut map, key);
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
                    self.trim(&mut map);
                    (cell, true)
                }
            }
        };
        if !owner {
            return cell.wait();
        }
        // The lookup runs outside the map lock: that is the point of the cell.
        // Holding the lock here would make the pin's interleavings (a set that
        // lands mid-lookup, an interrupt that removes the pending row)
        // inexpressible.
        let outcome = (self.lookup)(key);
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
                // unless a concurrent write already replaced it.
                if Self::owns(&map, key, &cell) {
                    self.forget(&mut map, key);
                }
            } else if Self::owns(&map, key, &cell) {
                self.record_expiry(key, ttl);
            }
        }
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
        let (cell, existing) = {
            let mut map = self.map.lock().unwrap();
            let existing = if self.expired_now(key) {
                self.forget(&mut map, key);
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
                self.trim(&mut map);
            }
            (cell, existing)
        };
        let outcome = (self.lookup)(key);
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
                // the latter (Cache.test.ts:995).
                if existing || Self::owns(&map, key, &cell) {
                    self.forget(&mut map, key);
                }
            } else if store {
                if existing {
                    // Replace the current row with this lookup's cell. If the
                    // key was evicted while the lookup ran it re-enters at the
                    // back and capacity runs again (Cache.test.ts:936).
                    let present = map.contains_key(key);
                    map.insert(key.clone(), Arc::clone(&cell));
                    if !present {
                        self.order.lock().unwrap().push_back(key.clone());
                        self.trim(&mut map);
                    }
                    self.record_expiry(key, ttl);
                } else if Self::owns(&map, key, &cell) {
                    // The row went up at the start; completing it is an expiry.
                    self.record_expiry(key, ttl);
                }
            } else if !existing && Self::owns(&map, key, &cell) {
                // Failures are not cached unless asked for, so a failed
                // creating refresh drops the row it put up.
                self.forget(&mut map, key);
            }
        }
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

    /// Forget `key` everywhere: cell, recency queue, and expiry.
    fn forget(&self, map: &mut HashMap<K, Arc<Cell<V, E>>>, key: &K) {
        map.remove(key);
        self.order.lock().unwrap().retain(|held| held != key);
        self.expiry.lock().unwrap().remove(key);
    }

    /// Drop the oldest rows until the map fits the capacity.
    fn trim(&self, map: &mut HashMap<K, Arc<Cell<V, E>>>) {
        while map.len() > self.capacity {
            let oldest = self.order.lock().unwrap().pop_front();
            match oldest {
                Some(oldest) => {
                    map.remove(&oldest);
                    self.expiry.lock().unwrap().remove(&oldest);
                }
                None => break,
            }
        }
    }

    /// Whether `key`'s current row is `cell`: the pin's entry identity.
    fn owns(map: &HashMap<K, Arc<Cell<V, E>>>, key: &K, cell: &Arc<Cell<V, E>>) -> bool {
        map.get(key).map_or(false, |held| Arc::ptr_eq(held, cell))
    }

    /// Whether `key` is held and unexpired; an expired entry is dropped.
    pub fn has(&self, key: &K) -> bool {
        let mut map = self.map.lock().unwrap();
        if !map.contains_key(key) {
            return false;
        }
        if self.expired_now(key) {
            self.forget(&mut map, key);
            return false;
        }
        true
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
        let outcome = Ok(value);
        let ttl = self.lifetime(&outcome, &key);
        let mut map = self.map.lock().unwrap();
        self.forget(&mut map, &key);
        if matches!(ttl, TimeToLive::For(duration) if duration.is_zero()) {
            // The pin removes any existing row and stores nothing under a
            // zero TTL, so `set` cannot leave a row expired on arrival.
            return;
        }
        map.insert(key.clone(), Arc::new(Cell::settled(outcome)));
        self.order.lock().unwrap().push_back(key.clone());
        self.trim(&mut map);
        self.record_expiry(&key, ttl);
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
        let mut map = self.map.lock().unwrap();
        self.forget(&mut map, key);
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
        let cell = {
            let mut map = self.map.lock().unwrap();
            if self.expired_now(key) {
                self.forget(&mut map, key);
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
                                    self.forget(&mut map, key);
                                    true
                                } else {
                                    false
                                }
                            }
                            Err(_) => false,
                        }
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
                    self.forget(&mut map, key);
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
        let mut map = self.map.lock().unwrap();
        let pending = map.get(key).map_or(false, |cell| cell.poll().is_none());
        if pending {
            self.forget(&mut map, key);
            true
        } else {
            false
        }
    }

    /// Active keys (`Cache.keys`): every held key whose entry has not
    /// expired. An expired row is REMOVED by the walk, not merely filtered
    /// out, so `size` falls with it. The pin hands back a lazy iterable over
    /// the live map; an eager snapshot is the honest Rust form here, since a
    /// lazy iterator would have to hold the map lock across yields.
    pub fn keys(&self) -> Vec<K> {
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
            self.forget(&mut map, &key);
        }
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
            self.forget(&mut map, &key);
        }
        live
    }

    /// Drop every entry (`Cache.invalidateAll`).
    pub fn invalidate_all(&self) {
        self.clear()
    }

    /// Drop every entry.
    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
        self.order.lock().unwrap().clear();
        self.expiry.lock().unwrap().clear();
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

    /// Configured entry capacity.
    pub fn capacity(&self) -> usize {
        self.cache.capacity()
    }

    /// Whether `key` is held and unexpired.
    pub fn has(&self, key: &K) -> bool {
        self.cache.has(key)
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
