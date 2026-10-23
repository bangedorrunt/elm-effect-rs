//! Law L5 (cycle part): layers wire services without running constructors.
//!
//! A [`Layer`] is a recipe: named constructors that turn required services
//! (`RIn`) into provided ones (`ROut`). Composition walks a tree of nodes;
//! constructors run in [`build`](Layer::build), once per memoized node
//! identity (the pin's `MemoMap`), each exactly once per handle
//! ([`fresh`](Layer::fresh) builds under a memo map of its own).
//! A cycle is a typed construction error ([`Cycle`]), never a per-request
//! failure.
//!
//! Two kinds of node exist, matching the pin's two constructors:
//! - memoized nodes (`Layer.effect`, `Layer.suspend`): the `MemoMap` answers
//!   a second request for the same node from the first build;
//! - plain nodes (`merge`, `provide`, `tap`, `withSpan`, `fresh`, `orDie`):
//!   each build runs their subtree again, and the memoized children inside
//!   decide what is actually reconstructed.
//!
//! Composition follows the pin's merge order: when two sides provide the same
//! tag, the later side wins. The fail-closed double-provision rule survives
//! where this port invented it, at the effect boundary (`Effect::provide`,
//! `Layer::provide`), not inside layer composition.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use crate::effect::Requirement;
use crate::fiber::{Fiber, FiberTag, ForkOpts};
use crate::mock::Mock;
use crate::scope::Scope;
use crate::service::{Cons, Provide, ServiceMaps, ServiceTag, Services, empty_services};
use crate::tracer::{
    AnySpan, CURRENT_SPAN, CURRENT_TRACER, ParentSpanTag, Span, SpanOptions, Tracer, TracerTag,
};
use crate::{Cause, Defect, Effect, Exit, Reason};

/// A dependency cycle: the tags involved, in dependency order. Returned
/// before any constructor runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cycle {
    /// Tags forming the cycle.
    pub path: Vec<&'static str>,
}

/// Flatten a requirement to its tag ids, in order.
pub trait RequirementIds: Requirement {
    /// Every tag id in this requirement.
    fn ids() -> Vec<&'static str>;
}

impl RequirementIds for () {
    fn ids() -> Vec<&'static str> {
        Vec::new()
    }
}

impl<T: ServiceTag> RequirementIds for T {
    fn ids() -> Vec<&'static str> {
        vec![T::ID]
    }
}

macro_rules! tuple_ids {
    ($(($($T:ident),+)),+) => { $(
        impl<$($T: ServiceTag),+> RequirementIds for ($($T,)+) {
            fn ids() -> Vec<&'static str> {
                vec![$($T::ID),+]
            }
        }
    )+ };
}

tuple_ids!(
    (T0),
    (T0, T1),
    (T0, T1, T2),
    (T0, T1, T2, T3),
    (T0, T1, T2, T3, T4),
    (T0, T1, T2, T3, T4, T5),
    (T0, T1, T2, T3, T4, T5, T6),
    (T0, T1, T2, T3, T4, T5, T6, T7),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13),
    (
        T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
    ),
    (
        T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15
    )
);

impl<H: RequirementIds, T: RequirementIds> RequirementIds for Cons<H, T> {
    fn ids() -> Vec<&'static str> {
        let mut out = H::ids();
        out.extend(T::ids());
        out
    }
}

/// One constructor: builds a single tagged service from the accumulated scope.
struct Stage<E> {
    out_id: &'static str,
    in_ids: Vec<&'static str>,
    make: Arc<dyn Fn() -> Box<dyn RunStage<E> + Send> + Send + Sync>,
}

impl<E> Clone for Stage<E> {
    fn clone(&self) -> Self {
        Self {
            out_id: self.out_id,
            in_ids: self.in_ids.clone(),
            make: Arc::clone(&self.make),
        }
    }
}

/// A constructor run with erased input/output requirement types. The stage's
/// own `in_ids`/`out_id` carry the graph; this only moves values. The build
/// scope is handed over so a scoped constructor can register finalizers on it
/// (plain constructors ignore it).
trait RunStage<E>: Send {
    fn run_with(
        self: Box<Self>,
        services: &Services,
        scope: &Scope,
    ) -> Result<Arc<dyn Any + Send + Sync>, Cause<E>>;
}

/// One stage that creates a span, hands it to the rest of the build, and
/// ends it when the build scope closes.
struct SpanStage<E> {
    name: String,
    options: SpanOptions,
    on_end: Option<Arc<dyn Fn(Arc<dyn Span>, Exit<(), String>) + Send + Sync>>,
    marker: PhantomData<fn() -> E>,
}

impl<E: Clone + Send + 'static> RunStage<E> for SpanStage<E> {
    fn run_with(
        self: Box<Self>,
        services: &Services,
        scope: &Scope,
    ) -> Result<Arc<dyn Any + Send + Sync>, Cause<E>> {
        let (span, timing) = crate::tracer::create_span(services, &self.name, self.options);
        let ambient = AnySpan::from(Arc::clone(&span));
        let end_services = Arc::clone(services);
        let end_span = Arc::clone(&span);
        let on_end = self.on_end;
        // The build scope has no exit value here, so a layer span always ends
        // Successful; a constructor failure still reaches the caller as the
        // build's cause. The pin's onEnd sees that exit, the port cannot.
        scope.add_finalizer(move || {
            if let Some(on_end) = on_end {
                on_end(Arc::clone(&end_span), Exit::Success(()));
            }
            crate::tracer::finish_span(&end_services, &end_span, timing, None);
        });
        Ok(Arc::new(ambient))
    }
}

/// A stage that hands over a span it does not own (`Layer.withParentSpan`).
struct ValueStage {
    span: AnySpan,
}

impl<E: Clone + Send + 'static> RunStage<E> for ValueStage {
    fn run_with(
        self: Box<Self>,
        _services: &Services,
        _scope: &Scope,
    ) -> Result<Arc<dyn Any + Send + Sync>, Cause<E>> {
        Ok(Arc::new(self.span))
    }
}

fn parent_span_stage<E: Clone + Send + 'static>(span: AnySpan) -> Stage<E> {
    Stage {
        out_id: ParentSpanTag::ID,
        in_ids: Vec::new(),
        make: Arc::new(move || {
            Box::new(ValueStage { span: span.clone() }) as Box<dyn RunStage<E> + Send>
        }),
    }
}

fn span_stage<E: Clone + Send + 'static>(name: String, options: LayerSpanOptions) -> Stage<E> {
    let LayerSpanOptions { span, on_end } = options;
    Stage {
        out_id: ParentSpanTag::ID,
        in_ids: Vec::new(),
        make: Arc::new(move || {
            Box::new(SpanStage {
                name: name.clone(),
                options: span.clone(),
                on_end: on_end.clone(),
                marker: PhantomData,
            }) as Box<dyn RunStage<E> + Send>
        }),
    }
}

/// Options for a layer span: the span's own options plus the finalizer the
/// pin runs when the layer span ends (`Layer.SpanOptions`).
#[derive(Clone, Default)]
pub struct LayerSpanOptions {
    /// Span creation options.
    pub span: SpanOptions,
    /// Runs when the layer span ends, before the span's status moves to
    /// `Ended`, like the pin's extra finalizer.
    pub on_end: Option<Arc<dyn Fn(Arc<dyn Span>, Exit<(), String>) + Send + Sync>>,
}

impl From<SpanOptions> for LayerSpanOptions {
    fn from(span: SpanOptions) -> Self {
        Self { span, on_end: None }
    }
}

/// Marker for a recipe producing `S`, failing with `E`, needing `RIn`.
type Recipe<S, E, RIn> = PhantomData<fn() -> (S, E, RIn)>;

struct Ctor<S, E, RIn, F> {
    make: Arc<F>,
    marker: Recipe<S, E, RIn>,
}

