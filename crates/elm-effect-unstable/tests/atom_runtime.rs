//! The runtime-layer atom: a layer built into a scoped context
//! (`Atom.context`, `reactivity/Atom.ts`).
//!
//! The build is synchronous here because this carrier's layers are closed
//! (`Layer<T, E, ()>`): `build_sync` runs the constructor into the runtime's
//! own scope, and the scope is closed when the atom is disposed or its node
//! is released by the registry's disposal pass. Memo-map sharing needs an
//! interpreted build and stays out of this slice.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::effect::Effect;
use elm_effect::{Layer, ServiceTag};
use elm_effect_unstable::reactivity::{AsyncResult, Registry, runtime_atom};

#[derive(Clone, Debug)]
struct Db {
    instance: usize,
}

struct DbTag;

impl ServiceTag for DbTag {
    type Service = Db;
    const ID: &'static str = "test/Db";
}

/// A layer whose constructor counts builds and registers a finalizer on the
/// build scope, so disposal is observable.
fn db_layer(
    builds: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
) -> Layer<DbTag, String, ()> {
    Layer::from_scoped_factory(move |scope| {
        let closed = Arc::clone(&closed);
        scope.add_finalizer(move || {
            closed.fetch_add(1, Ordering::SeqCst);
        });
        let instance = builds.fetch_add(1, Ordering::SeqCst);
        Effect::<Db, String>::succeed(Db { instance })
    })
    .expect("no cycle")
}

#[test]
fn a_runtime_atom_provides_the_built_service() {
    // Atom.test.ts :195 — a runtime atom reads the built layer's service.
    let builds = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_atom(db_layer(Arc::clone(&builds), Arc::clone(&closed)));
    let registry = Registry::make();

    assert!(registry.get(runtime.atom()).is_initial());
    let outcome = runtime.build_sync(&registry);
    match outcome {
        AsyncResult::Success { value, .. } => assert_eq!(value.instance, 0),
        other => panic!("expected the built service, got {other:?}"),
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(closed.load(Ordering::SeqCst), 0, "still mounted");
}

#[test]
fn a_released_runtime_atom_closes_its_scope() {
    // Atom.test.ts :205 — the layer's lifetime follows the registry: the
    // disposal pass that releases the node runs the scope's finalizers.
    let builds = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_atom(db_layer(Arc::clone(&builds), Arc::clone(&closed)));
    let registry = Registry::make();
    let _ = runtime.build_sync(&registry);

    registry.tick();
    assert_eq!(closed.load(Ordering::SeqCst), 1, "the build scope closed");
    assert!(registry.get(runtime.atom()).is_initial(), "released, so initial");
}

#[test]
fn dispose_closes_the_scope_without_a_disposal_pass() {
    // Atom.test.ts :205's explicit half: `dispose` releases the layers now.
    let builds = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_atom(db_layer(Arc::clone(&builds), Arc::clone(&closed)));
    let registry = Registry::make();
    let _ = runtime.build_sync(&registry);

    runtime.dispose(&registry);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert!(registry.get(runtime.atom()).is_initial());
}

#[test]
fn a_rebuild_closes_the_previous_scope() {
    // Atom.test.ts :305 — replacing a runtime's build releases the old one.
    let builds = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_atom(db_layer(Arc::clone(&builds), Arc::clone(&closed)));
    let registry = Registry::make();

    let _ = runtime.build_sync(&registry);
    let second = runtime.build_sync(&registry);
    assert_eq!(closed.load(Ordering::SeqCst), 1, "the first scope closed");
    match second {
        AsyncResult::Success { value, .. } => assert_eq!(value.instance, 1),
        other => panic!("expected the second build, got {other:?}"),
    }
    runtime.dispose(&registry);
    assert_eq!(closed.load(Ordering::SeqCst), 2, "and the second on dispose");
}
