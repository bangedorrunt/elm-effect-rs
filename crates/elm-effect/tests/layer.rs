//! Law L5 (cycle part): layers wire services; cycles fail construction.
#![cfg(feature = "tokio")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{
    BuiltServices, Effect, Exit, Layer, LayerMemo, ManagedRuntime, Runtime, Scope, ServiceTag,
    use_service,
};

struct Db;
struct DbConn(u64);

impl ServiceTag for Db {
    type Service = DbConn;
    const ID: &'static str = "test/db";
}

struct Cache;
struct CacheConn(u64);

impl ServiceTag for Cache {
    type Service = CacheConn;
    const ID: &'static str = "test/cache";
}

impl Clone for DbConn {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl Clone for CacheConn {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

#[test]
fn cycle_fails_construction() {
    let built = Arc::new(AtomicUsize::new(0));
    let la = Layer::<Db, std::convert::Infallible, Cache>::from_factory({
        let built = Arc::clone(&built);
        move || {
            built.fetch_add(1, Ordering::SeqCst);
            use_service::<Cache>().map(|_| DbConn(1))
        }
    })
    .expect("single layer builds");
    let lb = Layer::<Cache, std::convert::Infallible, Db>::from_factory({
        let built = Arc::clone(&built);
        move || {
            built.fetch_add(1, Ordering::SeqCst);
            use_service::<Db>().map(|_| CacheConn(1))
        }
    })
    .expect("single layer builds");
    // A requires B and B requires A: typed error, and no constructor ran.
    let err = la.provide_merge(lb).expect_err("cycle must fail");
    assert_eq!(err.path.len(), 3);
    assert_eq!(err.path[0], err.path[2], "cycle path closes the loop");
    assert_eq!(built.load(Ordering::SeqCst), 0);
}

#[test]
fn layer_constructor_failure_fails_scope() {
    let layer = Layer::<Db, String, ()>::from_factory(|| {
        Effect::<DbConn, String, ()>::fail("boom".to_string())
    })
    .expect("factory shape is fine");
    let scope = Scope::new(70);
    let err = layer.build(&scope).expect_err("constructor failure fails");
    assert_eq!(err.failure(), Some(&"boom".to_string()));
    assert!(scope.is_closed());
}

#[test]
fn fresh_builds_new_instance() {
    let counter = Arc::new(AtomicUsize::new(0));
    let layer = Layer::<Db, std::convert::Infallible, ()>::from_factory({
        let counter = Arc::clone(&counter);
        move || {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Effect::succeed(DbConn(n as u64))
        }
    })
    .expect("factory shape is fine");
    let scope = Scope::new(71);
    let first = layer.build(&scope).expect("builds");
    let again = layer.build(&scope).expect("memoized");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(first.get::<Db>().unwrap().0, 1);
    assert_eq!(again.get::<Db>().unwrap().0, 1);
    let second = layer.fresh().build(&scope).expect("fresh builds");
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    assert_eq!(second.get::<Db>().unwrap().0, 2);
}

#[test]
fn provide_wires_and_hides() {
    use elm_effect::{Exit, Runtime};

    let layer = Layer::<Db, std::convert::Infallible, ()>::from_value(DbConn(41));
    let effect = use_service::<Db>().map(|db| db.0);
    let scope = Scope::new(72);
    let provided: Effect<u64, std::convert::Infallible, ()> =
        layer.provide(effect, &scope).expect("wires");
    let exit = Runtime::run(provided);
    assert!(matches!(exit, Exit::Success(41)));
}

#[test]
fn subsequent_provides_share_the_memo_map() {
    // Effect.test.ts:4523 — a `local` provide always builds the layer;
    // every non-local provide of one handle shares that handle's memoized
    // build, so the two non-local provides build between them once. The
    // port's stand-in for `local: true` is a fresh handle, which shares
    // no memo with the original, so the whole run builds twice.
    use elm_effect::{Exit, Runtime};
    use std::sync::atomic::AtomicUsize;

    let counter = Arc::new(AtomicUsize::new(0));
    let layer = Layer::<Db, std::convert::Infallible, ()>::from_factory({
        let counter = Arc::clone(&counter);
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Effect::succeed(DbConn(42))
        }
    })
    .expect("factory shape is fine");
    let scope = Scope::new(80);

