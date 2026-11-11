//! A keyed, reference-counted resource cache.
//!
//! [`RcMap::get`] returns an [`RcRef`] lease. Dropping the lease returns one
//! reference; the value's release hook runs when the last lease for that
//! generation returns. Invalidation retires a generation instead of releasing
//! it under a live borrower, so an old lease can never release the replacement
//! installed under the same key.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use crate::{ClockImpl, Defect, Duration, SystemClock};

/// A lookup's expected failure, kept apart from a defect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupFailure<E> {
    /// The typed failure. It is cached and replayed like a value.
    Fail(E),
    /// An unexpected failure. It is cached and replayed as a defect.
    Defect(Defect),
}

/// Why [`RcMap::get`] could not answer with a cached value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RcMapError<E> {
    /// The lookup returned this typed failure.
    Lookup(E),
    /// The lookup panicked.
    Defect(Defect),
    /// The map already holds this many entries.
    ExceededCapacity(usize),
}

type Lookup<K, V, E> = Box<dyn Fn(&K) -> Result<V, LookupFailure<E>> + Send + Sync + 'static>;
type Release<V> = Box<dyn Fn(V) + Send + Sync + 'static>;
type IdleTtl<K> = Box<dyn Fn(&K) -> Option<Duration> + Send + Sync + 'static>;

struct RowState<V, E> {
    outcome: Option<Result<V, LookupFailure<E>>>,
    refs: usize,
    idle_since_ms: Option<u64>,
    last_used_ms: u64,
}

struct Row<K, V, E> {
    key: K,
    generation: u64,
    state: Mutex<RowState<V, E>>,
}

struct State<K, V, E> {
    entries: HashMap<K, Arc<Row<K, V, E>>>,
    retired: Vec<Arc<Row<K, V, E>>>,
    next_generation: u64,
    closed: bool,
}

struct Inner<K, V, E> {
    lookup: Lookup<K, V, E>,
    release: Release<V>,
    clock: Arc<dyn ClockImpl>,
    idle_ttl: Option<Duration>,
    idle_ttl_fn: Option<IdleTtl<K>>,
    capacity: Option<usize>,
    state: Mutex<State<K, V, E>>,
}

/// Options for [`RcMap::make_with_options`].
pub struct RcMapOptions<K> {
    clock: Arc<dyn ClockImpl>,
    idle_ttl: Option<Duration>,
    idle_ttl_fn: Option<IdleTtl<K>>,
    capacity: Option<usize>,
}

impl<K> Default for RcMapOptions<K> {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            idle_ttl: None,
            idle_ttl_fn: None,
            capacity: None,
        }
    }
}

impl<K> RcMapOptions<K> {
    /// Default options: wall clock, no idle TTL, no capacity limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read idle time from `clock`.
    pub fn with_clock(mut self, clock: Arc<dyn ClockImpl>) -> Self {
        self.clock = clock;
        self
    }

    /// Retain an unreferenced entry for `ttl` before releasing it. `None` and
    /// a zero duration release on the last reference.
    pub fn with_idle_ttl(mut self, ttl: Option<Duration>) -> Self {
        self.idle_ttl = ttl;
        self
    }

    /// Read the idle TTL per key. The function wins over a fixed TTL.
    pub fn with_idle_ttl_fn(
        mut self,
        ttl: impl Fn(&K) -> Option<Duration> + Send + Sync + 'static,
    ) -> Self {
        self.idle_ttl_fn = Some(Box::new(ttl));
        self
    }

    /// Refuse a new key once this many entries are retained. An existing key
    /// does not consume another slot.
    pub fn with_capacity(mut self, capacity: Option<usize>) -> Self {
        self.capacity = capacity;
        self
    }
}

/// A keyed resource map with per-generation reference counts.
///
/// Cloning the map shares the entries and configuration. A lease is bound to
/// the generation it acquired, not merely to the key.
pub struct RcMap<K, V, E> {
    inner: Arc<Inner<K, V, E>>,
}

