//! Law L9: shared state done in ladder order.
//!
//! Immutable data first; a pure atomic update may retry and must not log,
//! emit, or perform I/O (it runs outside any lock and repeats on contention);
//! two facts that move together are one value; two tasks communicate through
//! a bounded queue.
//!
//! Everything here is synchronous (`std` locks only): async variants arrive
//! with the executor beads. Locking follows one rule — never hold a guard
//! across effect interpretation, except [`SynchronizedRef`]'s decision step,
//! which runs its update computation under the permit.

use crate::Exit;
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::Effect;

/// Lock a cell even after a panic poisoned it. The guarded state here is a
/// plain value or a one-shot slot, and a panic in a user callback leaves it
/// unchanged rather than half-written, so recovery beats bricking the cell.
trait RecoverLock<T> {
    fn recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> RecoverLock<T> for std::sync::Mutex<T> {
    fn recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// An atomically updated cell. [`update`](Ref::update) snapshots outside the
/// lock and retries on a lost race, so the closure MUST be pure: it may run
/// more than once, and it must not log, emit, or perform I/O.
///
/// ```compile_fail
/// # use elm_effect::{Ref, Effect};
/// # let r = Ref::new(0u32);
/// // Effect-returning closures are rejected: updates are values, not effects.
/// r.update(|_| Effect::succeed(1u32));
/// ```
pub struct Ref<A> {
    inner: Mutex<Versioned<A>>,
}

struct Versioned<A> {
    value: A,
    version: u64,
}

impl<A: Clone + Send> Ref<A> {
    /// A cell holding `value`.
    pub fn new(value: A) -> Self {
        Self {
            inner: Mutex::new(Versioned { value, version: 0 }),
        }
    }

    /// The current value.
    pub fn get(&self) -> A {
        self.inner.lock().unwrap().value.clone()
    }

    /// `Ref.getAndSet`: store `value` and return the PREVIOUS value, in one
    /// step. Atomic by construction (the whole read-and-write happens under
    /// the same lock), unlike a get followed by a set.
    pub fn get_and_set(&self, value: A) -> A {
        let mut inner = self.inner.lock().unwrap();
        let previous = std::mem::replace(&mut inner.value, value);
        inner.version += 1;
        previous
    }

    /// `Ref.getAndUpdate`: store `f(current)` and return the PREVIOUS value,
    /// atomically.
    pub fn get_and_update(&self, f: impl Fn(&A) -> A) -> A {
        let mut inner = self.inner.lock().unwrap();
        // Evaluate the closure BEFORE taking the mutable borrow: replace
        // would otherwise borrow `value` mutably and immutably at once.
        let next = f(&inner.value);
        let previous = std::mem::replace(&mut inner.value, next);
        inner.version += 1;
        previous
    }

    /// `Ref.updateAndGet`: store `f(current)` and return the NEW value,
    /// atomically.
    pub fn update_and_get(&self, f: impl Fn(&A) -> A) -> A {
        let mut inner = self.inner.lock().unwrap();
        inner.value = f(&inner.value);
        inner.version += 1;
        inner.value.clone()
    }

    /// `Ref.getAndUpdateSome`: apply `f` only when it returns `Some`,
    /// storing the new value, and always return the PREVIOUS value. `None`
    /// leaves the cell untouched but still reports what was there.
    pub fn get_and_update_some(&self, f: impl Fn(&A) -> Option<A>) -> A {
        let mut inner = self.inner.lock().unwrap();
        let previous = inner.value.clone();
        if let Some(next) = f(&inner.value) {
            inner.value = next;
            inner.version += 1;
        }
        previous
    }

    /// `Ref.updateSome`: apply `f` only when it returns `Some`. `None`
    /// leaves the cell untouched (and, matching Effect 4, bumps no version).
    pub fn update_some(&self, f: impl Fn(&A) -> Option<A>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(next) = f(&inner.value) {
            inner.value = next;
            inner.version += 1;
        }
    }

    /// `Ref.updateSomeAndGet`: apply `f` when it returns `Some` and return
    /// the resulting CURRENT value. On `None` the cell is unchanged, so the
    /// current value is the one already held.
    pub fn update_some_and_get(&self, f: impl Fn(&A) -> Option<A>) -> A {
        let mut inner = self.inner.lock().unwrap();
        if let Some(next) = f(&inner.value) {
            inner.value = next;
            inner.version += 1;
        }
        inner.value.clone()
    }

    /// `Ref.modify`: compute `(result, next)` in one step, store `next`
    /// unconditionally, and return `result`. The whole read-compute-write
    /// happens under the lock, so it is atomic unlike a get followed by a
    /// set (`Ref.ts:461`).
    pub fn modify<B>(&self, f: impl Fn(&A) -> (B, A)) -> B {
        let mut inner = self.inner.lock().unwrap();
        let (out, next) = f(&inner.value);
        inner.value = next;
        inner.version += 1;
        out
    }

    /// `Ref.modifySome`: like [`modify`](Ref::modify), but `None` from `f`
    /// leaves the cell unchanged. The result is always returned; Effect 4
    /// implements this by delegating to `modify` and substituting the
    /// current value on `None` (`Ref.ts:521`), so a `None` stores nothing
    /// and bumps no version.
    pub fn modify_some<B>(&self, f: impl Fn(&A) -> (B, Option<A>)) -> B {
        let mut inner = self.inner.lock().unwrap();
        let (out, next) = f(&inner.value);
        if let Some(next) = next {
            inner.value = next;
            inner.version += 1;
        }
        out
    }

    /// `Ref.set`: store `value`, unconditionally. Verified in Effect 4
    /// (`Ref.ts:236`); the crate was missing it, so the row that asserts it
    /// could not be pinned without it.
    pub fn set(&self, value: A) {
        let mut inner = self.inner.lock().unwrap();
        inner.value = value;
        inner.version += 1;
    }

    /// Apply a pure function, retrying on contention. The closure runs
    /// outside the lock and may run more than once: keep it pure.
    pub fn update(&self, f: impl Fn(&A) -> A) -> A {
        loop {
            let (snapshot, version) = {
                let inner = self.inner.lock().unwrap();
                (inner.value.clone(), inner.version)
            };
            let next = f(&snapshot);
            let mut inner = self.inner.lock().unwrap();
            if inner.version == version {
                inner.value = next.clone();
                inner.version += 1;
                return next;
            }
        }
    }
}

/// A plain synchronized cell: direct get/set, one in-lock update.
pub struct MutableRef<A> {
    inner: Mutex<A>,
}

impl<A: Clone + Send> MutableRef<A> {
    /// A cell holding `value`.
    pub fn new(value: A) -> Self {
        Self {
            inner: Mutex::new(value),
        }
    }

    /// The current value.
    pub fn get(&self) -> A {
        self.inner.lock().unwrap().clone()
    }

    /// Replace the value.
    pub fn set(&self, value: A) {
        *self.inner.lock().unwrap() = value;
    }

    /// Apply a function once under the lock.
    pub fn update(&self, f: impl FnOnce(&mut A)) {
        f(&mut self.inner.lock().unwrap());
    }
}

/// A cell with effectful updates. The update computation takes the permit and
/// runs exactly once (never retried, unlike [`Ref`]); the produced effect
/// runs after, with the same requirements it was built with.
pub struct SynchronizedRef<A> {
    inner: std::sync::Arc<Mutex<A>>,
}

/// The effect a `SynchronizedRef` combinator parks between its decision step
/// and its `flat_map`. Named because the inline type trips clippy's
/// complexity check once an `Option` is nested inside.
type ParkedEffect<T> = std::sync::Arc<Mutex<Option<T>>>;

impl<A: Clone + Send + 'static> SynchronizedRef<A> {
    /// A cell holding `value`.
    pub fn new(value: A) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(value)),
        }
    }

    /// The current value.
    pub fn get(&self) -> A {
        self.inner.recover().clone()
    }

    /// Conditionally update with a fallback (`modifySomeEffect`): `f`
    /// returns the new value plus a result, or `None` to keep the current
    /// value and yield `fallback`. The decision runs once under the
    /// permit; curried closures work because the callback is the only
    /// configuration besides the fallback.
    pub fn modify_some_effect<B, E, R, F>(&self, fallback: B, f: F) -> Effect<B, E, R>
    where
        B: Send + 'static,
        E: Send + 'static,
        R: crate::effect::Requirement,
        F: FnOnce(&A) -> Option<(A, B)> + Send + 'static,
    {
        let inner = std::sync::Arc::clone(&self.inner);
        Effect::<B, E, R>::sync(move || {
            let mut guard = inner.recover();
            match f(&*guard) {
                Some((next, out)) => {
                    *guard = next;
                    out
                }
                None => fallback,
            }
        })
    }

    /// `SynchronizedRef.getAndUpdateSomeEffect`: run `f(current)`'s effect;
    /// `Some(next)` stores `next`, `None` leaves the cell unchanged. Either
    /// way the effect yields the PREVIOUS value (`SynchronizedRef.ts:249`).
    /// The decision step runs once under the permit; the effect runs after,
    /// so a lock is never held across effect interpretation.
    pub fn get_and_update_some_effect<E, R, F>(&self, f: F) -> Effect<A, E, R>
    where
        A: Clone + Send + 'static,
        E: Send + 'static,
        R: crate::effect::Requirement,
        F: FnOnce(&A) -> Effect<Option<A>, E, R> + Send + 'static,
    {
        let inner = std::sync::Arc::clone(&self.inner);
        let previous: std::sync::Arc<Mutex<Option<A>>> = std::sync::Arc::new(Mutex::new(None));
        let slot: ParkedEffect<Effect<Option<A>, E, R>> = std::sync::Arc::new(Mutex::new(None));
        let setup = Effect::<(), E, R>::sync({
            let (inner, previous, slot) = (
                std::sync::Arc::clone(&inner),
                std::sync::Arc::clone(&previous),
                std::sync::Arc::clone(&slot),
            );
            move || {
                let guard = inner.recover();
                let current = guard.clone();
                *slot.recover() = Some(f(&current));
                *previous.recover() = Some(current);
            }
        });
        let (inner, previous, slot) = (
            std::sync::Arc::clone(&inner),
            std::sync::Arc::clone(&previous),
            std::sync::Arc::clone(&slot),
        );
        setup.flat_map(move |_| {
            let effect = slot.recover().take().expect("recorded in setup");
            effect.flat_map(move |next| {
                if let Some(next) = next {
                    *inner.recover() = next;
                }
                Effect::succeed(previous.recover().take().expect("recorded in setup"))
            })
        })
    }

    /// `SynchronizedRef.getAndUpdateSome`: the pure conditional variant —
    /// apply `f` when it returns `Some`, and return the PREVIOUS value
    /// (`SynchronizedRef.ts:224`).
    pub fn get_and_update_some(&self, f: impl Fn(&A) -> Option<A>) -> A {
        let mut guard = self.inner.recover();
        let previous = guard.clone();
        if let Some(next) = f(&guard) {
            *guard = next;
        }
        previous
    }

    /// `SynchronizedRef.updateAndGetEffect`: run `f(current)`'s effect, store
    /// its result, and return it (`SynchronizedRef.ts:538`). Distinct from
    /// [`update_effect`](SynchronizedRef::update_effect), whose effect result
    /// is unrelated to the stored value.
    pub fn update_and_get_effect<E, R, F>(&self, f: F) -> Effect<A, E, R>
    where
        A: Clone + Send + 'static,
        E: Send + 'static,
        R: crate::effect::Requirement,
        F: FnOnce(&A) -> Effect<A, E, R> + Send + 'static,
    {
        let inner = std::sync::Arc::clone(&self.inner);
        let slot: std::sync::Arc<Mutex<Option<Effect<A, E, R>>>> =
            std::sync::Arc::new(Mutex::new(None));
        let setup = Effect::<(), E, R>::sync({
            let (inner, slot) = (std::sync::Arc::clone(&inner), std::sync::Arc::clone(&slot));
            move || {
                let guard = inner.recover();
                *slot.recover() = Some(f(&guard));
            }
        });
        let (inner, slot) = (std::sync::Arc::clone(&inner), std::sync::Arc::clone(&slot));
        setup.flat_map(move |_| {
            let effect = slot.recover().take().expect("recorded in setup");
            let inner = std::sync::Arc::clone(&inner);
            effect.map(move |new_value| {
                *inner.recover() = new_value.clone();
                new_value
            })
        })
    }

    /// `SynchronizedRef.getAndUpdateEffect`: store `f(current)` and return
    /// the PREVIOUS value, running `f`'s effect once afterwards.
    ///
    /// Distinct from [`update_effect`], which returns the effect's result:
    /// Effect 4's getAndUpdateEffect returns what was in the cell before the
    /// update, which is what makes the two rows distinguishable.
    pub fn get_and_update_effect<B, E, R, F>(&self, f: F) -> Effect<A, E, R>
    where
        A: Clone + Send + 'static,
        B: Send + 'static,
        E: Send + 'static,
        R: crate::effect::Requirement,
        F: FnOnce(&A) -> (A, Effect<B, E, R>) + Send + 'static,
    {
        let inner = std::sync::Arc::clone(&self.inner);
        let previous: std::sync::Arc<Mutex<Option<A>>> = std::sync::Arc::new(Mutex::new(None));
        let slot: std::sync::Arc<Mutex<Option<Effect<B, E, R>>>> =
            std::sync::Arc::new(Mutex::new(None));
        let setup = Effect::<(), E, R>::sync({
            let (inner, previous, slot) = (
                std::sync::Arc::clone(&inner),
                std::sync::Arc::clone(&previous),
                std::sync::Arc::clone(&slot),
            );
            move || {
                let mut guard = inner.recover();
                let (next, effect) = f(&*guard);
                *previous.recover() = Some(std::mem::replace(&mut *guard, next));
                *slot.recover() = Some(effect);
            }
        });
        let (previous, slot) = (
            std::sync::Arc::clone(&previous),
            std::sync::Arc::clone(&slot),
        );
        setup.flat_map(move |_| {
            let effect = slot.recover().take();
            let previous = std::sync::Arc::clone(&previous);
            match effect {
                Some(effect) => effect.flat_map(move |_| {
                    Effect::succeed(previous.recover().take().expect("recorded in setup"))
                }),
                None => Effect::succeed(previous.recover().take().expect("recorded in setup")),
            }
        })
    }

    /// Compute `(new_value, effect)` under the permit, store the value, then
    /// run the effect once. The permit serializes the computation, not the
    /// effect's execution.
    pub fn update_effect<B, E, R, F>(&self, f: F) -> Effect<B, E, R>
    where
        B: Send + 'static,
        E: Send + 'static,
        R: crate::effect::Requirement,
        F: FnOnce(&A) -> (A, Effect<B, E, R>) + Send + 'static,
    {
        let inner = std::sync::Arc::clone(&self.inner);
        let slot: std::sync::Arc<Mutex<Option<Effect<B, E, R>>>> =
            std::sync::Arc::new(Mutex::new(None));
        let setup = Effect::<(), E, R>::sync({
            let slot = std::sync::Arc::clone(&slot);
            move || {
                let mut guard = inner.recover();
                let (next, effect) = f(&*guard);
                *guard = next;
                *slot.recover() = Some(effect);
            }
        });
        let slot = std::sync::Arc::clone(&slot);
        setup.flat_map(move |_| {
            slot.lock()
                .unwrap()
                .take()
                .expect("setup stores the effect")
        })
    }
}

