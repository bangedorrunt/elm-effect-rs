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
static SPECS: std::sync::Mutex<Option<std::collections::HashMap<u64, Arc<AtomSpec>>>> =
    std::sync::Mutex::new(None);

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

/// What an atom is: a value node, a derived node over one source, or an
/// asynchronous node whose value is an [`AsyncResult`].
enum AtomKind {
    Value(AnyValue),
    Map {
        source: u64,
        derive: Box<dyn Fn(&dyn std::any::Any) -> AnyValue + Send + Sync>,
    },
    Async(AsyncSpec),
}

/// The two halves of an asynchronous atom: how to run the work, and how to
/// rebuild the typed [`AsyncResult`] from erased pieces (the node stores one
/// `AnyValue`, so the type is known only at construction).
struct AsyncSpec {
    fetch: Box<dyn Fn() -> Result<AnyValue, AnyValue> + Send + Sync>,
    ops: Arc<dyn AsyncOps>,
}

trait AsyncOps: Send + Sync {
    fn initial(&self) -> AnyValue;
    fn success(&self, value: AnyValue, previous: Option<AnyValue>) -> AnyValue;
    fn failure(&self, error: AnyValue, previous: Option<AnyValue>) -> AnyValue;
    fn waiting(&self, current: &AnyValue) -> AnyValue;
    fn previous(&self, current: &AnyValue) -> Option<AnyValue>;
    fn needs_fetch(&self, current: &AnyValue) -> bool;
}

struct TypedOps<T, E>(std::marker::PhantomData<fn() -> (T, E)>);

fn typed<T: Send + Sync + 'static>(value: AnyValue) -> Arc<T> {
    Arc::downcast::<T>(value).unwrap_or_else(|_| {
        unreachable!("an async atom's value carries its declared type")
    })
}

impl<T, E> AsyncOps for TypedOps<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn initial(&self) -> AnyValue {
        Arc::new(AsyncResult::<T, E>::initial_waiting())
    }

    fn success(&self, value: AnyValue, _previous: Option<AnyValue>) -> AnyValue {
        Arc::new(AsyncResult::<T, E>::success((*typed::<T>(value)).clone()))
    }

    fn failure(&self, error: AnyValue, previous: Option<AnyValue>) -> AnyValue {
        let cause = <elm_effect::cause::Cause<E>>::fail((*typed::<E>(error)).clone());
        let carried = previous.map(|value| {
            AsyncResult::<T, E>::success_state((*typed::<T>(value)).clone())
        });
        Arc::new(AsyncResult::<T, E>::failure_with(cause, carried))
    }

    fn waiting(&self, current: &AnyValue) -> AnyValue {
        let current = typed::<AsyncResult<T, E>>(Arc::clone(current));
        Arc::new((*current).clone().mark_waiting())
    }

    fn previous(&self, current: &AnyValue) -> Option<AnyValue> {
        let current = typed::<AsyncResult<T, E>>(Arc::clone(current));
        current
            .previous_value()
            .map(|value| Arc::new(value) as AnyValue)
    }

    fn needs_fetch(&self, current: &AnyValue) -> bool {
        let current = typed::<AsyncResult<T, E>>(Arc::clone(current));
        matches!(&*current, AsyncResult::Initial { .. })
    }
}

/// One atom's spec, shared by every registry that holds it (the pin's atom
/// object carries its own behavior; the registry carries node state).
struct AtomSpec {
    id: u64,
    kind: AtomKind,
    keep_alive: std::sync::atomic::AtomicBool,
    equality: std::sync::Mutex<Option<Box<dyn Fn(&AnyValue, &AnyValue) -> bool + Send + Sync>>>,
    /// `Atom.setLazy(false)`: a derived node rebuilds when its source changes
    /// instead of waiting for the next read. Default true, the pin's default.
    lazy: std::sync::atomic::AtomicBool,
    label: std::sync::Mutex<Option<String>>,
    /// `Atom.idleTTL`: how long an unlistened node lingers before a disposal
    /// pass may release it.
    idle_ttl: std::sync::Mutex<Option<std::time::Duration>>,
    /// `Atom.initialValue`: the value a freshly created node starts from.
    initial_override: std::sync::Mutex<Option<AnyValue>>,
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

