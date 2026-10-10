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

use std::collections::VecDeque;
use crate::Exit;
use std::sync::{Condvar, Mutex};

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
        let (previous, slot) = (std::sync::Arc::clone(&previous), std::sync::Arc::clone(&slot));
        setup.flat_map(move |_| {
            let effect = slot.lock().unwrap().take();
            let previous = std::sync::Arc::clone(&previous);
            match effect {
                Some(effect) => effect.flat_map(move |_| {
                    Effect::succeed(previous.lock().unwrap().take().expect("recorded in setup"))
                }),
                None => Effect::succeed(previous.lock().unwrap().take().expect("recorded in setup")),
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

/// A bounded multi-producer multi-consumer queue. Senders block when full,
/// receivers block when empty: backpressure by construction.
pub struct Queue<A> {
    inner: Mutex<VecDeque<A>>,
    capacity: usize,
    not_full: Condvar,
    not_empty: Condvar,
    shutdown: Mutex<bool>,
}

impl<A: Send> Queue<A> {
    /// A queue holding at most `capacity` items.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity,
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
            shutdown: Mutex::new(false),
        }
    }

    /// The bound.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Push, blocking while full.
    pub fn offer(&self, value: A) {
        let mut guard = self.inner.lock().unwrap();
        while guard.len() >= self.capacity {
            guard = self.not_full.wait(guard).unwrap();
        }
        guard.push_back(value);
        self.not_empty.notify_one();
    }

    /// Pop, blocking while empty.
    pub fn poll(&self) -> A {
        let mut guard = self.inner.lock().unwrap();
        while guard.is_empty() {
            guard = self.not_empty.wait(guard).unwrap();
        }
        let value = guard.pop_front().expect("checked");
        self.not_full.notify_one();
        value
    }

    /// Push without blocking. False when full.
    pub fn try_offer(&self, value: A) -> bool {
        if *self.shutdown.lock().unwrap() {
            return false;
        }
        let mut guard = self.inner.lock().unwrap();
        if guard.len() >= self.capacity {
            return false;
        }
        guard.push_back(value);
        self.not_empty.notify_one();
        true
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

    /// Remove everything buffered.
    pub fn take_all(&self) -> Vec<A> {
        let mut guard = self.inner.lock().unwrap();
        let out: Vec<A> = guard.drain(..).collect();
        self.not_full.notify_all();
        out
    }

    /// Drop everything buffered; returns the count dropped.
    pub fn clear(&self) -> usize {
        self.take_all().len()
    }

    /// Freeze offers; buffered items still drain. Wakes blocked takers
    /// so they observe the drain instead of sleeping.
    pub fn shutdown(&self) {
        *self.shutdown.lock().unwrap() = true;
        self.not_empty.notify_all();
    }

    /// True after [`Self::shutdown`].
    pub fn is_shutdown(&self) -> bool {
        *self.shutdown.lock().unwrap()
    }

    /// Items currently queued.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// No items queued.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
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
    pub fn acquire_interruptible(
        &self,
        partition: usize,
        interrupted: &dyn Fn() -> bool,
    ) -> bool {
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
