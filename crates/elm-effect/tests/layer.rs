//! Law L5 (cycle part): layers wire services; cycles fail construction.
#![cfg(feature = "tokio")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{
    BuiltServices, Effect, Exit, Layer, ManagedRuntime, Runtime, Scope, ServiceTag, use_service,
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
fn resource_layer(arr: &Arc<std::sync::Mutex<Vec<&'static str>>>) -> Layer<Db, std::convert::Infallible, ()> {
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
                arr.lock().unwrap().push(if seen == 1 { "tap" } else { "tap:wrong" })
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
    let layer = Layer::<Db, String, ()>::from_factory(|| {
        Effect::die(elm_effect::Defect::new("boom"))
    })
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
fn merge_rejects_two_values_for_one_tag() {
    // The doc claims this: merging a handle with its own fresh sibling is a
    // double provision, not last-writer-wins. Without the guard, one of the
    // two connections would be built and released while the other leaked.
    let arr = Arc::new(std::sync::Mutex::new(Vec::new()));
    let layer = tagged_resource::<Db>(&arr, DbConn(1))
        .merge(tagged_resource::<Db>(&arr, DbConn(2)))
        .expect("the graph has no cycle");
    let scope = Scope::new(98);
    let cause = layer.build(&scope).expect_err("one tag cannot hold two values");
    let rendered = format!("{cause:?}");
    assert!(
        rendered.contains("double-provision of `test/db`"),
        "the failure must name the tag, got: {rendered}"
    );
}