    let local: Effect<u64, std::convert::Infallible, ()> = layer
        .fresh()
        .provide(use_service::<Db>().map(|db| db.0), &scope)
        .expect("the local provide builds");
    assert_eq!(Runtime::run(local), Exit::Success(42));
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    let first: Effect<u64, std::convert::Infallible, ()> = layer
        .provide(use_service::<Db>().map(|db| db.0), &scope)
        .expect("wires");
    assert_eq!(Runtime::run(first), Exit::Success(42));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "the first non-local provide builds"
    );

    let second: Effect<u64, std::convert::Infallible, ()> = layer
        .provide(use_service::<Db>().map(|db| db.0), &scope)
        .expect("wires");
    assert_eq!(Runtime::run(second), Exit::Success(42));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "the second non-local provide shares the MemoMap"
    );
}

struct StringConn(String);

/// A tag distinct from the file's existing `Db` so there is no duplicate
/// ServiceTag implementation.
struct SrvTag;

impl ServiceTag for SrvTag {
    type Service = StringConn;
    const ID: &'static str = "test/managed-string";
}

impl Clone for StringConn {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[test]
fn managed_runtime_make_exposes_built_services() {
    // ManagedRuntime row "can be built synchronously": the build happens in
    // `make` (not lazily at first use), and the resulting services are
    // readable back -- the pin's contextEffect -> Context.get(services, tag).
    let layer =
        Layer::<SrvTag, std::convert::Infallible, ()>::from_value(StringConn("test".to_string()));
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");

    let services = runtime.context().expect("built services");
    assert_eq!(
        services.get::<SrvTag>().map(|s| s.0),
        Some("test".to_string())
    );
}

#[test]
fn managed_runtime_new_has_no_built_services() {
    // The default constructor builds nothing, which is what distinguishes
    // `new` from `make`: reading services off it yields None rather than an
    // empty (and therefore silently wrong) context.
    let runtime = ManagedRuntime::new();
    assert!(runtime.context().is_none());
}

#[test]
fn make_returns_a_construction_failure_instead_of_swallowing_it() {
    // The pin's shape: ManagedRuntime.make yields an Effect, so a layer whose
    // constructor fails fails the build and the caller cannot ignore it.
    // `build(..).ok()` used to turn a refused connection into a runtime
    // indistinguishable from `new()`, which a services interpreter then
    // reported as "no services" — naming the wrong cause.
    let layer = Layer::<SrvTag, String, ()>::from_factory(|| {
        Effect::fail("connection refused".to_string())
    })
    .expect("well-formed layer");
    match ManagedRuntime::make(&layer) {
        Err(cause) => {
            let rendered = format!("{cause:?}");
            assert!(
                rendered.contains("connection refused"),
                "the construction failure must reach the caller, got: {rendered}"
            );
        }
        Ok(_) => panic!("a failing layer must not produce a runtime"),
    }
}

#[test]
fn provide_from_closes_a_tag_from_a_runtime_built_services() {
    // The primitive a static handler needs: `provide` wants a value, and a
    // trait method has no `self` to be handed one, so this closes the tag
    // out of the runtime's layer instead. Without it, `use_service` inside
    // anything static is a dead end.
    let layer = Layer::<SrvTag, std::convert::Infallible, ()>::from_value(StringConn(
        "from-layer".to_string(),
    ));
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let services = runtime.context().expect("built services");
    let ready: Effect<String, std::convert::Infallible, ()> =
        use_service::<SrvTag>().map(|c| c.0).provide_from(services);
    assert!(matches!(Runtime::run(ready), Exit::Success(ref s) if s == "from-layer"));
}

#[test]
fn provide_from_with_a_layer_that_never_built_the_tag_is_a_defect_not_a_default() {
    // Fail-closed in the other direction. A runtime built from a layer for
    // a DIFFERENT tag must not hand the interpreter a default value when it
    // asks for this one: the missing tag surfaces at the run edge.
    let layer = Layer::<Db, std::convert::Infallible, ()>::from_value(DbConn(7));
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let services = runtime.context().expect("built services");
    let ready: Effect<String, std::convert::Infallible, ()> =
        use_service::<SrvTag>().map(|c| c.0).provide_from(services);
    match Runtime::run(ready) {
        Exit::Failure(cause) => {
            let rendered = format!("{cause:?}");
            assert!(
                rendered.contains("missing service"),
                "a missing tag must name itself, got: {rendered}"
            );
        }
        other => panic!("a missing tag produced {other:?}, not a failure"),
    }
}
/// A layer that owns a resource: it records its acquire and registers the
/// release on the build scope, the shape `makeLayer1` has at the pin.
fn resource_layer(
    arr: &Arc<std::sync::Mutex<Vec<&'static str>>>,
) -> Layer<Db, std::convert::Infallible, ()> {
    let arr = Arc::clone(arr);
    Layer::from_scoped_factory(move |scope| {
        arr.lock().unwrap().push("acquire");
        let released = Arc::clone(&arr);
        scope.add_finalizer(move || released.lock().unwrap().push("release"));
        Effect::succeed(DbConn(1))
    })
    .expect("a single closed constructor cannot cycle")
}

#[test]
fn suspend_is_lazy_and_evaluates_per_handle() {
    // Layer.test.ts:86. The thunk does not run at construction, and one build
    // per handle runs it once. The pin builds twice through `Effect.scoped`,
    // which forks a memo map per fiber; here one handle is one memo, so the
    // second build is the `fresh()` handle.
    let evaluated = Arc::new(AtomicUsize::new(0));
    let layer = Layer::<Db, std::convert::Infallible, ()>::suspend({
        let evaluated = Arc::clone(&evaluated);
        move || {
            evaluated.fetch_add(1, Ordering::SeqCst);
            Layer::from_value(DbConn(7))
        }
    });
    assert_eq!(evaluated.load(Ordering::SeqCst), 0, "suspend is lazy");

    let scope = Scope::new(90);
    let built = layer.build(&scope).expect("the suspended layer builds");
    assert_eq!(evaluated.load(Ordering::SeqCst), 1);
    assert_eq!(built.get::<Db>().unwrap().0, 7);

    layer.build(&scope).expect("the handle memoizes");
    assert_eq!(
        evaluated.load(Ordering::SeqCst),
        1,
        "one evaluation per handle"
    );

    layer
        .fresh()
        .build(&scope)
        .expect("a fresh handle evaluates again");
    assert_eq!(evaluated.load(Ordering::SeqCst), 2);
}

#[test]
fn suspend_shares_its_memo_across_builds() {
    // Layer.test.ts:597. Two scopes on one memo evaluate the thunk once, and
    // only the scope that acquired the resource releases it.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let evaluated = Arc::new(AtomicUsize::new(0));
    let layer = Layer::<Db, std::convert::Infallible, ()>::suspend({
        let evaluated = Arc::clone(&evaluated);
        let arr = Arc::clone(&arr);
        move || {
            evaluated.fetch_add(1, Ordering::SeqCst);
            resource_layer(&arr)
        }
    });

    let first = Scope::new(91);
    let second = Scope::new(92);
    layer.build(&first).expect("first scope builds");
    layer.build(&second).expect("second scope shares the memo");
    assert_eq!(evaluated.load(Ordering::SeqCst), 1);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire"]);
    assert!(second.close().is_empty());
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire"],
        "a memoized build owns its resource through the scope that acquired it"
    );
    assert!(first.close().is_empty());
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "release"]);
}