    fn is_lazy(&self) -> bool {
        self.lazy.load(std::sync::atomic::Ordering::SeqCst)
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
        Self {
            spec: Arc::clone(&self.spec),
            marker: std::marker::PhantomData,
        }
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
            lazy: std::sync::atomic::AtomicBool::new(true),
            label: std::sync::Mutex::new(None),
            idle_ttl: std::sync::Mutex::new(None),
            initial_override: std::sync::Mutex::new(None),
        });
        register_spec(&spec);
        Self {
            spec,
            marker: std::marker::PhantomData,
        }
    }

    /// `Atom.keepAlive`: the node survives the disposal pass.
    pub fn keep_alive(self) -> Self {
        self.spec
            .keep_alive
            .store(true, std::sync::atomic::Ordering::SeqCst);
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

    /// `Atom.autoDispose`: the node is releasable again, undoing `keepAlive`.
    pub fn auto_dispose(self) -> Self {
        self.spec
            .keep_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self
    }

    /// `Atom.setLazy(false)`: rebuild this derived atom as soon as its source
    /// changes, rather than on the next read.
    pub fn set_lazy(self, lazy: bool) -> Self {
        self.spec
            .lazy
            .store(lazy, std::sync::atomic::Ordering::SeqCst);
        self
    }

    /// `Atom.withLabel`: a debug name for the node.
    pub fn with_label(self, label: &str) -> Self {
        *self.spec.label.lock().unwrap() = Some(label.to_string());
        self
    }

    /// The label this atom carries, if any.
    pub fn label(&self) -> Option<String> {
        self.spec.label.lock().unwrap().clone()
    }

    /// `Atom.idleTTL`: an unlistened node survives this long before a
    /// disposal pass may release it.
    pub fn idle_ttl(self, ttl: std::time::Duration) -> Self {
        *self.spec.idle_ttl.lock().unwrap() = Some(ttl);
        self
    }

    /// `Atom.initialValue`: the value a freshly created node starts from,
    /// overriding the atom's own initial value.
    pub fn initial_value(self, value: T) -> Self {
        *self.spec.initial_override.lock().unwrap() = Some(Arc::new(value));
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
        lazy: std::sync::atomic::AtomicBool::new(true),
        label: std::sync::Mutex::new(None),
        idle_ttl: std::sync::Mutex::new(None),
        initial_override: std::sync::Mutex::new(None),
    });
    register_spec(&spec);
    Atom {
        spec,
        marker: std::marker::PhantomData,
    }
}

// ---------------------------------------------------------------------------
// AsyncResult: the state of asynchronous work (reactivity/AsyncResult.ts).
// ---------------------------------------------------------------------------

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// A success value with the time it arrived. Carried inside a failure so a
/// caller can keep showing the last good value while an error is surfaced.
#[derive(Clone, Debug, PartialEq)]
pub struct SuccessState<A, E> {
    pub value: A,
    pub waiting: bool,
    pub timestamp: u64,
    marker: std::marker::PhantomData<fn() -> E>,
}

/// The pin's `AsyncResult`: no value yet, a value, or a failure cause. Every
/// state carries `waiting`, so a refresh can show the old value while newer
/// work runs.
#[derive(Clone, Debug, PartialEq)]
pub enum AsyncResult<A, E> {
    Initial {
        waiting: bool,
    },
    Success {
        value: A,
        waiting: bool,
        timestamp: u64,
    },
    Failure {
        cause: elm_effect::cause::Cause<E>,
        previous_success: Option<SuccessState<A, E>>,
        waiting: bool,
    },
}

impl<A, E> AsyncResult<A, E> {
    /// `AsyncResult.initial`: the state before any work has answered.
    pub fn initial() -> Self {
        AsyncResult::Initial { waiting: false }
    }

    /// The initial state while the first run is in flight.
    pub fn initial_waiting() -> Self {
        AsyncResult::Initial { waiting: true }
    }

    /// `AsyncResult.success`.
    pub fn success(value: A) -> Self {
        AsyncResult::Success {
            value,
            waiting: false,
            timestamp: now_millis(),
        }
    }

    /// A success marked as waiting, for a refresh that keeps the old value.
    pub fn success_waiting(value: A) -> Self {
        AsyncResult::Success {
            value,
            waiting: true,
            timestamp: now_millis(),
        }
    }

    /// The success state a failure carries as `previous_success`.
    pub fn success_state(value: A) -> SuccessState<A, E> {
        SuccessState {
            value,
            waiting: false,
            timestamp: now_millis(),
            marker: std::marker::PhantomData,
        }
    }

