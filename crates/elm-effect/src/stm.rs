//! STM-lite: serializable transactions over one global lock.
//!
//! Every operation runs inside [`atomically`]: a global mutex serializes
//! transactions, so multi-cell blocks are atomic and isolated. Per-cell
//! mutexes nest inside (global first, always), so no lock order inverts.
//! This is coarse but correct; optimistic concurrency, blocking `retry`
//! and cross-thread fairness are residuals, not silent gaps.
//!
//! The effect face (`TxRef::get_effect` and friends under
//! [`crate::Effect::tx`]) is a staged journal rather than that lock:
//! writes land on the refs only when the boundary's body succeeds, a
//! body that calls `tx_retry` parks on its read set and runs again when
//! a commit moves it. The version counter and pending waiters that
//! needs live beside the staged views.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::service::{Reference, read_reference};
use crate::{Deferred, Effect};

static GLOBAL: Mutex<()> = Mutex::new(());

thread_local! {
    static IN_TX: Cell<bool> = const { Cell::new(false) };
}

/// Ids for refs and transactions: only uniqueness matters, and the two
/// never share a comparison.
static NEXT_TX_ID: AtomicU64 = AtomicU64::new(1);

/// Ids for `TxSubscriptionRef` subscriptions, on the same terms.
static NEXT_SUB_ID: AtomicU64 = AtomicU64::new(1);

/// Run `body` as one atomic, isolated transaction.
pub fn atomically<R>(body: impl FnOnce() -> R) -> R {
    if IN_TX.with(|flag| flag.get()) {
        return body();
    }
    let _guard: MutexGuard<'_, ()> = GLOBAL.lock().unwrap();
    IN_TX.with(|flag| flag.set(true));
    let result = body();
    IN_TX.with(|flag| flag.set(false));
    result
}

fn tx_cell<T>(cell: &Mutex<T>, f: impl FnOnce(&mut T)) {
    atomically(|| f(&mut cell.lock().unwrap()));
}

fn tx_read<T: Clone>(cell: &Mutex<T>) -> T {
    atomically(|| cell.lock().unwrap().clone())
}

/// One transaction's staged view of one ref. `version` is the version
/// the ref carried when this view was created: a ref whose version has
/// since moved makes the owning transaction's read set stale.
struct Staged<T> {
    version: u64,
    value: T,
    written: bool,
}

/// A registered wake on one ref. `live` is swapped false when the waiter
/// deregisters (a stale rerun), so a commit skips a dead entry instead of
/// waking a transaction that already moved on; the next commit of the
/// ref drains the list.
struct Waiter {
    live: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// A ref's committed value and version (bumped by every published
/// write), the staged views of the transactions touching it, and the
/// waiters parked on it.
struct RefState<T> {
    version: u64,
    value: T,
    staged: HashMap<u64, Staged<T>>,
    pending: Vec<Waiter>,
}

/// Publish or discard one transaction's staged view of a ref, and report
/// what the read set needs to know. Erased so a journal can hold refs of
/// different value types; the values themselves never leave their ref.
trait Touched: Send + Sync {
    fn commit_tx(&self, tx: u64);
    fn clear_tx(&self, tx: u64);
    fn staged_version(&self, tx: u64) -> Option<u64>;
    fn current_version(&self) -> u64;
    fn register_waiter(&self, waiter: Waiter);
}

struct RefHandle<T> {
    id: u64,
    state: Mutex<RefState<T>>,
}

impl<T: Send + 'static> Touched for RefHandle<T> {
    /// Publish the staged write (if any), bump the version, wake every
    /// live waiter, and drop the staged view.
    fn commit_tx(&self, tx: u64) {
        let mut state = self.state.lock().unwrap();
        if let Some(staged) = state.staged.remove(&tx) {
            if staged.written {
                // shortcut: publish and bump on every write. The pin skips
                // the bump when `Object.is` matches the old value, which
                // only matters for a row observing the version counter,
                // and none of these rows does.
                state.value = staged.value;
                state.version = state.version.wrapping_add(1);
            }
        }
        for waiter in state.pending.drain(..) {
            if waiter.live.swap(false, Ordering::SeqCst) {
                (waiter.wake)();
            }
        }
    }

    /// Discard the staged view without publishing.
    fn clear_tx(&self, tx: u64) {
        self.state.lock().unwrap().staged.remove(&tx);
    }

    /// The version this transaction's staged view recorded, if it has a
    /// view: one entry of the read set.
    fn staged_version(&self, tx: u64) -> Option<u64> {
        self.state
            .lock()
            .unwrap()
            .staged
            .get(&tx)
            .map(|staged| staged.version)
    }

    /// The ref's current version.
    fn current_version(&self) -> u64 {
        self.state.lock().unwrap().version
    }

    /// Register a wake, fired by this ref's next commit.
    fn register_waiter(&self, waiter: Waiter) {
        self.state.lock().unwrap().pending.push(waiter);
    }
}

/// A transactional cell. The [`atomically`] face is immediate: every op
/// is one atomic step. The effect face (`get_effect`/`set_effect`/
/// `modify_effect`) stages into the ambient [`Transaction`] and publishes
/// only when the enclosing boundary's body succeeds.
///
/// `Clone` shares the cell, never copies it: two names, one identity,
/// like the pin's `TxRef` handle.
#[derive(Clone)]
pub struct TxRef<T> {
    handle: Arc<RefHandle<T>>,
}

impl<T> TxRef<T> {
    /// A cell holding `initial`.
    pub fn new(initial: T) -> Self {
        Self {
            handle: Arc::new(RefHandle {
                id: NEXT_TX_ID.fetch_add(1, Ordering::Relaxed),
                state: Mutex::new(RefState {
                    version: 0,
                    value: initial,
                    staged: HashMap::new(),
                    pending: Vec::new(),
                }),
            }),
        }
    }

    /// The current value.
    pub fn read(&self) -> T
    where
        T: Clone,
    {
        atomically(|| self.handle.state.lock().unwrap().value.clone())
    }

    /// Replace the value.
    pub fn write(&self, value: T) {
        atomically(|| self.handle.state.lock().unwrap().value = value);
    }

    /// Run `f` on the value, returning its result.
    pub fn modify<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        atomically(|| f(&mut self.handle.state.lock().unwrap().value))
    }
}

/// The ambient transaction: `Some` while an outermost `Effect::tx` body
/// runs, visible to nested `tx` calls and every effect-face op. `None`
/// is both the default and the "no boundary is open" answer.
pub(crate) static TX_CURRENT: Reference<Option<Arc<Transaction>>> = Reference {
    id: "effect/Effect/Transaction",
    default: None,
};

/// One transaction. The journal is the set of refs it has touched; the
/// staged values live on the refs. [`crate::Effect::tx`] opens one
/// boundary per interpretation, commits when the body succeeds, and
/// clears otherwise; a boundary whose body asked for a retry, or whose
/// read set went stale, parks until a commit moves the set and runs
/// again.
pub struct Transaction {
    id: u64,
    touched: Mutex<Vec<Arc<dyn Touched>>>,
    retry: AtomicBool,
}

impl Transaction {
    pub(crate) fn new() -> Self {
        Self {
            id: NEXT_TX_ID.fetch_add(1, Ordering::Relaxed),
            touched: Mutex::new(Vec::new()),
            retry: AtomicBool::new(false),
        }
    }

