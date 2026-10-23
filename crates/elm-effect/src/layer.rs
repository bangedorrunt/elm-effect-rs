//! Law L5 (cycle part): layers wire services without running constructors.
//!
//! A [`Layer`] is a recipe: named constructors that turn required services
//! (`RIn`) into provided ones (`ROut`). Composition ([`provide_merge`]) only
//! walks the dependency graph; constructors run in [`build`](Layer::build),
//! each exactly once per handle ([`fresh`](Layer::fresh) re-runs them).
//! A cycle is a typed construction error ([`Cycle`]), never a per-request
//! failure. Layers compose by exact-match chaining: the second layer must
//! require exactly what the first provides. Independent layers combine at the
//! effect level with repeated [`Effect::provide`](crate::Effect::provide).

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use crate::effect::Requirement;
use crate::scope::Scope;
use crate::service::{Cons, ServiceTag, Services, empty_services};
use crate::tracer::{
    AnySpan, CURRENT_SPAN, CURRENT_TRACER, ParentSpanTag, Span, SpanOptions, Tracer, TracerTag,
};
use crate::{Cause, Defect, Effect, Exit};

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

/// A layer's build memo, shareable between handles.
///
/// The pin's `MemoMap`: pass one layer's memo to a second layer and the second
/// build reuses the first's already-constructed services instead of running the
/// constructors again. Sharing is by `Arc` clone, so [`Layer::with_memo`]
/// produces a handle onto the same memo rather than a copy of its contents.
pub struct LayerMemo(Arc<Mutex<Option<Services>>>);