#[test]
fn tap_runs_with_the_built_services_and_preserves_them() {
    // Layer.test.ts:177. The callback sees the built services, runs before
    // the build answers, and the caller keeps the same output.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = resource_layer(&arr).tap({
        let arr = Arc::clone(&arr);
        move |built: &BuiltServices| {
            let seen = built.get::<Db>().map(|db| db.0).unwrap_or_default();
            let arr = Arc::clone(&arr);
            Effect::<(), std::convert::Infallible, ()>::sync(move || {
                arr.lock()
                    .unwrap()
                    .push(if seen == 1 { "tap" } else { "tap:wrong" })
            })
        }
    });
    let scope = Scope::new(93);
    let built = layer.build(&scope).expect("the tapped layer builds");
    assert_eq!(built.get::<Db>().unwrap().0, 1);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "tap"]);
    scope.close();
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "tap", "release"]);
}

#[test]
fn tap_error_runs_and_preserves_the_original_error() {
    // Layer.test.ts:194. The callback runs on a typed construction failure and
    // the build still fails with the original error.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = Layer::<Db, String, ()>::from_factory(|| Effect::fail("failed!".to_string()))
        .expect("a single closed constructor cannot cycle")
        .tap_error({
            let seen = Arc::clone(&seen);
            move |error: &String| {
                let line = format!("tapError:{error}");
                let seen = Arc::clone(&seen);
                Effect::<(), String, ()>::sync(move || seen.lock().unwrap().push(line))
            }
        });
    let scope = Scope::new(94);
    let cause = layer.build(&scope).expect_err("the layer still fails");
    assert_eq!(cause.failure(), Some(&"failed!".to_string()));
    assert_eq!(*seen.lock().unwrap(), vec!["tapError:failed!"]);
}

