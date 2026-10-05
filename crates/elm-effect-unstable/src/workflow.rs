//! Workflow module (`workflow/Activity`, `DurableClock`, `DurableDeferred`,
//! `DurableQueue` at the pin, plus compensation).
//!
//! - [`Activity`]: idempotency-keyed units. [`execute`](Activity::execute)
//!   runs the implementation once per key and replays the stored result on
//!   retry ("activity results are stored and replayed").
//! - [`DurableClock`]: adjustable now plus interruptible waits.
//! - [`DurableDeferred`]: one-shot cell with blocking waiters.
//! - [`DurableQueue`]: FIFO with blocking takes and shutdown.
//! - [`compensate`](compensate): compensation runs on failure and on cancel
//!   ONLY, in reverse registration order; success runs none.
//!
//! Residuals: persistence across restarts (stores are in-memory here),
//! distributed worker dispatch, and clock-drift reconciliation.

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Activity implementation: unit of work returning a result.
type RunFn<T, E> = Box<dyn Fn() -> Result<T, E> + Send + Sync>;

/// Idempotency-keyed unit of work.
pub struct Activity<T, E> {
    /// Stable key: retries with the same key replay the stored result.
    pub key: String,
    run: RunFn<T, E>,
}

impl<T, E> Activity<T, E> {
    /// Build from a key and its implementation (`make`).
    pub fn make(key: &str, run: impl Fn() -> Result<T, E> + Send + Sync + 'static) -> Self {
        Self {
            key: key.to_owned(),
            run: Box::new(run),
        }
    }
}

/// Activity result store: first execution wins, replays are free.
#[derive(Default)]
pub struct ActivityStore<T, E> {
    results: Mutex<HashMap<String, Result<T, E>>>,
    runs: Mutex<HashMap<String, usize>>,
}

impl<T: Clone, E: Clone> ActivityStore<T, E> {
    /// Empty store.
    pub fn new() -> Self {
        Self {
            results: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
        }
    }

    /// Execute once per key; replay the stored result afterwards.
    pub fn execute(&self, activity: &Activity<T, E>) -> Result<T, E> {
        {
            let results = self.results.lock().expect("store");
            if let Some(cached) = results.get(&activity.key) {
                return cached.clone();
            }
        }
        let outcome = (activity.run)();
        *self
            .runs
            .lock()
            .expect("store")
            .entry(activity.key.clone())
            .or_insert(0) += 1;
        self.results
            .lock()
            .expect("store")
            .insert(activity.key.clone(), outcome.clone());
        outcome
    }

    /// Implementation runs so far for `key` (test aid: proves replay).
    pub fn runs(&self, key: &str) -> usize {
        self.runs
            .lock()
            .expect("store")
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// Retry up to `attempts` times, replaying through the store.
    pub fn retry(&self, activity: &Activity<T, E>, attempts: usize) -> Result<T, E> {
        let mut attempts = attempts.max(1);
        loop {
            match self.execute(activity) {
                ok @ Ok(_) => return ok,
                Err(e) => {
                    attempts -= 1;
                    if attempts == 0 {
                        return Err(e);
                    }
                    self.results.lock().expect("store").remove(&activity.key);
                }
            }
        }
    }
}

/// Adjustable clock with interruptible waits.
#[derive(Debug)]
pub struct DurableClock {
    now_ms: Mutex<u64>,
}

impl DurableClock {
    /// Clock starting at `now_ms`.
    pub fn new(now_ms: u64) -> Self {
        Self {
            now_ms: Mutex::new(now_ms),
        }
    }

    /// Current time in ms.
    pub fn now_ms(&self) -> u64 {
        *self.now_ms.lock().expect("clock")
    }

    /// Advance virtual time.
    pub fn advance(&self, delta_ms: u64) {
        *self.now_ms.lock().expect("clock") += delta_ms;
    }

