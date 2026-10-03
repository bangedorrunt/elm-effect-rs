//! Law L5 (memoization, references): sharing starts at the layer, a doubled
//! tag is a defect, and ambient references override by scope.
//!
//! Design pinned here: ambiguity is a BUILD-level defect. Nested provisions
//! of the same tag with distinct values poison the tag and ANY run of the
//! build Dies (a `Use` node cannot even be constructed under a doubled tag,
//! so the run check is the observer). Re-providing the SAME shared value
//! (layer memo) is fine, told apart by pointer equality.

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

    // (a2) Chained distinct layers in one build: constructs twice, then the
    // build is ambiguous and Dies (never last-writer-wins).
    {
        let count = Arc::new(AtomicUsize::new(0));
        let l1 = counting_layer(&count);
        let l2 = counting_layer(&count);
        let scope = Scope::new(102);
        let e: Effect<String, Infallible, (Database, Database)> = Effect::succeed("x".to_string());
        let e = l1.provide(e, &scope).expect("shapes are closed");
        let e: Effect<String, Infallible, ()> = l2.provide(e, &scope).expect("shapes are closed");
        let exit = rt.spawn_effect(e).join();
        assert_eq!(count_of(&count), 2);
        let cause = match exit {
            Exit::Failure(c) => c,
            Exit::Success(_) => panic!("doubled tag must Die, not last-win"),
        };
        assert!(cause.failure().is_none());
        assert_eq!(cause.defects().len(), 1);
        assert!(cause.defects()[0].message.contains("ambiguous"));
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
fn ambiguous_tag_is_a_defect() {
    let rt = Runtime::new();
    // Same tag provided twice with distinct values in one build: Die, even
    // with no Use node left to observe it.
    let e: Effect<String, Infallible, (Database, Database)> = Effect::succeed("x".to_string());
    let e = e.provide::<Database, (Database,)>(Db("one".to_string()));
    let e: Effect<String, Infallible, ()> = e.provide::<Database, ()>(Db("two".to_string()));
    let exit = rt.spawn_effect(e).join();
    let cause = match exit {
        Exit::Failure(c) => c,
        Exit::Success(_) => panic!("doubled tag must Die"),
    };
    assert!(cause.failure().is_none());
    assert_eq!(cause.defects().len(), 1);
    assert!(cause.defects()[0].message.contains("ambiguous"));
    assert!(cause.defects()[0].message.contains("myapp/db/Database"));
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