    /// Record one ref as touched. A repeated touch adds a second driver,
    /// which is harmless: committing or clearing an already-removed
    /// staged entry is a no-op.
    fn touch<T: Send + 'static>(&self, handle: &Arc<RefHandle<T>>) {
        let driver: Arc<dyn Touched> = Arc::clone(handle) as Arc<dyn Touched>;
        self.touched.lock().unwrap().push(driver);
    }

    /// Mark the boundary for another attempt ([`crate::Effect::tx_retry`]).
    pub(crate) fn set_retry(&self) {
        self.retry.store(true, Ordering::SeqCst);
    }

    /// True once the body asked for a retry.
    pub(crate) fn needs_retry(&self) -> bool {
        self.retry.load(Ordering::SeqCst)
    }

    /// True when no ref in the read set moved since this transaction
    /// read it.
    fn is_consistent(&self) -> bool {
        let drivers = self.touched.lock().unwrap();
        drivers.iter().all(|driver| match driver.staged_version(self.id) {
            Some(read_version) => read_version == driver.current_version(),
            None => true,
        })
    }

    /// Park until a commit touches the read set, or return at once when
    /// the read set is already stale. Registration happens BEFORE the
    /// consistency check, so a commit that lands first leaves a stale
    /// read set and an immediate rerun instead of a lost wake: the ref
    /// mutex serializes the two, and a stale registration is skipped and
    /// cleared by the next commit.
    fn await_pending(&self) {
        let parked = Arc::new(Deferred::<(), std::convert::Infallible>::new());
        let waiter = Waiter {
            live: Arc::new(AtomicBool::new(true)),
            wake: {
                let parked = Arc::clone(&parked);
                Arc::new(move || {
                    parked.complete(());
                })
            },
        };
        for driver in self.touched.lock().unwrap().iter() {
            driver.register_waiter(Waiter {
                live: Arc::clone(&waiter.live),
                wake: Arc::clone(&waiter.wake),
            });
        }
        if !self.is_consistent() {
            waiter.live.store(false, Ordering::SeqCst);
            return;
        }
        parked.wait();
    }

    /// Publish every written staged value. Nothing read-only staged
    /// survives as a write.
    pub(crate) fn commit(&self) {
        for driver in std::mem::take(&mut *self.touched.lock().unwrap()) {
            driver.commit_tx(self.id);
        }
    }

    /// Discard every staged value: the boundary failed, so nothing it
    /// staged is published.
    pub(crate) fn clear(&self) {
        for driver in std::mem::take(&mut *self.touched.lock().unwrap()) {
            driver.clear_tx(self.id);
        }
    }
}

impl<T: Clone + Send + 'static> TxRef<T> {
    /// `TxRef.get` at the pin: the transaction's staged value when the
    /// ambient boundary has touched the ref (read-your-writes), else the
    /// committed value. Outside a boundary the read is immediate.
    pub fn get_effect<E>(&self) -> Effect<T, E, ()>
    where
        E: Send + 'static,
    {
        let handle = Arc::clone(&self.handle);
        crate::effect::widen_infallible(read_reference(&TX_CURRENT)).flat_map(move |current| {
            match current {
                Some(tx) => Effect::sync(move || {
                    tx.touch(&handle);
                    let mut state = handle.state.lock().unwrap();
                    let read_version = state.version;
                    let committed = state.value.clone();
                    state
                        .staged
                        .entry(tx.id)
                        .or_insert_with(|| Staged {
                            version: read_version,
                            value: committed,
                            written: false,
                        })
                        .value
                        .clone()
                }),
                None => Effect::sync(move || handle.state.lock().unwrap().value.clone()),
            }
        })
    }

    /// `TxRef.set` at the pin: stage `value` in the ambient boundary, to
    /// be published with it. Outside a boundary the write is immediate.
    pub fn set_effect<E>(&self, value: T) -> Effect<(), E, ()>
    where
        E: Send + 'static,
    {
        let handle = Arc::clone(&self.handle);
        crate::effect::widen_infallible(read_reference(&TX_CURRENT)).flat_map(move |current| {
            match current {
                Some(tx) => Effect::sync(move || {
                    tx.touch(&handle);
                    let mut state = handle.state.lock().unwrap();
                    let read_version = state.version;
                    let committed = state.value.clone();
                    let staged = state
                        .staged
                        .entry(tx.id)
                        .or_insert_with(|| Staged {
                            version: read_version,
                            value: committed,
                            written: false,
                        });
                    staged.value = value;
                    staged.written = true;
                }),
                None => Effect::sync(move || handle.state.lock().unwrap().value = value),
            }
        })
    }

    /// `TxRef.modify` at the pin: `f` sees the staged value and returns
    /// `(result, next)`; the successor is staged and the result yielded.
    /// One atomic step with or without a boundary. The in-place `&mut`
    /// form stays `modify`; the pin's pair form lands here because that
    /// is the shape its journal step takes.
    pub fn modify_effect<B, E, F>(&self, f: F) -> Effect<B, E, ()>
    where
        B: Send + 'static,
        E: Send + 'static,
        F: FnOnce(T) -> (B, T) + Send + 'static,
    {
        let handle = Arc::clone(&self.handle);
        crate::effect::widen_infallible(read_reference(&TX_CURRENT)).flat_map(move |current| {
            match current {
                Some(tx) => Effect::sync(move || {
                    tx.touch(&handle);
                    let mut state = handle.state.lock().unwrap();
                    let read_version = state.version;
                    let committed = state.value.clone();
                    let staged = state
                        .staged
                        .entry(tx.id)
                        .or_insert_with(|| Staged {
                            version: read_version,
                            value: committed,
                            written: false,
                        });
                    let current = staged.value.clone();
                    let (result, next) = f(current);
                    staged.value = next;
                    staged.written = true;
                    result
                }),
                None => Effect::sync(move || {
                    let mut state = handle.state.lock().unwrap();
                    let current = state.value.clone();
                    let (result, next) = f(current);
                    state.value = next;
                    result
                }),
            }
        })
    }
}

/// Read the ambient transaction out of a scope: `Some` while a boundary
/// body is interpreting.
fn ambient_transaction(scope: &crate::service::Services) -> Option<Arc<Transaction>> {
    scope
        .get_ref(TX_CURRENT.id)
        .and_then(|any| Arc::downcast::<Option<Arc<Transaction>>>(any).ok())
        .and_then(|option| (*option).clone())
}

/// Add the transaction to a scope as the ambient value for the body and
/// everything it composes.
fn scope_with_transaction(
    scope: &crate::service::Services,
    tx: &Arc<Transaction>,
) -> crate::service::Services {
    let mut maps = (**scope).clone();
    maps.insert_ref(TX_CURRENT.id, Arc::new(Some(Arc::clone(tx))));
    Arc::new(maps)
}

/// The transaction boundary's interpreter: build the body under a fresh
/// transaction, and while it asks for a retry, or its read set went
/// stale, park until a commit moves the set, clear, and build it again.
/// The pin's `tx` loop lives in its runtime the same way; the commit or
/// clear at the end is the boundary's verdict on the final attempt.
///
/// The body is a factory, not a value: this port's descriptions are
/// single-shot (running consumes them, per Law L1's "a fresh build runs
/// again"), so an attempt loop needs a fresh build per attempt.
pub(crate) struct TxBoundary<A: Send + 'static, E: Send + 'static, R: crate::Requirement> {
    pub(crate) build: Box<dyn Fn() -> Effect<A, E, R> + Send>,
    pub(crate) env: crate::service::Services,
    pub(crate) marker: std::marker::PhantomData<fn() -> R>,
}

impl<A: Send + 'static, E: Send + 'static, R: crate::Requirement> crate::effect::ErasedStep<A, E>
    for TxBoundary<A, E, R>
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, crate::Cause<E>> {
        // Nested: compose into the running boundary, no new journal.
        if ambient_transaction(scope).is_some() {
            return (self.build)().run_sync_with(scope);
        }
        let Self { build, env, .. } = *self;
        let scope = crate::service::merged_services(scope, &env);
        loop {
            let tx = Arc::new(Transaction::new());
            let inner = scope_with_transaction(&scope, &tx);
            let outcome = build().run_sync_with(&inner);
            if !tx.needs_retry() && tx.is_consistent() {
                match &outcome {
                    Ok(_) => tx.commit(),
                    Err(_) => tx.clear(),
                }
                return outcome;
            }
            // Retry asked for, or a concurrent commit moved the read set:
            // park until the set moves (a stale set returns at once),
            // discard the attempt, and build the body again.
            tx.await_pending();
            tx.clear();
        }
    }
}

/// A transactional one-shot gate: first completion wins. The infallible
/// face (`complete`/`poll`) stays as-is; the fallible face (`succeed` /
/// `fail` / `done` / `await_outcome` / `poll_outcome`) records the winner
/// as a value or an error. `E` defaults so existing `TxDeferred<T>`
/// uses keep compiling.
#[derive(Debug, Clone)]
enum Slot<T, E> {
    Value(T),
    Error(E),
}

/// A transactional one-shot gate: first completion wins.
pub struct TxDeferred<T, E = std::convert::Infallible> {
    cell: Mutex<Option<Slot<T, E>>>,
    changed: std::sync::Condvar,
}

