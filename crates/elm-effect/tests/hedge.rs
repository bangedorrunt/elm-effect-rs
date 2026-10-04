//! Wave-3 policy bead (.14): `policy::hedge` for idempotent reads.
//!
//! Experience-first: latency wins only when the duplicate is provably safe,
//! so the fast test pins that a fast first sends zero duplicates.
//! Prove-it-works: the loser-cancel test pins the winner's value plus
//! observed cancellation of the loser.
#![cfg(feature = "tokio")]

use elm_effect::hedge;
use elm_effect::{Cause, ClockImpl, Duration, Exit, Runtime, TestClock};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Poll `cond` until true or 10s pass; panic with `what` on expiry.
fn wait_until(cond: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
}

#[test]
fn hedge_does_not_send_the_duplicate_when_fast() {
    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let first_calls = Arc::new(AtomicUsize::new(0));
    let dup_calls = Arc::new(AtomicUsize::new(0));

    let c1 = first_calls.clone();
    let d1 = dup_calls.clone();
    let clock_h: Arc<dyn ClockImpl> = clock.clone();
    let worker = std::thread::spawn(move || {
        hedge(
            &rt,
            clock_h,
            Duration::from_millis(100),
            move |_| {
                c1.fetch_add(1, Ordering::SeqCst);
                Exit::<u32, String>::succeed(42)
            },
            move |_| {
                d1.fetch_add(1, Ordering::SeqCst);
                Exit::<u32, String>::succeed(7)
            },
        )
    });

    // The hedge parks in its delay sleep; the immediate first is done by the
    // time we drive the clock, so waking it must return without forking the
    // duplicate.
    wait_until(
        || clock.pending_sleepers() == 1 && first_calls.load(Ordering::SeqCst) == 1,
        "hedge parked in delay sleep with first complete",
    );
    clock.adjust(Duration::from_millis(200));

    let exit = match worker.join() {
        Ok(exit) => exit,
        Err(_) => panic!("hedge thread panicked"),
    };
    assert_eq!(exit, Exit::Success(42));
    assert_eq!(dup_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn hedge_cancels_loser_after_duplicate_wins() {
    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let dup_calls = Arc::new(AtomicUsize::new(0));

    let c = cancelled.clone();
    let d1 = dup_calls.clone();
    let clock_h: Arc<dyn ClockImpl> = clock.clone();
    let worker = std::thread::spawn(move || {
        hedge(
            &rt,
            clock_h,
            Duration::from_millis(50),
            move |ctx| {
                loop {
                    if ctx.is_interrupted() {
                        c.store(true, Ordering::SeqCst);
                        return Exit::<u32, String>::Failure(Cause::interrupt(None));
                    }
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
            },
            move |_| {
                d1.fetch_add(1, Ordering::SeqCst);
                Exit::<u32, String>::succeed(7)
            },
        )
    });

    wait_until(
        || clock.pending_sleepers() == 1,
        "hedge parked in delay sleep",
    );
    clock.adjust(Duration::from_millis(100));

    // The duplicate runs once the delay elapses and wins immediately; the
    // still-blocked first is cancelled.
    wait_until(
        || dup_calls.load(Ordering::SeqCst) == 1,
        "duplicate started",
    );
    let exit = match worker.join() {
        Ok(exit) => exit,
        Err(_) => panic!("hedge thread panicked"),
    };
    assert_eq!(exit, Exit::Success(7));
    wait_until(
        || cancelled.load(Ordering::SeqCst),
        "loser observed interruption",
    );
}