#[test]
fn tap_cause_sees_the_defect_and_preserves_the_cause() {
    // Layer.test.ts:211. The callback gets the whole cause, including a
    // defect, and the build still fails with that cause.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer =
        Layer::<Db, String, ()>::from_factory(|| Effect::die(elm_effect::Defect::new("boom")))
            .expect("a single closed constructor cannot cycle")
            .tap_cause({
                let seen = Arc::clone(&seen);
                move |cause: &elm_effect::Cause<String>| {
                    let dies = cause.has_dies();
                    let seen = Arc::clone(&seen);
                    Effect::<(), String, ()>::sync(move || seen.lock().unwrap().push(dies))
                }
            });
    let scope = Scope::new(95);
    let cause = layer.build(&scope).expect_err("the layer still fails");
    assert!(cause.has_dies());
    assert_eq!(*seen.lock().unwrap(), vec![true]);
}

/// A resource layer for any tag, with the acquire/release trail `makeLayer1`
/// leaves at the pin.
fn tagged_resource<T>(
    arr: &Arc<std::sync::Mutex<Vec<&'static str>>>,
    value: T::Service,
) -> Layer<T, std::convert::Infallible, ()>
where
    T: ServiceTag,
    T::Service: Send + Sync + Clone + 'static,
{
    let arr = Arc::clone(arr);
    Layer::from_scoped_factory(move |scope| {
        arr.lock().unwrap().push("acquire");
        let released = Arc::clone(&arr);
        scope.add_finalizer(move || released.lock().unwrap().push("release"));
        Effect::succeed(value.clone())
    })
    .expect("a single closed constructor cannot cycle")
}

#[test]
fn merge_finalizes_both_layers() {
    // Layer.test.ts:116. `Layer.merge` builds both layers on one handle; both
    // acquire before the scope closes and both release when it does.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = tagged_resource::<Db>(&arr, DbConn(1))
        .merge(tagged_resource::<Cache>(&arr, CacheConn(2)))
        .expect("independent tags cannot cycle");
    let scope = Scope::new(96);
    let built = layer.build(&scope).expect("both layers build");
    assert_eq!(built.get::<Db>().unwrap().0, 1);
    assert_eq!(built.get::<Cache>().unwrap().0, 2);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "acquire", "release", "release"]
    );
}

#[test]
fn finalizers_with_provide_release_the_provider_last() {
    // Layer.test.ts:347. The provided layer builds first and the consumer
    // second, so the scope's LIFO release order is consumer, then provider.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = tagged_resource::<Db>(&arr, DbConn(1));
    let consumer = Layer::<Cache, std::convert::Infallible, Db>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release"));
            use_service::<Db>().map(|db| CacheConn(db.0 + 1))
        }
    })
    .expect("a single constructor cannot cycle");
    let layer = consumer
        .provide_layer(provider)
        .expect("the provider satisfies the consumer");
    let scope = Scope::new(97);
    let built = layer.build(&scope).expect("the composed layer builds");
    assert_eq!(built.get::<Cache>().unwrap().0, 2);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "acquire", "release", "release"]
    );
}

#[test]
fn merge_keeps_the_last_value_for_one_tag() {
    // Layer.test.ts :231/:240/:252 — `fresh` puts a second value behind one
    // tag, and the pin's `Context.merge` keeps the later one. Both builds
    // still acquire and release; only the winner is observable.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = tagged_resource::<Db>(&arr, DbConn(1))
        .merge(tagged_resource::<Db>(&arr, DbConn(2)))
        .expect("the graph has no cycle");
    let scope = Scope::new(98);
    let built = layer.build(&scope).expect("both sides build");
    assert_eq!(built.get::<Db>().unwrap().0, 2, "the later provision wins");
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "acquire", "release", "release"]
    );
}