    /// `AsyncResult.failure` with no earlier success to carry.
    pub fn failure(cause: elm_effect::cause::Cause<E>) -> Self {
        AsyncResult::Failure {
            cause,
            previous_success: None,
            waiting: false,
        }
    }

    /// `AsyncResult.failure` carrying the latest success.
    pub fn failure_with(
        cause: elm_effect::cause::Cause<E>,
        previous_success: Option<SuccessState<A, E>>,
    ) -> Self {
        AsyncResult::Failure {
            cause,
            previous_success,
            waiting: false,
        }
    }

    /// A typed failure, the pin's `AsyncResult.fail`.
    pub fn fail(error: E) -> Self
    where
        E: Clone,
    {
        AsyncResult::failure(elm_effect::cause::Cause::fail(error))
    }

    /// `AsyncResult.isInitial`.
    pub fn is_initial(&self) -> bool {
        matches!(self, AsyncResult::Initial { .. })
    }

    /// `AsyncResult.isSuccess`.
    pub fn is_success(&self) -> bool {
        matches!(self, AsyncResult::Success { .. })
    }

    /// `AsyncResult.isFailure`.
    pub fn is_failure(&self) -> bool {
        matches!(self, AsyncResult::Failure { .. })
    }

    /// `AsyncResult.isWaiting`.
    pub fn is_waiting(&self) -> bool {
        match self {
            AsyncResult::Initial { waiting }
            | AsyncResult::Success { waiting, .. }
            | AsyncResult::Failure { waiting, .. } => *waiting,
        }
    }

    /// `AsyncResult.value`: the current success, or the one a failure carries.
    pub fn value(&self) -> Option<&A> {
        match self {
            AsyncResult::Success { value, .. } => Some(value),
            AsyncResult::Failure {
                previous_success, ..
            } => previous_success.as_ref().map(|success| &success.value),
            AsyncResult::Initial { .. } => None,
        }
    }

    /// The previous success as a value, cloned for the erased carriers.
    pub fn previous_value(&self) -> Option<A>
    where
        A: Clone,
    {
        self.value().cloned()
    }

    /// `AsyncResult.error`: the typed error inside a failure, when the cause
    /// holds one (a defect or interruption yields `None`).
    pub fn error(&self) -> Option<&E> {
        match self {
            AsyncResult::Failure { cause, .. } => cause.find_error(),
            _ => None,
        }
    }

    /// `AsyncResult.cause`.
    pub fn cause(&self) -> Option<&elm_effect::cause::Cause<E>> {
        match self {
            AsyncResult::Failure { cause, .. } => Some(cause),
            _ => None,
        }
    }

    /// `AsyncResult.waiting`: the same state with `waiting` set. The pin
    /// returns `self` unchanged when it is already waiting.
    pub fn mark_waiting(self) -> Self {
        match self {
            AsyncResult::Initial { .. } => AsyncResult::Initial { waiting: true },
            AsyncResult::Success {
                value, timestamp, ..
            } => AsyncResult::Success {
                value,
                waiting: true,
                timestamp,
            },
            AsyncResult::Failure {
                cause,
                previous_success,
                ..
            } => AsyncResult::Failure {
                cause,
                previous_success,
                waiting: true,
            },
        }
    }

    /// `AsyncResult.map`: map the success value (and any success a failure
    /// carries), leaving initial results alone.
    pub fn map<B>(self, f: impl Fn(A) -> B) -> AsyncResult<B, E> {
        match self {
            AsyncResult::Initial { waiting } => AsyncResult::Initial { waiting },
            AsyncResult::Success {
                value,
                waiting,
                timestamp,
            } => AsyncResult::Success {
                value: f(value),
                waiting,
                timestamp,
            },
            AsyncResult::Failure {
                cause,
                previous_success,
                waiting,
            } => AsyncResult::Failure {
                cause,
                previous_success: previous_success.map(|success| SuccessState {
                    value: f(success.value),
                    waiting: success.waiting,
                    timestamp: success.timestamp,
                    marker: std::marker::PhantomData,
                }),
                waiting,
            },
        }
    }

    /// `AsyncResult.matchWithWaiting`: the waiting state wins over the
    /// variant, so a caller renders one waiting branch.
    pub fn match_with_waiting<X>(
        &self,
        on_waiting: impl Fn(&Self) -> X,
        on_success: impl Fn(&A) -> X,
        on_error: impl Fn(&E) -> X,
        on_defect: impl Fn() -> X,
    ) -> X {
        if self.is_waiting() || self.is_initial() {
            return on_waiting(self);
        }
        match self {
            AsyncResult::Success { value, .. } => on_success(value),
            AsyncResult::Failure { cause, .. } => match cause.find_error() {
                Some(error) => on_error(error),
                None => on_defect(),
            },
            AsyncResult::Initial { .. } => unreachable!("handled above"),
        }
    }
}