impl<T, E> TxDeferred<T, E> {
    /// An empty gate.
    pub fn new() -> Self {
        Self {
            cell: Mutex::new(None),
            changed: std::sync::Condvar::new(),
        }
    }

    fn settle(&self, slot: Slot<T, E>) -> bool {
        let done = atomically(|| match *self.cell.lock().unwrap() {
            Some(_) => false,
            ref mut cell => {
                *cell = Some(slot);
                true
            }
        });
        if done {
            self.changed.notify_all();
        }
        done
    }

    /// Complete once; `false` when already completed.
    pub fn complete(&self, value: T) -> bool {
        self.settle(Slot::Value(value))
    }

    /// Complete with a value once; `false` when already completed.
    /// (Fallible face: mirrors `Deferred.succeed`.)
    pub fn succeed(&self, value: T) -> bool {
        self.settle(Slot::Value(value))
    }

    /// Complete with an error once; `false` when already completed.
    /// (Fallible face: mirrors `Deferred.fail`.)
    pub fn fail(&self, error: E) -> bool {
        self.settle(Slot::Error(error))
    }

    /// Complete with an outcome once; `false` when already completed.
    /// (Fallible face: mirrors `Deferred.done`.)
    pub fn done(&self, outcome: Result<T, E>) -> bool {
        match outcome {
            Ok(value) => self.settle(Slot::Value(value)),
            Err(error) => self.settle(Slot::Error(error)),
        }
    }

    /// Block until completed, then report the winner. Every waiter
    /// unblocks; the first completion decides for all of them.
    pub fn await_outcome(&self) -> Result<T, E>
    where
        T: Clone,
        E: Clone,
    {
        let mut cell = self.cell.lock().unwrap();
        while cell.is_none() {
            cell = self.changed.wait(cell).unwrap();
        }
        match cell.clone().expect("guarded by the loop") {
            Slot::Value(value) => Ok(value),
            Slot::Error(error) => Err(error),
        }
    }

    /// The value without blocking (`None` until completed, and `None`
    /// when the gate completed with an error — use `poll_outcome` for
    /// the fallible face).
    pub fn poll(&self) -> Option<T>
    where
        T: Clone,
        E: Clone,
    {
        match tx_read(&self.cell) {
            Some(Slot::Value(value)) => Some(value),
            _ => None,
        }
    }

    /// The outcome without blocking (`None` until completed).
    /// `Some(Err)` after `fail`: mirrors `Deferred.poll`.
    pub fn poll_outcome(&self) -> Option<Result<T, E>>
    where
        T: Clone,
        E: Clone,
    {
        tx_read(&self.cell).map(|slot| match slot {
            Slot::Value(value) => Ok(value),
            Slot::Error(error) => Err(error),
        })
    }
}

impl<T, E> Default for TxDeferred<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

/// What a full bounded queue does on `offer`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum QueueStrategy {
    /// Reject the value (`false`).
    BackPressure,
    /// Keep the queue, drop the value (`true`).
    Dropping,
    /// Evict the oldest, keep the value (`true`).
    Sliding,
}

/// A transactional FIFO queue, optionally bounded.
pub struct TxQueue<T> {
    inner: Mutex<VecDeque<T>>,
    capacity: Option<usize>,
    strategy: QueueStrategy,
    closed: Mutex<bool>,
    failed: Mutex<bool>,
}

impl<T> TxQueue<T> {
    /// A queue holding at most `capacity` items; `offer` fails when full.
    pub fn bounded(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            strategy: QueueStrategy::BackPressure,
            closed: Mutex::new(false),
            failed: Mutex::new(false),
        }
    }

    /// A queue that never fills.
    pub fn unbounded() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: None,
            strategy: QueueStrategy::BackPressure,
            closed: Mutex::new(false),
            failed: Mutex::new(false),
        }
    }

    /// A bounded queue that drops new values when full (`offer` stays `true`).
    pub fn dropping(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            strategy: QueueStrategy::Dropping,
            closed: Mutex::new(false),
            failed: Mutex::new(false),
        }
    }

    /// A bounded queue that evicts the oldest value when full.
    pub fn sliding(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            strategy: QueueStrategy::Sliding,
            closed: Mutex::new(false),
            failed: Mutex::new(false),
        }
    }

    /// Push back; `false` when a bounded queue is full, closed, or failed.
    pub fn offer(&self, value: T) -> bool {
        if *self.closed.lock().unwrap() || *self.failed.lock().unwrap() {
            return false;
        }
        atomically(|| {
            let mut queue = self.inner.lock().unwrap();
            match (self.capacity, self.strategy) {
                (Some(limit), QueueStrategy::BackPressure) if queue.len() >= limit => false,
                (Some(limit), QueueStrategy::Dropping) if queue.len() >= limit => true,
                (Some(limit), QueueStrategy::Sliding) if queue.len() >= limit => {
                    queue.pop_front();
                    queue.push_back(value);
                    true
                }
                _ => {
                    queue.push_back(value);
                    true
                }
            }
        })
    }

    /// Pop front without blocking (`None` when empty).
    pub fn take(&self) -> Option<T> {
        atomically(|| self.inner.lock().unwrap().pop_front())
    }

    /// The current depth.
    pub fn size(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// True when no items wait.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Pop front without blocking (`None` when empty): the `poll` shape.
    pub fn poll(&self) -> Option<T> {
        self.take()
    }

    /// Stop accepting offers; queued items still drain, then `take`/`poll`
    /// settle on `None`.
    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }

    /// Halt the queue with a failure (`failCause`); queued items still
    /// drain. True on the first halt, false when already done.
    pub fn fail(&self) -> bool {
        let mut failed = self.failed.lock().unwrap();
        if *failed || *self.closed.lock().unwrap() {
            return false;
        }
        *failed = true;
        *self.closed.lock().unwrap() = true;
        true
    }

    /// Graceful shutdown (`end`); true when this call ends the queue,
    /// false when already done.
    pub fn end(&self) -> bool {
        let mut closed = self.closed.lock().unwrap();
        if *closed {
            return false;
        }
        *closed = true;
        true
    }

    /// True while offers are still accepted.
    pub fn is_open(&self) -> bool {
        !*self.closed.lock().unwrap()
    }

    /// The write-only face (`TxEnqueue` interface).
    pub fn as_enqueue(&self) -> TxEnqueue<'_, T> {
        TxEnqueue { queue: self }
    }

    /// The read-only face (`TxDequeue` interface).
    pub fn as_dequeue(&self) -> TxDequeue<'_, T> {
        TxDequeue { queue: self }
    }

    /// Push every value (`offerAll`), returning the rejected tail.
    /// Closed or failed queues take none.
    pub fn offer_all(&self, values: impl IntoIterator<Item = T>) -> Vec<T> {
        if *self.closed.lock().unwrap() || *self.failed.lock().unwrap() {
            return values.into_iter().collect();
        }
        let mut rejected = Vec::new();
        let mut taking = true;
        atomically(|| {
            let mut queue = self.inner.lock().unwrap();
            for value in values {
                if !taking {
                    rejected.push(value);
                    continue;
                }
                match (self.capacity, self.strategy) {
                    (Some(limit), QueueStrategy::BackPressure) if queue.len() >= limit => {
                        taking = false;
                        rejected.push(value);
                    }
                    (Some(limit), QueueStrategy::Dropping) if queue.len() >= limit => {}
                    (Some(limit), QueueStrategy::Sliding) if queue.len() >= limit => {
                        queue.pop_front();
                        queue.push_back(value);
                    }
                    _ => queue.push_back(value),
                }
            }
        });
        rejected
    }

    /// Drain everything in order (`takeAll`).
    pub fn take_all(&self) -> Vec<T> {
        atomically(|| self.inner.lock().unwrap().drain(..).collect())
    }

    /// Drain up to `n` items in order (`takeN`); `take_n(0)` is empty.
    pub fn take_n(&self, n: usize) -> Vec<T> {
        atomically(|| {
            let mut queue = self.inner.lock().unwrap();
            let take = n.min(queue.len());
            queue.drain(..take).collect()
        })
    }

    /// Front item without removing it (`peek`).
    pub fn peek(&self) -> Option<T>
    where
        T: Clone,
    {
        atomically(|| self.inner.lock().unwrap().front().cloned())
    }

    /// Drain up to `max` items, but nothing when fewer than `min` wait
    /// or the range is inverted (`takeBetween`).
    pub fn take_between(&self, min: usize, max: usize) -> Vec<T> {
        atomically(|| {
            let queue = self.inner.lock().unwrap();
            if min > max || queue.len() < min {
                return Vec::new();
            }
            let take = max.min(queue.len());
            drop(queue);
            self.take_n(take)
        })
    }

    /// True when at least one item waits (`isNonEmpty`).
    pub fn is_non_empty(&self) -> bool {
        !self.is_empty()
    }

    /// True when a bounded queue holds its capacity. An unbounded queue is
    /// never full, so `offer` always succeeds on one.
    pub fn is_full(&self) -> bool {
        match self.capacity {
            Some(limit) => self.size() >= limit,
            None => false,
        }
    }

    /// Remove every waiting item and hand them back (`clear`). The queue's
    /// open/closed state is untouched, so a cleared open queue still accepts
    /// offers. A queue that is already halted and drained returns nothing,
    /// which is the same empty answer `takeAll` would give.
    pub fn clear(&self) -> Vec<T> {
        atomically(|| self.inner.lock().unwrap().drain(..).collect())
    }

    /// True when the queue is halted, by `end`, `fail`, `shutdown`, or a
    /// drain that finished it (`isShutdown`).
    pub fn is_shutdown(&self) -> bool {
        !self.is_open()
    }

    /// Halt and empty the queue (`shutdown`): queued items are dropped rather
    /// than drained, so a later `poll` finds nothing. True when this call
    /// halts it, false when it was already halted.
    pub fn shutdown(&self) -> bool {
        let mut closed = self.closed.lock().unwrap();
        let first = !*closed;
        *closed = true;
        drop(closed);
        atomically(|| self.inner.lock().unwrap().clear());
        first
    }

    /// Complete the queue with an interrupt (`interrupt`): an empty queue goes
    /// straight to done, a non-empty one halts and drains. True on the first
    /// call, false when already halted.
    pub fn interrupt(&self) -> bool {
        let mut closed = self.closed.lock().unwrap();
        if *closed {
            return false;
        }
        *closed = true;
        true
    }

    /// Complete the queue with a halt cause (`done`). True on the first call,
    /// false when already halted.
    pub fn done(&self) -> bool {
        self.interrupt()
    }
}