#[test]
fn catch_uses_an_alternative_layer() {
    // Layer.test.ts:129. The failed attempt releases what it acquired before
    // the fallback builds, and the fallback's resource releases with the
    // caller's scope: [acquire1, release1, acquire2, release2].
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    // The provider shares the failing layer's error type: this port has no
    // error union to widen into, so a provider that cannot fail is spelled
    // with the consumer's `E`.
    let provider = Layer::<Db, String, ()>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release"));
            Effect::succeed(DbConn(1))
        }
    })
    .expect("a single closed constructor cannot cycle");
    let failing = Layer::<Cache, String, Db>::from_factory(|| Effect::fail("failed!".to_string()))
        .expect("a single constructor cannot cycle")
        .provide_layer(provider)
        .expect("the provider satisfies the consumer");
    // `catch` takes an `Fn`, so the fallback is a recipe: one fresh layer per
    // recovery, the way the pin's thunk builds one per call.
    let layer = failing.catch({
        let arr = Arc::clone(&arr);
        move |_error| {
            Layer::<Cache, String, ()>::from_scoped_factory({
                let arr = Arc::clone(&arr);
                move |scope| {
                    arr.lock().unwrap().push("acquire");
                    let released = Arc::clone(&arr);
                    scope.add_finalizer(move || released.lock().unwrap().push("release"));
                    Effect::succeed(CacheConn(2))
                }
            })
            .expect("a single closed constructor cannot cycle")
        }
    });
    let scope = Scope::new(99);
    let built = layer
        .build(&scope)
        .expect("the fallback replaces the failure");
    assert_eq!(built.get::<Cache>().unwrap().0, 2);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "release", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "release", "acquire", "release"]
    );
}

#[test]
fn catch_cause_recovers_and_exposes_the_original_cause() {
    // Layer.test.ts:158. The handler receives the whole cause, the fallback
    // builds normally, and the caller sees the fallback's provision.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = Layer::<Db, String, ()>::from_factory(|| Effect::fail("failed!".to_string()))
        .expect("a single constructor cannot cycle")
        .catch_cause({
            let seen = Arc::clone(&seen);
            move |cause: &elm_effect::Cause<String>| {
                seen.lock().unwrap().push(cause.has_fails());
                Layer::from_value(DbConn(1))
            }
        });
    let scope = Scope::new(100);
    let built = layer
        .build(&scope)
        .expect("the fallback replaces the failure");
    assert_eq!(built.get::<Db>().unwrap().0, 1);
    assert_eq!(*seen.lock().unwrap(), vec![true]);
}

#[test]
fn with_span_installs_the_parent_span_and_runs_the_on_end_finalizer() {
    // Layer.test.ts:717. The layer's constructor sees the created span as the
    // ambient parent, and `onEnd` runs once when the build scope closes with
    // a successful exit.
    use elm_effect::tracer::{CURRENT_SPAN, SpanOptions};
    use elm_effect::{Exit, LayerSpanOptions, read_reference};

    let endings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = Layer::<Db, std::convert::Infallible, ()>::from_factory(|| {
        read_reference(&CURRENT_SPAN)
            .map(|span| DbConn(span.map(|s| s.name().len()).unwrap_or(0) as u64))
    })
    .expect("a single constructor cannot cycle")
    .with_span(
        "layer-span",
        LayerSpanOptions {
            span: SpanOptions::default(),
            on_end: Some(Arc::new({
                let endings = Arc::clone(&endings);
                move |_span: Arc<dyn elm_effect::tracer::Span>, exit: Exit<(), String>| {
                    endings
                        .lock()
                        .unwrap()
                        .push(matches!(exit, Exit::Success(())));
                }
            })),
        },
    );
    let scope = Scope::new(101);
    let built = layer.build(&scope).expect("the span layer builds");
    assert_eq!(
        built.get::<Db>().unwrap().0,
        "layer-span".len() as u64,
        "the constructor reads the span this layer created"
    );
    assert!(endings.lock().unwrap().is_empty());
    scope.close();
    assert_eq!(*endings.lock().unwrap(), vec![true]);
}

#[test]
fn with_parent_span_installs_an_external_span_for_construction() {
    // Layer.test.ts:740. An external span becomes the ambient parent without
    // being created or ended by this layer.
    use elm_effect::Exit;
    use elm_effect::read_reference;
    use elm_effect::tracer::{AnySpan, CURRENT_SPAN, external_span};

    let parent = AnySpan::External(external_span(
        "0000000000000001",
        "00000000000000000000000000000001",
    ));
    let layer = Layer::<Db, std::convert::Infallible, ()>::from_factory(|| {
        read_reference(&CURRENT_SPAN)
            .map(|span| DbConn(span.map(|s| s.span_id().len()).unwrap_or(0) as u64))
    })
    .expect("a single constructor cannot cycle")
    .with_parent_span(parent);
    let scope = Scope::new(102);
    let built = layer.build(&scope).expect("the layer builds");
    assert_eq!(built.get::<Db>().unwrap().0, 16);
    assert!(matches!(
        elm_effect::Runtime::run(Effect::<u64, std::convert::Infallible, ()>::succeed(1)),
        Exit::Success(1)
    ));
}

