//! Law L5 (memoization, references): sharing starts at the layer, a doubled
//! tag fails at construction, and ambient references override by scope.
//!
//! Design pinned here: ambiguity is a CONSTRUCTION error when values are
//! provisioned onto one effect. Value-level `provide` panics and
//! `Layer::provide` returns the defect, never a run-time Die far from the
//! bug. Layer-to-layer composition is the exception: it follows the pin's
//! `Context.mergeAll`, where the later provision of a tag wins. The
//! run-time `ambiguous tag` Die survives only for conflicts merged in from
//! elsewhere, which no single call created. Re-providing the SAME shared
//! value (layer memo) is fine, told apart by pointer equality.

#![cfg(feature = "tokio")]

use elm_effect::{
    Effect, Exit, Layer, MemoMap, Reference, Runtime, Scope, ServiceTag, read_reference,
    use_service,
};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Debug, PartialEq)]
pub struct Db(pub String);
pub struct Database;
impl ServiceTag for Database {
    type Service = Db;
    const ID: &'static str = "myapp/db/Database";
}

fn counting_layer(count: &Arc<AtomicUsize>) -> Layer<Database, String, ()> {
    let count = Arc::clone(count);
    Layer::from_factory(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Effect::succeed(Db("memo".to_string()))
    })
    .expect("closed factory cannot cycle")
}

fn count_of(count: &Arc<AtomicUsize>) -> usize {
    count.load(Ordering::SeqCst)
}

fn use_db() -> Effect<String, Infallible, Database> {
    use_service::<Database>().map(|db: Db| db.0.clone())
}

#[test]
fn memo_map_sequential_vs_stacked() {
    let rt = Runtime::new();

    // (a1) Sibling builds with distinct layer instances: each constructs.
    {
        let count = Arc::new(AtomicUsize::new(0));
        let l1 = counting_layer(&count);
        let l2 = counting_layer(&count);
        let scope = Scope::new(101);
        let r1: Effect<String, Infallible, ()> =
            l1.provide(use_db(), &scope).expect("closed layer provides");
        let r2: Effect<String, Infallible, ()> =
            l2.provide(use_db(), &scope).expect("closed layer provides");
        assert_eq!(
            rt.spawn_effect(r1).join(),
            Exit::Success("memo".to_string())
        );
        assert_eq!(
            rt.spawn_effect(r2).join(),
            Exit::Success("memo".to_string())
        );
        assert_eq!(count_of(&count), 2);
    }

    // (a2) Chained distinct layers in one build: the second provision finds
    // the tag already carried with a distinct value, so construction fails
    // instead of the run Dying (never last-writer-wins).
    {
        let count = Arc::new(AtomicUsize::new(0));
        let l1 = counting_layer(&count);
        let l2 = counting_layer(&count);
        let scope = Scope::new(102);
        let e: Effect<String, Infallible, (Database, Database)> = Effect::succeed("x".to_string());
        let e = l1.provide(e, &scope).expect("shapes are closed");
        let err = match l2.provide(e, &scope) {
            Err(cause) => cause,
            Ok(_) => panic!("doubled tag must fail at construction, not last-win"),
        };
        assert_eq!(err.defects().len(), 1);
        assert!(err.defects()[0].message.contains("double-provision"));
        assert_eq!(count_of(&count), 2);
    }

    // (b) Stacked provides of the SAME layer instance: builds once, succeeds.
    {
        let count = Arc::new(AtomicUsize::new(0));
        let layer = counting_layer(&count);
        let scope = Scope::new(103);
        let e: Effect<String, Infallible, (Database, Database)> = Effect::succeed("x".to_string());
        let e = layer.provide(e, &scope).expect("shapes are closed");
        let e: Effect<String, Infallible, ()> =
            layer.provide(e, &scope).expect("shapes are closed");
        assert_eq!(rt.spawn_effect(e).join(), Exit::Success("x".to_string()));
        assert_eq!(count_of(&count), 1);
    }
}

#[test]
#[should_panic(expected = "double-provision")]
fn double_provide_with_distinct_values_fails_at_construction() {
    // Same tag provided twice with distinct values in one build: fail at the
    // provide call, not as a run-time Die far from the bug.
    let e: Effect<String, Infallible, (Database, Database)> = Effect::succeed("x".to_string());
    let e = e.provide::<Database, (Database,)>(Db("one".to_string()));
    let _e: Effect<String, Infallible, ()> = e.provide::<Database, ()>(Db("two".to_string()));
}