impl<S, E, RIn, F> RunStage<E> for Ctor<S, E, RIn, F>
where
    S: Send + Sync + 'static,
    E: Clone + Send + 'static,
    RIn: Requirement,
    F: Fn() -> Effect<S, E, RIn> + Send + Sync,
{
    fn run_with(
        self: Box<Self>,
        services: &Services,
        _scope: &Scope,
    ) -> Result<Arc<dyn Any + Send + Sync>, Cause<E>> {
        let value = (self.make)().run_sync_with(services)?;
        Ok(Arc::new(value))
    }
}

/// A constructor that also receives the build scope, so it can register a
/// finalizer the runtime runs when the built services are released. The pin's
/// `Layer.effect` whose body calls `Effect.addFinalizer`.
struct ScopedCtor<S, E, RIn, F> {
    make: Arc<F>,
    marker: Recipe<S, E, RIn>,
}

impl<S, E, RIn, F> RunStage<E> for ScopedCtor<S, E, RIn, F>
where
    S: Send + Sync + 'static,
    E: Clone + Send + 'static,
    RIn: Requirement,
    F: Fn(&Scope) -> Effect<S, E, RIn> + Send + Sync,
{
    fn run_with(
        self: Box<Self>,
        services: &Services,
        scope: &Scope,
    ) -> Result<Arc<dyn Any + Send + Sync>, Cause<E>> {
        let value = (self.make)(scope).run_sync_with(services)?;
        Ok(Arc::new(value))
    }
}

/// Marker for a layer producing `ROut`, failing with `E`, needing `RIn`.
type LayerMarker<ROut, E, RIn> = PhantomData<fn() -> (ROut, E, RIn)>;

/// A build-time hook: the pin's `Layer.tap*`, which wrap a build rather than
/// add a constructor.
///
/// A tap failure replaces the build's cause, as the pin's `andThen` does: the
/// callback's own failure is what the caller sees. A recovery hook answers a
/// replacement provision and gets the memo map, the scope, and the base
/// provision so the fallback builds exactly where the failed attempt did.
///
/// The hook stores the run, not the description: an `Effect` needs `E: Send`,
/// and the enum has no business forcing that bound on every `Layer`, so the
/// closure that already carries the bound interprets it.
enum Hook<E> {
    /// Runs after a successful build, with the built services.
    Tap(Arc<dyn Fn(&BuiltServices) -> Result<(), Cause<E>> + Send + Sync>),
    /// Runs after a failed build, before the attempt closes.
    OnError(Arc<dyn Fn(&Cause<E>) -> Result<(), Cause<E>> + Send + Sync>),
    /// Runs after a failed build and may answer a replacement provision. A
    /// hook returns `None` when the cause is not its to handle, and the first
    /// hook to answer ends the search.
    Recover(
        Arc<
            dyn Fn(
                    &Cause<E>,
                    &LayerMemo,
                    &Scope,
                    &Services,
                    Option<&Fiber>,
                ) -> Option<Result<Services, Cause<E>>>
                + Send
                + Sync,
        >,
    ),
}

/// A node of the composition tree. Only constructor nodes (`Stage`) are
/// memoized; the pin's `merge`/`provide`/`tap`/`withSpan`/`fresh` are built
/// by `fromBuild`, which runs their subtree again on every request.
struct Node<E> {
    id: u64,
    memoized: bool,
    kind: Kind<E>,
}

enum Kind<E> {
    /// Constructors run in order against the base provision.
    Stages(Vec<Stage<E>>),
    /// Install some stages, then build `inner` over what they produced
    /// (`withSpan`/`withParentSpan`).
    Wrap {
        head: Vec<Stage<E>>,
        inner: Arc<Node<E>>,
    },
    /// `merge`: build both sides, later side wins a duplicate tag.
    Merge(Arc<Node<E>>, Arc<Node<E>>),
    /// `provide`: build the provider, then the consumer over it; the result
    /// keeps both provisions.
    Provide {
        inner: Arc<Node<E>>,
        provider: Arc<Node<E>>,
    },
    /// `fresh`: build the child under a memo map of its own.
    Fresh(Arc<Node<E>>),
    /// `suspend`: evaluate the thunk at build time, then build the layer.
    Suspend(Arc<dyn Fn() -> Arc<Node<E>> + Send + Sync>),
    /// `orDie`: build a child of another error type and turn its typed
    /// failures into defects, so the result can only die or be interrupted.
    OrDie(Arc<dyn ErasedBuild>),
    /// `tap*`/`catch*`: run hooks around one subtree.
    Hook {
        inner: Arc<Node<E>>,
        hooks: Vec<Hook<E>>,
    },
}

impl<E> Node<E> {
    fn new(memoized: bool, kind: Kind<E>) -> Arc<Self> {
        Arc::new(Self {
            id: next_node_id(),
            memoized,
            kind,
        })
    }
}

fn next_node_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A child build erased over its error type, for `orDie`: the wrapper's node
/// is `Node<Infallible>`, but the child it builds fails with `E`.
trait ErasedBuild: Send + Sync {
    fn build_erased(
        &self,
        memo: &LayerMemo,
        scope: &Scope,
        base: &Services,
        ctx: Ctx<'_>,
    ) -> Result<Services, Cause<std::convert::Infallible>>;
}

struct OrDieNode<E> {
    inner: Arc<Node<E>>,
}

impl<E: Clone + std::fmt::Debug + Send + 'static> ErasedBuild for OrDieNode<E> {
    fn build_erased(
        &self,
        memo: &LayerMemo,
        scope: &Scope,
        base: &Services,
        ctx: Ctx<'_>,
    ) -> Result<Services, Cause<std::convert::Infallible>> {
        build_node(&self.inner, memo, scope, base, ctx).map_err(die_like)
    }
}

/// Turn every typed failure into a defect, keeping defects and interrupts.
fn die_like<E: std::fmt::Debug>(cause: Cause<E>) -> Cause<std::convert::Infallible> {
    let mut out = Cause::empty();
    for reason in cause.reasons {
        out.reasons.push(match reason {
            Reason::Fail(error) => Reason::Die(Defect::new(format!("orDie: {error:?}"))),
            Reason::Die(defect) => Reason::Die(defect),
            Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
        });
    }
    out
}

/// Widen an `Infallible` cause back to any `E`: there is no typed failure to
/// carry, so only defects and interrupts survive.
fn widen<E>(cause: Cause<std::convert::Infallible>) -> Cause<E> {
    let mut out = Cause::empty();
    for reason in cause.reasons {
        out.reasons.push(match reason {
            Reason::Fail(never) => match never {},
            Reason::Die(defect) => Reason::Die(defect),
            Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
        });
    }
    out
}

/// The state of a memo entry. `Building` is visible to a second requester,
/// which waits rather than starting a duplicate construction; `Failed` wakes
/// those waiters with the failure that ended the build and leaves the map, so
/// a later requester constructs again (`Layer.test.ts:505`/`:531`).
enum EntryState {
    Building,
    Done(Services),
    Failed {
        interrupted: bool,
        /// The cause that ended the build, type-erased because the map is
        /// keyed by node id and knows nothing of `E`. It holds
        /// `Mutex<Cause<E>>`, so every waiter takes a typed copy of the same
        /// failure rather than a defect standing in for it. `None` when no
        /// cause was published.
        cause: Option<Arc<dyn Any + Send + Sync>>,
    },
}

/// A memoized build: the services, the scope that owns them, and how many
/// caller scopes still observe them.
struct Entry {
    state: Mutex<EntryState>,
    ready: Condvar,
    scope: Arc<Scope>,
    observers: AtomicUsize,
    owner: std::sync::Weak<Mutex<EntryMap>>,
    id: u64,
}

type EntryMap = HashMap<u64, Arc<Entry>>;

/// The fiber a build runs on, when it has one. Constructors see it in their
/// ambient services and the stage loop polls it, which is what lets an
/// interrupt stop a build part-way (the pin's construction fiber).
#[derive(Clone, Copy)]
struct Ctx<'a> {
    fiber: Option<&'a Fiber>,
}

impl<'a> Ctx<'a> {
    fn none() -> Self {
        Self { fiber: None }
    }

    fn of(fiber: &'a Fiber) -> Self {
        Self { fiber: Some(fiber) }
    }

    fn from(fiber: Option<&'a Fiber>) -> Self {
        Self { fiber }
    }
}

