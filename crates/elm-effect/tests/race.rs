//! Races: first success wins, losers are released, failures concatenate.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use elm_effect::{
    Cause, Effect, Exit, Reason, Runtime, race, race_all, race_all_first, race_first,
};

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
// Virtual-time races in Effect 4 (Effect.test.ts describe race / raceFirst)
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
    // onInterrupt ran. Effect 4 drives its clock in one 500ms step, which
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
                ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                    slow_released.fetch_add(1, Ordering::SeqCst);
                    Effect::<(), String>::succeed(())
                })
                .expect("acquire cannot fail");
                // Past the adjust below, so this side is still parked when
                // the winner returns and is interrupted rather than started.
                clock_slow.sleep(Duration::from_secs(5), &|| ctx.is_interrupted());
                Exit::Failure(Cause::interrupt(None))
            },
        )
    });

    wait_until(
        || clock.pending_sleepers() == 2,
        "both sides parked on the clock",
    );
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
                ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                    slow_released.fetch_add(1, Ordering::SeqCst);
                    Effect::<(), String>::succeed(())
                })
                .expect("acquire cannot fail");
                clock_slow.sleep(Duration::from_secs(5), &|| ctx.is_interrupted());
                Exit::Failure(Cause::interrupt(None))
            },
        )
    });

    wait_until(
        || clock.pending_sleepers() == 2,
        "both sides parked on the clock",
    );
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
                ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                    slow_released.fetch_add(1, Ordering::SeqCst);
                    Effect::<(), String>::succeed(())
                })
                .expect("acquire cannot fail");
                clock_slow.sleep(Duration::from_secs(5), &|| ctx.is_interrupted());
                Exit::Failure(Cause::interrupt(None))
            },
        )
    });

    wait_until(
        || clock.pending_sleepers() == 2,
        "both sides parked on the clock",
    );
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
    // Ordering is load-bearing and follows Effect 4: onInterrupt wraps the
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
            ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                setter_released.fetch_add(1, Ordering::SeqCst);
                Effect::<(), String>::succeed(())
            })
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
    assert_eq!(
        released.load(Ordering::SeqCst),
        1,
        "the settler was interrupted"
    );
}

// ---------------------------------------------------------------------------
// raceAll / raceAllFirst in Effect 4 (Effect.test.ts, describe-level cases)
// ---------------------------------------------------------------------------

fn body<A: Send + 'static, E: Send + 'static>(
    f: impl FnOnce(&elm_effect::Ctx) -> elm_effect::Exit<A, E> + Send + 'static,
) -> elm_effect::Body<A, E> {
    Box::new(f)
}

#[test]
fn race_all_completes_on_the_first_success_not_the_first_completion() {
    // One body fails immediately (delay 0 then fail), the rest are delayed.
    // raceAll ignores that failure -- a failure is a loser, not an outcome --
    // and completes on the first SUCCESS (delay 100ms), which Effect 4
    // asserts as result === 100.
    use elm_effect::{ClockImpl, Duration, TestClock};

    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let clock_a = Arc::clone(&clock);
    let clock_b = Arc::clone(&clock);
    let clock_c = Arc::clone(&clock);

    let worker = std::thread::spawn(move || {
        race_all::<i64, String>(
            &rt,
            vec![
                // ms === 0: fails immediately.
                body(move |_| {
                    clock_a.sleep(Duration::from_millis(0), &|| false);
                    Exit::Failure(Cause::fail("boom".to_string()))
                }),
                // ms === 100: succeeds. Driven by the adjust below.
                body(move |_| {
                    clock_b.sleep(Duration::from_millis(100), &|| false);
                    Exit::Success(100)
                }),
                // ms === 5000: never reached, past the adjust. Deliberately
                // past the 500ms adjust (rather than Effect 4's 500) so that
                // only the 100ms body can answer -- otherwise both wake and
                // whichever sends first wins, which is nondeterministic.
                body(move |ctx| {
                    clock_c.sleep(Duration::from_millis(5000), &|| ctx.is_interrupted());
                    Exit::Success(5000)
                }),
            ],
        )
    });

    wait_until(|| clock.pending_sleepers() >= 1, "at least one body parked");
    clock.adjust(Duration::from_millis(500));

    let exit = worker.join().expect("worker thread");
    assert!(
        matches!(exit, Exit::Success(100)),
        "raceAll must complete on the first success, ignoring the immediate failure"
    );
}

#[test]
fn race_all_concatenates_causes_when_every_body_fails() {
    // With no success anywhere, the failures are concatenated.
    let rt = Runtime::new();
    let exit = race_all::<i32, String>(
        &rt,
        vec![
            body(|_| Exit::Failure(Cause::fail("first".to_string()))),
            body(|_| Exit::Failure(Cause::fail("second".to_string()))),
            body(|_| Exit::Failure(Cause::fail("third".to_string()))),
        ],
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
            failures.sort();
            assert_eq!(failures, vec!["first", "second", "third"]);
        }
        Exit::Success(_) => panic!("every body failed"),
    }
}

#[test]
fn race_all_first_completes_on_the_first_completion_even_a_failure() {
    // A failure ends raceAllFirst immediately, even though another body would
    // succeed later. Effect 4 asserts Exit.fail("boom") and that the later
    // success never starts.
    //
    // This test used to time out about one run in six. The 0ms failure is due
    // at virtual t=0, so it can settle the race before the main thread ever
    // observes a parked sleeper; `wait_until(pending_sleepers() >= 1)` then
    // spun to its 10s deadline against a race that had already finished. The
    // clock is never adjusted here. The success body arms its finalizer and
    // sets `armed`; the failing body waits for that signal before failing, so
    // every interleaving ends with the failure as the winner and the parked
    // success as the interrupted loser. The bounded recv turns a
    // wrong-semantics regression (a race that waits for a success, which would
    // park forever) into a named failure instead of a hung suite.
    use elm_effect::{ClockImpl, Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let clock_win = Arc::clone(&clock);
    let armed = Arc::new(AtomicBool::new(false));
    let armed_fail = Arc::clone(&armed);
    let armed_win = Arc::clone(&armed);
    let released = Arc::new(AtomicUsize::new(0));
    let released_win = Arc::clone(&released);

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let rt = Runtime::new();
        let exit = race_all_first::<i64, String>(
            &rt,
            vec![
                // delay 100 then succeed: parks on the clock, which is never
                // advanced, so it can only end by being interrupted.
                body(move |ctx| {
                    ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                        released_win.fetch_add(1, Ordering::SeqCst);
                        Effect::<(), String>::succeed(())
                    })
                    .expect("acquire cannot fail");
                    armed_win.store(true, Ordering::SeqCst);
                    clock_win.sleep(Duration::from_millis(100), &|| ctx.is_interrupted());
                    Exit::Success(100)
                }),
                // delay 0 then fail: completes first, so it wins the race.
                body(move |_| {
                    while !armed_fail.load(Ordering::SeqCst) {
                        std::thread::park_timeout(std::time::Duration::from_millis(1));
                    }
                    Exit::Failure(Cause::fail("boom".to_string()))
                }),
            ],
        );
        let _ = tx.send(exit);
    });

    let exit = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("raceAllFirst must end on the first completion, even a failure");
    worker.join().expect("race worker thread");
    assert!(
        matches!(exit, Exit::Failure(ref c) if c.failure() == Some(&"boom".to_string())),
        "raceAllFirst must end on the first completion, even a failure"
    );
    assert_eq!(
        released.load(Ordering::SeqCst),
        1,
        "the parked success was interrupted before its delay elapsed"
    );
}