/// The write-only face of a [`TxQueue`] (`TxEnqueue` interface):
/// enqueue and shutdown operations only, no reads.
pub struct TxEnqueue<'a, T> {
    queue: &'a TxQueue<T>,
}

impl<T> TxEnqueue<'_, T> {
    /// Push back; `false` when full, closed, or failed.
    pub fn offer(&self, value: T) -> bool {
        self.queue.offer(value)
    }

    /// Push every value, returning the rejected tail.
    pub fn offer_all(&self, values: impl IntoIterator<Item = T>) -> Vec<T> {
        self.queue.offer_all(values)
    }

    /// Halt the queue; true on the first halt.
    pub fn fail(&self) -> bool {
        self.queue.fail()
    }

    /// Graceful shutdown; false when already done.
    pub fn end(&self) -> bool {
        self.queue.end()
    }
}

/// The read-only face of a [`TxQueue`] (`TxDequeue` interface):
/// dequeue and inspection operations only, no writes.
pub struct TxDequeue<'a, T> {
    queue: &'a TxQueue<T>,
}

impl<T> TxDequeue<'_, T> {
    /// Pop front without blocking (`None` when empty).
    pub fn take(&self) -> Option<T> {
        self.queue.take()
    }

    /// Pop front without blocking (`None` when empty).
    pub fn poll(&self) -> Option<T> {
        self.queue.poll()
    }

    /// The current depth.
    pub fn size(&self) -> usize {
        self.queue.size()
    }

    /// True when no items wait.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// True while offers are still accepted.
    pub fn is_open(&self) -> bool {
        self.queue.is_open()
    }
}

impl<T: Clone> TxDequeue<'_, T> {
    /// Front item without removing it.
    pub fn peek(&self) -> Option<T> {
        self.queue.peek()
    }
}

/// A transactional counting semaphore (non-blocking end only).
pub struct TxSemaphore {
    permits: Mutex<usize>,
    capacity: usize,
}

impl TxSemaphore {
    /// A semaphore with `permits` available; releases never exceed it.
    pub fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
            capacity: permits,
        }
    }
}

/// Always true for `TxHashSet` (the type already narrows; mirrors
/// `isTxHashSet`). Foreign values are different types and cannot
/// reach this predicate.
pub fn is_tx_hash_set<T>(_: &TxHashSet<T>) -> bool {
    true
}

/// Always true for `TxHashMap` (the type already narrows; mirrors
/// `isTxHashMap`). The pin's negative cases are foreign values
/// (`{}`, `null`, `42`), which are different types and cannot reach
/// this predicate.
pub fn is_tx_hash_map<K, V>(_: &TxHashMap<K, V>) -> bool
where
    K: Eq + Hash,
{
    true
}

/// Always true for `TxDeferred` (the type already narrows; mirrors
/// `isTxDeferred`). Foreign values are different types and cannot
/// reach this predicate.
pub fn is_tx_deferred<T, E>(_: &TxDeferred<T, E>) -> bool {
    true
}

/// Always true for `TxSemaphore` (the type already narrows; mirrors
/// `isTxSemaphore`). Foreign values are different types and cannot
/// reach this predicate.
pub fn is_tx_semaphore(_: &TxSemaphore) -> bool {
    true
}

impl TxSemaphore {
    /// Take one permit without blocking (`false` when empty).
    pub fn try_acquire(&self) -> bool {
        atomically(|| {
            let mut permits = self.permits.lock().unwrap();
            if *permits == 0 {
                false
            } else {
                *permits -= 1;
                true
            }
        })
    }

    /// Return one permit, capped at capacity.
    pub fn release(&self) {
        self.release_n(1);
    }

    /// Take `n` permits without blocking (`false` when short).
    /// Panics on zero, mirroring the `acquireN` defect.
    pub fn acquire_n(&self, n: usize) -> bool {
        assert!(n > 0, "acquire_n needs a positive count");
        atomically(|| {
            let mut permits = self.permits.lock().unwrap();
            if *permits < n {
                false
            } else {
                *permits -= n;
                true
            }
        })
    }

    /// `tryAcquireN`: same non-blocking take as [`Self::acquire_n`].
    pub fn try_acquire_n(&self, n: usize) -> bool {
        self.acquire_n(n)
    }

    /// Return `n` permits, capped at capacity.
    /// Panics on zero, mirroring the `releaseN` defect.
    pub fn release_n(&self, n: usize) {
        assert!(n > 0, "release_n needs a positive count");
        tx_cell(&self.permits, |permits| {
            *permits = (*permits + n).min(self.capacity);
        });
    }

    /// Run `f` holding one permit, released on return (`withPermit`).
    /// `None` when no permit is available.
    pub fn with_permit<R>(&self, f: impl FnOnce() -> R) -> Option<R> {
        self.with_permits(1, f)
    }

    /// Run `f` holding `n` permits, released on return (`withPermits`).
    /// `None` when short (panics on zero, like `acquireN`).
    pub fn with_permits<R>(&self, n: usize, f: impl FnOnce() -> R) -> Option<R> {
        if !self.acquire_n(n) {
            return None;
        }
        struct Guard<'a> {
            sem: &'a TxSemaphore,
            n: usize,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.sem.release_n(self.n);
            }
        }
        let _guard = Guard { sem: self, n };
        Some(f())
    }

    /// Permits currently available.
    pub fn available(&self) -> usize {
        atomically(|| *self.permits.lock().unwrap())
    }
}

/// A transactional reentrant mutex: the holding thread relocks freely.
pub struct TxReentrantLock {
    inner: Mutex<ReentrantState>,
}

struct ReentrantState {
    owner: Option<std::thread::ThreadId>,
    count: usize,
}