/// The interrupt in hand, if the current fiber was asked to stop and is not
/// inside a masked region. Polled between constructors, never during one.
fn interrupted<E: Clone + Send + 'static>(ctx: Ctx<'_>) -> Option<Cause<E>> {
    let fiber = ctx.fiber?;
    if fiber.is_interrupted() && !fiber.is_masked() {
        return Some(Cause::interrupt(Some(fiber.id())));
    }
    None
}

impl Entry {
    /// Wait until the build finishes and answer its provision, or the cause
    /// that ended it.
    fn wait_done<E: Clone + Send + 'static>(&self, ctx: Ctx<'_>) -> Result<Services, Cause<E>> {
        loop {
            let state = self.state.lock().unwrap();
            match &*state {
                EntryState::Done(services) => return Ok(Arc::clone(services)),
                EntryState::Failed { interrupted, cause } => {
                    if let Some(cause) = published_cause::<E>(cause) {
                        return Err(cause);
                    }
                    // A build that failed without a published cause can only
                    // report the shape of its end, not what it carried.
                    return Err(if *interrupted {
                        Cause::interrupt(None)
                    } else {
                        Cause::die(Defect::new("shared layer build failed"))
                    });
                }
                EntryState::Building => {
                    if let Some(cause) = interrupted::<E>(ctx) {
                        return Err(cause);
                    }
                    let (state, _) = self
                        .ready
                        .wait_timeout(state, std::time::Duration::from_millis(20))
                        .unwrap();
                    drop(state);
                }
            }
        }
    }
}

/// A waiter's own copy of the cause that ended a shared build. The entry
/// stores `Mutex<Cause<E>>` erased, so this is the one place that knows the
/// waiter's `E`; the clone is what lets N waiters each own the same failure.
fn published_cause<E: Clone + Send + 'static>(
    published: &Option<Arc<dyn Any + Send + Sync>>,
) -> Option<Cause<E>> {
    let slot = Arc::downcast::<Mutex<Cause<E>>>(Arc::clone(published.as_ref()?)).ok()?;
    let cause = slot.lock().unwrap().clone();
    Some(cause)
}

/// The pin's `MemoMap`: pass one layer's memo to a second build and the
/// second reuses the first's already-constructed services instead of running
/// the constructors again.
///
/// An entry belongs to the scope that built it, and lives until the last
/// caller scope observing it closes. That refcount is what makes a shared
/// layer outlive the first build's scope (the pin's `memoMapObserve`).
/// [`fork`](LayerMemo::fork) adds a child map that reuses the parent's
/// entries but never writes a new one back.
pub struct LayerMemo {
    map: Arc<Mutex<EntryMap>>,
    parent: Option<Arc<LayerMemo>>,
}

impl LayerMemo {
    /// An empty memo: the next build runs every constructor.
    pub fn new() -> Self {
        Self {
            map: Arc::new(Mutex::new(HashMap::new())),
            parent: None,
        }
    }

    /// A child map: entries already present in `self` are reused, entries
    /// first built in the child never appear in the parent (`forkMemoMap`).
    pub fn fork(&self) -> Self {
        Self {
            map: Arc::new(Mutex::new(HashMap::new())),
            parent: Some(Arc::new(self.clone())),
        }
    }

    /// The number of entries this map holds locally (the pin's `map.size`).
    pub fn map_size(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    /// The build a memoized layer already has, as an effect that observes it
    /// into `scope` when it runs (`MemoMap.get`). `None` when no build is
    /// recorded, so a caller can fall back to a fresh build.
    ///
    /// The observation is what keeps the layer alive for this scope: a get
    /// that is never run does not count (`MemoMap` rows at
    /// `Layer.test.ts:554` and `:572`).
    pub fn get<O, E, I>(
        &self,
        layer: &Layer<O, E, I>,
        scope: &Arc<Scope>,
    ) -> Option<Effect<BuiltServices, E, ()>>
    where
        E: Clone + Send + 'static,
    {
        let id = layer.node.id;
        let (_, entry) = self.find(id)?;
        let scope = Arc::clone(scope);
        Some(Effect::suspend(move || {
            match entry.wait_done::<E>(Ctx::none()) {
                Ok(services) => {
                    if !scope.is_closed() {
                        observe_entry(&entry, &scope);
                    }
                    Effect::<BuiltServices, E, ()>::succeed(BuiltServices { values: services })
                }
                Err(cause) => Effect::<BuiltServices, E, ()>::fail_with(cause),
            }
        }))
    }

    fn find(&self, id: u64) -> Option<(Arc<Mutex<EntryMap>>, Arc<Entry>)> {
        if let Some(entry) = self.map.lock().unwrap().get(&id).cloned() {
            return Some((Arc::clone(&self.map), entry));
        }
        self.parent.as_ref().and_then(|parent| parent.find(id))
    }

    /// Start an entry before the constructors run, so a second requester can
    /// see the build in flight and wait on it.
    fn open(&self, id: u64, entry_scope: Arc<Scope>) -> Arc<Entry> {
        let entry = Arc::new(Entry {
            state: Mutex::new(EntryState::Building),
            ready: Condvar::new(),
            scope: entry_scope,
            observers: AtomicUsize::new(1),
            owner: Arc::downgrade(&self.map),
            id,
        });
        self.map.lock().unwrap().insert(id, Arc::clone(&entry));
        entry
    }

    fn finish(&self, entry: &Arc<Entry>, services: Services) {
        *entry.state.lock().unwrap() = EntryState::Done(services);
        entry.ready.notify_all();
    }
}

impl Default for LayerMemo {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LayerMemo {
    fn clone(&self) -> Self {
        Self {
            map: Arc::clone(&self.map),
            parent: self.parent.as_ref().map(Arc::clone),
        }
    }
}

/// Count one observer of `entry` on `scope`; the last one to leave closes the
/// entry's scope and forgets the entry.
fn observe_entry(entry: &Arc<Entry>, scope: &Scope) {
    entry.observers.fetch_add(1, Ordering::SeqCst);
    register_observer(entry, scope);
}

fn register_observer(entry: &Arc<Entry>, scope: &Scope) {
    let held = Arc::clone(entry);
    scope.add_finalizer(move || {
        if held.observers.fetch_sub(1, Ordering::SeqCst) == 1 {
            if let Some(map) = held.owner.upgrade() {
                map.lock().unwrap().remove(&held.id);
            }
            held.scope.close();
        }
    });
}

/// Built services: the values one closed layer produced.
pub struct BuiltServices {
    values: Services,
}

impl BuiltServices {
    /// The built value behind tag `T`, if this build provided it.
    pub fn get<T: ServiceTag>(&self) -> Option<T::Service> {
        self.values
            .get_tag(T::ID)?
            .downcast_ref::<T::Service>()
            .cloned()
    }

    /// The whole built provision, for a handler that boots a runtime from a
    /// layer and then hands the layer to a static interpreter: see
    /// [`Effect::provide_from`]. Crate-private so `Services` stays internal.
    pub(crate) fn env(&self) -> Services {
        Arc::clone(&self.values)
    }
}

/// A composable service recipe: builds `ROut` from `RIn`.
///
/// A handle carries its node in the composition tree plus the memo a bare
/// [`build`](Layer::build) uses. Cloning a handle shares both, which is this
/// port's stand-in for the pin's reference identity: `layer.merge(layer)`
/// asks for one build because both sides name the same node.
pub struct Layer<ROut, E, RIn> {
    node: Arc<Node<E>>,
    out_ids: Vec<&'static str>,
    in_ids: Vec<&'static str>,
    memo: LayerMemo,
    marker: LayerMarker<ROut, E, RIn>,
}

impl<O, E, I> Clone for Layer<O, E, I> {
    fn clone(&self) -> Self {
        Self {
            node: Arc::clone(&self.node),
            out_ids: self.out_ids.clone(),
            in_ids: self.in_ids.clone(),
            memo: self.memo.clone(),
            marker: PhantomData,
        }
    }
}

impl<O: RequirementIds, E, I: RequirementIds> std::fmt::Debug for Layer<O, E, I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("provides", &self.out_ids)
            .field("needs", &self.in_ids)
            .finish()
    }
}

