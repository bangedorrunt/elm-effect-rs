//! Cache single-flight, request batching, pools, and scoped caches.
//!
//! One [`Cache`] lookup runs once no matter how many threads miss together:
//! lookups serialize on the map lock (throughput ceiling, documented below),
//! so the second waiter finds the first waiter's value. Failures are not
//! cached unless [`Cache::cache_errors`] opts in. A [`RequestResolver`]
//! turns many distinct keys into one downstream call. [`Pool`] reuses values
//! and drops idle extras per its [`IdlePolicy`]. [`ScopedCache`] ties a
//! cache's lifetime to a scope.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash as StdHash;
use std::sync::{Arc, Condvar, Mutex};

// Cache

/// A shared lookup function.
type LookupFn<K, V, E> = Arc<dyn Fn(&K) -> Result<V, E> + Send + Sync>;

/// How long a stored entry stays valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeToLive {
    /// Entries never expire (the pin's default when `timeToLive` is absent).
    Infinite,
    /// Entries expire this long after they are stored. A zero duration
    /// expires at once, so a lookup is never served from the cache.
    For(elm_effect::Duration),
}

/// A bounded memo table with single-flight lookups and an optional TTL.
pub struct Cache<K, V, E> {
    map: Mutex<HashMap<K, Result<V, E>>>,
    order: Mutex<VecDeque<K>>,
    capacity: usize,
    lookup: LookupFn<K, V, E>,
    cache_errors: bool,
    ttl: TimeToLive,
    clock: Arc<dyn elm_effect::ClockImpl>,
    /// When each held key was stored, in clock millis (and which keys carry
    /// one at all: without a TTL nothing is stamped).
    stamps: Mutex<HashMap<K, u64>>,
}

impl<K, V, E> Cache<K, V, E>
where
    K: Clone + Eq + StdHash + Send + 'static,
    V: Clone + Send + 'static,
    E: Clone + Send + 'static,
{
    /// A cache with `capacity` entries sharing one lookup. Lookups serialize
    /// on the map lock: N identical concurrent misses run the lookup once
    /// (a waiting thread finds the stored value), at the cost of lookup
    /// parallelism. Use [`RequestResolver`] when distinct keys must batch.
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
            ttl: TimeToLive::Infinite,
            clock: Arc::new(elm_effect::SystemClock),
            stamps: Mutex::new(HashMap::new()),
        }
    }

    /// Give entries a lifetime, measured on `clock`. An expired entry is a
    /// miss everywhere it is read: `get` runs the lookup again, `has` and
    /// `get_option` report absence, and the stale entry is dropped.
    pub fn with_ttl(mut self, ttl: TimeToLive, clock: Arc<dyn elm_effect::ClockImpl>) -> Self {
        self.ttl = ttl;
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

    /// The value for `key`, running the lookup at most once per miss.
    pub fn get(&self, key: &K) -> Result<V, E> {
        let mut map = self.map.lock().unwrap();
        if self.expired_now(key) {
            self.forget(&mut map, key);
        } else if let Some(hit) = map.get(key) {
            return hit.clone();
        }
        let outcome = (self.lookup)(key);
        let store = match &outcome {
            Ok(_) => true,
            Err(_) => self.cache_errors,
        };
        if store {
            if map.len() >= self.capacity {
                let mut order = self.order.lock().unwrap();
                if let Some(oldest) = order.pop_front() {
                    map.remove(&oldest);
                    self.stamps.lock().unwrap().remove(&oldest);
                }
            }
            map.insert(key.clone(), outcome.clone());
            self.order.lock().unwrap().push_back(key.clone());
            self.stamps.lock().unwrap().insert(key.clone(), self.clock.now_millis());
        }
        outcome
    }

    /// Whether the stamp on `key` says it has expired. A never-stored key
    /// carries no stamp and is not expired (it is simply absent).
    fn expired_now(&self, key: &K) -> bool {
        match self.stamps.lock().unwrap().get(key).copied() {
            Some(stamp) => match self.ttl {
                TimeToLive::Infinite => false,
                TimeToLive::For(duration) => {
                    self.clock.now_millis().saturating_sub(stamp) >= duration.as_millis()
                }
            },
            None => false,
        }
    }

    /// Forget `key` everywhere: value, recency queue, and stamp.
    fn forget(&self, map: &mut HashMap<K, Result<V, E>>, key: &K) {
        map.remove(key);
        self.order.lock().unwrap().retain(|held| held != key);
        self.stamps.lock().unwrap().remove(key);
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
        let mut map = self.map.lock().unwrap();
        self.forget(&mut map, &key);
        if map.len() >= self.capacity {
            let mut order = self.order.lock().unwrap();
            if let Some(oldest) = order.pop_front() {
                map.remove(&oldest);
                self.stamps.lock().unwrap().remove(&oldest);
            }
        }
        map.insert(key.clone(), Ok(value));
        self.order.lock().unwrap().push_back(key.clone());
        self.stamps.lock().unwrap().insert(key, self.clock.now_millis());
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
            Some(Ok(value)) => Some(value.clone()),
            // A cached failure is not a value: getOption reports absence.
            Some(Err(_)) => None,
            None => None,
        }
    }

    /// Drop every entry.
    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
        self.order.lock().unwrap().clear();
        self.stamps.lock().unwrap().clear();
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