/// `AsyncResult.all`: the first non-success wins; a success is waiting when
/// any input success waits.
impl<A: Clone, E: Clone> AsyncResult<A, E> {
    pub fn all(results: &[AsyncResult<A, E>]) -> AsyncResult<Vec<A>, E> {
        let mut values = Vec::with_capacity(results.len());
        let mut waiting = false;
        for result in results {
            match result {
                AsyncResult::Success {
                    value,
                    waiting: waiting_here,
                    ..
                } => {
                    values.push(value.clone());
                    waiting |= *waiting_here;
                }
                AsyncResult::Initial { waiting: waiting_here } => {
                    return AsyncResult::Initial {
                        waiting: *waiting_here,
                    };
                }
                AsyncResult::Failure {
                    cause,
                    previous_success,
                    waiting: waiting_here,
                } => {
                    return AsyncResult::Failure {
                        cause: cause.clone(),
                        previous_success: previous_success.as_ref().map(|success| SuccessState {
                            value: values.clone(),
                            waiting: success.waiting,
                            timestamp: success.timestamp,
                            marker: std::marker::PhantomData,
                        }),
                        waiting: *waiting_here,
                    };
                }
            }
        }
        AsyncResult::Success {
            value: values,
            waiting,
            timestamp: now_millis(),
        }
    }
}

// ---------------------------------------------------------------------------
// Task scheduling: where an asynchronous atom's fetch runs.
// ---------------------------------------------------------------------------

/// Where a registry runs deferred work. The default runs inline, which makes
/// an asynchronous atom settle on the read that started it; a test or a UI
/// runtime supplies a queue and drains it at the frame boundary.
pub trait TaskScheduler: Send + Sync {
    fn schedule(&self, task: Box<dyn FnOnce() + Send>);
}

/// Runs every task on the thread that schedules it.
pub struct InlineScheduler;

impl TaskScheduler for InlineScheduler {
    fn schedule(&self, task: Box<dyn FnOnce() + Send>) {
        task();
    }
}

/// Queues tasks until [`DeferredScheduler::run`] drains them, so a test can
/// observe the waiting state before the fetch settles.
#[derive(Default)]
pub struct DeferredScheduler {
    queue: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl DeferredScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run every queued task, including tasks those tasks queue.
    pub fn run(&self) {
        loop {
            let task = self.queue.lock().unwrap().pop();
            match task {
                Some(task) => task(),
                None => break,
            }
        }
    }

    /// How many tasks are waiting to run.
    pub fn pending(&self) -> usize {
        self.queue.lock().unwrap().len()
    }
}

impl TaskScheduler for DeferredScheduler {
    fn schedule(&self, task: Box<dyn FnOnce() + Send>) {
        self.queue.lock().unwrap().push(task);
    }
}

// ---------------------------------------------------------------------------
// Atom constructors beyond `make` and `map`.
// ---------------------------------------------------------------------------

/// `Atom.make(Effect)`: an asynchronous atom whose value is an
/// [`AsyncResult`]. `fetch` runs when the node is first read and again on
/// every [`Registry::refresh`]; a superseded run's answer is discarded, which
/// is the pin's interruption.
pub fn async_atom<T, E>(
    fetch: impl Fn() -> Result<T, E> + Send + Sync + 'static,
) -> Atom<AsyncResult<T, E>>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    let spec = Arc::new(AtomSpec {
        id: NEXT_ATOM_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        kind: AtomKind::Async(AsyncSpec {
            fetch: Box::new(move || {
                fetch()
                    .map(|value| Arc::new(value) as AnyValue)
                    .map_err(|error| Arc::new(error) as AnyValue)
            }),
            ops: Arc::new(TypedOps::<T, E>(std::marker::PhantomData)),
        }),
        keep_alive: std::sync::atomic::AtomicBool::new(false),
        equality: std::sync::Mutex::new(None),
        lazy: std::sync::atomic::AtomicBool::new(true),
        label: std::sync::Mutex::new(None),
        idle_ttl: std::sync::Mutex::new(None),
        initial_override: std::sync::Mutex::new(None),
    });
    register_spec(&spec);
    Atom {
        spec,
        marker: std::marker::PhantomData,
    }
}

