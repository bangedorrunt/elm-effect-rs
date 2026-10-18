//! Stack safety: nesting depth is not stack depth.
//!
//! Row 2713 at the effect@4.0.0 pin. The pin's `loop` builds a hundred thousand
//! `map(suspend(() => loop(n - 1)), +1)` continuations; a recursive interpreter
//! overflows its stack long before that, and the process aborts (a stack
//! overflow is not a panic `catch_unwind` can see), so this witness fails
//! loudly rather than quietly.

use elm_effect::{Effect, Exit, Runtime};
use std::convert::Infallible;

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