/// A one-shot handoff: completed once, waited on by anyone.
///
/// Carries an [`Exit`], not a bare value, because Effect 4 completes a
/// `Deferred` "exactly once with a success, failure, defect, or
/// interruption" (`Deferred.ts:3`) -- four outcomes, not one. The previous
/// `Option<A>` shape could only record success, so every failure/interrupt
/// row was unreachable.
pub struct Deferred<A, E> {
    inner: Mutex<Option<Exit<A, E>>>,
    ready: Condvar,
}

impl<A: Clone + Send, E: Clone + Send> Deferred<A, E> {
    /// An empty handoff.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    /// Complete with a value. Returns false if already completed; the first
    /// completion wins and later ones are ignored.
    pub fn complete(&self, value: A) -> bool {
        self.complete_exit(Exit::Success(value))
    }

    /// Run `f` to produce the value, but only while this deferred is still
    /// empty (`Deferred.complete` with an effect). The body therefore runs
    /// exactly once: a second call returns false without calling `f`, which is
    /// the memoization the tag's row asserts.
    pub fn complete_with(&self, f: impl FnOnce() -> A) -> bool {
        let mut guard = self.inner.recover();
        if guard.is_some() {
            return false;
        }
        *guard = Some(Exit::Success(f()));
        self.ready.notify_all();
        true
    }

