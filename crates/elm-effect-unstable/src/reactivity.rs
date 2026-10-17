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

// ---------------------------------------------------------------------------
// The pin's Atom lifecycle (reactivity/atom.ts, reactivity/AtomRegistry.ts):
// a node per atom, mounted on read or subscribe, unmounted by the disposal
// pass when it has no listeners and no keepAlive, and reverting to its
// initial value when that happens. Values ride as `Arc<dyn Any>` so an
// equality-suppressed assignment keeps the same handle, which is what the
// pin's `toBe(initial)` assertion observes.
//
// Ceilings, stated rather than hidden: one registry pass models the pin's
// scheduled disposal (an explicit `tick`, since this port has no fiber
// scheduler inside the module), derived nodes rebuild on read rather than
// eagerly, and a derived node's listeners hear the change through the
// rebuild instead of a speculative push. `keepAlive` and `withEquality`
// configure the shared spec in place instead of minting a new atom object,
// which is observably identical for one handle per atom. Atom ids register
// their spec in a process table so a registry can find sources and
// dependents without borrowing the handle.
// ---------------------------------------------------------------------------

type AnyValue = Arc<dyn std::any::Any + Send + Sync>;

/// Every live atom spec, by id, so registries can resolve sources and
/// dependents from an id alone.
static SPECS: std::sync::Mutex<Option<
    std::collections::HashMap<u64, Arc<AtomSpec>>,
>> = std::sync::Mutex::new(None);

fn register_spec(spec: &Arc<AtomSpec>) {
    SPECS
        .lock()
        .unwrap()
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(spec.id, Arc::clone(spec));
}

fn spec_of(id: u64) -> Option<Arc<AtomSpec>> {
    SPECS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|specs| specs.get(&id).cloned())
}

/// What an atom is: a value node, or a derived node over one source.
enum AtomKind {
    Value(AnyValue),
    Map {
        source: u64,
        derive: Box<dyn Fn(&dyn std::any::Any) -> AnyValue + Send + Sync>,
    },
}

/// One atom's spec, shared by every registry that holds it (the pin's atom
/// object carries its own behavior; the registry carries node state).
struct AtomSpec {
    id: u64,
    kind: AtomKind,
    keep_alive: std::sync::atomic::AtomicBool,
    equality: std::sync::Mutex<Option<Box<dyn Fn(&AnyValue, &AnyValue) -> bool + Send + Sync>>>,
}

impl AtomSpec {
    /// Does an assignment of `new` over `old` compare equal? Without a
    /// configured equality every assignment differs, so every set notifies.
    fn equals(&self, old: &AnyValue, new: &AnyValue) -> bool {
        match self.equality.lock().unwrap().as_ref() {
            Some(equals) => equals(old, new),
            None => false,
        }
    }

    /// Is this atom derived from `source`?
    fn is_dependent_of(&self, source: u64) -> bool {
        matches!(&self.kind, AtomKind::Map { source: seen, .. } if *seen == source)
    }
}

/// An atom handle: `Atom.make(value)`, optionally `keep_alive`,
/// `with_equality`, or derived through [`map`].
pub struct Atom<T> {
    spec: Arc<AtomSpec>,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> Clone for Atom<T> {
    fn clone(&self) -> Self {
        Self { spec: Arc::clone(&self.spec), marker: std::marker::PhantomData }
    }
}

static NEXT_ATOM_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl<T: Clone + Send + Sync + 'static> Atom<T> {
    /// `Atom.make`: a value atom holding `initial`.
    pub fn make(initial: T) -> Self {
        let spec = Arc::new(AtomSpec {
            id: NEXT_ATOM_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            kind: AtomKind::Value(Arc::new(initial)),
            keep_alive: std::sync::atomic::AtomicBool::new(false),
            equality: std::sync::Mutex::new(None),
        });
        register_spec(&spec);
        Self { spec, marker: std::marker::PhantomData }
    }

    /// `Atom.keepAlive`: the node survives the disposal pass.
    pub fn keep_alive(self) -> Self {
        self.spec.keep_alive.store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }

