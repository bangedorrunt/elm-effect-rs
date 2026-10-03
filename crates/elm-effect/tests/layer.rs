//! Law L5 (cycle part): layers wire services; cycles fail construction.
#![cfg(feature = "tokio")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{Effect, Layer, Scope, ServiceTag, use_service};

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