impl TxReentrantLock {
    /// An unlocked lock.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ReentrantState {
                owner: None,
                count: 0,
            }),
        }
    }

    /// Lock (reentrant for the holder; other threads wait cooperatively).
    pub fn lock(&self) {
        let me = std::thread::current().id();
        loop {
            let claimed = atomically(|| {
                let mut state = self.inner.lock().unwrap();
                if state.owner == Some(me) || state.owner.is_none() {
                    state.owner = Some(me);
                    state.count += 1;
                    true
                } else {
                    false
                }
            });
            if claimed {
                return;
            }
            // The global lock is free here: holders can always complete.
            std::thread::yield_now();
        }
    }

    /// Unlock one level.
    pub fn unlock(&self) {
        tx_cell(&self.inner, |state| {
            state.count -= 1;
            if state.count == 0 {
                state.owner = None;
            }
        });
    }

    /// The current hold count.
    pub fn holds(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().count)
    }
}

impl Default for TxReentrantLock {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional growable chunk (push/get/concat).
pub struct TxChunk<T> {
    inner: Mutex<Vec<T>>,
}

impl<T> TxChunk<T> {
    /// An empty chunk.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Append one element.
    pub fn push(&self, value: T) {
        tx_cell(&self.inner, |items| items.push(value));
    }

    /// Append every element of `values`.
    pub fn extend(&self, values: Vec<T>) {
        tx_cell(&self.inner, |items| items.extend(values));
    }

    /// The element at `index`, if any.
    pub fn get(&self, index: usize) -> Option<T>
    where
        T: Clone,
    {
        atomically(|| self.inner.lock().unwrap().get(index).cloned())
    }

    /// The element count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Always true for `TxChunk` (the type already narrows; mirrors
/// `isTxChunk`, like `is_number` mirrors `isNumber`). Foreign values
/// are different types and cannot reach this predicate.
pub fn is_tx_chunk<T>(_: &TxChunk<T>) -> bool {
    true
}

impl<T> FromIterator<T> for TxChunk<T> {
    /// Build from values (`fromIterable`).
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let chunk = Self::new();
        chunk.extend(iter.into_iter().collect());
        chunk
    }
}

impl<T> TxChunk<T> {
    /// Replace the whole contents (`set`).
    pub fn set_all(&self, values: Vec<T>) {
        atomically(|| {
            *self.inner.lock().unwrap() = values;
        });
    }

    /// Insert at the front (`prepend`).
    pub fn prepend(&self, value: T) {
        atomically(|| {
            self.inner.lock().unwrap().insert(0, value);
        });
    }

    /// True when at least one element waits.
    pub fn is_non_empty(&self) -> bool {
        !self.is_empty()
    }

    /// Build from any iterable (`fromIterable`).
    pub fn from_iterable(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            inner: Mutex::new(values.into_iter().collect()),
        }
    }

    /// Transform in place, returning the previous length (`modify`).
    pub fn modify<R>(&self, f: impl FnOnce(&mut Vec<T>) -> R) -> R {
        atomically(|| f(&mut self.inner.lock().unwrap()))
    }

    /// Replace the contents with `f`'s result (`update`).
    pub fn update(&self, f: impl FnOnce(&[T]) -> Vec<T>) {
        let next = atomically(|| f(&self.inner.lock().unwrap()));
        self.set_all(next);
    }
}

impl<T: Clone> TxChunk<T> {
    /// Keep the first `n` elements.
    pub fn take(&self, n: usize) {
        self.update(|values| values.iter().take(n).cloned().collect());
    }

    /// Remove the first `n` elements.
    pub fn drop(&self, n: usize) {
        self.update(|values| values.iter().skip(n).cloned().collect());
    }

    /// Keep `[start, end)`, clamped to the length.
    pub fn slice(&self, start: usize, end: usize) {
        self.update(|values| {
            values
                .iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .cloned()
                .collect()
        });
    }

    /// Transform each element in place.
    pub fn map(&self, f: impl FnMut(T) -> T) {
        let mut f = f;
        self.update(|values| values.iter().cloned().map(&mut f).collect());
    }

    /// Keep the elements that satisfy `f`.
    pub fn filter(&self, f: impl FnMut(&T) -> bool) {
        let mut f = f;
        self.update(|values| values.iter().filter(|value| f(value)).cloned().collect());
    }

    /// Append every element of `other` to the end (`appendAll`).
    pub fn append_all(&self, other: &TxChunk<T>) {
        let extra = other.to_vec();
        self.update(move |values| {
            let mut next = values.to_vec();
            next.extend(extra.iter().cloned());
            next
        });
    }

    /// Prepend every element of `other` to the front (`prependAll`).
    pub fn prepend_all(&self, other: &TxChunk<T>) {
        let extra = other.to_vec();
        self.update(move |values| {
            let mut next = extra.clone();
            next.extend(values.iter().cloned());
            next
        });
    }

    /// Append `other` to the end (`concat`).
    pub fn concat(&self, other: &TxChunk<T>) {
        self.append_all(other);
    }

    /// A snapshot of the contents in order (`get`).
    pub fn to_vec(&self) -> Vec<T> {
        atomically(|| self.inner.lock().unwrap().clone())
    }
}

impl<T> Default for TxChunk<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional hash map.
pub struct TxHashMap<K, V> {
    inner: Mutex<HashMap<K, V>>,
}

impl<K: Eq + Hash, V> FromIterator<(K, V)> for TxHashMap<K, V> {
    /// Build from pairs; later pairs win (`fromIterable`).
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let map = Self::new();
        for (key, value) in iter {
            map.insert(key, value);
        }
        map
    }
}

impl<K: Eq + Hash, V> TxHashMap<K, V> {
    /// An empty map.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// True when `key` maps to anything (`has`).
    pub fn contains(&self, key: &K) -> bool {
        atomically(|| self.inner.lock().unwrap().contains_key(key))
    }

