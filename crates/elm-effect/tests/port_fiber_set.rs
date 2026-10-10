//! hi5 tail slice: FiberSet external-interruption cells.
//! Ports packages/effect/test/FiberSetRuntimePropagation.test.ts (external
//! interruption is recorded), left uncovered by the area beads.
//! Modules load via `#[path]` like fiber_sets.rs: no facade touch.
#![cfg(feature = "tokio")]

#[path = "../src/fiber_set.rs"]
mod fiber_set;

use elm_effect::{Cause, Exit, Runtime, is_interrupt_reason};
use fiber_set::FiberSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
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
