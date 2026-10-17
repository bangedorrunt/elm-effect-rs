//! Law L5 (cycle part): layers wire services; cycles fail construction.
#![cfg(feature = "tokio")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{Effect, Exit, Layer, ManagedRuntime, Runtime, Scope, ServiceTag, use_service};

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