    /// Insert, returning the displaced value if any.
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        atomically(|| self.inner.lock().unwrap().insert(key, value))
    }

    /// The value for `key`, if any.
    pub fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        atomically(|| self.inner.lock().unwrap().get(key).cloned())
    }

    /// Remove `key`, returning its value if any.
    pub fn remove(&self, key: &K) -> Option<V> {
        atomically(|| self.inner.lock().unwrap().remove(key))
    }

    /// The entry count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no entries are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop every entry.
    pub fn clear(&self) {
        atomically(|| self.inner.lock().unwrap().clear())
    }

    /// Replace the value at `key` with what `update` returns, handing back the
    /// value that was displaced. A key that is not there is left alone: the
    /// function is never called and nothing is inserted (`modify`).
    pub fn modify(&self, key: &K, update: impl FnOnce(&V) -> V) -> Option<V>
    where
        K: Clone,
        V: Clone,
    {
        atomically(|| {
            let mut inner = self.inner.lock().unwrap();
            let current = inner.get(key)?.clone();
            let next = update(&current);
            inner.insert(key.clone(), next)
        })
    }

    /// Insert or remove `key` according to what `update` answers, which is
    /// handed the value already there if any: `Some` stores it, `None` removes
    /// it (`modifyAt`).
    pub fn modify_at(&self, key: K, update: impl FnOnce(Option<&V>) -> Option<V>)
    where
        K: Clone,
        V: Clone,
    {
        atomically(|| {
            let mut inner = self.inner.lock().unwrap();
            let current = inner.get(&key).cloned();
            match update(current.as_ref()) {
                Some(value) => {
                    inner.insert(key, value);
                }
                None => {
                    inner.remove(&key);
                }
            }
        })
    }

    /// Merge another map's entries in, its value winning for a shared key
    /// (`union`).
    pub fn union(&self, other: &HashMap<K, V>)
    where
        K: Clone,
        V: Clone,
    {
        atomically(|| {
            let mut inner = self.inner.lock().unwrap();
            for (key, value) in other {
                inner.insert(key.clone(), value.clone());
            }
        })
    }

    /// Drop every named key (`removeMany`).
    pub fn remove_many(&self, keys: impl IntoIterator<Item = K>)
    where
        K: Eq + Hash,
    {
        atomically(|| {
            let mut inner = self.inner.lock().unwrap();
            for key in keys {
                inner.remove(&key);
            }
        })
    }

    /// Insert every pair, a later pair winning for a shared key (`setMany`).
    pub fn set_many(&self, entries: impl IntoIterator<Item = (K, V)>) {
        atomically(|| {
            let mut inner = self.inner.lock().unwrap();
            for (key, value) in entries {
                inner.insert(key, value);
            }
        })
    }

    /// The entries as they stand, copied (`snapshot`). A later write lands on
    /// the map and not on the copy.
    pub fn snapshot(&self) -> HashMap<K, V>
    where
        K: Clone,
        V: Clone,
    {
        atomically(|| self.inner.lock().unwrap().clone())
    }

    /// Every key (`keys`). Order is the map's own, so a caller that needs a
    /// stable order sorts.
    pub fn keys(&self) -> Vec<K>
    where
        K: Clone,
    {
        atomically(|| self.inner.lock().unwrap().keys().cloned().collect())
    }

    /// Every value (`values`).
    pub fn values(&self) -> Vec<V>
    where
        V: Clone,
    {
        atomically(|| self.inner.lock().unwrap().values().cloned().collect())
    }

    /// Every pair (`entries`).
    pub fn entries(&self) -> Vec<(K, V)>
    where
        K: Clone,
        V: Clone,
    {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
    }

    /// The same pairs as `entries` (`toEntries`).
    pub fn to_entries(&self) -> Vec<(K, V)>
    where
        K: Clone,
        V: Clone,
    {
        self.entries()
    }

    /// The same values as `values` (`toValues`).
    pub fn to_values(&self) -> Vec<V>
    where
        V: Clone,
    {
        self.values()
    }

    /// Replace every value with what `transform` returns, keeping the keys
    /// (`map`).
    pub fn map<W>(&self, mut transform: impl FnMut(&V, &K) -> W) -> TxHashMap<K, W>
    where
        K: Clone,
    {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .map(|(key, value)| (key.clone(), transform(value, key)))
                .collect()
        })
    }

    /// Keep the entries whose value satisfies `predicate` (`filter`).
    pub fn filter(&self, predicate: impl FnMut(&V, &K) -> bool) -> TxHashMap<K, V>
    where
        K: Clone,
        V: Clone,
    {
        let mut predicate = predicate;
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, value)| predicate(value, key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
    }

    /// Fold over every value, in the map's own order (`reduce`).
    pub fn reduce<A>(&self, initial: A, mut fold: impl FnMut(A, &V, &K) -> A) -> A
    where
        K: Clone,
    {
        atomically(|| {
            let inner = self.inner.lock().unwrap();
            let mut acc = initial;
            for (key, value) in inner.iter() {
                acc = fold(acc, value, key);
            }
            acc
        })
    }

    /// Keep and transform in one step: `None` drops the entry (`filterMap`).
    pub fn filter_map<W>(&self, mut transform: impl FnMut(&V, &K) -> Option<W>) -> TxHashMap<K, W>
    where
        K: Clone,
    {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(key, value)| {
                    transform(value, key).map(|next| (key.clone(), next))
                })
                .collect()
        })
    }

    /// Whether any entry's value satisfies `predicate` (`hasBy`).
    pub fn has_by(&self, predicate: impl Fn(&V, &K) -> bool) -> bool {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .any(|(key, value)| predicate(value, key))
        })
    }

    /// Whether every entry's value satisfies `predicate` (`every`).
    pub fn every(&self, predicate: impl Fn(&V, &K) -> bool) -> bool {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .all(|(key, value)| predicate(value, key))
        })
    }

    /// Whether any entry's value satisfies `predicate` (`some`).
    pub fn some(&self, predicate: impl Fn(&V, &K) -> bool) -> bool {
        self.has_by(predicate)
    }

    /// The first entry whose value satisfies `predicate`, in the map's own
    /// order (`findFirst`).
    pub fn find_first(&self, predicate: impl Fn(&V, &K) -> bool) -> Option<(K, V)>
    where
        K: Clone,
        V: Clone,
    {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .find(|(key, value)| predicate(value, key))
                .map(|(key, value)| (key.clone(), value.clone()))
        })
    }

    /// Run `visit` once per entry (`forEach`).
    pub fn for_each(&self, mut visit: impl FnMut(&V, &K)) {
        atomically(|| {
            for (key, value) in self.inner.lock().unwrap().iter() {
                visit(value, key);
            }
        })
    }

    /// Replace every value with the map `transform` builds for it and merge
    /// the results (`flatMap`).
    pub fn flat_map<K2, W>(
        &self,
        mut transform: impl FnMut(&V, &K) -> HashMap<K2, W>,
    ) -> TxHashMap<K2, W>
    where
        K2: Eq + Hash,
        W: Clone,
    {
        let mut out = TxHashMap::new();
        atomically(|| {
            let mut visit = |key: &K, value: &V| {
                for (other_key, other_value) in transform(value, key) {
                    out.insert(other_key, other_value);
                }
            };
            for (key, value) in self.inner.lock().unwrap().iter() {
                visit(key, value);
            }
        });
        out
    }

    /// Keep the `Some` values and drop the rest (`compact`). The value type
    /// has to be an `Option` for that to say anything, so it is bounded here
    /// rather than at every call.
    pub fn compact<W: Clone>(&self) -> TxHashMap<K, W>
    where
        K: Clone,
        V: Clone + Into<Option<W>>,
    {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(key, value)| {
                    value.clone().into().map(|inner| (key.clone(), inner))
                })
                .collect()
        })
    }
}

impl<K: Eq + Hash, V> Default for TxHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional hash set.
pub struct TxHashSet<T> {
    inner: Mutex<HashSet<T>>,
}

impl<T: Eq + Hash> FromIterator<T> for TxHashSet<T> {
    /// Build from values; duplicates collapse (`make`/`fromIterable`).
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let set = Self::new();
        for value in iter {
            set.insert(value);
        }
        set
    }
}

impl<T: Eq + Hash> TxHashSet<T> {
    /// An empty set.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashSet::new()),
        }
    }

    /// Insert; `false` when already present.
    pub fn insert(&self, value: T) -> bool {
        atomically(|| self.inner.lock().unwrap().insert(value))
    }

    /// Remove; `true` when present.
    pub fn remove(&self, value: &T) -> bool {
        atomically(|| self.inner.lock().unwrap().remove(value))
    }

    /// Drop every element.
    pub fn clear(&self) {
        atomically(|| self.inner.lock().unwrap().clear())
    }

    /// Whether `value` is present.
    pub fn contains(&self, value: &T) -> bool {
        atomically(|| self.inner.lock().unwrap().contains(value))
    }

    /// The element count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

}

/// The set algebra. Split from the block above so these methods can require
/// `Clone` without widening the bound on `new`, `insert` and the rest, which
/// hold a non-`Clone` `T` fine.
impl<T: Eq + Hash + Clone> TxHashSet<T> {
    /// An immutable snapshot of the current contents (`toHashSet`).
    pub fn to_hash_set(&self) -> HashSet<T> {
        atomically(|| HashSet::from_iter(self.inner.lock().unwrap().iter().cloned()))
    }

    /// Every element in `self` or `other` (`union`). Neither side is
    /// consumed; both are read, which also keeps `x.union(&x)` from
    /// deadlocking on one mutex held twice.
    pub fn union(&self, other: &Self) -> Self
    where
        T: Clone,
    {
        atomically(|| {
            let mut merged: HashSet<T> = self.to_snapshot();
            let theirs = other.inner.lock().unwrap();
            for value in theirs.iter() {
                merged.insert(value.clone());
            }
            Self { inner: Mutex::new(merged) }
        })
    }

    /// Only the elements in both (`intersection`).
    pub fn intersection(&self, other: &Self) -> Self
    where
        T: Clone,
    {
        atomically(|| {
            let theirs: HashSet<T> = {
                let guard = other.inner.lock().unwrap();
                guard.iter().cloned().collect()
            };
            let mine = self.inner.lock().unwrap();
            let kept: HashSet<T> = mine.iter().filter(|v| theirs.contains(*v)).cloned().collect();
            Self { inner: Mutex::new(kept) }
        })
    }

    /// Only the elements not in `other` (`difference`).
    pub fn difference(&self, other: &Self) -> Self
    where
        T: Clone,
    {
        atomically(|| {
            let theirs: HashSet<T> = {
                let guard = other.inner.lock().unwrap();
                guard.iter().cloned().collect()
            };
            let mine = self.inner.lock().unwrap();
            let kept: HashSet<T> = mine.iter().filter(|v| !theirs.contains(*v)).cloned().collect();
            Self { inner: Mutex::new(kept) }
        })
    }