/// `Atom.mapResult`: a derived atom over an asynchronous result, mapping only
/// the success value.
pub fn map_result<T, E, U, F>(source: &Atom<AsyncResult<T, E>>, f: F) -> Atom<AsyncResult<U, E>>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
    F: Fn(&T) -> U + Send + Sync + 'static,
{
    map(source, move |result: &AsyncResult<T, E>| {
        result.clone().map(|value| f(&value))
    })
}

/// `Atom.withFallback`: an asynchronous atom that reads as a success holding
/// `fallback` until the primary answers, and keeps that fallback on failure.
///
/// Ceiling: writes to the fallback atom are not forwarded to the primary, so
/// only the read half of the pin's combinator is carried.
pub fn with_fallback<T, E>(
    primary: &Atom<AsyncResult<T, E>>,
    fallback: T,
) -> Atom<AsyncResult<T, E>>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    map(primary, move |result: &AsyncResult<T, E>| match result {
        AsyncResult::Success { .. } => result.clone(),
        _ => AsyncResult::success(fallback.clone()),
    })
}

/// `Atom.family`: one atom per key, so a family member reads and caches like
/// any other atom while each key gets its own node.
pub struct Family<K, T> {
    make: Box<dyn Fn(&K) -> Atom<T> + Send + Sync>,
    cache: Mutex<std::collections::HashMap<K, Atom<T>>>,
}

impl<K, T> Family<K, T>
where
    K: Eq + std::hash::Hash + Clone,
{
    /// The atom for `key`, minted on first use and reused after that.
    pub fn get(&self, key: &K) -> Atom<T> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(atom) = cache.get(key) {
            return atom.clone();
        }
        let atom = (self.make)(key);
        cache.insert(key.clone(), atom.clone());
        atom
    }
}

/// `Atom.family`: build a family from a key to atom function.
pub fn family<K, T, F>(make: F) -> Family<K, T>
where
    K: Eq + std::hash::Hash + Clone,
    F: Fn(&K) -> Atom<T> + Send + Sync + 'static,
{
    Family {
        make: Box::new(make),
        cache: Mutex::new(std::collections::HashMap::new()),
    }
}

/// The registry's per-atom state.
struct AtomNodeState {
    value: AnyValue,
    listeners: Vec<(u64, Arc<dyn Fn(&AnyValue) + Send + Sync>)>,
    /// A derived node that owes a rebuild before its next read.
    dirty: bool,
    /// Bumped by every write and every refresh, so a fetch that was
    /// superseded cannot overwrite the newer answer.
    generation: u64,
    in_flight: bool,
    last_used: std::time::Instant,
}

impl AtomNodeState {
    fn new(value: AnyValue) -> Self {
        Self {
            value,
            listeners: Vec::new(),
            dirty: false,
            generation: 0,
            in_flight: false,
            last_used: std::time::Instant::now(),
        }
    }
}

/// Notifications collected by the outermost [`Registry::batch`].
#[derive(Default)]
struct BatchState {
    depth: usize,
    pending: Vec<u64>,
}

/// `AtomRegistry.make`: the runtime that mounts, reads, writes and disposes
/// atom nodes. Each registry is independent.
pub struct Registry {
    nodes: Mutex<std::collections::HashMap<u64, AtomNodeState>>,
    next_listener: std::sync::atomic::AtomicU64,
    scheduler: Arc<dyn TaskScheduler>,
    batch: Mutex<BatchState>,
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
        self.registry
            .remove_listener(self.atom_id, self.listener_id);
    }
}

impl Registry {
    /// An empty registry.
    pub fn make() -> Arc<Registry> {
        Self::with_scheduler(Arc::new(InlineScheduler))
    }

    /// A registry whose deferred work runs where `scheduler` puts it.
    pub fn with_scheduler(scheduler: Arc<dyn TaskScheduler>) -> Arc<Registry> {
        Arc::new(Self {
            nodes: Mutex::new(std::collections::HashMap::new()),
            next_listener: std::sync::atomic::AtomicU64::new(1),
            scheduler,
            batch: Mutex::new(BatchState::default()),
        })
    }

