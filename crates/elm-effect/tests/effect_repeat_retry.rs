//! `Effect.repeat`/`Effect.retry` option objects at the effect@4.0.0 pin.
//!
//! The sixteen rows here cover `{ until, while }` (plain and effectful) and
//! `{ schedule, until|while }`. The pin's rows push failures through
//! `Effect.flip` because `retry` only accepts a failing effect; this port's
//! driver takes a failing recipe directly, so the same counters drive it.

use elm_effect::{Effect, Exit, LoopOptions, Runtime, Schedule, repeat_with, retry_with};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Counter(Arc<Mutex<i64>>);

impl Counter {
    fn new(value: i64) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    fn get(&self) -> i64 {
        *self.0.lock().unwrap()
    }

    fn add(&self, delta: i64) -> i64 {
        let mut guard = self.0.lock().unwrap();
        *guard += delta;
        *guard
    }
}

fn success<A: std::fmt::Debug, E: std::fmt::Debug>(exit: Exit<A, E>) -> A {
    match exit {
        Exit::Success(value) => value,
        other => panic!("expected success, got {other:?}"),
    }
}

fn failure<A: std::fmt::Debug, E: Clone + std::fmt::Debug>(exit: Exit<A, E>) -> E {
    match exit {
        Exit::Failure(cause) => cause
            .failure()
            .cloned()
            .unwrap_or_else(|| panic!("expected a typed failure, got {cause:?}")),
        other => panic!("expected failure, got {other:?}"),
    }
}

/// The pin's repeat row: decrement the input on every run and count runs.
fn decrement(
    input: &Counter,
    output: &Counter,
) -> impl Fn() -> Effect<i64, i64> + Send + Sync + 'static {
    let (input, output) = (input.clone(), output.clone());
    move || {
        let (input, output) = (input.clone(), output.clone());
        Effect::sync(move || {
            output.add(1);
            input.add(-1)
        })
    }
}

/// The pin's retry row: the same decrement, flipped into a typed failure.
fn decrement_then_fail(
    input: &Counter,
    output: &Counter,
) -> impl Fn() -> Effect<i64, i64> + 'static {
    let (input, output) = (input.clone(), output.clone());
    move || {
        let (input, output) = (input.clone(), output.clone());
        Effect::<i64, i64>::sync(move || {
            output.add(1);
            input.add(-1)
        })
        .flat_map(|n| Effect::<i64, i64>::fail(n))
    }
}

// ---------------------------------------------------------------------------
// repeat
// ---------------------------------------------------------------------------

#[test]
fn repeat_until_repeats_until_a_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(
        decrement(&input, &output),
        LoopOptions::new().until(|n: &i64| *n == 0),
    ));
    assert_eq!(success(exit), 0);
    assert_eq!(output.get(), 10, "one run per decrement");
}

#[test]
fn repeat_until_repeats_until_an_effectful_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(
        decrement(&input, &output),
        LoopOptions::new().until_effect(|n: &i64| Effect::<bool, i64>::succeed(*n == 0)),
    ));
    assert_eq!(success(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn repeat_until_always_evaluates_at_least_once() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
        }
    };
    let exit: Exit<i64, i64> =
        Runtime::run(repeat_with(run, LoopOptions::new().until(|_: &i64| true)));
    assert_eq!(success(exit), 1);
    assert_eq!(n.get(), 1, "the condition is checked after the first run");
}

#[test]
fn repeat_while_repeats_while_a_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(
        decrement(&input, &output),
        LoopOptions::new().while_(|n: &i64| *n > 0),
    ));
    assert_eq!(success(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn repeat_while_repeats_while_an_effectful_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(
        decrement(&input, &output),
        LoopOptions::new().while_effect(|n: &i64| Effect::<bool, i64>::succeed(*n > 0)),
    ));
    assert_eq!(success(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn repeat_while_always_evaluates_at_least_once() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
        }
    };
    let exit: Exit<i64, i64> =
        Runtime::run(repeat_with(run, LoopOptions::new().while_(|_: &i64| false)));
    assert_eq!(success(exit), 1);
    assert_eq!(n.get(), 1);
}

#[test]
fn repeat_times_runs_twice_plus_the_first() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
        }
    };
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(run, LoopOptions::new().times(2)));
    assert_eq!(success(exit), 3);
    assert_eq!(n.get(), 3, "two repeats after the first run");
}

#[test]
fn repeat_schedule_with_until() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
        }
    };
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(
        run,
        LoopOptions::new()
            .schedule(Schedule::recurs(3).ignore_output())
            .until(|value: &i64| *value == 3),
    ));
    assert_eq!(success(exit), 3);
    assert_eq!(n.get(), 3);
}

#[test]
fn repeat_schedule_with_while() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
        }
    };
    let exit: Exit<i64, i64> = Runtime::run(repeat_with(
        run,
        LoopOptions::new()
            .schedule(Schedule::recurs(3).ignore_output())
            .while_(|value: &i64| *value < 3),
    ));
    assert_eq!(success(exit), 3);
    assert_eq!(n.get(), 3);
}

// ---------------------------------------------------------------------------
// retry predicates
// ---------------------------------------------------------------------------