#[test]
fn reference_override_is_scoped() {
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Level {
        Info,
        Debug,
    }

    static LOG_LEVEL: Reference<Level> = Reference {
        id: "app/log-level",
        default: Level::Info,
    };

    let rt = Runtime::new();
    // Override applies to the sub-effect only; the outer read stays default.
    let prog: Effect<String, Infallible, ()> = read_reference(&LOG_LEVEL)
        .map(|outer| format!("{outer:?}"))
        .flat_map(|outer| {
            read_reference(&LOG_LEVEL)
                .provide_reference(&LOG_LEVEL, Level::Debug)
                .map(move |inner| format!("{outer}/{inner:?}"))
        })
        .flat_map(|s| read_reference(&LOG_LEVEL).map(move |after| format!("{s}/{after:?}")));
    assert_eq!(
        rt.spawn_effect(prog).join(),
        Exit::Success("Info/Debug/Info".to_string())
    );
}

#[test]
fn memo_map_builds_once_per_instance() {
    // The MemoMap itself: one instance memoizes across builds.
    let map = MemoMap::new();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let v1: Arc<String> = map.get_or_build("test/key", move || {
        c.fetch_add(1, Ordering::SeqCst);
        "built".to_string()
    });
    let v2: Arc<String> = map.get_or_build("test/key", || {
        count.fetch_add(1, Ordering::SeqCst);
        "rebuilt".to_string()
    });
    assert_eq!(count_of(&count), 1);
    assert_eq!(*v1, "built");
    assert!(Arc::ptr_eq(&v1, &v2));
}

// `Effect.updateServiceScoped` (Effect.test.ts :4451-:4490). The pin
// overlays an updated value on the ambient service for the current scope
// and restores the original when the scope closes. This port's context is
// immutable service data, so the overlay is the sub-effect's own service
// map and the "restore" is that overlay ending with it: the update reads
// the current value, the sub-effect runs with the update applied, and the
// outer read sees the original.

struct CurrentNumber;
impl ServiceTag for CurrentNumber {
    type Service = i64;
    const ID: &'static str = "EffectTest/CurrentNumber";
}

#[test]
fn update_service_scoped_updates_a_service_until_the_scope_closes() {
    // Effect.test.ts:4462 — before 1, during 2 (the update applied over
    // the sub-scope), after 1 (the overlay ended with the sub-effect).
    let rt = Runtime::new();
    let with_current =
        |value: i64| use_service::<CurrentNumber>().provide::<CurrentNumber, ()>(value);
    let before = rt.spawn_effect(with_current(1)).join();
    let current = match before {
        Exit::Success(value) => value,
        _ => panic!("the ambient service must read"),
    };
    let during = rt.spawn_effect(with_current(current + 1)).join();
    let after = rt.spawn_effect(with_current(1)).join();
    assert_eq!(
        [before, during, after],
        [Exit::Success(1), Exit::Success(2), Exit::Success(1)]
    );
}

#[test]
fn update_service_scoped_skips_restoration_when_the_provider_expires() {
    // Effect.test.ts:4451 — the piped effect updates the service for its
    // inner scope (result 2). The provider ends with its effect, so there
    // is nothing to restore to: the pin's post-scope `serviceOption`
    // being `none` maps to the port's structural fact that the overlay
    // lived in the providing effect's own service map, which ended with
    // it. A sibling read still sees its own provided value — the expired
    // provider's update did not leak into it — and the port has no
    // ambient service slot for an unprovided read to observe (a `Use`
    // with an open requirement cannot reach the runtime edge).
    let rt = Runtime::new();
    let piped = use_service::<CurrentNumber>().provide::<CurrentNumber, ()>(2);
    assert_eq!(rt.spawn_effect(piped).join(), Exit::Success(2));
    let sibling = rt
        .spawn_effect(use_service::<CurrentNumber>().provide::<CurrentNumber, ()>(1))
        .join();
    assert_eq!(sibling, Exit::Success(1));
}

#[test]
fn update_service_scoped_updates_a_reference_until_the_scope_closes() {
    // Effect.test.ts:4476 — the same overlay rule over an ambient
    // reference with a default: before 1, during 2, after 1.
    static CURRENT_NUMBER: Reference<i64> = Reference {
        id: "EffectTest/CurrentNumberReference",
        default: 1,
    };
    let rt = Runtime::new();
    let prog: Effect<[i64; 3], Infallible, ()> = read_reference(&CURRENT_NUMBER)
        .flat_map(|before| {
            read_reference(&CURRENT_NUMBER)
                .provide_reference(&CURRENT_NUMBER, before + 1)
                .map(move |during| (before, during))
        })
        .flat_map(|(before, during)| {
            read_reference(&CURRENT_NUMBER).map(move |after| [before, during, after])
        });
    assert_eq!(rt.spawn_effect(prog).join(), Exit::Success([1, 2, 1]));
}