    /// Complete with any outcome, including a failure or an interrupt.
    pub fn complete_exit(&self, outcome: Exit<A, E>) -> bool {
        let mut guard = self.inner.recover();
        if guard.is_some() {
            return false;
        }
        *guard = Some(outcome);
        self.ready.notify_all();
        true
    }

    /// The recorded outcome, or `None` while still empty.
    pub fn poll(&self) -> Option<Exit<A, E>> {
        self.inner.recover().clone()
    }

    /// True once any outcome has been recorded.
    pub fn is_done(&self) -> bool {
        self.inner.recover().is_some()
    }

    /// Block until completed, then return the value. Panics only when the
    /// recorded outcome was not a success: use [`poll`] to inspect why.
    ///
    /// `E: Debug` is required by the failure message only, and is declared on
    /// this method rather than the `impl` so the rest of the type stays free
    /// of it.
    pub fn wait(&self) -> A
    where
        E: std::fmt::Debug,
        A: std::fmt::Debug,
    {
        let mut guard = self.inner.recover();
        while guard.is_none() {
            guard = self.ready.wait(guard).unwrap();
        }
        match guard.clone().expect("checked") {
            Exit::Success(value) => value,
            other => panic!("deferred did not complete with a success: {other:?}"),
        }
    }
}

impl<A: Clone + Send, E: Clone + Send> Default for Deferred<A, E> {
    fn default() -> Self {
        Self::new()
    }
}

/// How a full queue treats a new offer (`Queue.make`'s `strategy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// The offerer waits for room (`Queue.bounded`).
    Backpressure,
    /// A full queue drops the new item (`Queue.make` with `"dropping"`).
    Dropping,
    /// A full queue drops its oldest item to make room (`"sliding"`).
    Sliding,
}

/// A queue's terminal state: open, ended (no more offers, drains stay
/// readable), or failed with a reason.
enum QueueState {
    Open,
    Ended,
    Failed(String),
    /// Interrupted (`Queue.shutdown`): takers are interrupted and the
    /// buffered messages drain but offers are refused.
    Interrupted,
}

/// A take against a finished queue (`Queue.take`, `take_n`,
/// `take_between`): `Done` after `end`, the failure after `fail`,
/// `Interrupted` after `shutdown`.
#[derive(Clone, Debug, PartialEq)]
pub enum QueueError {
    /// The queue ended normally and held nothing more.
    Done,
    /// The queue failed with this reason after its buffer drained.
    Failed(String),
    /// The queue was shut down while the take was waiting.
    Interrupted,
}

/// What an interruptible offer got: accepted, or refused with the
/// message still in hand.
pub enum OfferOutcome<A> {
    /// The item entered the queue.
    Accepted,
    /// The queue was closed: the item is untouched.
    Refused(A),
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done => write!(f, "Done"),
            Self::Failed(message) => write!(f, "{message}"),
            Self::Interrupted => write!(f, "Interrupted"),
        }
    }
}

/// The write-only face (`Queue.Enqueue`): offers, nothing else.
pub struct Enqueue<A: Send + 'static> {
    queue: Arc<Queue<A>>,
}

impl<A: Send + 'static> Enqueue<A> {
    /// Offer one item, waiting for room under backpressure. False when the
    /// queue closed or refused it.
    pub fn offer(&self, value: A) -> bool {
        self.queue.offer(value)
    }

    /// Offer a batch, returning whatever was not accepted.
    pub fn offer_all(&self, values: impl IntoIterator<Item = A>) -> Vec<A> {
        self.queue.offer_all(values)
    }
}

/// The read-only face (`Queue.Dequeue`): takes, nothing else.
pub struct Dequeue<A: Send + 'static> {
    queue: Arc<Queue<A>>,
}

impl<A: Send + 'static> Dequeue<A> {
    /// Take one item, waiting while empty.
    pub fn take(&self) -> Result<A, QueueError> {
        self.queue.take()
    }

    /// Take up to `n` items.
    pub fn take_n(&self, n: usize) -> Result<Vec<A>, QueueError> {
        self.queue.take_n(n)
    }

    /// Take between `min` and `max` items.
    pub fn take_between(&self, min: usize, max: usize) -> Result<Vec<A>, QueueError> {
        self.queue.take_between(min, max)
    }

    /// Drain everything buffered.
    pub fn take_all(&self) -> Vec<A> {
        self.queue.take_all()
    }
}

/// `Queue.isEnqueue`: true for a queue or its write-only view.
pub fn is_enqueue<A: Send + 'static>(value: &dyn Any) -> bool {
    value.is::<Queue<A>>() || value.is::<Enqueue<A>>()
}

/// `Queue.isDequeue`: true for a queue or its read-only view.
pub fn is_dequeue<A: Send + 'static>(value: &dyn Any) -> bool {
    value.is::<Queue<A>>() || value.is::<Dequeue<A>>()
}

/// `Queue.isQueue`: true for a queue or either view.
pub fn is_queue<A: Send + 'static>(value: &dyn Any) -> bool {
    value.is::<Queue<A>>() || value.is::<Enqueue<A>>() || value.is::<Dequeue<A>>()
}

