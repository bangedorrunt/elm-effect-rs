//! `ManagedRuntime` at the pin (`ManagedRuntime.test.ts`): a layer built once,
//! services readable as context, a shared memo, and a dispose that interrupts
//! live fibers before it releases the layer's resources.
//!
//! Rows bound here (Schema.test.ts is elsewhere; these are
//! `packages/effect/test/ManagedRuntime.test.ts`):
//! :6 memoizes the layer build, :18 provides context, :27 allows sharing a
//! MemoMap, :50 supports await using, :65 fibers are interrupted on dispose,
//! :75 finishes request cleanup before releasing layer resources,
//! :127 completes sequential layer finalizers when disposeEffect is interrupted.
#![cfg(feature = "tokio")]

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use elm_effect::{
    Deferred, Effect, Exit, ForkOpts, Layer, ManagedRuntime, ServiceTag, use_service,
};

/// A service tag whose constructor counts its builds.
struct Counter;
struct Counted(u64);

impl ServiceTag for Counter {
    type Service = Counted;
    const ID: &'static str = "test/managed-runtime-counter";
}

impl Clone for Counted {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

/// A layer that bumps `count` every time its constructor actually runs.
fn counting_layer(count: Arc<AtomicUsize>) -> Layer<Counter, Infallible, ()> {
    Layer::<Counter, Infallible, ()>::from_factory(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Effect::succeed(Counted(1))
    })
    .expect("a closed layer cannot cycle")
}

/// A layer providing a string greeting under its own tag.
struct Greeting;

impl ServiceTag for Greeting {
    type Service = String;
    const ID: &'static str = "test/managed-runtime-greeting";
}

/// A layer that owns a resource and registers its release on the build scope.
struct Resource;
struct Held;

impl ServiceTag for Resource {
    type Service = Held;
    const ID: &'static str = "test/managed-runtime-resource";
}

impl Clone for Held {
    fn clone(&self) -> Self {
        Self
    }
}

/// :6 — the layer's constructor runs once no matter how many effects the
/// runtime runs, because the build memoizes per handle.
#[test]
fn memoizes_the_layer_build() {
    let count = Arc::new(AtomicUsize::new(0));
    let layer = counting_layer(Arc::clone(&count));
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let _ = runtime.run(Effect::<(), Infallible, ()>::succeed(()));
    let _ = runtime.run(Effect::<(), Infallible, ()>::succeed(()));
    runtime.dispose();
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "two runs must not rebuild the layer"
    );
}

/// :18 — the built services are the runtime's context, so a `use_service`
/// resolves from the layer without an explicit `provide`.
#[test]
fn provides_context() {
    let layer = Layer::<Greeting, Infallible, ()>::from_value("test".to_string());
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let exit = runtime.run(use_service::<Greeting>());
    runtime.dispose();
    assert!(
        matches!(exit, Exit::Success(ref s) if s == "test"),
        "the runtime's layer must answer the tag, got {exit:?}"
    );
}

/// :27 — a second layer built on the first runtime's memo reuses the first
/// build: the constructor runs once for both runtimes.
#[test]
fn allows_sharing_a_memo() {
    let count = Arc::new(AtomicUsize::new(0));
    let layer = counting_layer(Arc::clone(&count));
    let runtime_a = ManagedRuntime::make(&layer).expect("layer builds");
    let memo = runtime_a.memo().expect("a layer runtime hands out its memo");
    let layer_b = layer.with_memo(&memo);
    let runtime_b = ManagedRuntime::make(&layer_b).expect("shared memo builds");
    let _ = runtime_a.run(Effect::<(), Infallible, ()>::succeed(()));
    let _ = runtime_b.run(Effect::<(), Infallible, ()>::succeed(()));
    runtime_a.dispose();
    runtime_b.dispose();
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "the shared memo must serve both runtimes from one build"
    );
}

/// :50 — the layer's resource is released when the runtime is disposed (the
/// pin's `await using`), not while the runtime is still running.
#[test]
fn supports_dispose_releasing_the_layer() {
    let count = Arc::new(AtomicUsize::new(0));
    let layer = Layer::<Resource, Infallible, ()>::from_scoped_factory({
        let count = Arc::clone(&count);
        move |scope| {
            let count = Arc::clone(&count);
            scope.add_finalizer(move || {
                count.fetch_add(1, Ordering::SeqCst);
            });
            Effect::<Held, Infallible, ()>::succeed(Held)
        }
    })
    .expect("a closed layer cannot cycle");

    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let _ = runtime.run(Effect::<(), Infallible, ()>::succeed(()));
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "the resource must outlive the run"
    );
    runtime.dispose();
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "dispose must release the layer's resource exactly once"
    );
}