#[test]
fn retry_until_retries_until_a_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(retry_with(
        decrement_then_fail(&input, &output),
        LoopOptions::new().until(|n: &i64| *n == 0),
    ));
    assert_eq!(failure(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn retry_until_retries_until_an_effectful_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(retry_with(
        decrement_then_fail(&input, &output),
        LoopOptions::new().until_effect(|n: &i64| Effect::<bool, i64>::succeed(*n == 0)),
    ));
    assert_eq!(failure(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn retry_until_always_evaluates_at_least_once() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
                .flat_map(|value| Effect::<i64, i64>::fail(value))
        }
    };
    let exit: Exit<i64, i64> =
        Runtime::run(retry_with(run, LoopOptions::new().until(|_: &i64| true)));
    assert_eq!(failure(exit), 1);
    assert_eq!(
        n.get(),
        1,
        "the condition is checked after the first attempt"
    );
}

#[test]
fn retry_while_retries_while_a_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(retry_with(
        decrement_then_fail(&input, &output),
        LoopOptions::new().while_(|n: &i64| *n > 0),
    ));
    assert_eq!(failure(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn retry_while_retries_while_an_effectful_condition_is_true() {
    let input = Counter::new(10);
    let output = Counter::new(0);
    let exit: Exit<i64, i64> = Runtime::run(retry_with(
        decrement_then_fail(&input, &output),
        LoopOptions::new().while_effect(|n: &i64| Effect::<bool, i64>::succeed(*n > 0)),
    ));
    assert_eq!(failure(exit), 0);
    assert_eq!(output.get(), 10);
}

#[test]
fn retry_while_always_evaluates_at_least_once() {
    let n = Counter::new(0);
    let run = {
        let n = n.clone();
        move || {
            let n = n.clone();
            Effect::<i64, i64>::sync(move || n.add(1))
                .flat_map(|value| Effect::<i64, i64>::fail(value))
        }
    };
    let exit: Exit<i64, i64> =
        Runtime::run(retry_with(run, LoopOptions::new().while_(|_: &i64| false)));
    assert_eq!(failure(exit), 1);
    assert_eq!(n.get(), 1);
}

// ---------------------------------------------------------------------------
// retry with a base schedule
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum RetryError {
    Count(i64),
    Boom,
}

/// Count the attempt in the error so the schedule and predicates can see it.
fn count_then_fail(n: &Counter) -> impl Fn() -> Effect<i64, RetryError> + 'static {
    let n = n.clone();
    move || {
        let n = n.clone();
        Effect::<RetryError, RetryError>::sync(move || RetryError::Count(n.add(1)))
            .flat_map(|error| Effect::<i64, RetryError>::fail(error))
    }
}

/// `recurs(3)` outputs `u64`; `until`/`while` only need the decision.
fn recurs_3() -> Schedule<RetryError, ()> {
    Schedule::recurs(3).ignore_output()
}

#[test]
fn retry_schedule_retries_according_to_the_specified_schedule() {
    let n = Counter::new(0);
    let exit: Exit<i64, RetryError> = Runtime::run(retry_with(
        count_then_fail(&n),
        LoopOptions::new().schedule(recurs_3()),
    ));
    assert_eq!(
        failure(exit),
        RetryError::Count(4),
        "three retries after the first"
    );
    assert_eq!(n.get(), 4);
}

#[test]
fn retry_schedule_with_until() {
    let n = Counter::new(0);
    let exit: Exit<i64, RetryError> = Runtime::run(retry_with(
        count_then_fail(&n),
        LoopOptions::new()
            .schedule(recurs_3())
            .until(|error: &RetryError| *error == RetryError::Count(3)),
    ));
    assert_eq!(failure(exit), RetryError::Count(3));
    assert_eq!(n.get(), 3);
}

#[test]
fn retry_schedule_until_errors() {
    let n = Counter::new(0);
    let exit: Exit<i64, RetryError> = Runtime::run(retry_with(
        count_then_fail(&n),
        LoopOptions::new()
            .schedule(recurs_3())
            .until_effect(|_: &RetryError| Effect::<bool, RetryError>::fail(RetryError::Boom)),
    ));
    assert_eq!(failure(exit), RetryError::Boom);
    assert_eq!(
        n.get(),
        1,
        "the predicate fails on the first attempt and stops the loop"
    );
}

#[test]
fn retry_schedule_with_while() {
    let n = Counter::new(0);
    let exit: Exit<i64, RetryError> = Runtime::run(retry_with(
        count_then_fail(&n),
        LoopOptions::new()
            .schedule(recurs_3())
            .while_(|error: &RetryError| matches!(error, RetryError::Count(c) if *c < 3)),
    ));
    assert_eq!(failure(exit), RetryError::Count(3));
    assert_eq!(n.get(), 3);
}

#[test]
fn retry_schedule_while_errors() {
    let n = Counter::new(0);
    let exit: Exit<i64, RetryError> = Runtime::run(retry_with(
        count_then_fail(&n),
        LoopOptions::new()
            .schedule(recurs_3())
            .while_effect(|_: &RetryError| Effect::<bool, RetryError>::fail(RetryError::Boom)),
    ));
    assert_eq!(failure(exit), RetryError::Boom);
    assert_eq!(n.get(), 1);
}

// ---------------------------------------------------------------------------
// retry: a success is never retried
// ---------------------------------------------------------------------------

#[test]
fn retry_nothing_on_success() {
    // Pin :1573 — `retry({ times: 10000 })` over a successful effect runs it
    // once; the repeat budget only applies after a failure.
    let count = Counter::new(0);
    let run = {
        let count = count.clone();
        move || {
            let count = count.clone();
            Effect::<i64, i64>::sync(move || count.add(1))
        }
    };
    let exit: Exit<i64, i64> = Runtime::run(retry_with(run, LoopOptions::new().times(10_000)));
    assert_eq!(success(exit), 1);
    assert_eq!(count.get(), 1, "a success is not retried");
}
