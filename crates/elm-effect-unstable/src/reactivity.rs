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
pub type Watcher<T> = std::sync::Arc<dyn Fn(&T) + Send + Sync>;

/// Handle for one subscription: pass to [`Signal::unwatch`].
pub type WatcherId = u64;

struct SignalState<T> {
    value: T,
    version: u64,
    watchers: Vec<(WatcherId, Watcher<T>)>,
    next_watcher: WatcherId,
}

impl<T: Clone + Send + 'static> Signal<T> {
    /// A signal holding `initial` at version zero.
    pub fn new(initial: T) -> Self {
        Self {
            inner: Mutex::new(SignalState {
                value: initial,
                version: 0,
                watchers: Vec::new(),
                next_watcher: 0,
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
        // Snapshot ids first: a watcher may unsubscribe mid-notify, and
        // positional iteration would then skip its neighbour.
        let ids: Vec<WatcherId> = {
            let mut state = self.inner.lock().unwrap();
            state.value = value.clone();
            state.version += 1;
            state.watchers.iter().map(|(id, _)| *id).collect()
        };
        for id in ids {
            // Clone under the lock, call past it: a watcher may
            // unsubscribe (or be removed) mid-notify without deadlock.
            let watcher = {
                let guard = self.inner.lock().unwrap();
                guard
                    .watchers
                    .iter()
                    .find(|(seen, _)| *seen == id)
                    .map(|(_, w)| w.clone())
            };
            if let Some(watcher) = watcher {
                watcher(&value);
            }
        }
    }

    /// Register a callback run on every future `set`.
    pub fn watch(&self, watcher: impl Fn(&T) + Send + Sync + 'static) -> WatcherId {
        let mut state = self.inner.lock().unwrap();
        let id = state.next_watcher;
        state.next_watcher += 1;
        state.watchers.push((id, Arc::new(watcher)));
        id
    }

    /// Drop a subscription; a removed watcher misses later `set`s, even
    /// one already in flight. Returns false for an unknown id.
    pub fn unwatch(&self, id: WatcherId) -> bool {
        let mut state = self.inner.lock().unwrap();
        let before = state.watchers.len();
        state.watchers.retain(|(seen, _)| *seen != id);
        state.watchers.len() != before
    }

    /// Assign only when `value` differs: equivalent values neither bump
    /// the version nor notify (the `withEquality` shape).
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        if self.get() != value {
            self.set(value);
        }
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

/// A named collection of string atoms (`AtomRegistry` shape): atoms are
/// created on first access and shared by name. Values are strings so the
/// whole registry snapshots as JSON.
///
/// ```
/// use elm_effect_unstable::reactivity::AtomRegistry;
/// let registry = AtomRegistry::new();
/// registry.set("theme", "dark");
/// assert_eq!(registry.get("theme"), Some("dark".to_string()));
/// assert_eq!(registry.get("missing"), None);
/// ```
pub struct AtomRegistry {
    atoms: Mutex<std::collections::HashMap<String, SharedSignal<String>>>,
}

impl AtomRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            atoms: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The atom called `name`, creating it empty when missing.
    pub fn atom(&self, name: &str) -> SharedSignal<String> {
        let mut atoms = self.atoms.lock().unwrap();
        atoms
            .entry(name.to_string())
            .or_insert_with(|| shared(String::new()))
            .clone()
    }

    /// Assign an atom's value, creating it when missing.
    pub fn set(&self, name: &str, value: &str) {
        self.atom(name).set(value.to_string());
    }

    /// An atom's value, if the atom exists.
    pub fn get(&self, name: &str) -> Option<String> {
        self.atoms.lock().unwrap().get(name).map(|atom| atom.get())
    }

    /// Every atom name currently registered.
    pub fn names(&self) -> Vec<String> {
        self.atoms.lock().unwrap().keys().cloned().collect()
    }

    /// A snapshot of every atom (`Hydration` export shape).
    pub fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        self.atoms
            .lock()
            .unwrap()
            .iter()
            .map(|(name, atom)| (name.clone(), atom.get()))
            .collect()
    }

    /// Restore a snapshot (`Hydration` import shape): every entry becomes
    /// (or updates) an atom.
    pub fn restore(&self, snapshot: &std::collections::BTreeMap<String, String>) {
        for (name, value) in snapshot {
            self.set(name, value);
        }
    }
}

impl Default for AtomRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_get_set_bumps_version() {
        let signal = Signal::new(1);
        assert_eq!(signal.get(), 1);
        assert_eq!(signal.version(), 0);
        signal.set(2);
        assert_eq!(signal.get(), 2);
        assert_eq!(signal.version(), 1);
    }

    #[test]
    fn signal_subscribe_receives_every_set() {
        let signal = Signal::new(0);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        signal.watch(move |value| {
            log.lock().unwrap().push(*value);
        });
        signal.set(1);
        signal.set(2);
        assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn registry_snapshot_restores_into_empty() {
        let source = AtomRegistry::new();
        source.set("theme", "dark");
        let snapshot = source.snapshot();
        let target = AtomRegistry::new();
        target.restore(&snapshot);
        assert_eq!(target.get("theme"), Some("dark".to_string()));
    }

    #[test]
    fn unsubscribe_during_notify_does_not_skip_later_listeners() {
        let signal = Arc::new(Signal::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let removed = Arc::new(Mutex::new(None::<WatcherId>));
        // A runs first and drops B mid-flight; C must still run.
        let seen_a = seen.clone();
        let removed_a = removed.clone();
        let s = signal.clone();
        signal.clone().watch(move |_| {
            seen_a.lock().unwrap().push("a");
            if let Some(id) = *removed_a.lock().unwrap() {
                s.unwatch(id);
            }
        });
        let seen_b = seen.clone();
        *removed.lock().unwrap() = Some(signal.clone().watch(move |_| {
            seen_b.lock().unwrap().push("b");
        }));
        let seen_c = seen.clone();
        signal.watch(move |_| {
            seen_c.lock().unwrap().push("c");
        });
        signal.set(1);
        assert_eq!(*seen.lock().unwrap(), vec!["a", "c"]);
    }

    #[test]
    fn set_if_changed_skips_equivalent_notifications() {
        let signal = Signal::new("x".to_string());
        let calls = Arc::new(Mutex::new(0u32));
        let count = calls.clone();
        signal.watch(move |_| {
            *count.lock().unwrap() += 1;
        });
        signal.set_if_changed("x".to_string());
        assert_eq!(signal.version(), 0);
        signal.set_if_changed("y".to_string());
        assert_eq!(*calls.lock().unwrap(), 1);
        assert_eq!(signal.version(), 1);
    }
}