// ---- The pin's MemoMap: sharing is layer identity, and an entry lives while
// ---- any caller scope still observes it (Layer.test.ts:35..:660).

/// Build through an explicit memo map, failing the test on a construction
/// failure (the rows below all expect a successful build unless stated).
fn run_memo<O: elm_effect::RequirementIds, E: std::fmt::Debug + Send + 'static>(
    layer: &Layer<O, E, ()>,
    memo: &LayerMemo,
    scope: &Arc<Scope>,
) -> BuiltServices {
    match Runtime::run(layer.build_with_memo(memo, scope)) {
        Exit::Success(built) => built,
        Exit::Failure(cause) => panic!("layer build failed: {cause:?}"),
    }
}

/// A layer whose only observable is its release count: one constructor, one
/// finalizer on the build scope the memo owns.
fn counted_layer(released: &Arc<AtomicUsize>) -> Layer<Db, std::convert::Infallible, ()> {
    let released = Arc::clone(released);
    Layer::from_scoped_factory(move |scope| {
        let released = Arc::clone(&released);
        scope.add_finalizer(move || {
            released.fetch_add(1, Ordering::SeqCst);
        });
        Effect::succeed(DbConn(1))
    })
    .expect("a single constructor cannot cycle")
}

#[test]
fn a_handle_merged_with_itself_builds_once() {
    // Layer.test.ts:35. One handle on both sides of `merge` names one node,
    // so the second request is answered from the first build's memo entry.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = tagged_resource::<Db>(&arr, DbConn(1));
    let merged = layer.clone().merge(layer).expect("one tag, one value");
    let scope = Scope::new(110);
    merged.build(&scope).expect("the shared layer builds");
    assert_eq!(*arr.lock().unwrap(), vec!["acquire"]);
    scope.close();
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "release"]);
}

#[test]
fn a_handle_merged_three_times_builds_once() {
    // Layer.test.ts:75. `layer.merge(layer).merge(layer)` asks one node for
    // its services three times; only the first request constructs.
    let built = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&built);
    let layer = Layer::<Db, std::convert::Infallible, ()>::from_factory(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Effect::succeed(DbConn(7))
    })
    .expect("a single constructor cannot cycle");
    let merged = layer
        .clone()
        .merge(layer.clone())
        .expect("no cycle")
        .merge(layer)
        .expect("no cycle");
    let scope = Scope::new(111);
    let services = merged.build(&scope).expect("builds");
    assert_eq!(built.load(Ordering::SeqCst), 1, "one node, one build");
    assert_eq!(services.get::<Db>().unwrap().0, 7);
}

#[test]
fn a_suspended_handle_merged_with_itself_evaluates_once() {
    // Layer.test.ts:101. `suspend` is memoized like `Layer.effect`: the same
    // suspended handle on both sides of `merge` evaluates its thunk once.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let evaluated = Arc::new(AtomicUsize::new(0));
    let layer = Layer::<Db, std::convert::Infallible, ()>::suspend({
        let evaluated = Arc::clone(&evaluated);
        let arr = Arc::clone(&arr);
        move || {
            evaluated.fetch_add(1, Ordering::SeqCst);
            tagged_resource::<Db>(&arr, DbConn(1))
        }
    });
    let merged = layer.clone().merge(layer).expect("no cycle");
    let scope = Scope::new(112);
    merged.build(&scope).expect("builds");
    assert_eq!(evaluated.load(Ordering::SeqCst), 1);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire"]);
    scope.close();
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "release"]);
}

#[test]
fn merging_a_fresh_handle_rebuilds_the_layer() {
    // Layer.test.ts:231. `fresh` builds under a memo map of its own, so the
    // second side constructs again even though it wraps the same node.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = tagged_resource::<Db>(&arr, DbConn(1));
    let merged = layer.clone().merge(layer.fresh()).expect("no cycle");
    let scope = Scope::new(113);
    merged.build(&scope).expect("both sides build");
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "acquire", "release", "release"]
    );
}