    /// Whether every element of `self` is in `other` (`isSubset`). Reads each
    /// side separately, so `x.is_subset(&x)` holds one lock at a time rather
    /// than deadlocking on the same mutex.
    pub fn is_subset(&self, other: &Self) -> bool {
        atomically(|| {
            let theirs: HashSet<T> = {
                let guard = other.inner.lock().unwrap();
                guard.iter().cloned().collect()
            };
            let mine = self.inner.lock().unwrap();
            mine.iter().all(|v| theirs.contains(v))
        })
    }

    /// Whether any element satisfies `f` (`some`).
    pub fn some(&self, f: impl Fn(&T) -> bool) -> bool {
        atomically(|| self.inner.lock().unwrap().iter().any(|v| f(v)))
    }

    /// Whether every element satisfies `f` (`every`).
    pub fn every(&self, f: impl Fn(&T) -> bool) -> bool {
        atomically(|| self.inner.lock().unwrap().iter().all(|v| f(v)))
    }

    /// `f` applied to each element, collapsing duplicates (`map`).
    pub fn map<U>(&self, f: impl Fn(&T) -> U) -> TxHashSet<U>
    where
        U: Eq + Hash,
        T: 'static,
    {
        atomically(|| self.inner.lock().unwrap().iter().map(f).collect())
    }

    /// Only the elements `f` keeps (`filter`).
    pub fn filter(&self, f: impl Fn(&T) -> bool) -> Self {
        atomically(|| {
            self.inner
                .lock()
                .unwrap()
                .iter()
                .filter(|v| f(v))
                .cloned()
                .collect()
        })
    }

    /// A private clone of the current contents, used by the set algebra so no
    /// method holds this set's lock while acquiring another's.
    fn to_snapshot(&self) -> HashSet<T>
    where
        T: Clone,
    {
        atomically(|| {
            let guard = self.inner.lock().unwrap();
            guard.iter().cloned().collect()
        })
    }

    /// Fold the elements into one value (`reduce`).
    pub fn reduce<R>(&self, init: R, f: impl Fn(R, &T) -> R) -> R {
        atomically(|| self.inner.lock().unwrap().iter().fold(init, |acc, v| f(acc, v)))
    }
}

impl<T: Eq + Hash> Default for TxHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge `values` (already ascending under `ord`) into `arr` (ascending).
///
/// The pin's `mergeSorted`: each value's insert point is found by galloping
/// forward from the previous one and then bisecting the bracketed range, so a
/// batch that interleaves with the queue costs a few comparisons per value
/// instead of a full search each. Existing elements stay ahead of equal
/// incoming ones, and incoming ties keep their input order.
///
/// `arr` is drained through `Option` slots rather than cloned, so the merge
/// needs no `Clone` bound on `T`.
fn merge_sorted<T>(arr: Vec<T>, values: Vec<T>, ord: &crate::foundation::Order<T>) -> Vec<T> {
    let mut slots: Vec<Option<T>> = arr.into_iter().map(Some).collect();
    let len = slots.len();
    let mut out: Vec<T> = Vec::with_capacity(len + values.len());
    let mut i = 0usize;
    for value in values {
        let mut lo = i;
        let mut hi = i;
        let mut step = 1usize;
        while hi < len
            && ord.compare(slots[hi].as_ref().expect("live slot"), &value)
                != std::cmp::Ordering::Greater
        {
            lo = hi + 1;
            hi = hi.saturating_add(step);
            step = step.saturating_mul(2);
        }
        if hi > len {
            hi = len;
        }
        while lo < hi {
            let mid = (lo + hi) / 2;
            if ord.compare(slots[mid].as_ref().expect("live slot"), &value)
                != std::cmp::Ordering::Greater
            {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        while i < lo {
            out.push(slots[i].take().expect("live slot"));
            i += 1;
        }
        out.push(value);
    }
    while i < len {
        out.push(slots[i].take().expect("live slot"));
        i += 1;
    }
    out
}

/// A transactional priority queue: elements held ascending under the
/// [`Order`](crate::Order) supplied at construction, so `take` yields the
/// smallest and a custom order can make it max-first.
///
/// `take` blocks while the queue is empty. The pin's `take` is a transaction
/// that calls `Effect.txRetry`, so its observable behavior is "wait until an
/// offer commits"; a condvar is this port's stand-in, and it is the reason
/// this type does **not** route through the global [`atomically`] lock: a
/// blocking wait must not hold the STM lock other transactions need.
pub struct TxPriorityQueue<T> {
    inner: Mutex<Vec<T>>,
    ord: crate::foundation::Order<T>,
    ready: Condvar,
}

impl<T: Send + 'static> TxPriorityQueue<T> {
    /// An empty queue under `ord`.
    pub fn empty(ord: crate::foundation::Order<T>) -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            ord,
            ready: Condvar::new(),
        }
    }

    /// A queue from an iterable, sorted under `ord`.
    pub fn from_iterable(ord: crate::foundation::Order<T>, items: Vec<T>) -> Self {
        let mut sorted = items;
        sorted.sort_by(|a, b| ord.compare(a, b));
        Self {
            inner: Mutex::new(sorted),
            ord,
            ready: Condvar::new(),
        }
    }

    /// The curried form: `from_iterable(ord)` then the iterable. The pin's
    /// `fromIterable(Order.Number)([3, 1, 2])`.
    pub fn from_iterable_curried(
        ord: crate::foundation::Order<T>,
    ) -> impl FnOnce(Vec<T>) -> Self {
        move |items| Self::from_iterable(ord, items)
    }

    /// The pin's `make(Order.Number)(5, 3, 1, 4, 2)`: same as
    /// [`from_iterable`](Self::from_iterable) with the values in hand.
    pub fn make(ord: crate::foundation::Order<T>, items: Vec<T>) -> Self {
        Self::from_iterable(ord, items)
    }

    /// The ordering this queue keeps.
    pub fn ord(&self) -> &crate::foundation::Order<T> {
        &self.ord
    }

    /// The element count.
    pub fn size(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Whether no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Whether at least one element waits.
    pub fn is_non_empty(&self) -> bool {
        !self.is_empty()
    }

    /// Insert one element in sorted position.
    pub fn offer(&self, value: T) {
        let mut guard = self.inner.lock().unwrap();
        let arr = std::mem::take(&mut *guard);
        *guard = merge_sorted(arr, vec![value], &self.ord);
        drop(guard);
        self.ready.notify_all();
    }

    /// Insert every element of `values`, merging the batch as a whole.
    pub fn offer_all(&self, values: Vec<T>) {
        let mut sorted = values;
        sorted.sort_by(|a, b| self.ord.compare(a, b));
        let mut guard = self.inner.lock().unwrap();
        let arr = std::mem::take(&mut *guard);
        *guard = merge_sorted(arr, sorted, &self.ord);
        drop(guard);
        self.ready.notify_all();
    }

    /// Take the smallest element, waiting while the queue is empty (the pin's
    /// `take`, which retries the transaction).
    pub fn take(&self) -> T {
        let mut guard = self.inner.lock().unwrap();
        while guard.is_empty() {
            guard = self.ready.wait(guard).unwrap();
        }
        guard.remove(0)
    }

    /// Take the smallest element without waiting (`takeOption`).
    pub fn take_option(&self) -> Option<T> {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_empty() {
            return None;
        }
        Some(guard.remove(0))
    }

    /// Take every element in priority order (`takeAll`).
    pub fn take_all(&self) -> Vec<T> {
        std::mem::take(&mut *self.inner.lock().unwrap())
    }

    /// Take up to `n` elements in priority order (`takeUpTo`).
    pub fn take_up_to(&self, n: usize) -> Vec<T> {
        let mut guard = self.inner.lock().unwrap();
        let take = n.min(guard.len());
        guard.drain(..take).collect()
    }

    /// Drop every element the predicate matches (`removeIf`).
    pub fn remove_if(&self, predicate: impl Fn(&T) -> bool) {
        self.inner.lock().unwrap().retain(|value| !predicate(value));
    }

    /// Keep only the elements the predicate matches (`retainIf`).
    pub fn retain_if(&self, predicate: impl Fn(&T) -> bool) {
        self.inner.lock().unwrap().retain(|value| predicate(value));
    }
}

