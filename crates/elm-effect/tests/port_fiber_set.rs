//! hi5 tail slice: FiberSet external-interruption cells.
//! Ports packages/effect/test/FiberSetRuntimePropagation.test.ts (external
//! interruption is recorded), left uncovered by the area beads.
//! Modules load via `#[path]` like fiber_sets.rs: no facade touch.
#![cfg(feature = "tokio")]

#[allow(clippy::type_complexity)]
#[path = "../src/fiber_set.rs"]
mod fiber_set;

use elm_effect::{Cause, Ctx, Exit, Runtime, is_interrupt_reason};
use fiber_set::FiberSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

/// A member that runs until interrupted: it records that it entered, then
/// spins on the interrupt flag. This is how the pin's `Effect.never` bodies
/// end — an interruption, not a value.
fn blocking_body(
    entered: Arc<AtomicUsize>,
) -> impl FnOnce(&Ctx) -> Exit<i32, String> + Send + 'static {
    move |ctx| {
        entered.fetch_add(1, Ordering::SeqCst);
        while !ctx.is_interrupted() {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        Exit::Failure(Cause::interrupt(None))
    }
}

fn wait_until(cond: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
}

// FiberSetRuntimePropagation.test.ts:6 — an externally interrupted member
// is recorded as the set's failure instead of being swallowed.
#[test]
fn set_interrupt_all_records_external_interruption() {
    let mut set = FiberSet::new(Runtime::new());
    let entered = flag();
    let entered_body = entered.clone();
    set.run(move |ctx| {
        entered_body.store(true, Ordering::SeqCst);
        while !ctx.is_interrupted() {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        Exit::<i32, String>::Failure(Cause::interrupt(None))
    });
    wait_until(|| entered.load(Ordering::SeqCst), "blocked fiber entered");
    set.interrupt_all();
    match set.join() {
        Exit::Failure(cause) => assert!(
            cause.reasons.iter().any(is_interrupt_reason),
            "join records the external interruption, got {cause:?}"
        ),
        Exit::Success(()) => panic!("expected the interruption failure"),
    }
    assert!(
        matches!(set.join(), Exit::Success(())),
        "drained set joins clean"
    );
}

// FiberSet.test.ts:7 — `{ startImmediately: false }` forks without starting:
// the body runs when the set is next yielded to, which the join does.
#[test]
fn set_run_defers_startup() {
    let mut set = FiberSet::new(Runtime::new());
    let started = flag();
    let body_flag = started.clone();
    set.run_deferred(move |_| {
        body_flag.store(true, Ordering::SeqCst);
        Exit::<i32, String>::succeed(0)
    });
    assert!(
        !started.load(Ordering::SeqCst),
        "a deferred member has not started"
    );
    assert_eq!(set.join(), Exit::Success(()));
    assert!(started.load(Ordering::SeqCst), "the join started it");
}

// FiberSet.test.ts:23 — the tag's `toJSON` discriminant.
#[test]
fn set_identifies_itself_in_json() {
    let set = FiberSet::<i32, String>::new(Runtime::new());
    assert_eq!(set.to_json().id, "FiberSet");
}

// FiberSet.test.ts:29 — ten members blocked forever, each with an
// interrupt observer; closing the set's scope runs all ten.
#[test]
fn set_scope_close_interrupts_running_members() {
    let set = FiberSet::<i32, String>::new(Runtime::new());
    let entered = counter();
    let interrupted = counter();
    for _ in 0..10 {
        let on_interrupt = interrupted.clone();
        set.run_observed(blocking_body(entered.clone()), move || {
            on_interrupt.fetch_add(1, Ordering::SeqCst);
        });
    }
    wait_until(
        || entered.load(Ordering::SeqCst) == 10,
        "all ten members entered",
    );
    let _defects = set.scope().close();
    wait_until(
        || interrupted.load(Ordering::SeqCst) == 10,
        "every member ran its interrupt observer",
    );
    assert_eq!(interrupted.load(Ordering::SeqCst), 10);
}

// FiberSet.test.ts:50 — `FiberSet.runtime(set)` forks from outside an
// effect and tracks into the same set, so the same scope close stops them.
#[test]
fn set_runtime_forks_into_the_set() {
    let set = FiberSet::<i32, String>::new(Runtime::new());
    let run = set.runtime();
    let entered = counter();
    let interrupted = counter();
    for _ in 0..10 {
        let on_interrupt = interrupted.clone();
        run.run_observed(blocking_body(entered.clone()), move || {
            on_interrupt.fetch_add(1, Ordering::SeqCst);
        });
    }
    assert_eq!(set.size(), 10, "the runtime tracks into the set");
    wait_until(
        || entered.load(Ordering::SeqCst) == 10,
        "all ten members entered",
    );
    let _defects = set.scope().close();
    wait_until(
        || interrupted.load(Ordering::SeqCst) == 10,
        "the set's scope close interrupted every member the runtime forked",
    );
}

// FiberSet.test.ts:94 — `propagateInterruption: false` means an external
// interruption is the fiber's business, not the set's.
#[test]
fn set_propagation_false_ignores_external_interruption() {
    let mut set = FiberSet::<i32, String>::new(Runtime::new());
    let entered = counter();
    let index = set.run_propagating(blocking_body(entered.clone()), false);
    wait_until(|| entered.load(Ordering::SeqCst) == 1, "the member entered");
    set.interrupt_member(index);
    assert!(
        !set.is_settled(),
        "a non-propagating member leaves the set unsettled"
    );
    let _ = set.join();
}

// FiberSet.test.ts:105 — with `propagateInterruption: true` the same
// interruption settles the set and reaches a later join.
#[test]
fn set_propagation_true_fails_join_on_external_interruption() {
    let mut set = FiberSet::<i32, String>::new(Runtime::new());
    let entered = counter();
    let index = set.run_propagating(blocking_body(entered.clone()), true);
    wait_until(|| entered.load(Ordering::SeqCst) == 1, "the member entered");
    set.interrupt_member(index);
    assert!(set.is_settled(), "the propagating member settles the set");
    match set.join() {
        Exit::Failure(cause) => assert!(
            cause.has_interrupts(),
            "join reports the interruption, got {cause:?}"
        ),
        Exit::Success(()) => panic!("expected the interruption to reach join"),
    }
}

// FiberSet.test.ts:135 — `makeRuntimePromise`: a caller outside an effect
// runs a body and gets its value, the promise's resolve.
#[test]
fn set_make_runtime_promise_resolves_outside_an_effect() {
    let result = FiberSet::<String, String>::runtime_promise(|_| Exit::succeed("done".to_owned()));
    assert_eq!(result, Exit::Success("done".to_owned()));
}