    /// `Atom.batch`: listeners hear nothing until the outermost batch ends,
    /// then each changed node announces once.
    pub fn batch<R>(self: &Arc<Self>, f: impl FnOnce() -> R) -> R {
        self.batch.lock().unwrap().depth += 1;
        let result = f();
        let pending = {
            let mut batch = self.batch.lock().unwrap();
            batch.depth -= 1;
            if batch.depth == 0 {
                std::mem::take(&mut batch.pending)
            } else {
                Vec::new()
            }
        };
        let mut seen = std::collections::BTreeSet::new();
        for id in pending {
            if seen.insert(id) {
                self.deliver(id);
            }
        }
        // A derived node that went stale inside the batch owes its listeners
        // the committed value, not the interim one.
        let owed_ids: Vec<u64> = {
            let nodes = self.nodes.lock().unwrap();
            nodes
                .iter()
                .filter(|(_, node)| node.dirty)
                .map(|(id, _)| *id)
                .collect()
        };
        let owed: Vec<Arc<AtomSpec>> = owed_ids
            .into_iter()
            .filter_map(spec_of)
            .filter(|spec| !spec.is_lazy() || self.has_listeners(spec.id))
            .collect();
        for spec in owed {
            let _ = self.node_value(&spec);
        }
        result
    }