    /// `Atom.withEquality`: an assignment comparing equal neither notifies
    /// nor replaces the stored value.
    pub fn with_equality(self, equals: impl Fn(&T, &T) -> bool + Send + Sync + 'static) -> Self {
        *self.spec.equality.lock().unwrap() = Some(Box::new(move |old, new| {
            let old = old
                .downcast_ref::<T>()
                .expect("an atom's value carries its declared type");
            let new = new
                .downcast_ref::<T>()
                .expect("an atom's value carries its declared type");
            equals(old, new)
        }));
        self
    }
}

/// `Atom.map`: a derived atom rebuilt from its source when the source
/// changes, and never before a read asks for it.
pub fn map<S, U, F>(source: &Atom<S>, derive: F) -> Atom<U>
where
    S: Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
    F: Fn(&S) -> U + Send + Sync + 'static,
{
    let spec = Arc::new(AtomSpec {
        id: NEXT_ATOM_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        kind: AtomKind::Map {
            source: source.spec.id,
            derive: Box::new(move |value: &dyn std::any::Any| {
                let typed = value
                    .downcast_ref::<S>()
                    .expect("a map's source value carries the source type")
                    .clone();
                Arc::new(derive(&typed)) as AnyValue
            }),
        },
        keep_alive: std::sync::atomic::AtomicBool::new(false),
        equality: std::sync::Mutex::new(None),
    });
    register_spec(&spec);
    Atom { spec, marker: std::marker::PhantomData }
}

/// The registry's per-atom state.
struct AtomNodeState {
    value: AnyValue,
    listeners: Vec<(u64, Arc<dyn Fn(&AnyValue) + Send + Sync>)>,
    /// A derived node that owes a rebuild before its next read.
    dirty: bool,
}

/// `AtomRegistry.make`: the runtime that mounts, reads, writes and disposes
/// atom nodes. Each registry is independent.
pub struct Registry {
    nodes: Mutex<std::collections::HashMap<u64, AtomNodeState>>,
    next_listener: std::sync::atomic::AtomicU64,
}

/// A subscription handle: `cancel` releases its listener, after which the
/// node counts as unlistened for the next disposal pass.
pub struct Subscription {
    registry: Arc<Registry>,
    atom_id: u64,
    listener_id: u64,
}

impl Subscription {
    /// `cancel()` in the pin: drop this subscription's listener. Calling it
    /// twice is a no-op the second time.
    pub fn cancel(&self) {
        self.registry.remove_listener(self.atom_id, self.listener_id);
    }
}

impl Registry {
    /// An empty registry.
    pub fn make() -> Arc<Registry> {
        Arc::new(Self {
            nodes: Mutex::new(std::collections::HashMap::new()),
            next_listener: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// `registry.get`: the atom's current value, mounting (or rebuilding) its
    /// node as needed.
    pub fn get<T: Clone + Send + Sync + 'static>(&self, atom: &Atom<T>) -> T {
        (*self.get_shared(atom)).clone()
    }

    /// `registry.get` as the stored handle, so identity is observable after
    /// an equality-suppressed assignment.
    pub fn get_shared<T: Send + Sync + 'static>(&self, atom: &Atom<T>) -> Arc<T> {
        let value = self.node_value(&atom.spec);
        Arc::downcast::<T>(value)
            .unwrap_or_else(|_| unreachable!("an atom's value carries its declared type"))
    }

    /// `registry.set`: assign, notifying listeners unless the configured
    /// equality compares equal (in which case the stored handle is kept).
    pub fn set<T: Clone + Send + Sync + 'static>(&self, atom: &Atom<T>, value: T) {
        let new: AnyValue = Arc::new(value);
        // Built outside the lock: a derived atom's build reads the registry
        // (its source), and this mutex is not reentrant.
        let initial = if self.nodes.lock().unwrap().contains_key(&atom.spec.id) {
            None
        } else {
            Some(self.initial_value(&atom.spec))
        };
        let mut nodes = self.nodes.lock().unwrap();
        let node = nodes.entry(atom.spec.id).or_insert_with(|| AtomNodeState {
            value: initial.expect("built for an absent node"),
            listeners: Vec::new(),
            dirty: false,
        });
        if atom.spec.equals(&node.value, &new) {
            return;
        }
        node.value = Arc::clone(&new);
        node.dirty = false;
        let listeners: Vec<Arc<dyn Fn(&AnyValue) + Send + Sync>> = node
            .listeners
            .iter()
            .map(|(_, listener)| Arc::clone(listener))
            .collect();
        let dependents: Vec<u64> = nodes
            .keys()
            .copied()
            .filter(|id| {
                spec_of(*id).is_some_and(|spec| spec.is_dependent_of(atom.spec.id))
            })
            .collect();
        for id in dependents {
            if let Some(node) = nodes.get_mut(&id) {
                node.dirty = true;
            }
        }
        drop(nodes);
        for listener in listeners {
            listener(&new);
        }
    }