impl std::fmt::Debug for BuiltServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltServices")
            .field("services", &self.values.len())
            .finish()
    }
}

impl<T, E, RIn> Layer<T, E, RIn>
where
    T: ServiceTag,
    RIn: RequirementIds,
{
    /// A layer providing one tag. The factory runs once per memoized identity
    /// (never again across a shared [`LayerMemo`]); a tag that requires
    /// itself is a [`Cycle`].
    pub fn from_factory<S, F>(make: F) -> Result<Self, Cycle>
    where
        E: Clone + Send + 'static,
        T: ServiceTag<Service = S>,
        S: Send + Sync + 'static,
        F: Fn() -> Effect<S, E, RIn> + Send + Sync + 'static,
    {
        Self::build_factory(make, true)
    }

    /// Like [`from_factory`](Layer::from_factory), but the constructor also
    /// receives the build scope, so it can register finalizers that run when
    /// the built services are released. This is what a layer needs to own a
    /// resource (the pin's `Layer.effect` with `Effect.addFinalizer`).
    pub fn from_scoped_factory<S, F>(make: F) -> Result<Self, Cycle>
    where
        E: Clone + Send + 'static,
        T: ServiceTag<Service = S>,
        S: Send + Sync + 'static,
        F: Fn(&Scope) -> Effect<S, E, RIn> + Send + Sync + 'static,
    {
        let in_ids = RIn::ids();
        if in_ids.contains(&T::ID) {
            return Err(Cycle { path: vec![T::ID] });
        }
        let make: Arc<F> = Arc::new(make);
        let stage = Stage {
            out_id: T::ID,
            in_ids: in_ids.clone(),
            make: Arc::new(move || {
                Box::new(ScopedCtor {
                    make: Arc::clone(&make),
                    marker: PhantomData,
                }) as Box<dyn RunStage<E> + Send>
            }),
        };
        Ok(Layer {
            node: Node::new(true, Kind::Stages(vec![stage])),
            out_ids: vec![T::ID],
            in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }

    fn build_factory<S, F>(make: F, memoized: bool) -> Result<Self, Cycle>
    where
        E: Clone + Send + 'static,
        T: ServiceTag<Service = S>,
        S: Send + Sync + 'static,
        F: Fn() -> Effect<S, E, RIn> + Send + Sync + 'static,
    {
        let in_ids = RIn::ids();
        if in_ids.contains(&T::ID) {
            return Err(Cycle { path: vec![T::ID] });
        }
        let make: Arc<F> = Arc::new(make);
        let stage = Stage {
            out_id: T::ID,
            in_ids: in_ids.clone(),
            make: Arc::new(move || {
                Box::new(Ctor {
                    make: Arc::clone(&make),
                    marker: PhantomData,
                }) as Box<dyn RunStage<E> + Send>
            }),
        };
        Ok(Layer {
            node: Node::new(memoized, Kind::Stages(vec![stage])),
            out_ids: vec![T::ID],
            in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

impl<O, E, I> Layer<O, E, I> {
    /// The same recipe, but every build reconstructs it: constructors inside
    /// it run again, and it shares nothing with the caller's memo map
    /// (`Layer.fresh`).
    pub fn fresh(&self) -> Self {
        Self {
            node: Node::new(false, Kind::Fresh(Arc::clone(&self.node))),
            out_ids: self.out_ids.clone(),
            in_ids: self.in_ids.clone(),
            memo: LayerMemo::new(),
            marker: PhantomData,
        }
    }

    /// A handle onto this layer's memo. Share it with
    /// [`with_memo`](Layer::with_memo) so a second layer reuses the services
    /// this one already built, the pin's `{ memoMap: other.memoMap }`.
    pub fn memo(&self) -> LayerMemo {
        self.memo.clone()
    }

    /// The same recipe on a supplied memo. A memo that already holds a build
    /// answers [`build`](Layer::build) with those services and runs no
    /// constructor again.
    pub fn with_memo(&self, memo: &LayerMemo) -> Self {
        Self {
            node: Arc::clone(&self.node),
            out_ids: self.out_ids.clone(),
            in_ids: self.in_ids.clone(),
            memo: memo.clone(),
            marker: PhantomData,
        }
    }

    /// Build this layer under a new span (`Layer.withSpan`): the span is
    /// created before the layer's own nodes build, so every constructor of
    /// `self` sees it as the ambient parent, and it ends when the build
    /// scope closes.
    pub fn with_span(self, name: impl Into<String>, options: LayerSpanOptions) -> Self
    where
        E: Clone + Send + 'static,
    {
        let head = vec![span_stage(name.into(), options)];
        self.wrap(head)
    }

    /// `Layer.withParentSpan`: install an existing span as the ambient parent
    /// for this layer's constructors (`Layer.test.ts:740`). The span is not
    /// created or ended here; building installs it as both the
    /// [`ParentSpanTag`] service and the ambient [`CURRENT_SPAN`] reference.
    pub fn with_parent_span(self, span: AnySpan) -> Self
    where
        E: Clone + Send + 'static,
    {
        let head = vec![parent_span_stage(span)];
        self.wrap(head)
    }

    fn wrap(self, head: Vec<Stage<E>>) -> Self {
        let inner = Arc::clone(&self.node);
        Self {
            node: Node::new(false, Kind::Wrap { head, inner }),
            out_ids: self.out_ids,
            in_ids: self.in_ids,
            memo: self.memo,
            marker: PhantomData,
        }
    }
}

impl<T: ServiceTag> Layer<T, std::convert::Infallible, ()>
where
    T::Service: crate::mock::Mock,
{
    /// `Layer.mock`: a test layer from a partial implementation
    /// (`Layer.test.ts:402`). `missing` names the members the implementation
    /// left out; each becomes an [`Unimplemented`](crate::Unimplemented)
    /// placeholder, so exercising one fails naming the service and the member
    /// instead of quietly answering a wrong value.
    ///
    /// The pin finds the omitted keys through a Proxy. Rust cannot see a
    /// struct's fields, so the caller names them: that is the whole difference,
    /// and it is why `missing` exists at all.
    ///
    /// A mock never fails to build, so its construction error is `Infallible`,
    /// the pin's `Layer<I>` with no error channel.
    pub fn mock(implementation: T::Service, missing: &'static [&'static str]) -> Self {
        Self::from_value(implementation.fill_missing(T::ID, missing))
    }
}

impl<T: ServiceTag, E: Clone + Send + 'static> Layer<T, E, ()> {
    /// A layer providing a static value. Never cycles, never fails, and adds
    /// no resource of its own (`Layer.succeed`).
    pub fn from_value(service: T::Service) -> Self {
        Self::build_factory(move || Effect::succeed(service.clone()), false)
            .expect("a closed value layer cannot cycle")
    }
}

impl Layer<ParentSpanTag, std::convert::Infallible, ()> {
    /// Provide an existing span as the parent for layers built beside this
    /// one (`Layer.parentSpan`). The span is not created, ended, or owned
    /// here; building this layer only makes it ambient.
    pub fn parent_span(span: AnySpan) -> Self {
        Self::from_value(span)
    }

    /// Create a span while this layer builds, make it the ambient parent for
    /// the rest of the build, and end it when the build scope closes
    /// (`Layer.span`). `on_end` runs at that close, before the span ends.
    pub fn span(name: impl Into<String>, options: LayerSpanOptions) -> Self {
        let stage = span_stage(name.into(), options);
        Self {
            node: Node::new(true, Kind::Stages(vec![stage])),
            out_ids: vec![ParentSpanTag::ID],
            in_ids: Vec::new(),
            memo: LayerMemo::new(),
            marker: PhantomData,
        }
    }
}

impl Layer<TracerTag, std::convert::Infallible, ()> {
    /// Provide a tracer as the ambient backend for layers built beside this
    /// one and for effects run through them. The pin's `Layer.effect` over
    /// `Tracer.Tracer`.
    pub fn tracer(tracer: Arc<dyn Tracer>) -> Self {
        Self::from_value(tracer)
    }
}

impl<O1: RequirementIds, E: Clone + Send + 'static, I1: RequirementIds> Layer<O1, E, I1> {
    /// Wire `other` through `self`: `other` must require exactly what `self`
    /// provides. The graph is walked before anything is returned, so a cycle
    /// fails here with [`Cycle`] and no constructor runs. Needs that `self`
    /// leaves open stay open on the result.
    pub fn provide_merge<O2: RequirementIds>(
        self,
        other: Layer<O2, E, O1>,
    ) -> Result<Layer<O2, E, I1>, Cycle> {
        let node = Node::new(
            false,
            Kind::Provide {
                inner: Arc::clone(&other.node),
                provider: Arc::clone(&self.node),
            },
        );
        check_cycle(&flatten(&node))?;
        Ok(Layer {
            node,
            out_ids: other.out_ids,
            in_ids: self.in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

impl<O1: RequirementIds, E: Clone + Send + 'static, I1: RequirementIds> Layer<O1, E, I1> {
    /// `Layer.merge`: build both layers on one handle and answer both
    /// provisions.
    ///
    /// Both sides build at once, each on its own construction fiber, and their
    /// provisions are merged with the later side winning a doubled tag, as
    /// `Context.merge` does at the pin. A handle merged with itself still
    /// builds once: one side constructs and the other waits on the same memo
    /// entry.
    ///
    /// A build with no construction fiber (a synchronous `build` with no
    /// runtime fiber to hang a child on) falls back to building in order, so
    /// a side that never completes blocks it. Build on a fiber
    /// ([`build_with_fiber`](Layer::build_with_fiber)) when the sides are
    /// independent and either may take its time.
    pub fn merge<O2: RequirementIds>(
        self,
        other: Layer<O2, E, I1>,
    ) -> Result<Layer<Cons<O1, O2>, E, I1>, Cycle> {
        let node = Node::new(
            false,
            Kind::Merge(Arc::clone(&self.node), Arc::clone(&other.node)),
        );
        check_cycle(&flatten(&node))?;
        let mut out_ids = self.out_ids;
        out_ids.extend(other.out_ids);
        Ok(Layer {
            node,
            out_ids,
            in_ids: self.in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

impl<O1: RequirementIds, E: Clone + Send + 'static, O3: RequirementIds> Layer<O3, E, O1> {
    /// `Layer.provide(provider)` between two layers: build `provider` first,
    /// then this layer over its services (the pin's `provide`, not
    /// `provideMerge`).
    ///
    /// The provider builds first because this layer's constructors read its
    /// tags; when the scope closes, the LIFO order releases this layer before
    /// the provider it consumed. The provider's tags stay in `BuiltServices`:
    /// this port's build answers everything it constructed, so hiding is a
    /// typing fact (`out_ids` names only this layer's provision), not a value
    /// filter. Filtering would strip the tracer and span references a
    /// provider installs.
    pub fn provide_layer<O2: RequirementIds>(
        self,
        provider: Layer<O1, E, O2>,
    ) -> Result<Layer<O3, E, O2>, Cycle> {
        let node = Node::new(
            false,
            Kind::Provide {
                inner: Arc::clone(&self.node),
                provider: Arc::clone(&provider.node),
            },
        );
        check_cycle(&flatten(&node))?;
        Ok(Layer {
            node,
            out_ids: self.out_ids,
            in_ids: provider.in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

impl<O1: RequirementIds, E: Clone + Send + 'static, I1: RequirementIds> Layer<O1, E, I1> {
    /// The pin's `Layer.provide` with a provider this layer does **not**
    /// require: the provider still builds, first, and its services join the
    /// provision, so a build of two `fresh` copies of one layer acquires
    /// twice (`Layer.test.ts:264`).
    ///
    /// The pin allows this because its `provide` only asks that the
    /// provider's output be available, never that the consumer use it; here
    /// [`provide_layer`](Layer::provide_layer) demands that the consumer's
    /// requirements *be* the provider's output, so a provider nothing needs
    /// has no spelling without this method.
    ///
    /// Both layers build at the same level, so the provider must leave open
    /// exactly what this layer leaves open.
    pub fn install_layer<P: RequirementIds>(
        self,
        provider: Layer<P, E, I1>,
    ) -> Result<Layer<Cons<P, O1>, E, I1>, Cycle> {
        let node = Node::new(
            false,
            Kind::Provide {
                inner: Arc::clone(&self.node),
                provider: Arc::clone(&provider.node),
            },
        );
        check_cycle(&flatten(&node))?;
        let mut out_ids = provider.out_ids;
        out_ids.extend(self.out_ids);
        Ok(Layer {
            node,
            out_ids,
            in_ids: self.in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

impl<O: RequirementIds, E: Clone + Send + 'static> Layer<O, E, ()> {
    /// `Layer.launch`: build the layer, keep its scope open, and never
    /// complete on its own. An interrupt closes the scope, running every
    /// finalizer the build registered, then ends the effect interrupted
    /// (`Layer.test.ts:389`).
    ///
    /// A construction failure fails the effect and closes what the attempt
    /// acquired, exactly as a scoped build does.
    pub fn launch(self) -> Effect<std::convert::Infallible, E, ()> {
        let node = self.node;
        let memo = self.memo;
        Effect::suspend(move || {
            let scope = Arc::new(Scope::new(crate::fiber::next_id().0));
            match build_root(&node, &memo, &scope, Ctx::none()) {
                Ok(_) => {
                    let parked = Arc::clone(&scope);
                    Effect::with_fiber(move |fiber| {
                        let fiber = fiber.clone();
                        let parked = Arc::clone(&parked);
                        Effect::<std::convert::Infallible, E, ()>::suspend(move || {
                            while !fiber.is_interrupted() {
                                std::thread::park_timeout(std::time::Duration::from_millis(1));
                            }
                            parked.close();
                            Effect::<std::convert::Infallible, E, ()>::interrupt()
                        })
                    })
                }
                Err(cause) => {
                    scope.close();
                    Effect::<std::convert::Infallible, E, ()>::fail_with(cause)
                }
            }
        })
    }

    /// Run every constructor this build needs. A constructor failure fails
    /// the build with the constructor's cause, and closes `scope`: a caller
    /// that handed a scope over owns no half-built layer.
    ///
    /// Memoized nodes run at most once per memo map; a node that fails
    /// leaves no entry, so a retry builds again.
    pub fn build(&self, scope: &Scope) -> Result<BuiltServices, Cause<E>> {
        self.build_in(&self.memo, scope, Ctx::none())
    }

    /// Build with a construction fiber: an interrupt requested on `fiber`
    /// stops the build between constructors, closes what the attempt
    /// acquired, and ends the build interrupted (`Layer.test.ts:292`/:307/
    /// :322). The constructors also see the fiber ambiently, so an effect
    /// inside one polls the same flag.
    pub fn build_with_fiber(
        &self,
        fiber: &Fiber,
        scope: &Scope,
    ) -> Result<BuiltServices, Cause<E>> {
        self.build_in(&self.memo, scope, Ctx::of(fiber))
    }

    /// [`build_with_fiber`](Layer::build_with_fiber) on an explicit memo map,
    /// so a waiter can share one in-flight build (`Layer.test.ts:505`).
    pub fn build_with_fiber_and_memo(
        &self,
        fiber: &Fiber,
        memo: &LayerMemo,
        scope: &Scope,
    ) -> Result<BuiltServices, Cause<E>> {
        self.build_in(memo, scope, Ctx::of(fiber))
    }

    fn build_in(
        &self,
        memo: &LayerMemo,
        scope: &Scope,
        ctx: Ctx<'_>,
    ) -> Result<BuiltServices, Cause<E>> {
        match build_root(&self.node, memo, scope, ctx) {
            Ok(built) => Ok(built),
            Err(cause) => {
                scope.close();
                Err(cause)
            }
        }
    }

    /// Build against an explicit memo map, as an effect (`buildWithMemoMap`).
    ///
    /// The description is lazy: calling this writes nothing, so a build that
    /// is never run leaves the map empty (`Layer.test.ts:586`).
    pub fn build_with_memo(
        &self,
        memo: &LayerMemo,
        scope: &Arc<Scope>,
    ) -> Effect<BuiltServices, E, ()> {
        let node = Arc::clone(&self.node);
        let memo = memo.clone();
        let scope = Arc::clone(scope);
        Effect::suspend(
            move || match build_root(&node, &memo, &scope, Ctx::none()) {
                Ok(built) => Effect::<BuiltServices, E, ()>::succeed(built),
                Err(cause) => Effect::<BuiltServices, E, ()>::fail_with(cause),
            },
        )
    }
}

impl<T: ServiceTag, E: Clone + Send + 'static> Layer<T, E, ()> {
    /// Wire a closed layer into an effect and hide its tag: the effect keeps
    /// its other requirements, `T` is satisfied from a build.
    ///
    /// A double-provision this call would create (the effect already carries
    /// the tag with a distinct value) is returned as an error here, at
    /// construction, instead of Dying at interpretation.
    pub fn provide<A, E2, R2, Rest>(
        &self,
        effect: Effect<A, E2, R2>,
        scope: &Scope,
    ) -> Result<Effect<A, E2, Rest>, Cause<E>>
    where
        A: Send + 'static,
        E2: Send + 'static,
        R2: Provide<T, Rest>,
        Rest: Requirement,
    {
        let services = self.merged_into(&effect.services, scope)?;
        Ok(Effect {
            step: effect.step,
            services,
            marker: PhantomData,
        })
    }

    /// Merge everything a build produced into `effect` without changing its
    /// requirement type. The span and tracer layers install references rather
    /// than satisfying a required tag, so they use this instead of
    /// [`provide`](Layer::provide).
    pub fn install<A, E2, R2>(
        &self,
        effect: Effect<A, E2, R2>,
        scope: &Scope,
    ) -> Result<Effect<A, E2, R2>, Cause<E>>
    where
        A: Send + 'static,
        E2: Send + 'static,
        R2: Requirement,
    {
        let services = self.merged_into(&effect.services, scope)?;
        Ok(Effect {
            step: effect.step,
            services,
            marker: PhantomData,
        })
    }

    /// Build once, then merge the tags and references into `own`.
    fn merged_into(&self, own: &Services, scope: &Scope) -> Result<Services, Cause<E>> {
        let built = self.build(scope)?;
        let mut maps = (**own).clone();
        for (id, value, conflicted) in built.values.tag_list() {
            if maps.insert_tag(id, value) {
                return Err(Cause::die(Defect::new(format!(
                    "double-provision of `{id}` with distinct values: provide each tag once"
                ))));
            }
            // A conflict inside the built services must not vanish into the
            // effect's map: ambiguity is a property of the provision, and the
            // law is that the run Dies rather than picking a winner.
            if conflicted {
                maps.poison(id);
            }
        }
        // References override, so merging them is always defined: the newly
        // built layer is the inner provider.
        for (id, value) in built.values.ref_list() {
            maps.insert_ref(id, value);
        }
        Ok(Arc::new(maps))
    }
}

impl<O: RequirementIds, E: Clone + Send + 'static> Layer<O, E, ()> {
    /// `Layer.catch`: build an alternative layer when this one fails with a
    /// typed error. Defects and interruptions are not caught, as the pin's
    /// `catch_` does not catch them; use [`catch_cause`](Layer::catch_cause)
    /// for those.
    pub fn catch<F>(self, on_error: F) -> Self
    where
        F: Fn(&E) -> Self + Send + Sync + 'static,
    {
        self.with_hook(Hook::Recover(Arc::new(
            move |cause: &Cause<E>,
                  memo: &LayerMemo,
                  scope: &Scope,
                  base: &Services,
                  fiber: Option<&Fiber>| {
                cause.failure().map(|error| {
                    build_node(&on_error(error).node, memo, scope, base, Ctx::from(fiber))
                })
            },
        )))
    }

    /// `Layer.catchCause`: build an alternative layer for any construction
    /// failure and hand the handler the whole cause.
    pub fn catch_cause<F>(self, on_cause: F) -> Self
    where
        F: Fn(&Cause<E>) -> Self + Send + Sync + 'static,
    {
        self.with_hook(Hook::Recover(Arc::new(
            move |cause: &Cause<E>,
                  memo: &LayerMemo,
                  scope: &Scope,
                  base: &Services,
                  fiber: Option<&Fiber>| {
                Some(build_node(
                    &on_cause(cause).node,
                    memo,
                    scope,
                    base,
                    Ctx::from(fiber),
                ))
            },
        )))
    }
}

impl<O: RequirementIds, E: Clone + crate::tagged::TaggedError + Send + 'static> Layer<O, E, ()> {
    /// `Layer.catchTag`: build an alternative layer only when the failure
    /// carries `tag` (`Layer.test.ts:143`). A defect, an interruption, or a
    /// typed failure with another tag passes through as it was; use
    /// [`catch`](Layer::catch) to recover every typed failure.
    ///
    /// The handler sees the error value, so it can read the fields the tag
    /// brought with it; the fallback then builds exactly where the failed
    /// attempt did.
    pub fn catch_tag<F>(self, tag: &'static str, on_error: F) -> Self
    where
        F: Fn(&E) -> Self + Send + Sync + 'static,
    {
        self.with_hook(Hook::Recover(Arc::new(
            move |cause: &Cause<E>,
                  memo: &LayerMemo,
                  scope: &Scope,
                  base: &Services,
                  fiber: Option<&Fiber>| {
                cause
                    .failure()
                    .filter(|error| error.tag() == tag)
                    .map(|error| {
                        build_node(&on_error(error).node, memo, scope, base, Ctx::from(fiber))
                    })
            },
        )))
    }
}

impl<O, E: Clone + Send + std::fmt::Debug + 'static, I> Layer<O, E, I> {
    /// `Layer.orDie`: narrow the construction error to `never` by turning a
    /// typed failure into a defect. The child keeps its identity, so wrapping
    /// it with `orDie` does not break sharing with a plain build of the same
    /// layer (`Layer.test.ts:368`).
    pub fn or_die(self) -> Layer<O, std::convert::Infallible, I> {
        Layer {
            node: Node::new(false, Kind::OrDie(Arc::new(OrDieNode { inner: self.node }))),
            out_ids: self.out_ids,
            in_ids: self.in_ids,
            memo: self.memo,
            marker: PhantomData,
        }
    }
}

impl<O, E: Clone + Send + 'static, I> Layer<O, E, I> {
    fn with_hook(self, hook: Hook<E>) -> Self {
        Self {
            node: Node::new(
                false,
                Kind::Hook {
                    inner: Arc::clone(&self.node),
                    hooks: vec![hook],
                },
            ),
            out_ids: self.out_ids,
            in_ids: self.in_ids,
            memo: self.memo,
            marker: PhantomData,
        }
    }

    /// `Layer.tap`: run an effect with the built services when the build
    /// succeeds, and answer the same services.
    ///
    /// The pin hands the callback a `Context` and widens `E` with the
    /// callback's error; here the callback reads [`BuiltServices`] and shares
    /// the layer's error type, because Rust has no union to widen into.
    pub fn tap<F>(self, f: F) -> Self
    where
        F: Fn(&BuiltServices) -> Effect<(), E, ()> + Send + Sync + 'static,
    {
        self.with_hook(Hook::Tap(Arc::new(move |built| f(built).run_sync())))
    }

    /// `Layer.tapError`: run an effect with the typed construction failure,
    /// then fail with that same error unless the callback itself fails.
    pub fn tap_error<F>(self, f: F) -> Self
    where
        F: Fn(&E) -> Effect<(), E, ()> + Send + Sync + 'static,
    {
        self.with_hook(Hook::OnError(Arc::new(
            move |cause: &Cause<E>| match cause.failure() {
                Some(error) => f(error).run_sync(),
                None => Ok(()),
            },
        )))
    }

    /// `Layer.tapCause`: run an effect with the whole cause, defects and
    /// interruptions included, then fail with that same cause unless the
    /// callback itself fails.
    pub fn tap_cause<F>(self, f: F) -> Self
    where
        F: Fn(&Cause<E>) -> Effect<(), E, ()> + Send + Sync + 'static,
    {
        self.with_hook(Hook::OnError(Arc::new(move |cause| f(cause).run_sync())))
    }
}

impl<O: RequirementIds + 'static, E: Clone + Send + 'static> Layer<O, E, ()> {
    /// `Layer.suspend`: evaluate the thunk at build time, once per memoized
    /// identity, then build the layer it answers.
    pub fn suspend<F>(evaluate: F) -> Self
    where
        F: Fn() -> Self + Send + Sync + 'static,
    {
        let evaluate: Arc<dyn Fn() -> Arc<Node<E>> + Send + Sync> =
            Arc::new(move || Arc::clone(&evaluate().node));
        Self {
            node: Node::new(true, Kind::Suspend(evaluate)),
            out_ids: O::ids(),
            in_ids: Vec::new(),
            memo: LayerMemo::new(),
            marker: PhantomData,
        }
    }
}

/// Build a root: no base provision, and the caller's scope observes the top
/// entry.
fn build_root<E: Clone + Send + 'static>(
    node: &Arc<Node<E>>,
    memo: &LayerMemo,
    scope: &Scope,
    ctx: Ctx<'_>,
) -> Result<BuiltServices, Cause<E>> {
    let base = empty_services();
    let values = build_node(node, memo, scope, &base, ctx)?;
    Ok(BuiltServices { values })
}

/// Build one node under `memo`. A memoized node answers from its entry when
/// one exists, waits when one is in flight, and otherwise opens an entry
/// before its constructors run so a second requester can see the build. A
/// plain node builds into a forked scope, so a failure releases exactly what
/// it acquired.
fn build_node<E: Clone + Send + 'static>(
    node: &Arc<Node<E>>,
    memo: &LayerMemo,
    scope: &Scope,
    base: &Services,
    ctx: Ctx<'_>,
) -> Result<Services, Cause<E>> {
    if let Some(cause) = interrupted::<E>(ctx) {
        return Err(cause);
    }
    if !node.memoized {
        let attempt = scope.fork();
        return match run_kind(node, memo, &attempt, base, ctx) {
            Ok(services) => Ok(services),
            Err(cause) => {
                attempt.close();
                Err(cause)
            }
        };
    }
    if let Some((_, entry)) = memo.find(node.id) {
        let services = entry.wait_done(ctx)?;
        if !scope.is_closed() {
            observe_entry(&entry, scope);
        }
        return Ok(services);
    }
    if scope.is_closed() {
        let attempt = scope.fork();
        return run_kind(node, memo, &attempt, base, ctx);
    }
    let entry_scope = Arc::new(Scope::new(crate::fiber::next_id().0));
    let entry = memo.open(node.id, Arc::clone(&entry_scope));
    match run_kind(node, memo, &entry_scope, base, ctx) {
        Ok(services) => {
            memo.finish(&entry, Arc::clone(&services));
            register_observer(&entry, scope);
            Ok(services)
        }
        Err(cause) => {
            let was_interrupted = cause.is_interrupted();
            // Publish before waking: a waiter that wakes on `Failed` must
            // find the cause already there, or it reports the bare shape.
            let published: Arc<dyn Any + Send + Sync> = Arc::new(Mutex::new(cause.clone()));
            *entry.state.lock().unwrap() = EntryState::Failed {
                interrupted: was_interrupted,
                cause: Some(published),
            };
            entry.ready.notify_all();
            if let Some(map) = entry.owner.upgrade() {
                map.lock().unwrap().remove(&node.id);
            }
            entry_scope.close();
            Err(cause)
        }
    }
}

fn run_kind<E: Clone + Send + 'static>(
    node: &Arc<Node<E>>,
    memo: &LayerMemo,
    scope: &Scope,
    base: &Services,
    ctx: Ctx<'_>,
) -> Result<Services, Cause<E>> {
    match &node.kind {
        Kind::Stages(stages) => run_stages(stages, base, scope, ctx),
        Kind::Wrap { head, inner } => {
            let installed = run_stages(head, base, scope, ctx)?;
            let over = overlay(base, &installed);
            let built = build_node(inner, memo, scope, &over, ctx)?;
            Ok(overlay(&installed, &built))
        }
        Kind::Merge(left, right) => match ctx.fiber {
            Some(parent) => merge_parallel(parent, left, right, memo, scope, base),
            // No construction fiber means nowhere to run the second side, so
            // the sides build in order. `merge` says what that costs.
            None => {
                let first = build_node(left, memo, scope, base, ctx)?;
                let second = build_node(right, memo, scope, base, ctx)?;
                Ok(overlay(&first, &second))
            }
        },
        Kind::Provide { inner, provider } => {
            let provided = build_node(provider, memo, scope, base, ctx)?;
            let over = overlay(base, &provided);
            let built = build_node(inner, memo, scope, &over, ctx)?;
            Ok(overlay(&built, &provided))
        }
        Kind::Fresh(inner) => build_node(inner, &LayerMemo::new(), scope, base, ctx),
        Kind::OrDie(inner) => inner
            .build_erased(memo, scope, base, ctx)
            .map_err(widen::<E>),
        Kind::Suspend(evaluate) => {
            let child = evaluate();
            build_node(&child, memo, scope, base, ctx)
        }
        Kind::Hook { inner, hooks } => match build_node(inner, memo, scope, base, ctx) {
            Ok(services) => {
                let built = BuiltServices {
                    values: Arc::clone(&services),
                };
                for hook in hooks {
                    if let Hook::Tap(f) = hook {
                        f(&built)?;
                    }
                }
                Ok(services)
            }
            Err(cause) => {
                let mut cause = cause;
                for hook in hooks {
                    if let Hook::OnError(f) = hook
                        && let Err(raised) = f(&cause)
                    {
                        cause = raised;
                    }
                }
                for hook in hooks {
                    if let Hook::Recover(f) = hook
                        && let Some(outcome) = f(&cause, memo, scope, base, ctx.fiber)
                    {
                        return outcome;
                    }
                }
                Err(cause)
            }
        },
    }
}

/// Build both sides of a merge at once, each on its own construction fiber
/// (`Layer.test.ts:15`). A side that never completes on its own would hang a
/// sequential build before the other side ever ran, so the latch the pin's
/// witness opens from the second side could never open.
///
/// An interrupt of the parent, or a failure on either side, reaches both
/// children: the build never waits out a side that has nothing left to say.
/// A real failure outranks the parent's interrupt, because a failure is the
/// stronger fact and the interrupt may only be how the other side ended.
fn merge_parallel<E: Clone + Send + 'static>(
    parent: &Fiber,
    left: &Arc<Node<E>>,
    right: &Arc<Node<E>>,
    memo: &LayerMemo,
    scope: &Scope,
    base: &Services,
) -> Result<Services, Cause<E>> {
    let (tx, rx) = mpsc::channel::<(u8, Result<Services, Cause<E>>)>();
    // One child scope owns both sides, forked at the merge's position in the
    // caller's LIFO order, so a release still happens when the caller's scope
    // closes rather than when the build returns.
    let child_scope = scope.fork();
    let spawn = |node: Arc<Node<E>>, slot: u8| {
        let tx = tx.clone();
        let memo = memo.clone();
        let scope = Arc::clone(&child_scope);
        let base = Arc::clone(base);
        parent.fork_child(
            move |ctx| {
                let fiber = ctx.fiber();
                let out = build_node(&node, &memo, &scope, &base, Ctx::of(&fiber));
                let _ = tx.send((slot, out));
                Exit::<(), E>::succeed(())
            },
            ForkOpts::immediate(),
        )
    };
    let first = spawn(Arc::clone(left), 0);
    let second = spawn(Arc::clone(right), 1);
    // A panicking child would otherwise leave the loop waiting on a sender it
    // will never use.
    drop(tx);

    let mut first_out = None;
    let mut second_out = None;
    let mut stopped = None;
    while first_out.is_none() || second_out.is_none() {
        if stopped.is_none() && parent.is_interrupted() && !parent.is_masked() {
            stopped = Some(Cause::interrupt(Some(parent.id())));
            first.interrupt();
            second.interrupt();
        }
        match rx.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok((0, out)) => {
                first_out = Some(out);
                if first_out.as_ref().is_some_and(Result::is_err) {
                    second.interrupt();
                }
            }
            Ok((_, out)) => {
                second_out = Some(out);
                if second_out.as_ref().is_some_and(Result::is_err) {
                    first.interrupt();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // A child that never sent ended in a panic; its handle carries the cause.
    let first_out = first_out.unwrap_or_else(|| Err(handle_failure(first.join())));
    let second_out = second_out.unwrap_or_else(|| Err(handle_failure(second.join())));

    let (first_ok, first_err) = match first_out {
        Ok(services) => (Some(services), None),
        Err(cause) => (None, Some(cause)),
    };
    let (second_ok, second_err) = match second_out {
        Ok(services) => (Some(services), None),
        Err(cause) => (None, Some(cause)),
    };
    // A real failure outranks an interrupt-only outcome: the interrupt may
    // only be how the other side ended, while the failure is the fact worth
    // keeping (`Layer.test.ts:44`).
    let cause = match (first_err, second_err) {
        (Some(first), Some(second)) => {
            Some(if first.is_interrupted() && !second.is_interrupted() {
                second
            } else {
                first
            })
        }
        (first, second) => first.or(second),
    };
    if let Some(cause) = cause {
        return match stopped {
            Some(stopped) if cause.is_interrupted() => Err(stopped),
            _ => Err(cause),
        };
    }
    match (first_ok, second_ok) {
        (Some(first), Some(second)) => Ok(overlay(&first, &second)),
        _ => Err(Cause::die(Defect::new(
            "a merge side ended without a build result",
        ))),
    }
}

/// A merge side's handle outcome when it sent no build result: a panic is the
/// only way here, and its cause is what the build should report.
fn handle_failure<E>(outcome: Exit<(), E>) -> Cause<E> {
    match outcome {
        Exit::Failure(cause) => cause,
        Exit::Success(()) => Cause::die(Defect::new("a merge side ended without a build result")),
    }
}

/// Run a node's own constructors over `base`. The answer holds only what
/// these stages produced, so a memoized entry never captures a caller's
/// provision; later stages see earlier ones through `base` plus this run.
///
/// The construction fiber, when there is one, rides in the ambient services
/// and is polled before every constructor: a build stops at the next boundary
/// after an interrupt, never mid-constructor.
fn run_stages<E: Clone + Send + 'static>(
    stages: &[Stage<E>],
    base: &Services,
    scope: &Scope,
    ctx: Ctx<'_>,
) -> Result<Services, Cause<E>> {
    let mut acc = Arc::clone(base);
    if let Some(fiber) = ctx.fiber {
        let mut maps = (*acc).clone();
        maps.replace_tag(<FiberTag as ServiceTag>::ID, Arc::new(fiber.clone()));
        acc = Arc::new(maps);
    }
    let mut own = ServiceMaps::default();
    let mut produced: HashSet<&'static str> = HashSet::new();
    for stage in stages {
        if let Some(cause) = interrupted::<E>(ctx) {
            return Err(cause);
        }
        let runner = (stage.make)();
        let value = runner.run_with(&acc, scope)?;
        if !produced.insert(stage.out_id) {
            return Err(double_provision(stage.out_id));
        }
        let mut next = (*acc).clone();
        next.replace_tag(stage.out_id, Arc::clone(&value));
        own.replace_tag(stage.out_id, Arc::clone(&value));
        // Tracing layers name a service and an ambient reference; building
        // one installs both, so later stages see the span or tracer without
        // reading the tag. The references hold `Option<T>`, because an absent
        // value and an unset reference differ: `None` shadows an outer span,
        // no reference does not.
        if stage.out_id == ParentSpanTag::ID
            && let Some(span) = value.downcast_ref::<AnySpan>()
        {
            let any: Arc<dyn Any + Send + Sync> = Arc::new(Some(span.clone()));
            next.insert_ref(CURRENT_SPAN.id, Arc::clone(&any));
            own.insert_ref(CURRENT_SPAN.id, any);
        }
        if stage.out_id == TracerTag::ID
            && let Some(tracer) = value.downcast_ref::<Arc<dyn Tracer>>()
        {
            let any: Arc<dyn Any + Send + Sync> = Arc::new(Some(Arc::clone(tracer)));
            next.insert_ref(CURRENT_TRACER.id, Arc::clone(&any));
            own.insert_ref(CURRENT_TRACER.id, any);
        }
        acc = Arc::new(next);
    }
    Ok(Arc::new(own))
}

/// Layer-composition merge: `later` wins a tag both sides provide, matching
/// the pin's `Context.mergeAll` (`Layer.test.ts`'s `fresh` rows).
fn overlay(earlier: &Services, later: &Services) -> Services {
    if later.len() == 0 {
        return Arc::clone(earlier);
    }
    let mut maps = (**earlier).clone();
    for (id, value, conflicted) in later.tag_list() {
        maps.replace_tag(id, value);
        if conflicted {
            maps.poison(id);
        }
    }
    for (id, value) in later.ref_list() {
        maps.insert_ref(id, value);
    }
    Arc::new(maps)
}

fn double_provision<E>(id: &'static str) -> Cause<E> {
    Cause::die(Defect::new(format!(
        "double-provision of `{id}` with distinct values: provide each tag once"
    )))
}

/// Every constructor stage a node can reach, in build order. `suspend` is
/// opaque until it runs, so it contributes nothing.
fn flatten<E>(node: &Arc<Node<E>>) -> Vec<Stage<E>> {
    let mut out = Vec::new();
    flatten_into(node, &mut out);
    out
}

fn flatten_into<E>(node: &Arc<Node<E>>, out: &mut Vec<Stage<E>>) {
    match &node.kind {
        Kind::Stages(stages) => out.extend(stages.iter().cloned()),
        Kind::Wrap { head, inner } => {
            out.extend(head.iter().cloned());
            flatten_into(inner, out);
        }
        Kind::Merge(left, right) => {
            flatten_into(left, out);
            flatten_into(right, out);
        }
        Kind::Provide { inner, provider } => {
            flatten_into(provider, out);
            flatten_into(inner, out);
        }
        Kind::Fresh(inner) | Kind::Hook { inner, .. } => flatten_into(inner, out),
        // `orDie` keeps its child's stages behind an erased builder, and it
        // cannot rewire a graph, so there is no new cycle to catch here.
        Kind::Suspend(_) | Kind::OrDie(_) => {}
    }
}

/// Walk the composed dependency graph; reject cycles before any constructor
/// runs. `needs` edges point at the stage providing each tag.
fn check_cycle<E>(stages: &[Stage<E>]) -> Result<(), Cycle> {
    let mut provider: HashMap<&'static str, usize> = HashMap::new();
    for (idx, stage) in stages.iter().enumerate() {
        provider.entry(stage.out_id).or_insert(idx);
    }
    let mut state = vec![0u8; stages.len()];
    for idx in 0..stages.len() {
        if state[idx] == 0 {
            let mut stack = vec![stages[idx].out_id];
            visit(idx, stages, &provider, &mut state, &mut stack)?;
        }
    }
    Ok(())
}

fn visit<E>(
    idx: usize,
    stages: &[Stage<E>],
    provider: &HashMap<&'static str, usize>,
    state: &mut [u8],
    stack: &mut Vec<&'static str>,
) -> Result<(), Cycle> {
    state[idx] = 1;
    for need in &stages[idx].in_ids {
        if let Some(&dep) = provider.get(need) {
            if state[dep] == 1 {
                let mut path = stack.clone();
                path.push(need);
                return Err(Cycle { path });
            } else if state[dep] == 0 {
                stack.push(stages[dep].out_id);
                visit(dep, stages, provider, state, stack)?;
                stack.pop();
            }
        }
    }
    state[idx] = 2;
    Ok(())
}