    fn has_listeners(&self, id: u64) -> bool {
        self.nodes
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|node| !node.listeners.is_empty())
    }

    /// `registry.mount`: hold the atom open for as long as the subscription.
    pub fn mount<T: Clone + Send + Sync + 'static>(self: &Arc<Self>, atom: &Atom<T>) -> Subscription {
        self.subscribe(atom, |_| {})
    }

    /// `registry.modify`: apply `f` to the current value, write its second
    /// half, and return the first.
    pub fn modify<T, R, F>(self: &Arc<Self>, atom: &Atom<T>, f: F) -> R
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> (R, T),
    {
        let current = (*self.get_shared(atom)).clone();
        let (out, next) = f(current);
        self.set(atom, next);
        out
    }

    /// `registry.update`: write the result of `f` applied to the current
    /// value.
    pub fn update<T, F>(self: &Arc<Self>, atom: &Atom<T>, f: F)
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> T,
    {
        let current = (*self.get_shared(atom)).clone();
        self.set(atom, f(current));
    }

    /// `registry.refresh`: re-run an asynchronous atom, or rebuild a derived
    /// one even though its source did not change.
    pub fn refresh<T: Clone + Send + Sync + 'static>(self: &Arc<Self>, atom: &Atom<T>) {
        match &atom.spec.kind {
            AtomKind::Async(async_spec) => {
                {
                    let mut nodes = self.nodes.lock().unwrap();
                    if let Some(node) = nodes.get_mut(&atom.spec.id) {
                        node.value = async_spec.ops.waiting(&node.value);
                        node.in_flight = false;
                        node.generation += 1;
                        node.last_used = std::time::Instant::now();
                    }
                }
                self.start_async(&atom.spec);
                self.notify(atom.spec.id);
            }
            AtomKind::Map { .. } => {
                if let Some(node) = self.nodes.lock().unwrap().get_mut(&atom.spec.id) {
                    node.dirty = true;
                }
                let _ = self.node_value(&atom.spec);
                self.notify(atom.spec.id);
            }
            AtomKind::Value(_) => {}
        }
    }

    /// `registry.getResult`: the latest settled result, or `None` while the
    /// asynchronous atom has not answered yet.
    pub fn get_result<T, E>(
        self: &Arc<Self>,
        atom: &Atom<AsyncResult<T, E>>,
    ) -> Option<Result<T, elm_effect::cause::Cause<E>>>
    where
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    {
        match self.get(atom) {
            AsyncResult::Initial { .. } => None,
            AsyncResult::Success { value, .. } => Some(Ok(value)),
            AsyncResult::Failure { cause, .. } => Some(Err(cause)),
        }
    }

    /// `registry.get`: the atom's current value, mounting (or rebuilding) its
    /// node as needed.
    pub fn get<T: Clone + Send + Sync + 'static>(self: &Arc<Self>, atom: &Atom<T>) -> T {
        (*self.get_shared(atom)).clone()
    }

    /// `registry.get` as the stored handle, so identity is observable after
    /// an equality-suppressed assignment.
    pub fn get_shared<T: Send + Sync + 'static>(self: &Arc<Self>, atom: &Atom<T>) -> Arc<T> {
        let value = self.node_value(&atom.spec);
        Arc::downcast::<T>(value)
            .unwrap_or_else(|_| unreachable!("an atom's value carries its declared type"))
    }

    /// `registry.set`: assign, notifying listeners unless the configured
    /// equality compares equal (in which case the stored handle is kept).
    pub fn set<T: Clone + Send + Sync + 'static>(self: &Arc<Self>, atom: &Atom<T>, value: T) {
        self.assign(&atom.spec, Arc::new(value));
    }

    /// The write path shared by `set` and an asynchronous atom's completion:
    /// an equal assignment keeps the stored handle and notifies nobody, any
    /// other assignment drops derived nodes to stale, and listeners hear the
    /// new value once the batch commits.
    fn assign(self: &Arc<Self>, spec: &Arc<AtomSpec>, new: AnyValue) {
        // Built outside the lock: a derived atom's build reads the registry
        // (its source), and this mutex is not reentrant.
        let absent = !self.nodes.lock().unwrap().contains_key(&spec.id);
        let initial = if absent {
            Some(self.initial_value(spec))
        } else {
            None
        };
        let mut nodes = self.nodes.lock().unwrap();
        let node = nodes
            .entry(spec.id)
            .or_insert_with(|| AtomNodeState::new(initial.expect("built for an absent node")));
        if spec.equals(&node.value, &new) {
            node.last_used = std::time::Instant::now();
            return;
        }
        node.value = Arc::clone(&new);
        node.dirty = false;
        // A write supersedes any fetch still running for this node.
        node.generation += 1;
        node.in_flight = false;
        node.last_used = std::time::Instant::now();
        let dependents: Vec<u64> = nodes
            .keys()
            .copied()
            .filter(|id| {
                spec_of(*id).is_some_and(|candidate| candidate.is_dependent_of(spec.id))
            })
            .collect();
        for id in &dependents {
            if let Some(node) = nodes.get_mut(id) {
                node.dirty = true;
            }
        }
        drop(nodes);
        // `Atom.setLazy(false)` rebuilds now instead of on the next read.
        for id in dependents {
            if let Some(dependent) = spec_of(id) {
                if !dependent.is_lazy() {
                    let _ = self.node_value(&dependent);
                }
            }
        }
        self.notify(spec.id);
    }

    /// Queue (inside a batch) or run this node's listeners.
    fn notify(&self, id: u64) {
        {
            let mut batch = self.batch.lock().unwrap();
            if batch.depth > 0 {
                batch.pending.push(id);
                return;
            }
        }
        self.deliver(id);
    }

    fn deliver(&self, id: u64) {
        let (value, listeners) = {
            let nodes = self.nodes.lock().unwrap();
            let Some(node) = nodes.get(&id) else {
                return;
            };
            (
                Arc::clone(&node.value),
                node.listeners
                    .iter()
                    .map(|(_, listener)| Arc::clone(listener))
                    .collect::<Vec<_>>(),
            )
        };
        for listener in listeners {
            listener(&value);
        }
    }

    /// Start an asynchronous atom's fetch when its node has no answer yet.
    fn start_async(self: &Arc<Self>, spec: &Arc<AtomSpec>) {
        let AtomKind::Async(_) = &spec.kind else {
            return;
        };
        let generation = {
            let mut nodes = self.nodes.lock().unwrap();
            let Some(node) = nodes.get_mut(&spec.id) else {
                return;
            };
            if node.in_flight {
                return;
            }
            node.in_flight = true;
            node.generation += 1;
            node.generation
        };
        let this = Arc::clone(self);
        let task_spec = Arc::clone(spec);
        self.scheduler.schedule(Box::new(move || {
            let result = match &task_spec.kind {
                AtomKind::Async(async_spec) => (async_spec.fetch)(),
                _ => return,
            };
            this.apply_async(&task_spec, generation, result);
        }));
    }

    /// Commit a fetch's answer unless a later write or refresh superseded it.
    fn apply_async(
        self: &Arc<Self>,
        spec: &Arc<AtomSpec>,
        generation: u64,
        result: Result<AnyValue, AnyValue>,
    ) {
        let AtomKind::Async(async_spec) = &spec.kind else {
            return;
        };
        let mut nodes = self.nodes.lock().unwrap();
        let Some(node) = nodes.get_mut(&spec.id) else {
            return;
        };
        if node.generation != generation {
            return;
        }
        node.in_flight = false;
        let previous = async_spec.ops.previous(&node.value);
        node.value = match result {
            Ok(value) => async_spec.ops.success(value, previous),
            Err(error) => async_spec.ops.failure(error, previous),
        };
        node.dirty = false;
        node.last_used = std::time::Instant::now();
        drop(nodes);
        // An answered fetch reaches derived nodes exactly as a write does.
        let dependents: Vec<u64> = {
            let mut nodes = self.nodes.lock().unwrap();
            let dependents: Vec<u64> = nodes
                .keys()
                .copied()
                .filter(|id| {
                    spec_of(*id).is_some_and(|candidate| candidate.is_dependent_of(spec.id))
                })
                .collect();
            for id in &dependents {
                if let Some(node) = nodes.get_mut(id) {
                    node.dirty = true;
                }
            }
            dependents
        };
        self.notify(spec.id);
        for id in dependents {
            if let Some(dependent) = spec_of(id) {
                if !dependent.is_lazy() || self.has_listeners(id) {
                    let _ = self.node_value(&dependent);
                }
            }
        }
    }

    /// `registry.subscribe`: add a listener that receives every new value.
    /// The node stays mounted while the subscription lives.
    pub fn subscribe<T: Clone + Send + Sync + 'static>(
        self: &Arc<Self>,
        atom: &Atom<T>,
        listener: impl Fn(T) + Send + Sync + 'static,
    ) -> Subscription {
        let listener_id = self
            .next_listener
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        let node = nodes
            .entry(atom.spec.id)
            .or_insert_with(|| AtomNodeState::new(initial.expect("built for an absent node")));
        node.listeners.push((listener_id, erased));
        drop(nodes);
        Subscription {
            registry: Arc::clone(self),
            atom_id: atom.spec.id,
            listener_id,
        }
    }

    /// The disposal pass: every node with no listeners and no keepAlive is
    /// released, so its next read starts from the initial value again. A node
    /// whose atom declares an idle TTL is kept until that TTL has passed
    /// since its last touch.
    pub fn tick(&self) {
        let now = std::time::Instant::now();
        let mut nodes = self.nodes.lock().unwrap();
        let releasable: Vec<u64> = nodes
            .iter()
            .filter(|(id, node)| {
                if !node.listeners.is_empty() {
                    return false;
                }
                spec_of(**id).is_some_and(|spec| {
                    if spec.keep_alive.load(std::sync::atomic::Ordering::SeqCst) {
                        return false;
                    }
                    match *spec.idle_ttl.lock().unwrap() {
                        Some(ttl) => now.duration_since(node.last_used) >= ttl,
                        None => true,
                    }
                })
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
    fn node_value(self: &Arc<Self>, spec: &Arc<AtomSpec>) -> AnyValue {
        let present = self.nodes.lock().unwrap().contains_key(&spec.id);
        if !present {
            let value = self.initial_value(spec);
            let mut nodes = self.nodes.lock().unwrap();
            nodes.entry(spec.id).or_insert(AtomNodeState::new(value));
        }
        let (dirty, start_async) = {
            let mut nodes = self.nodes.lock().unwrap();
            let node = nodes.get_mut(&spec.id).expect("present");
            node.last_used = std::time::Instant::now();
            let start_async = match &spec.kind {
                AtomKind::Async(async_spec) => {
                    !node.in_flight && async_spec.ops.needs_fetch(&node.value)
                }
                _ => false,
            };
            (node.dirty, start_async)
        };
        if dirty {
            let rebuilt = self.initial_value(spec);
            let changed = {
                let mut nodes = self.nodes.lock().unwrap();
                let node = nodes.get_mut(&spec.id).expect("present");
                let changed = !spec.equals(&node.value, &rebuilt);
                node.value = rebuilt;
                node.dirty = false;
                changed
            };
            // A listener on a derived atom hears the rebuild, not just the
            // source's write.
            if changed {
                self.notify(spec.id);
            }
        }
        if start_async {
            self.start_async(spec);
        }
        Arc::clone(
            &self
                .nodes
                .lock()
                .unwrap()
                .get(&spec.id)
                .expect("present")
                .value,
        )
    }

    /// A node's value straight from its spec: the initial value for a value
    /// atom, or a fresh build for a derived atom (which recurses into its
    /// source).
    fn initial_value(self: &Arc<Self>, spec: &Arc<AtomSpec>) -> AnyValue {
        match &spec.kind {
            AtomKind::Value(initial) => spec
                .initial_override
                .lock()
                .unwrap()
                .as_ref()
                .map_or_else(|| Arc::clone(initial), Arc::clone),
            AtomKind::Async(async_spec) => async_spec.ops.initial(),
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
    fn source_value(self: &Arc<Self>, source: u64) -> AnyValue {
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
        assert_eq!(
            second.get(&counter),
            7,
            "a registry starts from the initial value"
        );
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
