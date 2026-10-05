//! Minimal reactivity: versioned signals with subscriber callbacks.
//!
//! A [`Signal`] holds one value plus a version counter. [`computed`]
//! derives a value from other signals on demand (no memo graph: the
//! function re-runs per read). Effects subscribe with [`watch`]-style
//! callbacks invoked on every `set`. No cycle detection, no batching.

use std::sync::{Arc, Mutex};

/// A versioned cell: every `set` bumps the version and notifies watchers.
pub struct Signal<T> {
    inner: Mutex<SignalState<T>>,
}

/// A subscriber callback: sees every assigned value.
pub type Watcher<T> = Box<dyn Fn(&T) + Send>;

struct SignalState<T> {
    value: T,
    version: u64,
    watchers: Vec<Watcher<T>>,
}

impl<T: Clone + Send + 'static> Signal<T> {
    /// A signal holding `initial` at version zero.
    pub fn new(initial: T) -> Self {
        Self {
            inner: Mutex::new(SignalState {
                value: initial,
                version: 0,
                watchers: Vec::new(),
            }),
        }
    }

    /// The current value.
    pub fn get(&self) -> T {
        self.inner.lock().unwrap().value.clone()
    }

    /// The current version: bumped once per `set`.
    pub fn version(&self) -> u64 {
        self.inner.lock().unwrap().version
    }

    /// Assign and notify every watcher with the new value.
    pub fn set(&self, value: T) {
        let mut state = self.inner.lock().unwrap();
        state.value = value.clone();
        state.version += 1;
        for watcher in &state.watchers {
            watcher(&value);
        }
    }

    /// Register a callback run on every future `set`.
    pub fn watch(&self, watcher: impl Fn(&T) + Send + 'static) {
        self.inner.lock().unwrap().watchers.push(Box::new(watcher));
    }
}

/// Derive a value from a signal on demand (recomputed per call).
pub fn computed<T, R>(signal: &Signal<T>, derive: impl Fn(&T) -> R) -> R
where
    T: Clone + Send + 'static,
{
    derive(&signal.get())
}

/// A shared signal behind reference counting.
pub type SharedSignal<T> = Arc<Signal<T>>;

/// Wrap a signal for sharing across threads.
pub fn shared<T: Clone + Send + 'static>(initial: T) -> SharedSignal<T> {
    Arc::new(Signal::new(initial))
}