impl<K, V, E> Clone for RcMap<K, V, E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V, E> RcMap<K, V, E>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    /// A map with the default options.
    pub fn make(
        lookup: impl Fn(&K) -> Result<V, LookupFailure<E>> + Send + Sync + 'static,
        release: impl Fn(V) + Send + Sync + 'static,
    ) -> Self {
        Self::make_with_options(lookup, release, RcMapOptions::default())
    }

    /// A map with explicit clock, idle TTL, and capacity settings.
    pub fn make_with_options(
        lookup: impl Fn(&K) -> Result<V, LookupFailure<E>> + Send + Sync + 'static,
        release: impl Fn(V) + Send + Sync + 'static,
        options: RcMapOptions<K>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                lookup: Box::new(lookup),
                release: Box::new(release),
                clock: options.clock,
                idle_ttl: options.idle_ttl,
                idle_ttl_fn: options.idle_ttl_fn,
                capacity: options.capacity,
                state: Mutex::new(State {
                    entries: HashMap::new(),
                    retired: Vec::new(),
                    next_generation: 0,
                    closed: false,
                }),
            }),
        }
    }

    /// Acquire `key`, running the lookup on a miss.
    ///
    /// Returns `None` after [`close`](Self::close). A full map answers
    /// [`RcMapError::ExceededCapacity`]; it never evicts to make room.
    pub fn get(&self, key: &K) -> Option<RcRef<K, V, E>> {
        let mut releases = Vec::new();
        let acquired = {
            let mut state = self.inner.state.lock().unwrap();
            if state.closed {
                return None;
            }
            let now = self.inner.clock.now_millis();
            self.inner.sweep(&mut state, now, &mut releases);
            if let Some(row) = state.entries.get(key).cloned() {
                Some(self.acquire(row, now))
            } else if let Some(capacity) = self.inner.capacity
                && state.entries.len() >= capacity
            {
                Some(RcRef::untracked(
                    Arc::clone(&self.inner),
                    Err(RcMapError::ExceededCapacity(capacity)),
                ))
            } else {
                let outcome = run_lookup(&self.inner.lookup, key);
                let generation = state.next_generation;
                state.next_generation += 1;
                let row = Arc::new(Row {
                    key: key.clone(),
                    generation,
                    state: Mutex::new(RowState {
                        outcome: Some(outcome.clone()),
                        refs: 1,
                        idle_since_ms: None,
                        last_used_ms: now,
                    }),
                });
                state.entries.insert(key.clone(), Arc::clone(&row));
                Some(RcRef::make_tracked(Arc::clone(&self.inner), row, outcome))
            }
        };
        release_all(releases, &self.inner.release);
        acquired
    }

    /// Acquire only a cached entry; a miss does not run the lookup.
    pub fn get_option(&self, key: &K) -> Option<RcRef<K, V, E>> {
        let mut releases = Vec::new();
        let acquired = {
            let mut state = self.inner.state.lock().unwrap();
            if state.closed {
                return None;
            }
            let now = self.inner.clock.now_millis();
            self.inner.sweep(&mut state, now, &mut releases);
            state
                .entries
                .get(key)
                .cloned()
                .map(|row| self.acquire(row, now))
        };
        release_all(releases, &self.inner.release);
        acquired
    }

    /// The cached outcome for `key`, with the temporary lease released before
    /// returning. Use [`get`](Self::get) when the value must stay retained.
    pub fn get_value(&self, key: &K) -> Option<Result<V, RcMapError<E>>> {
        self.get(key).map(RcRef::into_result)
    }

    /// The cached outcome without a lookup, with the temporary lease released.
    pub fn get_option_value(&self, key: &K) -> Option<Result<V, RcMapError<E>>> {
        self.get_option(key).map(RcRef::into_result)
    }

    fn acquire(&self, row: Arc<Row<K, V, E>>, now: u64) -> RcRef<K, V, E> {
        let outcome = {
            let mut state = row.state.lock().unwrap();
            state.refs += 1;
            state.idle_since_ms = None;
            state.last_used_ms = now;
            state.outcome.clone().expect("a live row has an outcome")
        };
        RcRef::make_tracked(Arc::clone(&self.inner), row, outcome)
    }

    /// Whether `key` is retained, including an idle entry.
    pub fn has(&self, key: &K) -> bool {
        self.inner.state.lock().unwrap().entries.contains_key(key)
    }

    /// References held for the current generation of `key`.
    pub fn ref_count(&self, key: &K) -> usize {
        self.inner
            .state
            .lock()
            .unwrap()
            .entries
            .get(key)
            .map(|row| row.state.lock().unwrap().refs)
            .unwrap_or(0)
    }

    /// Restart the idle clock for `key`; false when it is absent.
    pub fn touch(&self, key: &K) -> bool {
        let state = self.inner.state.lock().unwrap();
        let Some(row) = state.entries.get(key) else {
            return false;
        };
        let now = self.inner.clock.now_millis();
        let mut row_state = row.state.lock().unwrap();
        row_state.idle_since_ms = Some(now);
        row_state.last_used_ms = now;
        true
    }

    /// Remove the current generation from the map. A generation still held by
    /// a lease is retired until that lease returns.
    pub fn invalidate(&self, key: &K) -> bool {
        let release = {
            let mut state = self.inner.state.lock().unwrap();
            let Some(row) = state.entries.remove(key) else {
                return false;
            };
            let mut row_state = row.state.lock().unwrap();
            if row_state.refs == 0 {
                row_state.outcome.take()
            } else {
                drop(row_state);
                state.retired.push(row);
                None
            }
        };
        release_value(release, &self.inner.release);
        true
    }

    /// Release every idle entry whose TTL elapsed.
    pub fn expire_idle(&self) -> usize {
        let mut releases = Vec::new();
        let expired = {
            let mut state = self.inner.state.lock().unwrap();
            self.inner
                .sweep(&mut state, self.inner.clock.now_millis(), &mut releases)
        };
        release_all(releases, &self.inner.release);
        expired
    }

    /// Retained keys.
    pub fn keys(&self) -> Vec<K> {
        self.inner
            .state
            .lock()
            .unwrap()
            .entries
            .keys()
            .cloned()
            .collect()
    }

    /// Retained entries, including idle ones.
    pub fn size(&self) -> usize {
        self.inner.state.lock().unwrap().entries.len()
    }

    /// True when nothing is retained.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Release every retained and retired value; later gets return `None`.
    pub fn close(&self) {
        let releases = {
            let mut state = self.inner.state.lock().unwrap();
            state.closed = true;
            Inner::drain(&mut state)
        };
        release_all(releases, &self.inner.release);
    }

    /// True after [`close`](Self::close).
    pub fn is_closed(&self) -> bool {
        self.inner.state.lock().unwrap().closed
    }
}

