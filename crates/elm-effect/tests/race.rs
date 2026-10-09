//! Races: first success wins, losers are released, failures concatenate.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use elm_effect::{Cause, Effect, Exit, Reason, Runtime, race, race_first};

#[test]
fn race_failure_does_not_beat_later_success() {
    let rt = Runtime::new();
    let exit = race(
        &rt,
        |_| Exit::Failure(Cause::fail("early".to_string())),
        |_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            Exit::Success(7u32)
        },
    );
    assert!(matches!(exit, Exit::Success(7)));
}

#[test]
fn race_all_fail_concatenates_causes() {
    let rt = Runtime::new();
    let exit: Exit<u32, String> = race(
        &rt,
        |_| Exit::Failure(Cause::fail("left".to_string())),
        |_| Exit::Failure(Cause::fail("right".to_string())),
    );
    match exit {
        Exit::Failure(cause) => {
            let mut failures: Vec<&String> = cause
                .reasons
                .iter()
                .filter_map(|r| match r {
                    Reason::Fail(e) => Some(e),
                    _ => None,
                })
                .collect();
            // Simultaneous failures arrive in either order; both are kept.
            failures.sort();
            assert_eq!(failures, vec!["left", "right"]);
        }
        Exit::Success(_) => panic!("expected failure"),
    }
}

#[test]
fn race_loser_is_released() {
    let rt = Runtime::new();
    let released = Arc::new(AtomicUsize::new(0));
    let exit = race::<u32, String>(
        &rt,
        |_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            Exit::Success(1u32)
        },
        {
            let released = Arc::clone(&released);
            move |ctx| {
                ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                    released.fetch_add(1, Ordering::SeqCst);
                    Effect::<(), String>::succeed(())
                })
                .expect("acquire cannot fail");
                while !ctx.is_interrupted() {
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
                Exit::Failure(Cause::interrupt(None))
            }
        },
    );
    assert!(matches!(exit, Exit::Success(1)));
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn race_first_takes_any_completion() {
    let rt = Runtime::new();
    let exit: Exit<u32, String> = race_first(
        &rt,
        |_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Exit::Success(1)
        },
        |_| Exit::Failure(Cause::fail("fast".to_string())),
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"fast".to_string())),
        Exit::Success(_) => panic!("expected the fast failure"),
    }
}

// ---------------------------------------------------------------------------
// Virtual-time races at the pin (Effect.test.ts describe race / raceFirst)
// ---------------------------------------------------------------------------

/// Poll until `cond` holds or 10s pass; panic naming `what` on expiry.
fn wait_until(cond: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cond() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
}