    /// `registry.subscribe`: add a listener that receives every new value.
    /// The node stays mounted while the subscription lives.
    pub fn subscribe<T: Clone + Send + Sync + 'static>(
        self: &Arc<Self>,
        atom: &Atom<T>,
        listener: impl Fn(T) + Send + Sync + 'static,
    ) -> Subscription {
        let listener_id = self.next_listener.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let erased: Arc<dyn Fn(&AnyValue) + Send + Sync> = Arc::new(move |value: &AnyValue| {
            let typed = value
                .downcast_ref::<T>()
                .expect("an atom's value carries its declared type")
                .clone();
            listener(typed);
        });
        // Built outside the lock: a derived atom's build reads the registry
        // (its source), and this mutex is not reentrant.
        let initial = if self.nodes.lock().unwrap().contains_key(&atom.spec.id) {
            None
        } else {
            Some(self.initial_value(&atom.spec))
        };
        let mut nodes = self.nodes.lock().unwrap();
        let node = nodes.entry(atom.spec.id).or_insert_with(|| AtomNodeState {
            value: initial.expect("built for an absent node"),
            listeners: Vec::new(),
            dirty: false,
        });
        node.listeners.push((listener_id, erased));
        drop(nodes);
        Subscription { registry: Arc::clone(self), atom_id: atom.spec.id, listener_id }
    }

    /// The disposal pass: every node with no listeners and no keepAlive is
    /// released, so its next read starts from the initial value again.
    pub fn tick(&self) {
        let mut nodes = self.nodes.lock().unwrap();
        let releasable: Vec<u64> = nodes
            .iter()
            .filter(|(id, node)| {
                node.listeners.is_empty()
                    && spec_of(**id)
                        .is_some_and(|spec| !spec.keep_alive.load(std::sync::atomic::Ordering::SeqCst))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in releasable {
            nodes.remove(&id);
        }
    }

    fn remove_listener(&self, atom_id: u64, listener_id: u64) {
        let mut nodes = self.nodes.lock().unwrap();
        if let Some(node) = nodes.get_mut(&atom_id) {
            node.listeners.retain(|(seen, _)| *seen != listener_id);
        }
    }

    /// The node's current value, building or rebuilding a derived node on
    /// the way. A read keeps the node present, which is the mount. The build
    /// happens outside the lock: a derived atom's build reads the registry
    /// (its source), and this mutex is not reentrant.
    fn node_value(&self, spec: &Arc<AtomSpec>) -> AnyValue {
        let present = self.nodes.lock().unwrap().contains_key(&spec.id);
        if !present {
            let value = self.initial_value(spec);
            self.nodes
                .lock()
                .unwrap()
                .entry(spec.id)
                .or_insert(AtomNodeState { value, listeners: Vec::new(), dirty: false });
        }
        let dirty = self
            .nodes
            .lock()
            .unwrap()
            .get(&spec.id)
            .expect("present")
            .dirty;
        if dirty {
            let rebuilt = self.initial_value(spec);
            let mut nodes = self.nodes.lock().unwrap();
            let node = nodes.get_mut(&spec.id).expect("present");
            node.value = rebuilt;
            node.dirty = false;
        }
        Arc::clone(&self.nodes.lock().unwrap().get(&spec.id).expect("present").value)
    }

    /// A node's value straight from its spec: the initial value for a value
    /// atom, or a fresh build for a derived atom (which recurses into its
    /// source).
    fn initial_value(&self, spec: &Arc<AtomSpec>) -> AnyValue {
        match &spec.kind {
            AtomKind::Value(initial) => Arc::clone(initial),
            AtomKind::Map { source, derive } => {
                let source_value = self.source_value(*source);
                // `&*source_value` hands the derive the INNER value: a
                // reference to the Arc itself would erase to
                // `Arc<dyn Any + Send + Sync>`, whose type is the wrapper.
                derive(&*source_value)
            }
        }
    }

    /// A source atom's current value, mounting the source in turn. An
    /// unregistered id (a source from a registry that never held it) has no
    /// spec to mount and yields a placeholder. Recursive maps would recurse
    /// forever here, which no carried row constructs.
    fn source_value(&self, source: u64) -> AnyValue {
        match spec_of(source) {
            Some(spec) => self.node_value(&spec),
            None => Arc::new(()),
        }
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

    // ---- the Atom lifecycle core ----

    #[test]
    fn atom_registries_are_independent() {
        // The same atom holds different values in two registries, and a
        // disposal in one does not touch the other.
        let counter = Atom::make(7i64);
        let first = Registry::make();
        let second = Registry::make();
        first.set(&counter, 1);
        assert_eq!(first.get(&counter), 1);
        assert_eq!(second.get(&counter), 7, "a registry starts from the initial value");
        second.set(&counter, 2);
        assert_eq!(first.get(&counter), 1);
        first.tick();
        second.tick();
        assert_eq!(first.get(&counter), 7, "released nodes revert per registry");
        assert_eq!(second.get(&counter), 7);
    }

    #[test]
    fn atom_cancel_is_idempotent() {
        let counter = Atom::make(0i64);
        let registry = Registry::make();
        let sub = registry.subscribe(&counter, |_| {});
        sub.cancel();
        sub.cancel();
        registry.tick();
        assert_eq!(registry.get(&counter), 0);
    }

    #[test]
    fn atom_listener_snapshot_survives_an_unsubscribe_mid_notify() {
        // The notified snapshot is taken before any callback runs, so a
        // listener that cancels a later subscription does not skip it.
        let counter = Atom::make(0i64);
        let registry = Registry::make();
        let seen = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let cancel_holder: Arc<Mutex<Option<Subscription>>> = Arc::new(Mutex::new(None));
        let first_log = seen.clone();
        let holder = cancel_holder.clone();
        let sub_first = registry.subscribe(&counter, move |_| {
            first_log.lock().unwrap().push("first");
            if let Some(sub) = holder.lock().unwrap().take() {
                sub.cancel();
            }
        });
        let second_log = seen.clone();
        let sub_second = registry.subscribe(&counter, move |_| {
            second_log.lock().unwrap().push("second");
        });
        *cancel_holder.lock().unwrap() = Some(sub_second);
        registry.set(&counter, 1);
        let _ = sub_first;
        assert_eq!(*seen.lock().unwrap(), vec!["first", "second"]);
    }

    #[test]
    fn atom_derived_node_rebuilds_only_after_a_change() {
        let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = builds.clone();
        let source = Atom::make(3i64);
        let doubled = map(&source, move |n: &i64| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            n * 2
        });
        let registry = Registry::make();
        assert_eq!(registry.get(&doubled), 6);
        assert_eq!(registry.get(&doubled), 6, "a cached read does not rebuild");
        assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
        registry.set(&source, 5);
        assert_eq!(registry.get(&doubled), 10);
        assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn atom_release_invalidates_dependents() {
        // Releasing a source reverts it; the dependent rebuilds from the
        // reverted value on its next read.
        let source = Atom::make(1i64);
        let plus_one = map(&source, |n: &i64| n + 1);
        let registry = Registry::make();
        assert_eq!(registry.get(&plus_one), 2);
        registry.set(&source, 10);
        assert_eq!(registry.get(&plus_one), 11);
        registry.tick();
        // Both nodes are unlistened, so both released: a fresh read rebuilds
        // from the initial values.
        assert_eq!(registry.get(&plus_one), 2);
    }
}
