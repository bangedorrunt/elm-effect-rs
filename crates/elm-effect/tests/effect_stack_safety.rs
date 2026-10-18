//! Stack safety: nesting depth is not stack depth.
//!
//! Row 2713 at the effect@4.0.0 pin. The pin's `loop` builds a hundred thousand
//! `map(suspend(() => loop(n - 1)), +1)` continuations; a recursive interpreter
//! overflows its stack long before that, and the process aborts (a stack
//! overflow is not a panic `catch_unwind` can see), so this witness fails
//! loudly rather than quietly.

use elm_effect::{Effect, Exit, ManagedRuntime, Runtime};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

fn nested(depth: usize) -> Effect<usize, Infallible> {
    if depth == 0 {
        Effect::succeed(0)
    } else {
        // `suspend` is what keeps the *build* finite: the inner level does not
        // exist until the interpreter asks for it.
        Effect::suspend(move || nested(depth - 1)).map(|n| n + 1)
    }
}

#[test]
fn deeply_nested_map_continuations_run() {
    match Runtime::run(nested(100_000)) {
        Exit::Success(value) => assert_eq!(value, 100_000),
        Exit::Failure(cause) => panic!("the chain failed: {cause:?}"),
    }
}

/// An endless tail-recursive chain: `void.flatMap(_ => loop)`, the pin's
/// `const loop = Effect.void.pipe(Effect.flatMap(() => loop))`. Each level is
/// built only when the interpreter asks for it, so the description stays
/// finite and the loop's `frames` stays at one.
fn runaway_loop(steps: Arc<AtomicU64>) -> Effect<(), Infallible> {
    Effect::suspend(move || {
        let steps = Arc::clone(&steps);
        Effect::succeed(()).flat_map(move |()| {
            steps.fetch_add(1, Ordering::Relaxed);
            runaway_loop(Arc::clone(&steps))
        })
    })
}

/// Effect.test.ts:2704 `stack safety / recursion`. The pin runs this loop under
/// `Effect.timeoutOption(50)`, which interrupts the loser; the port interrupts
/// the fiber directly, the same flag and the same wait.
///
/// Without a poll inside the interpreter loop this test does not fail, it
/// hangs: the fiber only ends when the effect ends, and this effect never
/// does. The watchdog turns that into a loud abort naming the cause.
#[test]
fn an_endless_flat_map_chain_stops_on_interrupt() {
    let steps = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog_done = Arc::clone(&done);
    std::thread::spawn(move || {
        let start = Instant::now();
        while !watchdog_done.load(Ordering::SeqCst) {
            if start.elapsed() > Duration::from_secs(10) {
                eprintln!(
                    "the endless chain never stopped: the interpreter is not polling interruption"
                );
                std::process::abort();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });

    let rt = ManagedRuntime::new();
    // Immediate start: the chain has to be running before the test can stop it.
    let handle = rt.run_fork(runaway_loop(Arc::clone(&steps)));
    // Wait for the loop to prove it really is endless, then stop it.
    while steps.load(Ordering::SeqCst) < 100_000 {
        std::thread::sleep(Duration::from_millis(1));
    }
    handle.interrupt();
    let exit = handle.join();
    done.store(true, Ordering::SeqCst);

    match exit {
        Exit::Success(()) => panic!("the endless chain finished"),
        Exit::Failure(cause) => assert!(
            cause.is_interrupted(),
            "stopping the chain must be an interruption, not a failure: {cause:?}"
        ),
    }
    assert!(
        steps.load(Ordering::SeqCst) >= 100_000,
        "the chain stopped before it proved it was endless"
    );
}

#[test]
fn a_nested_failure_still_short_circuits() {
    // The trampoline must not change what a failure does: no continuation
    // below the failing step runs, and the cause survives unchanged.
    let effect: Effect<usize, &'static str> = Effect::suspend(|| Effect::fail("boom"))
        .map(|n: usize| n + 1)
        .flat_map(|n| Effect::succeed(n + 1));
    match Runtime::run(effect) {
        Exit::Success(value) => panic!("expected the failure, got {value}"),
        Exit::Failure(cause) => {
            assert_eq!(cause.failures().len(), 1);
            assert_eq!(cause.failures()[0], &"boom");
        }
    }
}
