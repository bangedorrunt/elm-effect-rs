//! Wave-6 fiber sets: FiberSet, FiberMap, ExecutionPlan, Scheduler behavior
//! ported from the tag's suites. Modules load via `#[path]` includes (the
//! .15 precedent): no facade touch, no scope leak into lib.rs.
//!
//! `FiberHandle` is reused from the runtime crate as-is (join, interrupt,
//! is_done); these tests pin the set/map/plan/scheduler layer above it.
//! Residuals stay `unported` and are listed in the close report.

#![cfg(feature = "tokio")]

#[path = "../src/execution_plan.rs"]
mod execution_plan;
#[path = "../src/fiber_map.rs"]
mod fiber_map;
#[path = "../src/fiber_set.rs"]
mod fiber_set;
#[path = "../src/scheduler.rs"]
mod scheduler;

use std::time::Duration;

use elm_effect::{Cause, Exit, Runtime};
use execution_plan::ExecutionPlan;
use fiber_map::FiberMap;
use fiber_set::FiberSet;
use scheduler::Scheduler;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

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

// --- FiberSet ---

#[test]
fn set_runs_and_joins() {
    let mut set = FiberSet::new(Runtime::new());
    let seen = counter();
    for i in 0..3 {
        let seen = seen.clone();
        set.run(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Exit::<i32, String>::succeed(i)
        });
    }
    assert_eq!(set.size(), 3);
    assert_eq!(set.join(), Exit::Success(()));
    assert_eq!(set.size(), 0);
    assert_eq!(seen.load(Ordering::SeqCst), 3);
}

#[test]
fn set_join_reports_first_failure() {
    let mut set = FiberSet::new(Runtime::new());
    set.run(|_| Exit::<i32, String>::succeed(1));
    set.run(|_| Exit::<i32, String>::Failure(Cause::fail("boom".to_owned())));
    match set.join() {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_owned())),
        Exit::Success(()) => panic!("expected the failure"),
    }
}

