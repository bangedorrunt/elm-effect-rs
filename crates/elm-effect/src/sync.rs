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
pub struct Deferred<A> {
    inner: Mutex<Option<A>>,
    ready: Condvar,
}

impl<A: Clone + Send> Deferred<A> {
    /// An empty handoff.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    /// Complete with `value`. Returns false if already completed; the first
    /// value wins.
    pub fn complete(&self, value: A) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return false;
        }
        *guard = Some(value);
        self.ready.notify_all();
        true
    }

    /// Block until completed, then return the value.
    pub fn wait(&self) -> A {
        let mut guard = self.inner.lock().unwrap();
        while guard.is_none() {
            guard = self.ready.wait(guard).unwrap();
        }
        guard.clone().expect("checked")
    }
}

impl<A: Clone + Send> Default for Deferred<A> {
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
}

impl<A: Send> Queue<A> {
    /// A queue holding at most `capacity` items.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity,
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
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