#[test]
fn providing_a_fresh_handle_rebuilds_the_layer() {
    // Layer.test.ts:240. The fresh consumer ignores the caller's memo map,
    // so its provider builds once as the provider and once inside the fresh
    // subtree.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = tagged_resource::<Db>(&arr, DbConn(1));
    let consumer = Layer::<Cache, std::convert::Infallible, Db>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release"));
            use_service::<Db>().map(|db| CacheConn(db.0 + 1))
        }
    })
    .expect("a single constructor cannot cycle");
    let env = consumer
        .fresh()
        .provide_layer(provider)
        .expect("the provider satisfies the fresh consumer");
    let scope = Scope::new(114);
    let built = env.build(&scope).expect("both build");
    assert_eq!(built.get::<Cache>().unwrap().0, 2);
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "acquire", "release", "release"]
    );
}

#[test]
fn a_fresh_subtree_isolates_its_own_provision() {
    // Layer.test.ts:252. `layer.merge(layer)` shares one build, and the same
    // expression under `fresh` shares a second, separate build.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = tagged_resource::<Db>(&arr, DbConn(1));
    let left = layer.clone().merge(layer.clone()).expect("no cycle");
    let right = layer.clone().merge(layer).expect("no cycle").fresh();
    let env = left.merge(right).expect("no cycle");
    let scope = Scope::new(115);
    env.build(&scope).expect("builds");
    assert_eq!(*arr.lock().unwrap(), vec!["acquire", "acquire"]);
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire", "acquire", "release", "release"]
    );
}

#[test]
fn provide_chains_build_inside_out_and_release_in_reverse() {
    // Layer.test.ts:357. layer3 needs layer2's service and layer2 needs
    // layer1's; the outer provide builds the innermost provider first, and
    // the scope releases consumer before provider at each layer.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer1 = Layer::<Db, std::convert::Infallible, ()>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire1");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release1"));
            Effect::succeed(DbConn(1))
        }
    })
    .expect("a single constructor cannot cycle");
    let layer2 = Layer::<Cache, std::convert::Infallible, Db>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire2");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release2"));
            use_service::<Db>().map(|db| CacheConn(db.0 + 1))
        }
    })
    .expect("a single constructor cannot cycle");
    let layer3 = Layer::<SrvTag, std::convert::Infallible, Cache>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire3");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release3"));
            use_service::<Cache>().map(|cache| StringConn(cache.0.to_string()))
        }
    })
    .expect("a single constructor cannot cycle");
    let env = layer3
        .provide_layer(layer2)
        .expect("layer2 satisfies layer3")
        .provide_layer(layer1)
        .expect("layer1 satisfies layer2");
    let scope = Scope::new(116);
    let built = env.build(&scope).expect("the chain builds");
    assert_eq!(built.get::<SrvTag>().unwrap().0, "2");
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire1", "acquire2", "acquire3"]
    );
    scope.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec![
            "acquire1", "acquire2", "acquire3", "release3", "release2", "release1"
        ]
    );
}

#[test]
fn a_provide_layer_rebuilds_while_its_provider_stays_memoized() {
    // Layer.test.ts:621. `provide` is a plain node (the pin's `fromBuild`),
    // so building the composed layer twice rebuilds the consumer, while the
    // memoized provider inside it is constructed once.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = Layer::<Db, std::convert::Infallible, ()>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire1");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release1"));
            Effect::succeed(DbConn(1))
        }
    })
    .expect("a single constructor cannot cycle");
    let consumer = Layer::<Cache, std::convert::Infallible, Db>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire2");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release2"));
            use_service::<Db>().map(|db| CacheConn(db.0 + 1))
        }
    })
    .expect("a single constructor cannot cycle");
    let composed = consumer
        .provide_layer(provider.clone())
        .expect("the provider satisfies the consumer");

    let memo = LayerMemo::new();
    let scope1 = Arc::new(Scope::new(1));
    let scope2 = Arc::new(Scope::new(2));
    run_memo(&provider, &memo, &scope1);
    run_memo(&composed, &memo, &scope2);
    scope2.close();
    run_memo(&composed, &memo, &scope1);
    scope1.close();

    assert_eq!(
        *arr.lock().unwrap(),
        vec![
            "acquire1", "acquire2", "release2", "acquire2", "release2", "release1"
        ]
    );
}