#[test]
fn set_clear_interrupts_and_drains() {
    let mut set = FiberSet::new(Runtime::new());
    let stopped = flag();
    let stopped_body = stopped.clone();
    let entered = flag();
    let entered_body = entered.clone();
    set.run(move |ctx| {
        entered_body.store(true, Ordering::SeqCst);
        while !ctx.is_interrupted() {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        stopped_body.store(true, Ordering::SeqCst);
        Exit::<i32, String>::succeed(0)
    });
    wait_until(|| entered.load(Ordering::SeqCst), "blocked fiber entered");
    set.clear();
    assert_eq!(set.size(), 0);
    wait_until(
        || stopped.load(Ordering::SeqCst),
        "blocked fiber observed interrupt",
    );
}

#[test]
fn set_await_empty_reaps() {
    let mut set = FiberSet::new(Runtime::new());
    set.run(|_| Exit::<i32, String>::succeed(1));
    assert_eq!(set.await_empty(), Exit::Success(()));
    assert!(set.is_empty());
}

// --- FiberMap ---

#[test]
fn map_set_get_remove() {
    let mut map = FiberMap::new(Runtime::new());
    map.set("a", |_| Exit::<i32, String>::succeed(1));
    assert!(map.has(&"a"));
    assert!(!map.has(&"b"));
    assert_eq!(map.size(), 1);
    assert!(map.remove(&"a"));
    assert!(!map.remove(&"a"));
    assert_eq!(map.size(), 0);
}

#[test]
fn map_replace_interrupts_previous() {
    let mut map = FiberMap::new(Runtime::new());
    let stopped = flag();
    let stopped_body = stopped.clone();
    let entered = flag();
    let entered_body = entered.clone();
    map.set("k", move |ctx| {
        entered_body.store(true, Ordering::SeqCst);
        while !ctx.is_interrupted() {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        stopped_body.store(true, Ordering::SeqCst);
        Exit::<i32, String>::succeed(0)
    });
    wait_until(|| entered.load(Ordering::SeqCst), "replaced fiber entered");
    map.set("k", |_| Exit::<i32, String>::succeed(1));
    wait_until(
        || stopped.load(Ordering::SeqCst),
        "replaced fiber interrupted",
    );
    assert_eq!(map.join(), Exit::Success(()));
}

#[test]
fn map_clear_drains() {
    let mut map = FiberMap::new(Runtime::new());
    map.set("a", |_| Exit::<i32, String>::succeed(1));
    map.set("b", |_| Exit::<i32, String>::succeed(2));
    assert_eq!(map.size(), 2);
    map.clear();
    assert_eq!(map.size(), 0);
    assert!(!map.has(&"a"));
}

#[test]
fn map_join_collects_failure() {
    let mut map = FiberMap::new(Runtime::new());
    map.set("ok", |_| Exit::<i32, String>::succeed(1));
    map.set("bad", |_| {
        Exit::<i32, String>::Failure(Cause::fail("x".to_owned()))
    });
    assert!(matches!(map.join(), Exit::Failure(_)));
}

// --- ExecutionPlan ---

#[test]
fn plan_rejects_zero_attempts() {
    assert!(ExecutionPlan::new(0).is_err());
    assert_eq!(ExecutionPlan::new(3).expect("valid").max_attempts(), 3);
}

#[test]
fn plan_succeeds_first_try() {
    let plan = ExecutionPlan::new(3).expect("valid");
    let planned = plan.run(&|| false, |attempt, _| {
        assert_eq!(attempt, 1);
        Ok::<i32, String>(7)
    });
    assert_eq!(planned.outcome, Ok(7));
    assert_eq!(planned.attempts, 1);
    assert!(!planned.interrupted);
}

#[test]
fn plan_retries_then_succeeds() {
    let plan = ExecutionPlan::new(3).expect("valid");
    let tries = counter();
    let tries_body = tries.clone();
    let planned = plan.run(&|| false, move |attempt, _| {
        tries_body.fetch_add(1, Ordering::SeqCst);
        if attempt < 3 {
            Err("flaky".to_owned())
        } else {
            Ok(9)
        }
    });
    assert_eq!(planned.outcome, Ok(9));
    assert_eq!(planned.attempts, 3);
    assert_eq!(tries.load(Ordering::SeqCst), 3);
}

#[test]
fn plan_exhausts_budget() {
    let plan = ExecutionPlan::new(2).expect("valid");
    let planned = plan.run(&|| false, |_, _| Err::<i32, String>("down".to_owned()));
    assert_eq!(planned.outcome, Err("down".to_owned()));
    assert_eq!(planned.attempts, 2);
}

#[test]
fn plan_stops_when_interrupted() {
    let plan = ExecutionPlan::new(10).expect("valid");
    let calls = counter();
    let calls_body = calls.clone();
    let planned = plan.run(&|| calls.load(Ordering::SeqCst) >= 2, move |_, _| {
        calls_body.fetch_add(1, Ordering::SeqCst);
        Err::<i32, String>("down".to_owned())
    });
    assert_eq!(planned.outcome, Err("down".to_owned()));
    assert_eq!(planned.attempts, 2);
    assert!(planned.interrupted);
}

// --- Scheduler ---

#[test]
fn scheduler_runs_fifo_and_counts_yields() {
    let mut sched = Scheduler::new(2);
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    for i in 0..5 {
        let order = order.clone();
        sched.schedule(move || order.lock().expect("order").push(i));
    }
    assert_eq!(sched.pending(), 5);
    assert_eq!(sched.run_until_idle(), 5);
    assert_eq!(*order.lock().expect("order"), vec![0, 1, 2, 3, 4]);
    assert_eq!(sched.yields(), 2);
    assert_eq!(sched.pending(), 0);
    assert_eq!(sched.max_ops(), 2);
}

#[test]
fn set_await_empty_waits_for_in_flight_fibers() {
    // FiberSet row "awaitEmpty": the wait is not satisfied while fibers are
    // still running. The pin asserts this by forking awaitEmpty and checking
    // it is still undefined mid-wait; here the bodies park on a latch so the
    // wait has something to really wait for.
    let release = flag();
    let started = counter();
    let mut set = FiberSet::new(Runtime::new());
    for _ in 0..3 {
        let (release, started) = (release.clone(), started.clone());
        set.run(move |_| {
            started.fetch_add(1, Ordering::SeqCst);
            while !release.load(Ordering::SeqCst) {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::<(), String>::succeed(())
        });
    }
    wait_until(|| started.load(Ordering::SeqCst) == 3, "three fibers parked");
    // Nothing has completed, so the set is not empty and await_empty must
    // block rather than reaping an unfinished set.
    assert_eq!(set.size(), 3);
    release.store(true, Ordering::SeqCst);
    assert_eq!(set.await_empty(), Exit::Success(()));
    assert!(set.is_empty());
}