impl LayerMemo {
    /// An empty memo: the next build runs every constructor.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

impl Default for LayerMemo {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LayerMemo {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
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
    ) -> Result<Arc<dyn std::any::Any + Send + Sync>, Cause<E>>;
}

/// One stage that creates a span, hands it to the rest of the build, and
/// ends it when the build scope closes.
struct SpanStage<E> {
    name: String,
    options: SpanOptions,
    on_end: Option<Arc<dyn Fn(Arc<dyn Span>, Exit<(), String>) + Send + Sync>>,
    marker: PhantomData<fn() -> E>,
}

impl<E: Send + 'static> RunStage<E> for SpanStage<E> {
    fn run_with(
        self: Box<Self>,
        services: &Services,
        scope: &Scope,
    ) -> Result<Arc<dyn std::any::Any + Send + Sync>, Cause<E>> {
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

fn span_stage<E: Send + 'static>(name: String, options: LayerSpanOptions) -> Stage<E> {
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
        Self {
            span,
            on_end: None,
        }
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
    E: Send + 'static,
    RIn: Requirement,
    F: Fn() -> Effect<S, E, RIn> + Send + Sync,
{
    fn run_with(
        self: Box<Self>,
        services: &Services,
        _scope: &Scope,
    ) -> Result<Arc<dyn std::any::Any + Send + Sync>, Cause<E>> {
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
    E: Send + 'static,
    RIn: Requirement,
    F: Fn(&Scope) -> Effect<S, E, RIn> + Send + Sync,
{
    fn run_with(
        self: Box<Self>,
        services: &Services,
        scope: &Scope,
    ) -> Result<Arc<dyn std::any::Any + Send + Sync>, Cause<E>> {
        let value = (self.make)(scope).run_sync_with(services)?;
        Ok(Arc::new(value))
    }
}

/// Marker for a layer producing `ROut`, failing with `E`, needing `RIn`.
type LayerMarker<ROut, E, RIn> = PhantomData<fn() -> (ROut, E, RIn)>;

/// A composable service recipe: builds `ROut` from `RIn`.
///
/// Constructors run at most once per handle, in stage order, against the
/// accumulated services. [`fresh`](Layer::fresh) returns the same recipe
/// with an empty memo so tests get a new instance.
pub struct Layer<ROut, E, RIn> {
    stages: Vec<Stage<E>>,
    out_ids: Vec<&'static str>,
    in_ids: Vec<&'static str>,
    memo: LayerMemo,
    marker: LayerMarker<ROut, E, RIn>,
}

impl<T, E, RIn> Layer<T, E, RIn>
where
    T: ServiceTag,
    RIn: RequirementIds,
{
    /// A layer providing one tag. The factory runs per build (never memoized
    /// across [`fresh`](Layer::fresh) handles); a tag that requires itself
    /// is a [`Cycle`].
    pub fn from_factory<S, F>(make: F) -> Result<Self, Cycle>
    where
        E: Send + 'static,
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
            stages: vec![stage],
            out_ids: vec![T::ID],
            in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }

    /// Like [`from_factory`](Layer::from_factory), but the constructor also
    /// receives the build scope, so it can register finalizers that run when
    /// the built services are released. This is what a layer needs to own a
    /// resource (the pin's `Layer.effect` with `Effect.addFinalizer`).
    pub fn from_scoped_factory<S, F>(make: F) -> Result<Self, Cycle>
    where
        E: Send + 'static,
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
            stages: vec![stage],
            out_ids: vec![T::ID],
            in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

use std::collections::HashMap;

use crate::service::Provide;

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

impl<O, E, I> Layer<O, E, I> {
    /// The same recipe with an empty memo: the next [`build`](Layer::build)
    /// re-runs every constructor, so tests get a new instance.
    pub fn fresh(&self) -> Self {
        Self {
            stages: self.stages.clone(),
            out_ids: self.out_ids.clone(),
            in_ids: self.in_ids.clone(),
            memo: LayerMemo::new(),
            marker: PhantomData,
        }
    }

    /// A handle onto this layer's build memo. Share it with
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
            stages: self.stages.clone(),
            out_ids: self.out_ids.clone(),
            in_ids: self.in_ids.clone(),
            memo: memo.clone(),
            marker: PhantomData,
        }
    }

    /// Build this layer under a new span (`Layer.withSpan`): the span stage is
    /// first, so every constructor of `self` sees it as the ambient parent,
    /// and it ends when the build scope closes.
    pub fn with_span(mut self, name: impl Into<String>, options: LayerSpanOptions) -> Self
    where
        E: Send + 'static,
    {
        self.stages.insert(0, span_stage(name.into(), options));
        self
    }
}

impl<T: ServiceTag, E: Send + 'static> Layer<T, E, ()> {
    /// A layer providing a static value. Never cycles, never fails.
    pub fn from_value(service: T::Service) -> Self {
        Self::from_factory(move || Effect::succeed(service.clone()))
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
        Self {
            stages: vec![span_stage(name.into(), options)],
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

impl<O1: RequirementIds, E: Send + 'static, I1: RequirementIds> Layer<O1, E, I1> {
    /// Wire `other` through `self`: `other` must require exactly what `self`
    /// provides. The graph is walked before anything is returned, so a cycle
    /// fails here with [`Cycle`] and no constructor runs. Needs that `self`
    /// leaves open stay open on the result.
    pub fn provide_merge<O2: RequirementIds>(
        self,
        other: Layer<O2, E, O1>,
    ) -> Result<Layer<O2, E, I1>, Cycle> {
        let mut stages = self.stages;
        stages.extend(other.stages);
        check_cycle(&stages)?;
        Ok(Layer {
            stages,
            out_ids: other.out_ids,
            in_ids: self.in_ids,
            memo: LayerMemo::new(),
            marker: PhantomData,
        })
    }
}

impl<T: ServiceTag, E: Send + 'static> Layer<T, E, ()> {
    /// Wire a closed layer into an effect and hide its tag: the effect keeps
    /// its other requirements, `T` is satisfied from a build. A constructor
    /// failure closes `scope` and is returned.
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

impl<O: RequirementIds, E: Send + 'static> Layer<O, E, ()> {
    /// Run every constructor once (memoized per handle) against the
    /// accumulated services. A constructor failure closes `scope` and fails
    /// the build with the constructor's cause. Two stages emitting the same
    /// tag with distinct values fail the same way: double-provision is a
    /// construction error, not a run-time Die.
    pub fn build(&self, scope: &Scope) -> Result<BuiltServices, Cause<E>> {
        if let Some(memo) = self.memo.0.lock().unwrap().clone() {
            return Ok(BuiltServices { values: memo });
        }
        let mut acc = empty_services();
        for stage in &self.stages {
            let runner = (stage.make)();
            match runner.run_with(&acc, scope) {
                Ok(value) => {
                    let mut maps = (*acc).clone();
                    if maps.insert_tag(stage.out_id, value.clone()) {
                        scope.close();
                        return Err(Cause::die(Defect::new(format!(
                            "double-provision of `{}` with distinct values: provide each tag once",
                            stage.out_id
                        ))));
                    }
                    // Tracing layers name a service and an ambient reference;
                    // building one installs both, so later stages see the span
                    // or tracer without reading the tag.
                    // The references hold `Option<T>`, because an absent
                    // value and an unset reference differ: `None` shadows an
                    // outer span, no reference does not.
                    if stage.out_id == ParentSpanTag::ID
                        && let Some(span) = value.downcast_ref::<AnySpan>()
                    {
                        maps.insert_ref(CURRENT_SPAN.id, Arc::new(Some(span.clone())));
                    }
                    if stage.out_id == TracerTag::ID
                        && let Some(tracer) = value.downcast_ref::<Arc<dyn Tracer>>()
                    {
                        maps.insert_ref(CURRENT_TRACER.id, Arc::new(Some(Arc::clone(tracer))));
                    }
                    acc = Arc::new(maps);
                }
                Err(cause) => {
                    scope.close();
                    return Err(cause);
                }
            }
        }
        *self.memo.0.lock().unwrap() = Some(Arc::clone(&acc));
        Ok(BuiltServices { values: acc })
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
