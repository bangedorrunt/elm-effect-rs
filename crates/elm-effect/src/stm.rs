//! STM-lite: serializable transactions over one global lock.
//!
//! Every operation runs inside [`atomically`]: a global mutex serializes
//! transactions, so multi-cell blocks are atomic and isolated. Per-cell
//! mutexes nest inside (global first, always), so no lock order inverts.
//! This is coarse but correct; optimistic concurrency, blocking `retry`
//! and cross-thread fairness are residuals, not silent gaps.

use std::cell::Cell;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::service::{Reference, read_reference};
use crate::Effect;

static GLOBAL: Mutex<()> = Mutex::new(());

thread_local! {
    static IN_TX: Cell<bool> = const { Cell::new(false) };
}

/// Ids for refs and transactions: only uniqueness matters, and the two
/// never share a comparison.
static NEXT_TX_ID: AtomicU64 = AtomicU64::new(1);

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

/// One transaction's staged view of one ref.
struct Staged<T> {
    value: T,
    written: bool,
}

/// A ref's committed value plus the staged views of every transaction
/// that has touched it, keyed by transaction id so concurrent
/// transactions never share a staging slot.
struct RefState<T> {
    value: T,
    staged: HashMap<u64, Staged<T>>,
}

/// Publish or discard one transaction's staged view of a ref. Erased so a
/// journal can hold refs of different value types; the values themselves
/// never leave their ref.
trait Touched: Send + Sync {
    fn commit_tx(&self, tx: u64);
    fn clear_tx(&self, tx: u64);
}

struct RefHandle<T> {
    id: u64,
    state: Mutex<RefState<T>>,
}

impl<T: Send + 'static> Touched for RefHandle<T> {
    fn commit_tx(&self, tx: u64) {
        let mut state = self.state.lock().unwrap();
        if let Some(staged) = state.staged.remove(&tx) {
            if staged.written {
                state.value = staged.value;
            }
        }
    }

    fn clear_tx(&self, tx: u64) {
        self.state.lock().unwrap().staged.remove(&tx);
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
                    value: initial,
                    staged: HashMap::new(),
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
/// clears otherwise.
pub struct Transaction {
    id: u64,
    touched: Mutex<Vec<Arc<dyn Touched>>>,
}

impl Transaction {
    pub(crate) fn new() -> Self {
        Self {
            id: NEXT_TX_ID.fetch_add(1, Ordering::Relaxed),
            touched: Mutex::new(Vec::new()),
        }
    }

    /// Record one ref as touched. A repeated touch adds a second driver,
    /// which is harmless: committing or clearing an already-removed
    /// staged entry is a no-op.
    fn touch<T: Send + 'static>(&self, handle: &Arc<RefHandle<T>>) {
        let driver: Arc<dyn Touched> = Arc::clone(handle) as Arc<dyn Touched>;
        self.touched.lock().unwrap().push(driver);
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
                    let committed = state.value.clone();
                    state
                        .staged
                        .entry(tx.id)
                        .or_insert_with(|| Staged {
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
                    let committed = state.value.clone();
                    let staged = state
                        .staged
                        .entry(tx.id)
                        .or_insert_with(|| Staged {
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
                    let committed = state.value.clone();
                    let staged = state
                        .staged
                        .entry(tx.id)
                        .or_insert_with(|| Staged {
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

/// A transactional max-heap priority queue.
pub struct TxPriorityQueue<T> {
    inner: Mutex<BinaryHeap<T>>,
}

impl<T: Ord> TxPriorityQueue<T> {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Push one element.
    pub fn push(&self, value: T) {
        tx_cell(&self.inner, |heap| heap.push(value));
    }

    /// Pop the maximum, if any.
    pub fn pop(&self) -> Option<T> {
        atomically(|| self.inner.lock().unwrap().pop())
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

impl<T: Ord> Default for TxPriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
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

/// A transactional cell remembering every assigned value: subscribers
/// replay the full history including the initial value.
pub struct TxSubscriptionRef<T> {
    current: Mutex<T>,
    history: Mutex<Vec<T>>,
}

impl<T: Clone> TxSubscriptionRef<T> {
    /// A cell holding `initial` with one history entry.
    pub fn new(initial: T) -> Self {
        Self {
            current: Mutex::new(initial.clone()),
            history: Mutex::new(vec![initial]),
        }
    }

    /// The current value.
    pub fn get(&self) -> T {
        tx_read(&self.current)
    }

    /// Assign and record.
    pub fn set(&self, value: T) {
        atomically(|| {
            *self.current.lock().unwrap() = value.clone();
            self.history.lock().unwrap().push(value);
        });
    }

    /// Every assigned value, oldest first.
    pub fn changes(&self) -> Vec<T> {
        atomically(|| self.history.lock().unwrap().clone())
    }

    /// `TxSubscriptionRef.modify`: derive a result AND a successor value in
    /// one step, storing the successor and recording it. Returns the derived
    /// result.
    ///
    /// The other read-then-write forms are derived from this exactly as the
    /// pin derives them, so there is one place where the value moves.
    pub fn modify<B>(&self, f: impl FnOnce(T) -> (B, T)) -> B {
        let (result, next) = f(tx_read(&self.current));
        self.set(next);
        result
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
