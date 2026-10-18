//! `Effect.timeout` rows from `packages/effect/test/Effect.test.ts` at the
//! effect@4.0.0 pin, run against `runtime::{timeout, timeout_fail}`.
//!
//! The pin drives `TestClock`; `runtime::timeout` blocks until a racer answers,
//! so the test thread calls it and a helper thread advances the clock once the
//! deadline sleeper has parked.

use elm_effect::{Cause, CauseError, ClockImpl, Ctx, Duration, Exit, Runtime, TestClock};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Advance `clock` by `millis` once a sleeper has parked, from another thread.
/// Without the wait the adjust can land before the deadline is scheduled.
fn advance_after_park(clock: Arc<TestClock>, millis: u64) {
    std::thread::spawn(move || {
        while clock.pending_sleepers() == 0 {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        clock.adjust(Duration::from_millis(millis));
    });
}

/// A body that sleeps for `sleep_ms` on the clock and then succeeds. The
/// interrupted sleep is what tells the timeout's interrupt arrived.
fn sleeping_body(
    clock: Arc<TestClock>,
    sleep_ms: u64,
    seen: Arc<AtomicBool>,
) -> impl FnOnce(&Ctx) -> Exit<bool, CauseError> + Send + 'static {
    move |ctx| {
        // The interrupted flag is the timeout's own signal: the sleeper polls
        // it, so an interrupt returns the sleep early.
        let outcome = clock.sleep(Duration::from_millis(sleep_ms), &|| ctx.is_interrupted());
        if outcome == elm_effect::SleepOutcome::Interrupted {
            seen.store(true, Ordering::SeqCst);
        }
        Exit::succeed(true)
    }
}

/// Effect.test.ts:2004 `timeout a long computation`. The 60s body never
/// answers: `timeoutOption` stands in for the port's `Option`-returning shape.
#[test]
fn timeout_option_returns_none_for_a_long_computation() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let seen = Arc::new(AtomicBool::new(false));
    advance_after_park(Arc::clone(&clock), 10);

    let exit = elm_effect::timeout(
        &rt,
        clock.clone(),
        Duration::from_millis(10),
        sleeping_body(Arc::clone(&clock), 60_000, Arc::clone(&seen)),
    );
    assert_eq!(exit, Exit::Success(None));
    // The loser is interrupted, not merely abandoned.
    assert!(seen.load(Ordering::SeqCst), "the body observed the interrupt");
}

/// Effect.test.ts:2064 `timeout a long computation` (the failing variant). The
/// error carries the pin's message.
#[test]
fn timeout_fail_reports_the_pins_message() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let seen = Arc::new(AtomicBool::new(false));
    advance_after_park(Arc::clone(&clock), 10);

    let exit = elm_effect::timeout_fail(
        &rt,
        clock.clone(),
        Duration::from_millis(10),
        sleeping_body(Arc::clone(&clock), 60_000, Arc::clone(&seen)),
        CauseError::timeout_after,
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&CauseError::timeout("Operation timed out after '10ms'"))
        ),
        Exit::Success(_) => panic!("the deadline arrived first"),
    }
}

/// Effect.test.ts:2049 `timeout produces a useful error message`. 1500ms
/// renders as `1s 500ms`, and the body is interrupted on the way out.
#[test]
fn timeout_fail_renders_seconds_and_millis() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let seen = Arc::new(AtomicBool::new(false));
    advance_after_park(Arc::clone(&clock), 2000);

    let exit = elm_effect::timeout_fail(
        &rt,
        clock.clone(),
        Duration::from_millis(1500),
        sleeping_body(Arc::clone(&clock), 60_000, Arc::clone(&seen)),
        CauseError::timeout_after,
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&CauseError::timeout(
                "Operation timed out after '1s 500ms'"
            ))
        ),
        Exit::Success(_) => panic!("the deadline arrived first"),
    }
    assert!(seen.load(Ordering::SeqCst), "the body observed the interrupt");
}

/// Effect.test.ts:2077 `timeout interrupts the effect`. The body is still
/// running when the deadline arrives, and it sees the interrupt.
#[test]
fn timeout_fail_interrupts_the_effect() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let finished = Arc::new(AtomicBool::new(false));
    advance_after_park(Arc::clone(&clock), 10);

    let body_finished = Arc::clone(&finished);
    let exit = elm_effect::timeout_fail(
        &rt,
        clock.clone(),
        Duration::from_millis(10),
        move |ctx: &Ctx| {
            // Wait for the interrupt rather than finishing: this is `never`.
            while !ctx.is_interrupted() {
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            body_finished.store(true, Ordering::SeqCst);
            Exit::succeed(())
        },
        CauseError::timeout_after,
    );
    match exit {
        Exit::Failure(cause) => assert!(matches!(
            cause.failure(),
            Some(CauseError::Timeout(_))
        )),
        Exit::Success(_) => panic!("the deadline arrived first"),
    }
    assert!(
        finished.load(Ordering::SeqCst),
        "the interrupt reached the body"
    );
}