/// A bounded multi-producer multi-consumer queue. Senders wait for room
/// under the backpressure strategy, receivers wait for items; `end`,
/// `fail`, and `shutdown` close the offer side while the buffer drains.
///
/// Two queue rules need state beyond the buffer. Offers that are parked
/// when `end`/`fail` arrive keep going: the terminal flip is deferred
/// until they settle, which is why `await` waits and why the offers a
/// fail interrupts still land. A zero-capacity queue rendezvouses through
/// a reservation: a taker claims the next item, an offerer may only push
/// while one is outstanding, so a message is never enqueued twice.
pub struct Queue<A> {
    // `Arc` so an interrupt hook can lock the same mutex before notifying,
    // which is what closes the lost-wakeup gap between re-check and wait.
    inner: Arc<Mutex<VecDeque<A>>>,
    capacity: usize,
    strategy: Strategy,
    state: Mutex<QueueState>,
    /// The terminal state waiting for parked offers to settle.
    pending_close: Mutex<Option<QueueState>>,
    /// Offers currently parked on a full queue.
    in_flight: Mutex<usize>,
    /// Takers parked on the zero-capacity rendezvous. An offer may hand its
    /// item over only while one of them waits, which is what makes the
    /// handoff a rendezvous instead of a buffered push.
    waiting_takers: Mutex<usize>,
    /// Releases scheduled by `offerUnsafe` and performed by `flushUnsafe`.
    scheduled_releases: Mutex<usize>,
    // Shared so an interrupt hook can notify the wait without borrowing the
    // queue for `'static`.
    not_full: Arc<Condvar>,
    not_empty: Arc<Condvar>,
    finished: Condvar,
}
impl<A: Send> Queue<A> {
    /// A queue holding at most `capacity` items (backpressure).
    pub fn new(capacity: usize) -> Self {
        Self::with_strategy(capacity, Strategy::Backpressure)
    }

    /// A queue that never fills (`Queue.unbounded`).
    pub fn unbounded() -> Self {
        Self::with_strategy(usize::MAX, Strategy::Backpressure)
    }