    /// Block until `target_ms` or `interrupted()`. Returns `true` on time.
    pub fn wait_until(&self, target_ms: u64, interrupted: &dyn Fn() -> bool) -> bool {
        loop {
            if self.now_ms() >= target_ms {
                return true;
            }
            if interrupted() {
                return false;
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
}

/// One-shot cell with blocking waiters.
#[derive(Debug, Default)]
pub struct DurableDeferred<T> {
    state: Mutex<Option<T>>,
    changed: Condvar,
}

impl<T> DurableDeferred<T> {
    /// Empty cell.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    /// Complete once; later completions are ignored, returning `false`.
    pub fn complete(&self, value: T) -> bool {
        let mut guard = self.state.lock().expect("deferred");
        if guard.is_some() {
            return false;
        }
        *guard = Some(value);
        self.changed.notify_all();
        true
    }

    /// Block until completed or interrupted. Clones the value out.
    pub fn await_value(&self, interrupted: &dyn Fn() -> bool) -> Option<T>
    where
        T: Clone,
    {
        let mut guard = self.state.lock().expect("deferred");
        loop {
            if let Some(value) = guard.as_ref() {
                return Some(value.clone());
            }
            if interrupted() {
                return None;
            }
            let (g, _) = self
                .changed
                .wait_timeout(guard, Duration::from_millis(1))
                .expect("deferred");
            guard = g;
        }
    }

    /// `true` once completed.
    pub fn is_done(&self) -> bool {
        self.state.lock().expect("deferred").is_some()
    }
}

/// FIFO with blocking takes and shutdown.
#[derive(Debug)]
pub struct DurableQueue<T> {
    items: Mutex<VecDeque<T>>,
    changed: Condvar,
    shutdown: Mutex<bool>,
}

impl<T> DurableQueue<T> {
    /// Empty queue. (`Default` is not derived on purpose: `T` carries no
    /// `Default` bound anywhere in this module.)
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
            changed: Condvar::new(),
            shutdown: Mutex::new(false),
        }
    }

    /// Append one item.
    pub fn offer(&self, item: T) {
        self.items.lock().expect("queue").push_back(item);
        self.changed.notify_one();
    }

    /// Take one item, blocking until present, shutdown, or interrupted.
    /// `None` on shutdown/interrupt (a shutdown wakes every waiter).
    pub fn take(&self, interrupted: &dyn Fn() -> bool) -> Option<T> {
        let mut guard = self.items.lock().expect("queue");
        loop {
            if let Some(item) = guard.pop_front() {
                return Some(item);
            }
            if *self.shutdown.lock().expect("queue") || interrupted() {
                return None;
            }
            let (g, _) = self
                .changed
                .wait_timeout(guard, Duration::from_millis(1))
                .expect("queue");
            guard = g;
        }
    }

    /// Queued item count.
    pub fn len(&self) -> usize {
        self.items.lock().expect("queue").len()
    }

    /// `true` with no items.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Shut down: waiters wake with `None`, takes stay `None`.
    pub fn shutdown(&self) {
        *self.shutdown.lock().expect("queue") = true;
        self.changed.notify_all();
    }

    /// `true` after [`shutdown`](DurableQueue::shutdown).
    pub fn is_shutdown(&self) -> bool {
        *self.shutdown.lock().expect("queue")
    }
}

/// Compensation scope: register undos, run the body. Compensations fire on
/// failure and on cancel ONLY, in reverse order; success runs none.
pub struct CompScope {
    compensations: Vec<Box<dyn FnOnce() + Send>>,
}

impl CompScope {
    fn new() -> Self {
        Self {
            compensations: Vec::new(),
        }
    }

    /// Register one compensation (runs LIFO on failure/cancel).
    pub fn compensate(&mut self, undo: impl FnOnce() + Send + 'static) {
        self.compensations.push(Box::new(undo));
    }
}

/// Run `body` with a compensation scope. Success with a live probe runs
/// no compensation; failure runs them LIFO and keeps the error; a true
/// `interrupted()` probe runs them LIFO and maps through `on_cancel`.
/// Compensation fires on failure and on cancel ONLY.
pub fn compensate<T, E>(
    interrupted: &dyn Fn() -> bool,
    on_cancel: impl FnOnce() -> E,
    body: impl FnOnce(&mut CompScope) -> Result<T, E>,
) -> Result<T, E> {
    let mut scope = CompScope::new();
    match body(&mut scope) {
        Ok(value) => {
            if interrupted() {
                while let Some(undo) = scope.compensations.pop() {
                    undo();
                }
                Err(on_cancel())
            } else {
                Ok(value)
            }
        }
        Err(e) => {
            while let Some(undo) = scope.compensations.pop() {
                undo();
            }
            Err(e)
        }
    }
}