#[test]
fn a_shared_layer_is_not_released_when_the_first_scope_closes() {
    // Layer.test.ts:641. The provider is observed by both scopes; closing
    // the outer one leaves one observer, so nothing releases yet.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = Layer::<Db, std::convert::Infallible, ()>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire1");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release1"));
            Effect::succeed(DbConn(1))
        }
    })
    .expect("a single constructor cannot cycle");
    let consumer = Layer::<Cache, std::convert::Infallible, Db>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire2");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release2"));
            use_service::<Db>().map(|db| CacheConn(db.0 + 1))
        }
    })
    .expect("a single constructor cannot cycle");
    let composed = consumer
        .provide_layer(provider.clone())
        .expect("the provider satisfies the consumer");

    let memo = LayerMemo::new();
    let scope1 = Arc::new(Scope::new(1));
    let scope2 = Arc::new(Scope::new(2));
    run_memo(&provider, &memo, &scope1);
    run_memo(&composed, &memo, &scope2);
    scope1.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire1", "acquire2"],
        "the consumer's scope still observes the shared provider"
    );
    scope2.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec!["acquire1", "acquire2", "release2", "release1"]
    );
}

#[test]
fn a_get_effect_counts_one_observer_each_time_it_runs() {
    // Layer.test.ts:554. Reusing a memoized layer through `MemoMap.get`
    // twice does not drop the shared entry when the second scope closes.
    let released = Arc::new(AtomicUsize::new(0));
    let layer = counted_layer(&released);
    let memo = LayerMemo::new();
    let scope1 = Arc::new(Scope::new(1));
    let scope2 = Arc::new(Scope::new(2));
    run_memo(&layer, &memo, &scope1);

    let first = memo.get(&layer, &scope2).expect("the build is recorded");
    let _ = Runtime::run(first);
    let second = memo.get(&layer, &scope2).expect("still recorded");
    let _ = Runtime::run(second);

    scope2.close();
    assert_eq!(
        released.load(Ordering::SeqCst),
        0,
        "the acquiring scope still holds the shared layer"
    );
    scope1.close();
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn a_get_effect_that_never_runs_does_not_keep_the_layer_alive() {
    // Layer.test.ts:572. Peeking at the memo is not observing it; only
    // running the returned effect counts.
    let released = Arc::new(AtomicUsize::new(0));
    let layer = counted_layer(&released);
    let memo = LayerMemo::new();
    let scope1 = Arc::new(Scope::new(1));
    let scope2 = Arc::new(Scope::new(2));
    run_memo(&layer, &memo, &scope1);

    let reuse = memo.get(&layer, &scope2);
    assert!(reuse.is_some(), "the recorded build is visible");
    drop(reuse);

    scope2.close();
    scope1.close();
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn a_build_with_a_memo_map_writes_nothing_until_it_runs() {
    // Layer.test.ts:586. `buildWithMemoMap` is a description; a map written
    // at call time would report a build that never happened.
    let memo = LayerMemo::new();
    let scope = Arc::new(Scope::new(1));
    let layer =
        Layer::<Db, std::convert::Infallible, ()>::from_factory(|| Effect::succeed(DbConn(1)))
            .expect("a single constructor cannot cycle");

    let build = layer.build_with_memo(&memo, &scope);
    assert_eq!(memo.map_size(), 0, "a description is not a build");
    let _ = Runtime::run(build);
    assert_eq!(memo.map_size(), 1, "running it records the entry");
}

#[test]
fn a_forked_memo_map_reuses_parent_entries_without_writing_back() {
    // Layer.test.ts:660. The child map reads through to the parent, but a
    // layer first built in the child is invisible to the parent.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let parent_layer = Layer::<Db, std::convert::Infallible, ()>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire1");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release1"));
            Effect::succeed(DbConn(1))
        }
    })
    .expect("a single constructor cannot cycle");
    let child_layer = Layer::<Cache, std::convert::Infallible, ()>::from_scoped_factory({
        let arr = Arc::clone(&arr);
        move |scope| {
            arr.lock().unwrap().push("acquire2");
            let released = Arc::clone(&arr);
            scope.add_finalizer(move || released.lock().unwrap().push("release2"));
            Effect::succeed(CacheConn(2))
        }
    })
    .expect("a single constructor cannot cycle");

    let parent = LayerMemo::new();
    let child = parent.fork();
    let scope1 = Arc::new(Scope::new(1));
    let scope2 = Arc::new(Scope::new(2));

    run_memo(&parent_layer, &parent, &scope1);
    run_memo(&parent_layer, &child, &scope2);
    run_memo(&child_layer, &child, &scope2);
    run_memo(&child_layer, &parent, &scope1);

    scope2.close();
    scope1.close();
    assert_eq!(
        *arr.lock().unwrap(),
        vec![
            "acquire1", "acquire2", "acquire2", "release2", "release2", "release1"
        ]
    );
}