    /// A queue with the given capacity and overflow strategy.
    pub fn with_strategy(capacity: usize, strategy: Strategy) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
            capacity,
            strategy,
            state: Mutex::new(QueueState::Open),
            pending_close: Mutex::new(None),
            in_flight: Mutex::new(0),
            waiting_takers: Mutex::new(0),
            scheduled_releases: Mutex::new(0),
            not_full: Arc::new(Condvar::new()),
            not_empty: Arc::new(Condvar::new()),
            finished: Condvar::new(),
        }
    }

    /// The bound (`Queue.capacity`).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The overflow strategy.
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    /// Push, blocking while full under backpressure. Dropping and sliding
    /// never block: a full dropping queue drops this item, a sliding one
    /// drops its oldest. A queue that closes while this offer is parked
    /// withdraws the message instead of pushing it.
    /// Offer one item (`Queue.offer`). Under backpressure it waits for
    /// room; a full dropping queue refuses it; a full sliding queue drops
    /// its oldest. A queue that closes while this offer is parked withdraws
    /// the message instead of pushing it. Returns whether the item landed.
    pub fn offer(&self, value: A) -> bool {
        if self.strategy != Strategy::Backpressure {
            return self.try_offer(value);
        }
        let mut guard = self.inner.lock().unwrap();
        let mut parked = false;
        loop {
            // The closed check comes first: a queue that shut down while
            // this offer was parked withdraws the message even if capacity
            // freed in the same wakeup.
            if !matches!(&*self.state.lock().unwrap(), QueueState::Open) {
                if parked {
                    self.leave();
                }
                return false;
            }
            if !self.is_full(&guard) {
                break;
            }
            if !parked {
                parked = true;
                *self.in_flight.lock().unwrap() += 1;
            }
            guard = self.not_full.wait(guard).unwrap();
        }
        if parked {
            self.leave();
        }
        self.push_item(&mut guard, value);
        true
    }

    /// Push without blocking. False when full or closed.
    pub fn try_offer(&self, value: A) -> bool {
        if !matches!(&*self.state.lock().unwrap(), QueueState::Open) {
            return false;
        }
        let mut guard = self.inner.lock().unwrap();
        if self.is_full(&guard) {
            if self.strategy != Strategy::Sliding {
                return false;
            }
            guard.pop_front();
        }
        self.push_item(&mut guard, value);
        true
    }

    /// `Queue.offerUnsafe`: push without capacity checks, without parking,
    /// and WITHOUT waking a waiting taker — the release is scheduled, and
    /// only [`Queue::flush_unsafe`] performs it, so repeated offers
    /// coalesce into one wakeup.
    pub fn offer_unsafe(&self, value: A) {
        let mut guard = self.inner.lock().unwrap();
        guard.push_back(value);
        let mut releases = self.scheduled_releases.lock().unwrap();
        *releases += 1;
    }

    /// `Queue.flushUnsafe`: perform the scheduled releases from every
    /// `offerUnsafe` since the last flush as one wakeup, then clear the
    /// schedule. A no-op after shutdown, and a no-op when nothing is
    /// scheduled.
    pub fn flush_unsafe(&self) {
        if !matches!(&*self.state.lock().unwrap(), QueueState::Open) {
            return;
        }
        let scheduled = {
            let mut releases = self.scheduled_releases.lock().unwrap();
            let scheduled = *releases;
            *releases = 0;
            scheduled
        };
        if scheduled > 0 {
            self.not_empty.notify_all();
        }
    }

    /// `Queue.flush`: the effectful flush. The crate's queue is
    /// synchronous, so this is `flush_unsafe`.
    pub fn flush(&self) {
        self.flush_unsafe();
    }

    /// Whether an offer must wait. A capacity-0 queue has no buffer to
    /// speak of: an offer may hand its item over only while a taker waits,
    /// and a second offer waits until the first item is taken.
    fn is_full(&self, guard: &VecDeque<A>) -> bool {
        if self.capacity == 0 {
            return !guard.is_empty() || *self.waiting_takers.lock().unwrap() == 0;
        }
        guard.len() >= self.capacity
    }

    /// A taker joins the rendezvous, waking offerers parked on `not_full`.
    fn claim_rendezvous(&self) {
        *self.waiting_takers.lock().unwrap() += 1;
        self.not_full.notify_all();
    }

    /// A taker leaves the rendezvous, waking both sides. The offer side must
    /// re-check whether anyone still waits; the take side must re-check the
    /// terminal state. Without this a taker that interrupted would leave an
    /// offerer parked forever, which is the leak the reservation pair used
    /// to cause.
    fn release_rendezvous(&self) {
        let mut waiting = self.waiting_takers.lock().unwrap();
        *waiting -= 1;
        drop(waiting);
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    /// The one place an item enters the buffer.
    fn push_item(&self, guard: &mut VecDeque<A>, value: A) {
        guard.push_back(value);
        self.not_empty.notify_one();
    }

    /// A parked offer finished: the last one out applies a deferred
    /// terminal close.
    fn leave(&self) {
        let mut in_flight = self.in_flight.lock().unwrap();
        *in_flight -= 1;
        let last = *in_flight == 0;
        drop(in_flight);
        if last {
            let pending = self.pending_close.lock().unwrap().take();
            if let Some(next) = pending {
                let mut state = self.state.lock().unwrap();
                if matches!(&*state, QueueState::Open) {
                    *state = next;
                }
                drop(state);
                self.finished.notify_all();
            }
        }
    }

    /// Push while watching `interrupted`: an interrupted offer withdraws
    /// its message (`Err(())`), a closed queue refuses it untouched
    /// (`Ok(Refused(value))`), and an accepted one reports
    /// `Ok(Accepted)`. The crate's interruptible-wait pattern, the one the
    /// semaphore uses.
    // `Err(())` is the interrupt marker itself: it carries no data, and the
    // caller's only decision is to withdraw the offer. A named zero-sized
    // error would spell the same thing one type longer.
    #[allow(clippy::result_unit_err)]
    pub fn offer_interruptible(
        &self,
        value: A,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<OfferOutcome<A>, ()>
    where
        A: Send + 'static,
    {
        let mut guard = self.inner.lock().unwrap();
        let value = value;
        let mut parked = false;
        loop {
            if !matches!(&*self.state.lock().unwrap(), QueueState::Open) {
                if parked {
                    self.leave();
                }
                return Ok(OfferOutcome::Refused(value));
            }
            if interrupted() {
                if parked {
                    self.leave();
                }
                return Err(());
            }
            if !self.is_full(&guard) {
                if parked {
                    self.leave();
                }
                self.push_item(&mut guard, value);
                return Ok(OfferOutcome::Accepted);
            }
            if self.strategy == Strategy::Sliding && guard.len() >= self.capacity {
                guard.pop_front();
                continue;
            }
            // A full dropping queue refuses the new item instead of parking:
            // Effect 4's offer reports false immediately.
            if self.strategy == Strategy::Dropping && guard.len() >= self.capacity {
                return Ok(OfferOutcome::Refused(value));
            }
            if !parked {
                parked = true;
                *self.in_flight.lock().unwrap() += 1;
            }
            let hook = crate::fiber::subscribe_current_interrupt({
                let inner = Arc::clone(&self.inner);
                let not_full = Arc::clone(&self.not_full);
                move || {
                    let _guard = inner.lock().unwrap();
                    not_full.notify_all();
                }
            });
            // Subscribe first, then re-check: an interrupt between the check
            // above and the subscription is caught here, and one after the
            // subscription wakes the condvar.
            if interrupted() {
                if parked {
                    self.leave();
                }
                return Err(());
            }
            guard = if hook.is_some() {
                self.not_full.wait(guard).unwrap()
            } else {
                self.not_full
                    .wait_timeout(guard, crate::COOPERATIVE_QUANTUM)
                    .unwrap()
                    .0
            };
        }
    }

    /// Offer a batch, returning whatever was not accepted. Once the
    /// queue closes, or a full dropping queue refuses the rest, the
    /// remainder comes back untouched.
    pub fn offer_all(&self, values: impl IntoIterator<Item = A>) -> Vec<A>
    where
        A: Send + 'static,
    {
        let mut unaccepted = Vec::new();
        for value in values {
            match self.offer_interruptible(value, &|| false) {
                Ok(OfferOutcome::Accepted) => {}
                Ok(OfferOutcome::Refused(value)) => unaccepted.push(value),
                Err(()) => unreachable!("an offer with a false interrupted predicate never fails"),
            }
        }
        unaccepted
    }

    /// Offer a batch while watching `interrupted`: the suffix that never
    /// got room comes back (`Ok`), or the offer itself was interrupted
    /// (`Err`) and every un-offered item is withdrawn with it.
    #[allow(clippy::result_unit_err)]
    pub fn offer_all_interruptible(
        &self,
        values: impl IntoIterator<Item = A>,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<Vec<A>, ()>
    where
        A: Send + 'static,
    {
        let mut remaining = Vec::new();
        for value in values {
            match self.offer_interruptible(value, interrupted) {
                Ok(OfferOutcome::Accepted) => {}
                Ok(OfferOutcome::Refused(value)) => remaining.push(value),
                Err(()) => return Err(()),
            }
        }
        Ok(remaining)
    }

    /// Pop without blocking. None when empty.
    pub fn try_poll(&self) -> Option<A> {
        let mut guard = self.inner.lock().unwrap();
        let value = guard.pop_front()?;
        self.not_full.notify_one();
        Some(value)
    }

    /// Oldest item without removing it. Needs `Clone` because the lock
    /// cannot hand out references.
    pub fn peek(&self) -> Option<A>
    where
        A: Clone,
    {
        self.inner.lock().unwrap().front().cloned()
    }

    /// Pop, blocking while empty. Panics when the queue finishes empty
    /// (use [`take`](Self::take) for the terminal outcome).
    pub fn poll(&self) -> A {
        self.take()
            .expect("queue finished while a poll waited for an item")
    }

    /// Pop, blocking while empty, with the terminal outcome (`Queue.take`).
    pub fn take(&self) -> Result<A, QueueError> {
        let mut guard = self.inner.lock().unwrap();
        let mut claimed = false;
        loop {
            if let Some(value) = guard.pop_front() {
                if claimed {
                    self.release_rendezvous();
                }
                self.not_full.notify_one();
                return Ok(value);
            }
            if let Err(error) = self.open_state() {
                if claimed {
                    self.release_rendezvous();
                }
                return Err(error);
            }
            if self.capacity == 0 && !claimed {
                self.claim_rendezvous();
                claimed = true;
            }
            guard = self.not_empty.wait(guard).unwrap();
        }
    }

    /// The queue's terminal state as an error, or `Ok` while it is open.
    fn open_state(&self) -> Result<(), QueueError> {
        match &*self.state.lock().unwrap() {
            QueueState::Open => Ok(()),
            QueueState::Ended => Err(QueueError::Done),
            QueueState::Failed(message) => Err(QueueError::Failed(message.clone())),
            QueueState::Interrupted => Err(QueueError::Interrupted),
        }
    }

    /// Pop while watching `interrupted`.
    #[allow(clippy::result_unit_err)]
    pub fn take_interruptible(
        &self,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<Result<A, QueueError>, ()>
    where
        A: Send + 'static,
    {
        let mut guard = self.inner.lock().unwrap();
        let mut claimed = false;
        loop {
            if let Some(value) = guard.pop_front() {
                if claimed {
                    self.release_rendezvous();
                }
                self.not_full.notify_one();
                return Ok(Ok(value));
            }
            if let Err(error) = self.open_state() {
                if claimed {
                    self.release_rendezvous();
                }
                return Ok(Err(error));
            }
            if interrupted() {
                // The claim is a permit for one offer; leaving it behind
                // would let a later offer push with no taker.
                if claimed {
                    self.release_rendezvous();
                }
                return Err(());
            }
            if self.capacity == 0 && !claimed {
                self.claim_rendezvous();
                claimed = true;
            }
            let hook = crate::fiber::subscribe_current_interrupt({
                let inner = Arc::clone(&self.inner);
                let not_empty = Arc::clone(&self.not_empty);
                move || {
                    let _guard = inner.lock().unwrap();
                    not_empty.notify_all();
                }
            });
            if interrupted() {
                if claimed {
                    self.release_rendezvous();
                }
                return Err(());
            }
            guard = if hook.is_some() {
                self.not_empty.wait(guard).unwrap()
            } else {
                self.not_empty
                    .wait_timeout(guard, crate::COOPERATIVE_QUANTUM)
                    .unwrap()
                    .0
            };
        }
    }

    /// Up to `n` items, waiting for the whole batch when the queue can
    /// supply it; a finished queue drains its insufficient batch first
    /// (`Queue.takeN`).
    pub fn take_n(&self, n: usize) -> Result<Vec<A>, QueueError> {
        self.take_between(n, n)
    }

    /// Between `min` and `max` items: waits for `min`, then takes up to
    /// `max` (`Queue.takeBetween`). Counts arrive already normalized by the
    /// Effect 4's `Count.normalize` — NaN and non-positive become 0, a positive
    /// value floors — so a zero `min` takes nothing without waiting, and an
    /// inverted pair swaps (Effect 4's row :379).
    pub fn take_between(&self, min: usize, max: usize) -> Result<Vec<A>, QueueError> {
        let (mut min, mut max) = (min, max);
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        let mut out = Vec::new();
        let mut claimed = false;
        loop {
            let mut guard = self.inner.lock().unwrap();
            while out.len() < max {
                match guard.pop_front() {
                    Some(value) => out.push(value),
                    None => break,
                }
            }
            if !out.is_empty() {
                self.not_full.notify_all();
            }
            if out.len() >= min {
                if claimed {
                    self.release_rendezvous();
                }
                return Ok(out);
            }
            if let Err(error) = self.open_state() {
                if claimed {
                    self.release_rendezvous();
                }
                return if out.is_empty() { Err(error) } else { Ok(out) };
            }
            // A capacity-0 queue needs a waiting taker before an offer may
            // push; without this claim take_between waits while every
            // offerer waits for a taker (the deadlock the review witnessed).
            if self.capacity == 0 && !claimed {
                self.claim_rendezvous();
                claimed = true;
            }
            guard = self.not_empty.wait(guard).unwrap();
        }
    }

    /// Remove everything buffered.
    pub fn take_all(&self) -> Vec<A> {
        let mut guard = self.inner.lock().unwrap();
        let out: Vec<A> = guard.drain(..).collect();
        self.not_full.notify_all();
        out
    }

    /// `Queue.takeAll` in its waiting form: block until at least one item
    /// is available (or the queue finishes), then drain the buffer. A
    /// finished empty queue reports the terminal error.
    pub fn take_all_blocking(&self) -> Result<Vec<A>, QueueError>
    where
        A: Send + 'static,
    {
        let first = self
            .take_interruptible(&|| false)
            .expect("uninterrupted wait")?;
        let mut out = vec![first];
        out.extend(self.take_all());
        Ok(out)
    }

    /// Drop everything buffered; returns the count dropped.
    pub fn clear(&self) -> usize {
        self.take_all().len()
    }

    /// Close the offer side, let the buffer drain, and fail a take that
    /// waits on an empty ended queue with `Done` (`Queue.end`). Offers
    /// parked when this lands keep going: the terminal flip waits for
    /// them, which is why `await` holds until they settle.
    pub fn end(&self) -> bool {
        self.request_close(QueueState::Ended)
    }

    /// Close the offer side and remember the failure: the buffer drains
    /// first, then takes and `await` report it (`Queue.fail`). Parked
    /// offers finish first, exactly as `end` defers.
    pub fn fail(&self, message: &str) -> bool {
        self.request_close(QueueState::Failed(message.to_string()))
    }

    /// Freeze offers and interrupt waiting takers, drains stay readable
    /// (`Queue.shutdown`). Parked offers withdraw at once. True when this
    /// call performed the shutdown.
    pub fn shutdown(&self) -> bool {
        self.close_now(QueueState::Interrupted)
    }

    /// `Queue.interrupt`: the graceful form of shutdown — offers are
    /// refused and waiting takers interrupted, buffered items drain.
    pub fn interrupt(&self) -> bool {
        self.close_now(QueueState::Interrupted)
    }

    /// `Queue.shutdownUnsafe`: clear the buffer, settle every waiter
    /// synchronously, and keep an existing failure (`fail` before this
    /// still reports it). True when this call performed the shutdown.
    pub fn shutdown_unsafe(&self) -> bool {
        let already_interrupted = matches!(&*self.state.lock().unwrap(), QueueState::Interrupted);
        let next = self
            .pending_close
            .lock()
            .unwrap()
            .take()
            .unwrap_or(QueueState::Interrupted);
        if !already_interrupted {
            let mut state = self.state.lock().unwrap();
            if !matches!(&*state, QueueState::Failed(_)) {
                *state = next;
            }
        }
        self.take_all();
        self.not_empty.notify_all();
        self.not_full.notify_all();
        self.finished.notify_all();
        !already_interrupted
    }

    /// Wait for the queue to finish: `Ok(())` after `end`, the failure
    /// after `fail`, `Err(Interrupted)` after shutdown (`Queue.await`).
    pub fn await_finished(&self) -> Result<(), QueueError> {
        let mut state = self.state.lock().unwrap();
        loop {
            match &*state {
                QueueState::Open => {
                    state = self.finished.wait(state).unwrap();
                }
                QueueState::Ended => return Ok(()),
                QueueState::Failed(message) => return Err(QueueError::Failed(message.clone())),
                QueueState::Interrupted => return Err(QueueError::Interrupted),
            }
        }
    }

    /// Everything buffered, including items offered before a terminal
    /// close (`Queue.collect`: the crate's offers are synchronous, so the
    /// buffer is the whole story).
    pub fn collect(&self) -> Vec<A> {
        self.take_all()
    }

    /// True after a terminal close.
    pub fn is_shutdown(&self) -> bool {
        !matches!(&*self.state.lock().unwrap(), QueueState::Open)
    }

    /// Items currently queued (`Queue.size`).
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// No items queued.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    /// The write-only view (`Queue.asEnqueue`).
    pub fn as_enqueue(queue: &Arc<Self>) -> Arc<Enqueue<A>>
    where
        A: Send + 'static,
    {
        Arc::new(Enqueue {
            queue: Arc::clone(queue),
        })
    }

    /// The read-only view (`Queue.asDequeue`).
    pub fn as_dequeue(queue: &Arc<Self>) -> Arc<Dequeue<A>>
    where
        A: Send + 'static,
    {
        Arc::new(Dequeue {
            queue: Arc::clone(queue),
        })
    }

    /// Ask for a terminal close that waits for parked offers: with none
    /// in flight the flip happens now; otherwise the state stays Open so
    /// the parked offers keep landing, and the last offerer to leave
    /// applies the request (why `await` holds and why offers survive a
    /// fail that lands mid-batch).
    fn request_close(&self, next: QueueState) -> bool {
        let parked = *self.in_flight.lock().unwrap() > 0;
        if parked {
            *self.pending_close.lock().unwrap() = Some(next);
            self.not_empty.notify_all();
            self.not_full.notify_all();
            return true;
        }
        self.close_now(next)
    }

    /// Close at once, waking parked offers to observe it.
    fn close_now(&self, next: QueueState) -> bool {
        let transitioned = {
            let mut state = self.state.lock().unwrap();
            let open = matches!(&*state, QueueState::Open);
            if open {
                *state = next;
            }
            open
        };
        self.not_empty.notify_all();
        self.not_full.notify_all();
        self.finished.notify_all();
        transitioned
    }
}

/// A counting semaphore (`Semaphore.ts` in Effect 4): one pool of `permits`
/// permits, `taken` of them out on loan, and takes that are all-or-nothing.
///
/// A take larger than the pool waits forever, which is Effect 4's `never`
/// (row 115): there is no partial grant on this type. The partitioned
/// semaphore next door is where a wait accumulates permits in stages.
/// Releasing returns the count actually restored, so a caller cannot
/// invent permits by releasing more than it took.
struct SemState {
    permits: usize,
    taken: usize,
}

/// A counting semaphore: at most `permits` permits loaned at once.
pub struct Semaphore {
    inner: Arc<Mutex<SemState>>,
    changed: Arc<Condvar>,
}

impl Semaphore {
    /// A semaphore with `permits` permits.
    pub fn new(permits: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SemState { permits, taken: 0 })),
            changed: Arc::new(Condvar::new()),
        }
    }

    /// `Semaphore.take(n)`: block until `n` permits are free, then take all
    /// of them. A take of zero returns at once, and a take above the pool
    /// never returns.
    pub fn take(&self, permits: usize) -> usize {
        let mut state = self.inner.lock().unwrap();
        // `resize` may shrink the pool below what is on loan, so the free
        // count can be negative in all but type. Saturating keeps a take
        // parked until a release catches up instead of panicking.
        while state.permits.saturating_sub(state.taken) < permits {
            state = self.changed.wait(state).unwrap();
        }
        state.taken += permits;
        permits
    }

    /// `Semaphore.takeIfAvailable(n)`: take `n` when they are free, else
    /// report false without waiting or taking a part.
    pub fn take_if_available(&self, permits: usize) -> bool {
        let mut state = self.inner.lock().unwrap();
        if state.permits.saturating_sub(state.taken) < permits {
            return false;
        }
        state.taken += permits;
        true
    }

    /// `Semaphore.release(n)`: restore up to `n` permits and answer how many
    /// were actually out on loan, which is Effect 4's count.
    pub fn release(&self, permits: usize) -> usize {
        let mut state = self.inner.lock().unwrap();
        let restored = permits.min(state.taken);
        state.taken -= restored;
        drop(state);
        self.changed.notify_all();
        restored
    }

    /// `Semaphore.releaseAll`: restore every loaned permit and answer the
    /// count.
    pub fn release_all(&self) -> usize {
        let mut state = self.inner.lock().unwrap();
        let restored = state.taken;
        state.taken = 0;
        drop(state);
        self.changed.notify_all();
        restored
    }

    /// `Semaphore.resize(n)`: change the pool size. Shrinking below what is
    /// loaned leaves `available` at zero until the loans come back.
    pub fn resize(&self, permits: usize) {
        self.inner.lock().unwrap().permits = permits;
        self.changed.notify_all();
    }

    /// Permits currently free.
    pub fn available(&self) -> usize {
        let state = self.inner.lock().unwrap();
        state.permits.saturating_sub(state.taken)
    }

    /// The pool size (`Semaphore.size`).
    pub fn size(&self) -> usize {
        self.inner.lock().unwrap().permits
    }

    /// Run `f` while holding `permits`.
    pub fn with_permits<R>(&self, permits: usize, f: impl FnOnce() -> R) -> R {
        self.take(permits);
        let _held = HeldPermits {
            semaphore: self,
            permits,
        };
        f()
    }

    /// Run `f` while holding `permits`, or answer `None` without running it
    /// when they are not free (`withPermitsIfAvailable`).
    pub fn with_permits_if_available<R>(&self, permits: usize, f: impl FnOnce() -> R) -> Option<R> {
        if !self.take_if_available(permits) {
            return None;
        }
        let _held = HeldPermits {
            semaphore: self,
            permits,
        };
        Some(f())
    }

    /// A take that observes an external interruption: waits on the changed
    /// latch in the cooperative quantum and gives up holding nothing once
    /// `interrupted` turns true. This is the crateable stand-in for a fiber
    /// interrupt landing on a parked `take`.
    pub fn take_interruptible(
        &self,
        permits: usize,
        interrupted: &dyn Fn() -> bool,
    ) -> Option<usize> {
        let mut state = self.inner.lock().unwrap();
        loop {
            if state.permits.saturating_sub(state.taken) >= permits {
                state.taken += permits;
                return Some(permits);
            }
            if interrupted() {
                return None;
            }
            let hook = crate::fiber::subscribe_current_interrupt({
                let inner = Arc::clone(&self.inner);
                let changed = Arc::clone(&self.changed);
                move || {
                    let _guard = inner.lock().unwrap();
                    changed.notify_all();
                }
            });
            if interrupted() {
                return None;
            }
            state = if hook.is_some() {
                self.changed.wait(state).unwrap()
            } else {
                self.changed
                    .wait_timeout(state, crate::COOPERATIVE_QUANTUM)
                    .unwrap()
                    .0
            };
        }
    }

    /// The interruption-aware `withPermits`: an interrupted wait takes
    /// nothing and does not run `f`, so no permit leaks out of a take that
    /// never finished.
    pub fn with_permits_interruptible<R>(
        &self,
        permits: usize,
        interrupted: &dyn Fn() -> bool,
        f: impl FnOnce() -> R,
    ) -> Option<R> {
        self.take_interruptible(permits, interrupted)?;
        let _held = HeldPermits {
            semaphore: self,
            permits,
        };
        Some(f())
    }
}