/// The reading operations need a value out of the queue without consuming it.
impl<T: Send + 'static + Clone> TxPriorityQueue<T> {
    /// Observe the smallest element without removing it, waiting while the
    /// queue is empty (the pin's `peek`, which retries the transaction).
    pub fn peek(&self) -> T {
        let mut guard = self.inner.lock().unwrap();
        while guard.is_empty() {
            guard = self.ready.wait(guard).unwrap();
        }
        guard[0].clone()
    }

    /// Observe the smallest element without removing it, `None` when empty.
    pub fn peek_option(&self) -> Option<T> {
        self.inner.lock().unwrap().first().cloned()
    }

    /// Every element in priority order, without removing any.
    pub fn to_array(&self) -> Vec<T> {
        self.inner.lock().unwrap().clone()
    }
}

/// Always true for `TxPriorityQueue` (the type already narrows; mirrors
/// `isTxPriorityQueue`, like `is_number` mirrors `isNumber`). Foreign values
/// are different types and cannot reach this predicate.
pub fn is_tx_priority_queue<T>(_: &TxPriorityQueue<T>) -> bool {
    true
}

/// A transactional broadcast bus: every published value is retained for
/// draining subscribers. Bounded buses drop oldest (sliding) or newest
/// (dropping) past capacity; unbounded buses retain everything.
pub struct TxPubSub<T> {
    inner: Mutex<VecDeque<T>>,
    capacity: Option<usize>,
    dropping: bool,
}

impl<T> TxPubSub<T> {
    /// A bus retaining at most `capacity` values, dropping oldest past it.
    pub fn new(capacity: usize) -> Self {
        Self::sliding(capacity)
    }

    /// A bus retaining at most `capacity` values.
    pub fn bounded(capacity: usize) -> Self {
        Self::sliding(capacity)
    }

    /// A bus retaining everything.
    pub fn unbounded() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: None,
            dropping: false,
        }
    }

    /// A bus dropping newest values past `capacity`.
    pub fn dropping(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            dropping: true,
        }
    }

    /// A bus dropping oldest values past `capacity`.
    pub fn sliding(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            dropping: false,
        }
    }

    /// Capacity (`None` when unbounded).
    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// Publish one value, applying the overflow strategy past capacity.
    pub fn publish(&self, value: T) {
        tx_cell(&self.inner, |buffer| {
            if let Some(cap) = self.capacity {
                if buffer.len() >= cap {
                    if self.dropping {
                        return;
                    }
                    buffer.pop_front();
                }
            }
            buffer.push_back(value);
        });
    }

    /// Take every retained value in order.
    pub fn drain(&self) -> Vec<T> {
        atomically(|| self.inner.lock().unwrap().drain(..).collect())
    }
}

/// One live subscription's queue, shared between the ref (which offers into
/// it) and the handle (which takes from it and deregisters it).
type SubscriberQueue<T> = Arc<TxQueue<T>>;

/// The ref's live subscriptions, keyed by the id each handle deregisters
/// with. Shared with every handle so a release can remove its own entry
/// without a back-reference to the ref it came from.
type SubscriberRegistry<T> = Arc<Mutex<Vec<(u64, SubscriberQueue<T>)>>>;

/// One live subscription to a [`TxSubscriptionRef`]: its own queue, seeded
/// with the value current when it subscribed, then every later assigned
/// value in order. Dropping the handle is the pin's scope close (the
/// `release` of its `acquireRelease`); the ref forgets the queue and stops
/// offering to it.
pub struct TxChangeSubscription<T> {
    id: u64,
    queue: SubscriberQueue<T>,
    registry: SubscriberRegistry<T>,
}

impl<T> TxChangeSubscription<T> {
    /// The next value for this subscription, without blocking.
    pub fn take(&self) -> Option<T> {
        self.queue.take()
    }

    /// Detach now (`TxPubSub.releaseSubscriber` at the pin). Idempotent, and
    /// what `Drop` does, so an explicit call is only for releasing early.
    pub fn release(&self) {
        self.detach();
    }

    fn detach(&self) {
        self.registry.lock().unwrap().retain(|(id, _)| *id != self.id);
    }
}

impl<T> Drop for TxChangeSubscription<T> {
    /// Scope close: the pin's subscription is scoped, so leaving the scope
    /// releases the subscriber queue. Without this a dropped handle would
    /// stay in the ref's list forever and grow with every subscriber.
    fn drop(&mut self) {
        self.detach();
    }
}

/// A transactional cell publishing every assigned value to its live
/// subscriptions (`TxSubscriptionRef`): [`changes`](Self::changes) opens a
/// subscription that first yields the current value, then each later
/// assigned value in order. A subscriber only ever sees values assigned
/// after it subscribed, plus the current value it was seeded with.
pub struct TxSubscriptionRef<T> {
    current: Mutex<T>,
    /// Live subscriptions by id, each with its own queue. A released
    /// subscriber is removed, so the write path never touches a dead queue.
    subscribers: SubscriberRegistry<T>,
}

impl<T: Clone> TxSubscriptionRef<T> {
    /// A cell holding `initial` with no subscribers.
    pub fn new(initial: T) -> Self {
        Self {
            current: Mutex::new(initial),
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The current value.
    pub fn get(&self) -> T {
        tx_read(&self.current)
    }

    /// Assign and publish to every live subscriber.
    pub fn set(&self, value: T) {
        atomically(|| {
            *self.current.lock().unwrap() = value.clone();
            self.publish(value);
        });
    }

    /// Offer `value` to every live subscription.
    fn publish(&self, value: T) {
        for (_, queue) in self.subscribers.lock().unwrap().iter() {
            queue.offer(value.clone());
        }
    }

    /// `TxSubscriptionRef.changes`: a scoped subscription whose first
    /// [`take`](TxChangeSubscription::take) is the current value, then every
    /// later assigned value in order.
    ///
    /// The pin acquires a pubsub subscriber and offers the current value into
    /// it, because a subscriber sees only values published after it
    /// subscribed; this is that, with one owner for the queue.
    pub fn changes(&self) -> TxChangeSubscription<T> {
        let id = NEXT_SUB_ID.fetch_add(1, Ordering::SeqCst);
        let queue = Arc::new(TxQueue::unbounded());
        atomically(|| {
            queue.offer(self.current.lock().unwrap().clone());
            self.subscribers.lock().unwrap().push((id, Arc::clone(&queue)));
        });
        TxChangeSubscription {
            id,
            queue,
            registry: Arc::clone(&self.subscribers),
        }
    }

    /// Live subscriptions; a released or dropped one is already gone.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }

    /// `TxSubscriptionRef.modify`: derive a result AND a successor value in
    /// one step, storing the successor and publishing it. Returns the derived
    /// result.
    ///
    /// The other read-then-write forms are derived from this exactly as the
    /// pin derives them, so there is one place where the value moves.
    pub fn modify<B>(&self, f: impl FnOnce(T) -> (B, T)) -> B {
        let (result, next) = f(tx_read(&self.current));
        self.set(next);
        result
    }

    /// `TxSubscriptionRef.update`: derive the successor and publish it.
    pub fn update(&self, f: impl FnOnce(T) -> T) {
        self.modify(|current| ((), f(current)));
    }

    /// `TxSubscriptionRef.getAndSet`: store `value`, return the PREVIOUS.
    pub fn get_and_set(&self, value: T) -> T {
        self.modify(|current| (current, value))
    }

    /// `TxSubscriptionRef.getAndUpdate`: store `f(current)`, return the
    /// PREVIOUS.
    pub fn get_and_update(&self, f: impl FnOnce(T) -> T) -> T {
        let previous = tx_read(&self.current);
        self.set(f(previous.clone()));
        previous
    }

    /// `TxSubscriptionRef.updateAndGet`: store `f(current)`, return the NEW.
    pub fn update_and_get(&self, f: impl FnOnce(T) -> T) -> T {
        let next = f(tx_read(&self.current));
        self.set(next.clone());
        next
    }
}

/// Always true for `TxSubscriptionRef` (the type already narrows; mirrors
/// `isTxSubscriptionRef`). Foreign values are different types and cannot
/// reach this predicate.
pub fn is_tx_subscription_ref<T>(_: &TxSubscriptionRef<T>) -> bool {
    true
}
