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
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::Effect;

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
    /// leaves the cell untouched (and, matching the pin, bumps no version).
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
    /// leaves the cell unchanged. The result is always returned; the pin
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

    /// `Ref.set`: store `value`, unconditionally. Verified at the pin
    /// (`Ref.ts:236`); the port was missing it, so the row that asserts it
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
        self.inner.lock().unwrap().clone()
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
            let mut guard = inner.lock().unwrap();
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
                let guard = inner.lock().unwrap();
                let current = guard.clone();
                *slot.lock().unwrap() = Some(f(&current));
                *previous.lock().unwrap() = Some(current);
            }
        });
        let (inner, previous, slot) = (
            std::sync::Arc::clone(&inner),
            std::sync::Arc::clone(&previous),
            std::sync::Arc::clone(&slot),
        );
        setup.flat_map(move |_| {
            let effect = slot.lock().unwrap().take().expect("recorded in setup");
            effect.flat_map(move |next| {
                if let Some(next) = next {
                    *inner.lock().unwrap() = next;
                }
                Effect::succeed(previous.lock().unwrap().take().expect("recorded in setup"))
            })
        })
    }

    /// `SynchronizedRef.getAndUpdateSome`: the pure conditional variant —
    /// apply `f` when it returns `Some`, and return the PREVIOUS value
    /// (`SynchronizedRef.ts:224`).
    pub fn get_and_update_some(&self, f: impl Fn(&A) -> Option<A>) -> A {
        let mut guard = self.inner.lock().unwrap();
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
                let guard = inner.lock().unwrap();
                *slot.lock().unwrap() = Some(f(&guard));
            }
        });
        let (inner, slot) = (std::sync::Arc::clone(&inner), std::sync::Arc::clone(&slot));
        setup.flat_map(move |_| {
            let effect = slot.lock().unwrap().take().expect("recorded in setup");
            let inner = std::sync::Arc::clone(&inner);
            effect.map(move |new_value| {
                *inner.lock().unwrap() = new_value.clone();
                new_value
            })
        })
    }

    /// `SynchronizedRef.getAndUpdateEffect`: store `f(current)` and return
    /// the PREVIOUS value, running `f`'s effect once afterwards.
    ///
    /// Distinct from [`update_effect`], which returns the effect's result:
    /// the pin's getAndUpdateEffect returns what was in the cell before the
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
                let mut guard = inner.lock().unwrap();
                let (next, effect) = f(&*guard);
                *previous.lock().unwrap() = Some(std::mem::replace(&mut *guard, next));
                *slot.lock().unwrap() = Some(effect);
            }
        });
        let (previous, slot) = (
            std::sync::Arc::clone(&previous),
            std::sync::Arc::clone(&slot),
        );
        setup.flat_map(move |_| {
            let effect = slot.lock().unwrap().take();
            let previous = std::sync::Arc::clone(&previous);
            match effect {
                Some(effect) => effect.flat_map(move |_| {
                    Effect::succeed(previous.lock().unwrap().take().expect("recorded in setup"))
                }),
                None => {
                    Effect::succeed(previous.lock().unwrap().take().expect("recorded in setup"))
                }
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
                let mut guard = inner.lock().unwrap();
                let (next, effect) = f(&*guard);
                *guard = next;
                *slot.lock().unwrap() = Some(effect);
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
/// Carries an [`Exit`], not a bare value, because the pin completes a
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
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return false;
        }
        *guard = Some(Exit::Success(f()));
        self.ready.notify_all();
        true
    }

    /// Complete with any outcome, including a failure or an interrupt.
    pub fn complete_exit(&self, outcome: Exit<A, E>) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return false;
        }
        *guard = Some(outcome);
        self.ready.notify_all();
        true
    }

    /// The recorded outcome, or `None` while still empty.
    pub fn poll(&self) -> Option<Exit<A, E>> {
        self.inner.lock().unwrap().clone()
    }

    /// True once any outcome has been recorded.
    pub fn is_done(&self) -> bool {
        self.inner.lock().unwrap().is_some()
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
        let mut guard = self.inner.lock().unwrap();
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
/// Two pin rules need state beyond the buffer. Offers that are parked
/// when `end`/`fail` arrive keep going: the terminal flip is deferred
/// until they settle, which is why `await` waits and why the offers a
/// fail interrupts still land. A zero-capacity queue rendezvouses through
/// a reservation: a taker claims the next item, an offerer may only push
/// while one is outstanding, so a message is never enqueued twice.
pub struct Queue<A> {
    inner: Mutex<VecDeque<A>>,
    capacity: usize,
    strategy: Strategy,
    state: Mutex<QueueState>,
    /// The terminal state waiting for parked offers to settle.
    pending_close: Mutex<Option<QueueState>>,
    /// Offers currently parked on a full queue.
    in_flight: Mutex<usize>,
    /// Outstanding taker reservations for the zero-capacity rendezvous.
    reservations: Mutex<usize>,
    /// Releases scheduled by `offerUnsafe` and performed by `flushUnsafe`.
    scheduled_releases: Mutex<usize>,
    not_full: Condvar,
    not_empty: Condvar,
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
            inner: Mutex::new(VecDeque::new()),
            capacity,
            strategy,
            state: Mutex::new(QueueState::Open),
            pending_close: Mutex::new(None),
            in_flight: Mutex::new(0),
            reservations: Mutex::new(0),
            scheduled_releases: Mutex::new(0),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
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

    /// `Queue.flush`: the effectful flush. The port's queue is
    /// synchronous, so this is `flush_unsafe`.
    pub fn flush(&self) {
        self.flush_unsafe();
    }

    /// Whether an offer must wait: the buffer is at capacity and the
    /// zero-capacity rendezvous holds no reservation for this push.
    fn is_full(&self, guard: &VecDeque<A>) -> bool {
        if guard.len() < self.capacity {
            return false;
        }
        // Zero capacity: a reservation is a permit for exactly one item.
        !(self.capacity == 0 && *self.reservations.lock().unwrap() > 0)
    }

    /// A taker claims the next zero-capacity message, waking offerers.
    /// Returns false when another taker already holds the claim.
    fn reserve(&self) -> bool {
        let mut reservations = self.reservations.lock().unwrap();
        if *reservations == 0 {
            *reservations = 1;
            drop(reservations);
            self.not_full.notify_all();
            true
        } else {
            false
        }
    }

    /// Give up a reservation (a rendezvous that produced nothing).
    fn release_reservation(&self) {
        let mut reservations = self.reservations.lock().unwrap();
        if *reservations > 0 {
            *reservations -= 1;
        }
        drop(reservations);
        self.not_full.notify_all();
    }

    /// The one place an item enters the buffer: a zero-capacity push
    /// consumes the reservation that permitted it.
    fn push_item(&self, guard: &mut VecDeque<A>, value: A) {
        guard.push_back(value);
        if self.capacity == 0 {
            let mut reservations = self.reservations.lock().unwrap();
            if *reservations > 0 {
                *reservations -= 1;
            }
        }
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
    /// `Ok(Accepted)`. The port's interruptible-wait pattern, the one the
    /// semaphore uses.
    pub fn offer_interruptible(
        &self,
        value: A,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<OfferOutcome<A>, ()> {
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
            // the pin's offer reports false immediately.
            if self.strategy == Strategy::Dropping && guard.len() >= self.capacity {
                return Ok(OfferOutcome::Refused(value));
            }
            if !parked {
                parked = true;
                *self.in_flight.lock().unwrap() += 1;
            }
            let (next, _) = self
                .not_full
                .wait_timeout(guard, std::time::Duration::from_millis(1))
                .unwrap();
            guard = next;
        }
    }

    /// Offer a batch, returning whatever was not accepted. Once the
    /// queue closes, or a full dropping queue refuses the rest, the
    /// remainder comes back untouched.
    pub fn offer_all(&self, values: impl IntoIterator<Item = A>) -> Vec<A> {
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
    pub fn offer_all_interruptible(
        &self,
        values: impl IntoIterator<Item = A>,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<Vec<A>, ()> {
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
        let mut reserved = false;
        loop {
            if let Some(value) = guard.pop_front() {
                self.not_full.notify_one();
                return Ok(value);
            }
            match &*self.state.lock().unwrap() {
                QueueState::Open => {}
                QueueState::Ended => return Err(QueueError::Done),
                QueueState::Failed(message) => {
                    return Err(QueueError::Failed(message.clone()));
                }
                QueueState::Interrupted => return Err(QueueError::Interrupted),
            }
            if !reserved && self.capacity == 0 {
                reserved = self.reserve();
            }
            guard = self.not_empty.wait(guard).unwrap();
        }
    }

    /// Pop while watching `interrupted`.
    pub fn take_interruptible(
        &self,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<Result<A, QueueError>, ()> {
        let mut guard = self.inner.lock().unwrap();
        let mut reserved = false;
        loop {
            if let Some(value) = guard.pop_front() {
                self.not_full.notify_one();
                return Ok(Ok(value));
            }
            match &*self.state.lock().unwrap() {
                QueueState::Open => {}
                QueueState::Ended => return Ok(Err(QueueError::Done)),
                QueueState::Failed(message) => {
                    return Ok(Err(QueueError::Failed(message.clone())));
                }
                QueueState::Interrupted => return Ok(Err(QueueError::Interrupted)),
            }
            if interrupted() {
                return Err(());
            }
            if !reserved && self.capacity == 0 {
                reserved = self.reserve();
            }
            let (next, _) = self
                .not_empty
                .wait_timeout(guard, std::time::Duration::from_millis(1))
                .unwrap();
            guard = next;
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
    /// pin's `Count.normalize` — NaN and non-positive become 0, a positive
    /// value floors — so a zero `min` takes nothing without waiting, and an
    /// inverted pair swaps (the pin's row :379).
    pub fn take_between(&self, min: usize, max: usize) -> Result<Vec<A>, QueueError> {
        let (mut min, mut max) = (min, max);
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        let mut out = Vec::new();
        loop {
            {
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
            }
            if out.len() >= min {
                return Ok(out);
            }
            match &*self.state.lock().unwrap() {
                QueueState::Open => {}
                QueueState::Ended => {
                    return if out.is_empty() {
                        Err(QueueError::Done)
                    } else {
                        Ok(out)
                    };
                }
                QueueState::Failed(message) => {
                    return if out.is_empty() {
                        Err(QueueError::Failed(message.clone()))
                    } else {
                        Ok(out)
                    };
                }
                QueueState::Interrupted => {
                    return if out.is_empty() {
                        Err(QueueError::Interrupted)
                    } else {
                        Ok(out)
                    };
                }
            }
            let mut guard = self.inner.lock().unwrap();
            while guard.len() + out.len() < min {
                // A terminal state releases the wait even below the minimum:
                // the closing queue drains what the taker already holds.
                if !matches!(&*self.state.lock().unwrap(), QueueState::Open) {
                    break;
                }
                let (next, _) = self
                    .not_empty
                    .wait_timeout(guard, std::time::Duration::from_millis(1))
                    .unwrap();
                guard = next;
            }
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
    pub fn take_all_blocking(&self) -> Result<Vec<A>, QueueError> {
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
    /// close (`Queue.collect`: the port's offers are synchronous, so the
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

/// A counting semaphore: at most `permits` holders at once.
pub struct Semaphore {
    inner: Mutex<usize>,
    changed: Condvar,
}

impl Semaphore {
    /// A semaphore with `permits` permits.
    pub fn new(permits: usize) -> Self {
        Self {
            inner: Mutex::new(permits),
            changed: Condvar::new(),
        }
    }

    /// Take a permit, blocking until one is free.
    pub fn acquire(&self) {
        let mut guard = self.inner.lock().unwrap();
        while *guard == 0 {
            guard = self.changed.wait(guard).unwrap();
        }
        *guard -= 1;
    }

    /// Take a permit without blocking. False when none is free.
    pub fn try_acquire(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if *guard == 0 {
            return false;
        }
        *guard -= 1;
        true
    }

    /// Return a permit.
    pub fn release(&self) {
        *self.inner.lock().unwrap() += 1;
        self.changed.notify_one();
    }

    /// Permits currently free.
    pub fn available(&self) -> usize {
        *self.inner.lock().unwrap()
    }
}

/// A snapshot cell broadcasting every assignment to its subscribers
/// (`SubscriptionRef` shape): `set` updates the current value and pushes
/// a copy to every subscriber buffer; subscribers poll their own queue.
///
/// ```
/// use elm_effect::SubscriptionRef;
/// let watched = SubscriptionRef::new(0);
/// let sub = watched.subscribe();
/// watched.set(5);
/// assert_eq!(watched.get(), 5);
/// assert_eq!(watched.poll(sub), Some(0));
/// assert_eq!(watched.poll(sub), Some(5));
/// ```
pub struct SubscriptionRef<A> {
    current: Mutex<A>,
    sinks: Mutex<Vec<std::collections::VecDeque<A>>>,
}

impl<A: Clone + Send> SubscriptionRef<A> {
    /// A cell holding `initial` with no subscribers.
    pub fn new(initial: A) -> Self {
        Self {
            current: Mutex::new(initial),
            sinks: Mutex::new(Vec::new()),
        }
    }

    /// The current value.
    pub fn get(&self) -> A {
        self.current.lock().unwrap().clone()
    }

    /// Assign and broadcast to every subscriber buffer.
    pub fn set(&self, value: A) {
        *self.current.lock().unwrap() = value.clone();
        for sink in self.sinks.lock().unwrap().iter_mut() {
            sink.push_back(value.clone());
        }
    }

    /// Subscribe: returns an index seeded with the current value.
    pub fn subscribe(&self) -> usize {
        let mut sinks = self.sinks.lock().unwrap();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(self.current.lock().unwrap().clone());
        sinks.push(queue);
        sinks.len() - 1
    }

    /// Pop the subscriber's next value, if any.
    pub fn poll(&self, subscriber: usize) -> Option<A> {
        self.sinks.lock().unwrap().get_mut(subscriber)?.pop_front()
    }
}

/// Independent permit pools behind one handle (`PartitionedSemaphore`
/// shape): each partition counts down separately; `acquire` blocks on
/// its partition's condvar while `try_acquire` never does.
///
/// ```
/// use elm_effect::PartitionedSemaphore;
/// let sem = PartitionedSemaphore::new(2, 1);
/// assert!(sem.try_acquire(0));
/// assert!(!sem.try_acquire(0));
/// assert!(sem.try_acquire(1));
/// sem.release(0);
/// assert_eq!(sem.available(0), 1);
/// ```
pub struct PartitionedSemaphore {
    partitions: Mutex<Vec<usize>>,
    changed: Condvar,
}

impl PartitionedSemaphore {
    /// `partitions` pools with `permits_each` permits apiece.
    pub fn new(partitions: usize, permits_each: usize) -> Self {
        Self {
            partitions: Mutex::new(vec![permits_each; partitions]),
            changed: Condvar::new(),
        }
    }

    /// Take one permit from `partition`, blocking until free.
    pub fn acquire(&self, partition: usize) {
        let mut pools = self.partitions.lock().unwrap();
        while pools[partition] == 0 {
            pools = self.changed.wait(pools).unwrap();
        }
        pools[partition] -= 1;
    }

    /// Take one permit without blocking (`false` when empty or missing).
    pub fn try_acquire(&self, partition: usize) -> bool {
        match self.partitions.lock().unwrap().get_mut(partition) {
            Some(permits) if *permits > 0 => {
                *permits -= 1;
                true
            }
            _ => false,
        }
    }

    /// Blocking acquire that observes external interruption: waits on the
    /// changed latch in a 1ms quantum (the TestClock sleep precedent) and
    /// gives up, holding nothing, once `interrupted` turns true.
    pub fn acquire_interruptible(&self, partition: usize, interrupted: &dyn Fn() -> bool) -> bool {
        let mut pools = self.partitions.lock().unwrap();
        loop {
            if interrupted() {
                return false;
            }
            if pools[partition] > 0 {
                pools[partition] -= 1;
                return true;
            }
            let (guard, _waited) = self
                .changed
                .wait_timeout(pools, std::time::Duration::from_millis(1))
                .unwrap();
            pools = guard;
            if interrupted() {
                return false;
            }
        }
    }

    /// Scoped multi-permit take with interruption restore (`withPermits`
    /// shape): takes `count` permits, runs `f`, releases everything after.
    /// An interruption before the take completes releases the partial
    /// take and yields `None`; an interruption after the take but before
    /// `f` runs also releases and yields `None`. A resumed waiter never
    /// steals another partition's permits: takes are per-partition.
    pub fn with_permits_interruptible<R>(
        &self,
        partition: usize,
        count: usize,
        interrupted: &dyn Fn() -> bool,
        f: impl FnOnce() -> R,
    ) -> Option<R> {
        let mut taken = 0;
        while taken < count {
            if interrupted() {
                self.release_n(partition, taken);
                return None;
            }
            if !self.acquire_interruptible(partition, interrupted) {
                self.release_n(partition, taken);
                return None;
            }
            taken += 1;
        }
        if interrupted() {
            self.release_n(partition, taken);
            return None;
        }
        let out = f();
        self.release_n(partition, taken);
        Some(out)
    }

    fn release_n(&self, partition: usize, mut count: usize) {
        while count > 0 {
            self.release(partition);
            count -= 1;
        }
    }

    /// Return one permit to `partition`.
    pub fn release(&self, partition: usize) {
        if let Some(permits) = self.partitions.lock().unwrap().get_mut(partition) {
            *permits += 1;
        }
        self.changed.notify_all();
    }

    /// Permits currently free in `partition` (0 when missing).
    pub fn available(&self, partition: usize) -> usize {
        self.partitions
            .lock()
            .unwrap()
            .get(partition)
            .copied()
            .unwrap_or(0)
    }
}