/// Restores a semaphore's permits when it leaves scope, so a panic inside the
/// guarded body cannot leak them (the same guard the partitioned semaphore
/// already uses).
struct HeldPermits<'a> {
    semaphore: &'a Semaphore,
    permits: usize,
}

impl Drop for HeldPermits<'_> {
    fn drop(&mut self) {
        self.semaphore.release(self.permits);
    }
}

/// Effect 4's `PartitionedSemaphore` (`PartitionedSemaphore.ts`): ONE shared
/// pool of `capacity` permits with parked takes grouped by partition key. A
/// take that cannot be met in full absorbs whatever is free and parks its
/// shortfall under its key; each released permit goes to the next key holding
/// a waiter, in round-robin order, so one busy key cannot monopolise the pool.
///
/// Effect 4 resumes waiters through fiber callbacks; here a parked take is a
/// thread on the pool's condvar. The `interruptible` variants subscribe to the
/// running fiber's interrupt notification, so the request wakes the condvar;
/// a thread outside a fiber keeps the bounded wait. An interrupted take hands
/// its partial grant back.
///
/// ```
/// use elm_effect::PartitionedSemaphore;
/// let sem = PartitionedSemaphore::new(2);
/// sem.with_permits("a", 2, || {});
/// assert_eq!(sem.available(), 2);
/// assert!(sem.try_take(2));
/// assert!(!sem.try_take(1));
/// sem.release(2);
/// assert_eq!(sem.available(), 2);
/// ```
pub struct PartitionedSemaphore<K> {
    inner: Arc<PoolInner<K>>,
}