#[test]
fn race_interrupts_the_loser_when_the_other_side_succeeds() {
    // Effect.race(delay(100, succeed("fast")), delay(500, succeed("slow")))
    // with TestClock.adjust("500 millis") => "fast", and the slow side's
    // onInterrupt ran. The pin drives its clock in one 500ms step, which
    // would wake BOTH sleepers here, so the slow side sleeps past the adjust
    // instead: it stays parked, race interrupts it, and its finalizer is the
    // observable "interrupted" signal.
    use elm_effect::{ClockImpl, Duration, TestClock};

    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let clock_fast = Arc::clone(&clock);
    let clock_slow = Arc::clone(&clock);
    let released = Arc::new(AtomicUsize::new(0));
    let slow_released = Arc::clone(&released);

    let worker = std::thread::spawn(move || {
        race::<&'static str, String>(
            &rt,
            move |_| {
                clock_fast.sleep(Duration::from_millis(100), &|| false);
                Exit::Success("fast")
            },
            move |ctx| {
                ctx.acquire_release(
                    Effect::<(), String>::succeed(()),
                    move |_| {
                        slow_released.fetch_add(1, Ordering::SeqCst);
                        Effect::<(), String>::succeed(())
                    },
                )
                .expect("acquire cannot fail");
                // Past the adjust below, so this side is still parked when
                // the winner returns and is interrupted rather than started.
                clock_slow.sleep(Duration::from_secs(5), &|| ctx.is_interrupted());
                Exit::Failure(Cause::interrupt(None))
            },
        )
    });

    wait_until(|| clock.pending_sleepers() == 2, "both sides parked on the clock");
    clock.adjust(Duration::from_millis(500));

    let exit = worker.join().expect("worker thread");
    assert!(matches!(exit, Exit::Success("fast")));
    // The loser was interrupted and released before the winner returned.
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn race_first_interrupts_the_loser_when_the_other_side_succeeds() {
    // raceFirst's observable claim is the same as race's here: the winner
    // returns and the parked loser is released.
    use elm_effect::{ClockImpl, Duration, TestClock};

    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let clock_fast = Arc::clone(&clock);
    let clock_slow = Arc::clone(&clock);
    let released = Arc::new(AtomicUsize::new(0));
    let slow_released = Arc::clone(&released);

    let worker = std::thread::spawn(move || {
        race_first::<&'static str, String>(
            &rt,
            move |_| {
                clock_fast.sleep(Duration::from_millis(100), &|| false);
                Exit::Success("fast")
            },
            move |ctx| {
                ctx.acquire_release(
                    Effect::<(), String>::succeed(()),
                    move |_| {
                        slow_released.fetch_add(1, Ordering::SeqCst);
                        Effect::<(), String>::succeed(())
                    },
                )
                .expect("acquire cannot fail");
                clock_slow.sleep(Duration::from_secs(5), &|| ctx.is_interrupted());
                Exit::Failure(Cause::interrupt(None))
            },
        )
    });

    wait_until(|| clock.pending_sleepers() == 2, "both sides parked on the clock");
    clock.adjust(Duration::from_millis(500));

    let exit = worker.join().expect("worker thread");
    assert!(matches!(exit, Exit::Success("fast")));
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn race_first_interrupts_the_loser_when_the_other_side_fails() {
    // raceFirst(any completion wins): the FAILING side decides, and the
    // still-parked loser is released.
    use elm_effect::{ClockImpl, Duration, TestClock};

    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let clock_fast = Arc::clone(&clock);
    let clock_slow = Arc::clone(&clock);
    let released = Arc::new(AtomicUsize::new(0));
    let slow_released = Arc::clone(&released);

    let worker = std::thread::spawn(move || {
        race_first::<&'static str, String>(
            &rt,
            move |_| {
                clock_fast.sleep(Duration::from_millis(100), &|| false);
                Exit::Failure(Cause::fail("boom".to_string()))
            },
            move |ctx| {
                ctx.acquire_release(
                    Effect::<(), String>::succeed(()),
                    move |_| {
                        slow_released.fetch_add(1, Ordering::SeqCst);
                        Effect::<(), String>::succeed(())
                    },
                )
                .expect("acquire cannot fail");
                clock_slow.sleep(Duration::from_secs(5), &|| ctx.is_interrupted());
                Exit::Failure(Cause::interrupt(None))
            },
        )
    });

    wait_until(|| clock.pending_sleepers() == 2, "both sides parked on the clock");
    clock.adjust(Duration::from_millis(500));

    let exit = worker.join().expect("worker thread");
    // Any completion decides, so the failure carries the outcome.
    assert!(matches!(exit, Exit::Failure(ref c) if c.failure() == Some(&"boom".to_string())));
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn race_interrupts_a_loser_that_settles_the_race_while_starting() {
    // "interrupts a loser that settles the race while starting": one side
    // awaits a deferred, the other SETTLES that deferred and then never
    // completes. The awaiter wins, and the settler -- now suspended in
    // never -- is interrupted, which is what its onInterrupt observes.
    //
    // Ordering is load-bearing and follows the pin: onInterrupt wraps the
    // whole settle-then-never effect, so the finalizer is registered BEFORE
    // the deferred is settled. Registering first closes the window where the
    // winner returns and the loser is interrupted before it armed its
    // finalizer, which would make this flaky.
    let gate = Arc::new(AtomicBool::new(false));
    let settler = Arc::clone(&gate);
    let awaiter = Arc::clone(&gate);
    let released = Arc::new(AtomicUsize::new(0));
    let setter_released = Arc::clone(&released);

    let rt = Runtime::new();
    let exit = race::<&'static str, String>(
        &rt,
        move |_| {
            while !awaiter.load(Ordering::SeqCst) {
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            Exit::Success("awaited")
        },
        move |ctx| {
            ctx.acquire_release(
                Effect::<(), String>::succeed(()),
                move |_| {
                    setter_released.fetch_add(1, Ordering::SeqCst);
                    Effect::<(), String>::succeed(())
                },
            )
            .expect("acquire cannot fail");
            // Settle the deferred the other side is waiting on, then never
            // complete: this side is the loser.
            settler.store(true, Ordering::SeqCst);
            while !ctx.is_interrupted() {
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            Exit::Failure(Cause::interrupt(None))
        },
    );

    assert!(matches!(exit, Exit::Success("awaited")));
    assert_eq!(released.load(Ordering::SeqCst), 1, "the settler was interrupted");
}
