//! `Effect.timeout`, run against `runtime::{timeout, timeout_fail}`.
//!
//! Effect 4 drives `TestClock`; `runtime::timeout` blocks until a racer answers,
//! so the test thread calls it and a helper thread advances the clock. The
//! helper cannot advance once and stop: the deadline sleeper may park after
//! that adjust, and its deadline would then move past the single step. It
//! advances until the call returns instead.

use elm_effect::{CauseError, ClockImpl, Ctx, Duration, Exit, Runtime, SleepOutcome, TestClock};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How long the body parks, far past any step the driver takes.
const BODY_SLEEP_MS: u64 = 600_000;

/// Advance `clock` by `step_ms` whenever a sleeper has parked, until `done`.
///
/// Every step is followed by a 1ms park: the loop is far faster than the body
/// it is racing, and an unthrottled driver can burn past the losing body's own
/// deadline before that body sees the interrupt, which turns an interrupted
/// sleep into an elapsed one. A wall-clock cap keeps a broken driver from
/// running forever.
fn drive(clock: Arc<TestClock>, step_ms: u64, done: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        while !done.load(Ordering::SeqCst) {
            if clock.pending_sleepers() > 0 {
                clock.adjust(Duration::from_millis(step_ms));
            }
            std::thread::park_timeout(std::time::Duration::from_millis(1));
            if start.elapsed() > std::time::Duration::from_secs(30) {
                return;
            }
        }
    });
}

/// A body that sleeps on the clock and then succeeds. The interrupted sleep is
/// what tells the timeout's interrupt arrived.
fn sleeping_body(
    clock: Arc<TestClock>,
    seen: Arc<AtomicBool>,
) -> impl FnOnce(&Ctx) -> Exit<bool, CauseError> + Send + 'static {
    move |ctx| {
        let outcome = clock.sleep(Duration::from_millis(BODY_SLEEP_MS), &|| {
            ctx.is_interrupted()
        });
        if outcome == SleepOutcome::Interrupted {
            seen.store(true, Ordering::SeqCst);
        }
        Exit::succeed(true)
    }
}

/// Effect.test.ts:2004 `timeout a long computation`. The 60s body never
/// answers: `timeoutOption` stands in for the crate's `Option`-returning shape.
#[test]
fn timeout_option_returns_none_for_a_long_computation() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let seen = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    drive(Arc::clone(&clock), 10, Arc::clone(&done));

    let exit = elm_effect::timeout(
        &rt,
        clock.clone(),
        Duration::from_millis(10),
        sleeping_body(Arc::clone(&clock), Arc::clone(&seen)),
    );
    done.store(true, Ordering::SeqCst);
    assert_eq!(exit, Exit::Success(None));
    // The loser is interrupted, not merely abandoned.
    assert!(
        seen.load(Ordering::SeqCst),
        "the body observed the interrupt"
    );
}

/// Effect.test.ts:2064 `timeout a long computation` (the failing variant). The
/// error carries Effect 4's message.
#[test]
fn timeout_fail_reports_the_pins_message() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let seen = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    drive(Arc::clone(&clock), 10, Arc::clone(&done));

    let exit = elm_effect::timeout_fail(
        &rt,
        clock.clone(),
        Duration::from_millis(10),
        sleeping_body(Arc::clone(&clock), Arc::clone(&seen)),
        CauseError::timeout_after,
    );
    done.store(true, Ordering::SeqCst);
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
    let done = Arc::new(AtomicBool::new(false));
    drive(Arc::clone(&clock), 2000, Arc::clone(&done));

    let exit = elm_effect::timeout_fail(
        &rt,
        clock.clone(),
        Duration::from_millis(1500),
        sleeping_body(Arc::clone(&clock), Arc::clone(&seen)),
        CauseError::timeout_after,
    );
    done.store(true, Ordering::SeqCst);
    match exit {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&CauseError::timeout("Operation timed out after '1s 500ms'"))
        ),
        Exit::Success(_) => panic!("the deadline arrived first"),
    }
    assert!(
        seen.load(Ordering::SeqCst),
        "the body observed the interrupt"
    );
}

/// Effect.test.ts:2077 `timeout interrupts the effect`. The body is still
/// running when the deadline arrives, and it sees the interrupt.
#[test]
fn timeout_fail_interrupts_the_effect() {
    let clock = Arc::new(TestClock::new());
    let rt = Runtime::new();
    let finished = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    drive(Arc::clone(&clock), 10, Arc::clone(&done));

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
    done.store(true, Ordering::SeqCst);
    match exit {
        Exit::Failure(cause) => {
            assert!(matches!(cause.failure(), Some(CauseError::Timeout(_))))
        }
        Exit::Success(_) => panic!("the deadline arrived first"),
    }
    assert!(
        finished.load(Ordering::SeqCst),
        "the interrupt reached the body"
    );
}