/// Cloning a handle shares the pool, like Effect 4's value being the pool.
impl<K> Clone for PartitionedSemaphore<K> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct PoolInner<K> {
    pool: Mutex<PermitPool<K>>,
    changed: Arc<Condvar>,
}

/// Fallback wait for a parked take on a thread with no fiber to subscribe to.
const PARK_QUANTUM: Duration = crate::COOPERATIVE_QUANTUM;

struct PermitPool<K> {
    capacity: usize,
    /// Permits free right now.
    available: usize,
    /// Sum of the shortfalls parked takes still wait for.
    waiting: usize,
    /// Keys holding waiters, in the order each key's first waiter arrived.
    order: Vec<K>,
    /// Index in `order` that receives the next released permit.
    cursor: usize,
    queues: HashMap<K, VecDeque<u64>>,
    /// Parked take id -> permits still needed. Missing means satisfied.
    needs: HashMap<u64, usize>,
    next_id: u64,
}

impl<K: Eq + Hash + Clone> PermitPool<K> {
    fn enqueue(&mut self, key: K, need: usize) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.needs.insert(id, need);
        match self.queues.get_mut(&key) {
            Some(queue) => queue.push_back(id),
            None => {
                self.order.push(key.clone());
                let mut queue = VecDeque::new();
                queue.push_back(id);
                self.queues.insert(key, queue);
            }
        }
        id
    }

    /// Hand one released permit to the next key in rotation. `false` when
    /// nobody is parked, so the caller can grow `available` instead.
    fn grant_one(&mut self) -> bool {
        if self.waiting == 0 || self.order.is_empty() {
            return false;
        }
        if self.cursor >= self.order.len() {
            self.cursor = 0;
        }
        let key = self.order[self.cursor].clone();
        let id = {
            let queue = self
                .queues
                .get_mut(&key)
                .expect("every ordered key keeps its queue");
            *queue.front().expect("every ordered key keeps a waiter")
        };
        let need = self
            .needs
            .get_mut(&id)
            .expect("a queued take keeps its remaining need");
        *need -= 1;
        self.waiting -= 1;
        // A partly satisfied waiter keeps its place at the head of its key's
        // queue; only a satisfied one leaves, which is how Effect 4 mutates the
        // waiter entry and resumes it on the third permit.
        let satisfied = *need == 0;
        let queue_now_empty = if satisfied {
            // Absence from `needs` is how the parked take learns it holds
            // everything it asked for.
            self.needs.remove(&id);
            let queue = self
                .queues
                .get_mut(&key)
                .expect("every ordered key keeps its queue");
            queue.pop_front();
            queue.is_empty()
        } else {
            false
        };
        if queue_now_empty {
            self.queues.remove(&key);
            self.order.remove(self.cursor);
            // The next key slid into this slot, so the cursor stays put: that
            // is the rotation.
        } else {
            self.cursor += 1;
        }
        true
    }

    /// Distribution half of `release`: waiters first, then the free pool.
    fn distribute(&mut self, mut permits: usize) -> usize {
        while permits > 0 {
            if self.waiting == 0 {
                self.available = (self.available + permits).min(self.capacity);
                return self.available;
            }
            if !self.grant_one() {
                return self.available;
            }
            permits -= 1;
        }
        self.available
    }

    /// Drop an interrupted take's charge and answer how much it still needs,
    /// so the caller can hand back everything it had already absorbed.
    fn forget(&mut self, key: &K, id: u64) -> Option<usize> {
        let need = self.needs.remove(&id)?;
        self.waiting -= need;
        let empty = match self.queues.get_mut(key) {
            Some(queue) => {
                queue.retain(|queued| *queued != id);
                queue.is_empty()
            }
            None => false,
        };
        if empty {
            self.queues.remove(key);
            if let Some(pos) = self.order.iter().position(|ordered| ordered == key) {
                self.order.remove(pos);
                if pos < self.cursor {
                    self.cursor -= 1;
                }
            }
        }
        Some(need)
    }
}