impl<K, V, E> Inner<K, V, E>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn ttl_for(&self, key: &K) -> Option<Duration> {
        match &self.idle_ttl_fn {
            Some(ttl) => ttl(key),
            None => self.idle_ttl,
        }
    }

    fn ttl_configured(&self) -> bool {
        self.idle_ttl_fn.is_some() || self.idle_ttl.is_some_and(|ttl| !ttl.is_zero())
    }

    fn expired(&self, key: &K, idle_since_ms: u64, now: u64) -> bool {
        match self.ttl_for(key) {
            None => true,
            Some(ttl) if ttl.is_zero() => true,
            Some(ttl) => now.saturating_sub(idle_since_ms) >= ttl.as_millis(),
        }
    }

    fn sweep(
        &self,
        state: &mut State<K, V, E>,
        now: u64,
        releases: &mut Vec<Result<V, LookupFailure<E>>>,
    ) -> usize {
        if !self.ttl_configured() {
            return 0;
        }
        let victims: Vec<K> = state
            .entries
            .iter()
            .filter(|(key, row)| {
                let row_state = row.state.lock().unwrap();
                row_state.refs == 0
                    && row_state
                        .idle_since_ms
                        .is_some_and(|since| self.expired(key, since, now))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &victims {
            if let Some(row) = state.entries.remove(key) {
                let outcome = row.state.lock().unwrap().outcome.take();
                if let Some(outcome) = outcome {
                    releases.push(outcome);
                }
            }
        }
        victims.len()
    }

    fn release_row(&self, row: Arc<Row<K, V, E>>) {
        let release = {
            let mut state = self.state.lock().unwrap();
            let mut row_state = row.state.lock().unwrap();
            row_state.refs = row_state.refs.saturating_sub(1);
            if row_state.refs > 0 {
                return;
            }
            if state
                .entries
                .get(&row.key)
                .is_some_and(|held| Arc::ptr_eq(held, &row))
            {
                if self.ttl_for(&row.key).is_some_and(|ttl| !ttl.is_zero()) {
                    row_state.idle_since_ms = Some(self.clock.now_millis());
                    row_state.last_used_ms = self.clock.now_millis();
                    return;
                }
                state.entries.remove(&row.key);
            } else if let Some(index) = state
                .retired
                .iter()
                .position(|held| Arc::ptr_eq(held, &row))
            {
                state.retired.remove(index);
            } else {
                return;
            }
            row_state.outcome.take()
        };
        release_value(release, &self.release);
    }
}

impl<K, V, E> Inner<K, V, E> {
    fn drain(state: &mut State<K, V, E>) -> Vec<Result<V, LookupFailure<E>>> {
        let mut releases = Vec::new();
        let live: Vec<Arc<Row<K, V, E>>> = state.entries.drain().map(|(_, row)| row).collect();
        let retired = std::mem::take(&mut state.retired);
        for row in live.into_iter().chain(retired) {
            if let Some(outcome) = row.state.lock().unwrap().outcome.take() {
                releases.push(outcome);
            }
        }
        releases
    }
}

impl<K, V, E> Drop for Inner<K, V, E> {
    fn drop(&mut self) {
        let Ok(state) = self.state.get_mut() else {
            return;
        };
        let releases = Inner::drain(state);
        for value in releases.into_iter().flatten() {
            (self.release)(value);
        }
    }
}

/// A lease on one generation of a retained value.
///
/// `RcRef` is not `Clone`: acquiring again through [`RcMap::get`] is the way to
/// hold another reference. Dropping it returns exactly one reference.
pub struct RcRef<K, V, E>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    inner: Arc<Inner<K, V, E>>,
    row: Option<Arc<Row<K, V, E>>>,
    outcome: Option<Result<V, RcMapError<E>>>,
}