/// :65 — a fiber still running when the runtime is disposed ends interrupted.
#[test]
fn fibers_are_interrupted_on_dispose() {
    let runtime = ManagedRuntime::new();
    let fiber = runtime.run_fork(Effect::<(), Infallible, ()>::never());
    runtime.dispose();
    let exit = fiber.join();
    assert!(
        matches!(exit, Exit::Failure(ref cause) if cause.has_interrupts()),
        "a never-ending fiber must end interrupted, got {exit:?}"
    );
}

/// :75 — dispose waits for an in-flight request's cleanup before it releases
/// the layer's resources, so the cleanup event always precedes the release.
///
/// The pin's row runs both `disposeEffect` and `dispose`; this port has a
/// single synchronous `dispose`, so the two variants are one test here.
#[test]
fn dispose_finishes_request_cleanup_before_releasing_the_layer() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(Deferred::<(), Infallible>::new());
    let started = Arc::new(Deferred::<(), Infallible>::new());

    let layer = Layer::<Resource, Infallible, ()>::from_scoped_factory({
        let events = Arc::clone(&events);
        move |scope| {
            let events = Arc::clone(&events);
            scope.add_finalizer(move || events.lock().unwrap().push("pool closed"));
            Effect::<Held, Infallible, ()>::succeed(Held)
        }
    })
    .expect("a closed layer cannot cycle");

    let runtime = Arc::new(ManagedRuntime::make(&layer).expect("layer builds"));

    // The request fiber: signal that it is running, then never finish. Its
    // cleanup waits for the gate, so dispose cannot complete ahead of it.
    let fiber = runtime.run_fork({
        let gate = Arc::clone(&gate);
        let started = Arc::clone(&started);
        let events = Arc::clone(&events);
        Effect::sync(move || {
            started.complete(());
        })
        .flat_map(move |_| {
            let gate = Arc::clone(&gate);
            let events = Arc::clone(&events);
            Effect::<(), Infallible, ()>::never().ensuring(move || {
                gate.wait();
                events.lock().unwrap().push("request cleanup: Success");
            })
        })
    });
    started.wait();

    // Dispose blocks on the request cleanup, so it runs off the main thread.
    let disposing = std::thread::spawn({
        let runtime = Arc::clone(&runtime);
        move || runtime.dispose()
    });
    gate.complete(());
    disposing.join().expect("the disposing thread must not panic");
    let _ = fiber.join();

    assert_eq!(
        *events.lock().unwrap(),
        vec!["request cleanup: Success", "pool closed"],
        "request cleanup must precede the layer release"
    );
}

/// :127 — interrupting dispose mid-finalizer does not skip the remaining
/// layer finalizers (they run masked, in LIFO order, all of them).
#[test]
fn dispose_completes_sequential_layer_finalizers_when_interrupted() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Deferred::<(), Infallible>::new());
    let started = Arc::new(Deferred::<(), Infallible>::new());

    let layer = Layer::<Resource, Infallible, ()>::from_scoped_factory({
        let ran = Arc::clone(&ran);
        let release = Arc::clone(&release);
        let started = Arc::clone(&started);
        move |scope| {
            // Registered first, so it runs LAST (finalizers are LIFO).
            let mine = Arc::clone(&ran);
            scope.add_finalizer(move || mine.lock().unwrap().push("close database"));
            // Registered second, so it runs first and parks on `release`.
            // `scope` is a `Fn` parameter, so each finalizer takes its own
            // clones rather than moving the outer captures.
            let mine = Arc::clone(&ran);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            scope.add_finalizer(move || {
                started.complete(());
                release.wait();
                mine.lock().unwrap().push("flush logs");
            });
            Effect::<Held, Infallible, ()>::succeed(Held)
        }
    })
    .expect("a closed layer cannot cycle");

    let runtime = Arc::new(ManagedRuntime::make(&layer).expect("layer builds"));

    // Dispose on a fiber of a SEPARATE runtime, so the interruptible fiber and
    // the runtime being disposed are not the same. `shutdown` waits for its
    // own live fibers, so a dispose fiber on the disposed runtime would be
    // waiting on itself. Immediate start: nothing here drains a latch.
    let host = ManagedRuntime::new();
    let disposing = host.runtime().fork_detach_with(
        {
            let runtime = Arc::clone(&runtime);
            move |_ctx| {
                runtime.dispose();
                Exit::<(), Infallible>::succeed(())
            }
        },
        ForkOpts::immediate(),
    );
    started.wait();
    // `interrupt()` would wait for the fiber to finish, and the fiber cannot
    // finish until we release the finalizer: request without waiting.
    disposing.interrupt_unsafe();
    release.complete(());
    let _ = disposing.join();
    // The pin disposes once more after the interruption; idempotence holds.
    runtime.dispose();

    assert_eq!(
        *ran.lock().unwrap(),
        vec!["flush logs", "close database"],
        "an interrupted dispose must still run every remaining finalizer"
    );
}