/// Releases a with-permits body's permits on the way out, including an unwind
/// (Effect 4 gets this from `Effect.ensuring`).
struct Hold<'a, K: Eq + Hash + Clone + Send + Sync + 'static> {
    sem: &'a PartitionedSemaphore<K>,
    permits: usize,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> Drop for Hold<'_, K> {
    fn drop(&mut self) {
        self.sem.release(self.permits);
    }
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> PartitionedSemaphore<K> {
    /// A shared pool of `capacity` permits partitioned by key.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                pool: Mutex::new(PermitPool {
                    capacity,
                    available: capacity,
                    waiting: 0,
                    order: Vec::new(),
                    cursor: 0,
                    queues: HashMap::new(),
                    needs: HashMap::new(),
                    next_id: 0,
                }),
                changed: Arc::new(Condvar::new()),
            }),
        }
    }

    /// The pool size (`PartitionedSemaphore.capacity`).
    pub fn capacity(&self) -> usize {
        self.inner.pool.lock().unwrap().capacity
    }

    /// Permits free right now (`PartitionedSemaphore.available`), across every
    /// key.
    pub fn available(&self) -> usize {
        self.inner.pool.lock().unwrap().available
    }

    /// Permits parked takes are still short of: the pool's outstanding charge.
    pub fn waiting(&self) -> usize {
        self.inner.pool.lock().unwrap().waiting
    }

    /// `PartitionedSemaphore.take`: block until `permits` arrive for `key`.
    /// A take above the pool never returns, which is Effect 4's `Effect.never`.
    pub fn take(&self, key: K, permits: usize) {
        let _ = self.take_interruptible(key, permits, &|| false);
    }

    /// A take that observes an external interrupt: it hands back whatever it
    /// had absorbed and answers `None`, holding nothing.
    ///
    /// `interrupted` runs on the thread holding the pool lock, so it must not
    /// call back into this semaphore (`available`, `release`, another take) or
    /// it will deadlock on the non-reentrant mutex.
    pub fn take_interruptible(
        &self,
        key: K,
        permits: usize,
        interrupted: &dyn Fn() -> bool,
    ) -> Option<()> {
        if permits == 0 {
            return Some(());
        }
        let mut pool = self.inner.pool.lock().unwrap();
        if permits > pool.capacity {
            // No release can ever satisfy a need larger than the whole pool,
            // and Effect 4 never grants such a take a partial permit either.
            loop {
                if interrupted() {
                    return None;
                }
                let hook = crate::fiber::subscribe_current_interrupt({
                    let inner = Arc::clone(&self.inner);
                    move || {
                        let _guard = inner.pool.lock().unwrap();
                        inner.changed.notify_all();
                    }
                });
                if interrupted() {
                    continue;
                }
                pool = if hook.is_some() {
                    self.inner.changed.wait(pool).unwrap()
                } else {
                    self.inner
                        .changed
                        .wait_timeout(pool, PARK_QUANTUM)
                        .unwrap()
                        .0
                };
            }
        }
        if pool.available >= permits {
            pool.available -= permits;
            return Some(());
        }
        let need = permits - pool.available;
        pool.available = 0;
        pool.waiting += need;
        let id = pool.enqueue(key.clone(), need);
        loop {
            if !pool.needs.contains_key(&id) {
                return Some(());
            }
            if interrupted() {
                let remaining = pool
                    .forget(&key, id)
                    .expect("a parked take is dropped once");
                let absorbed = permits - remaining;
                pool.distribute(absorbed);
                self.inner.changed.notify_all();
                return None;
            }
            let hook = crate::fiber::subscribe_current_interrupt({
                let inner = Arc::clone(&self.inner);
                move || {
                    let _guard = inner.pool.lock().unwrap();
                    inner.changed.notify_all();
                }
            });
            if interrupted() {
                continue;
            }
            pool = if hook.is_some() {
                self.inner.changed.wait(pool).unwrap()
            } else {
                self.inner
                    .changed
                    .wait_timeout(pool, PARK_QUANTUM)
                    .unwrap()
                    .0
            };
        }
    }

    /// `withPermitsIfAvailable`: take all `permits` or nothing at all. Never
    /// waits.
    pub fn try_take(&self, permits: usize) -> bool {
        let mut pool = self.inner.pool.lock().unwrap();
        if permits > pool.capacity || permits > pool.available {
            return false;
        }
        pool.available -= permits;
        true
    }

    /// `PartitionedSemaphore.release`: waiters first in round-robin order, the
    /// remainder back into the pool. Answers the pool's free count.
    pub fn release(&self, permits: usize) -> usize {
        let total = self.inner.pool.lock().unwrap().distribute(permits);
        self.inner.changed.notify_all();
        total
    }

    /// `withPermits`: run `f` while `key` holds `permits`, and give them back
    /// even when `f` unwinds.
    pub fn with_permits<R>(&self, key: K, permits: usize, f: impl FnOnce() -> R) -> R {
        self.take(key, permits);
        let _held = Hold { sem: self, permits };
        f()
    }

    /// `withPermit`: the single-permit `withPermits`.
    pub fn with_permit<R>(&self, key: K, f: impl FnOnce() -> R) -> R {
        self.with_permits(key, 1, f)
    }

    /// `withPermitsIfAvailable`: run `f` only when the pool can give it every
    /// permit at once, and give them back afterwards.
    pub fn with_permits_if_available<R>(&self, permits: usize, f: impl FnOnce() -> R) -> Option<R> {
        if !self.try_take(permits) {
            return None;
        }
        let _held = Hold { sem: self, permits };
        Some(f())
    }

    /// `withPermits` under an external interrupt: an interrupted wait takes
    /// nothing and does not run `f`, so no permit leaks out of a take that
    /// never finished. The same lock rule as [`take_interruptible`] applies to
    /// `interrupted`.
    pub fn with_permits_interruptible<R>(
        &self,
        key: K,
        permits: usize,
        interrupted: &dyn Fn() -> bool,
        f: impl FnOnce() -> R,
    ) -> Option<R> {
        self.take_interruptible(key, permits, interrupted)?;
        if interrupted() {
            // The take was satisfied but the interrupt landed before the body:
            // Effect 4's `ensuring` still gives the permits back.
            self.release(permits);
            return None;
        }
        let _held = Hold { sem: self, permits };
        Some(f())
    }
}