impl<K, V, E> RcRef<K, V, E>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn make_tracked(
        inner: Arc<Inner<K, V, E>>,
        row: Arc<Row<K, V, E>>,
        outcome: Result<V, LookupFailure<E>>,
    ) -> Self
    where
        E: Clone,
    {
        let outcome = match outcome {
            Ok(value) => Ok(value),
            Err(LookupFailure::Fail(error)) => Err(RcMapError::Lookup(error)),
            Err(LookupFailure::Defect(defect)) => Err(RcMapError::Defect(defect)),
        };
        Self {
            inner,
            row: Some(row),
            outcome: Some(outcome),
        }
    }

    fn untracked(inner: Arc<Inner<K, V, E>>, outcome: Result<V, RcMapError<E>>) -> Self {
        Self {
            inner,
            row: None,
            outcome: Some(outcome),
        }
    }

    /// The result held by this lease.
    pub fn result(&self) -> &Result<V, RcMapError<E>> {
        self.outcome.as_ref().expect("a lease has an outcome")
    }

    /// The value, or `None` when the lookup failed.
    pub fn value(&self) -> Option<&V> {
        self.outcome
            .as_ref()
            .and_then(|outcome| outcome.as_ref().ok())
    }

    /// The error, or `None` when the lookup succeeded.
    pub fn error(&self) -> Option<&RcMapError<E>> {
        self.outcome
            .as_ref()
            .and_then(|outcome| outcome.as_ref().err())
    }

    /// Consume the lease and return its result. Dropping it returns the
    /// reference after the value is copied out.
    pub fn into_result(mut self) -> Result<V, RcMapError<E>> {
        self.outcome.take().expect("a lease has an outcome")
    }

    /// Generation of the retained row, or zero for a capacity refusal.
    pub fn generation(&self) -> u64 {
        self.row.as_ref().map(|row| row.generation).unwrap_or(0)
    }
}

impl<K, V, E> Drop for RcRef<K, V, E>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn drop(&mut self) {
        if let Some(row) = self.row.take() {
            self.inner.release_row(row);
        }
    }
}

impl<K, V, E> std::fmt::Debug for RcRef<K, V, E>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + std::fmt::Debug + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RcRef")
            .field("generation", &self.generation())
            .field("result", &self.outcome)
            .finish()
    }
}

fn run_lookup<K, V, E>(lookup: &Lookup<K, V, E>, key: &K) -> Result<V, LookupFailure<E>> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lookup(key))) {
        Ok(outcome) => outcome,
        Err(payload) => Err(LookupFailure::Defect(crate::runtime::defect_from_panic(
            payload,
        ))),
    }
}

fn release_value<V, E>(outcome: Option<Result<V, LookupFailure<E>>>, release: &Release<V>) {
    if let Some(Ok(value)) = outcome {
        release(value);
    }
}

fn release_all<V, E>(outcomes: Vec<Result<V, LookupFailure<E>>>, release: &Release<V>) {
    for outcome in outcomes {
        release_value(Some(outcome), release);
    }
}
