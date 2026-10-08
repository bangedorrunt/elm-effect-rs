//! `Effect` conversions at the pin (`packages/effect/test/Effect.test.ts`,
//! describe blocks `fromOption`, `transposeOption`, `fromResult`).
//!
//! The edge is `ManagedRuntime::run` (L3: only the edge interprets a
//! description). A clean typed failure carries `Reason::Fail`; that is what
//! these rows assert on.

use elm_effect::{Effect, Exit, ManagedRuntime, Reason};

fn run<A: Send + 'static>(effect: Effect<A, String, ()>) -> Exit<A, String> {
    ManagedRuntime::new().run(effect)
}

#[test]
fn from_option_some_succeeds() {
    // Effect.fromOption(Some("A")) succeeds with the inner value.
    let effect: Effect<&str, String, ()> =
        Effect::from_option(Some("A"), || "missing".to_string());
    assert!(matches!(run(effect), Exit::Success("A")));
}

#[test]
fn from_option_none_fails_with_the_supplied_error() {
    // The pin defaults to NoSuchElementError; this port takes the error
    // builder explicitly, so a `None` fails with exactly that error.
    let effect: Effect<&str, String, ()> =
        Effect::from_option(None, || "Missing value".to_string());
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"Missing value".to_string())),
        Exit::Success(_) => panic!("expected a typed failure"),
    }
}

#[test]
fn from_option_none_failure_is_a_typed_fail_not_a_defect() {
    let effect: Effect<&str, String, ()> =
        Effect::from_option(None, || "Missing value".to_string());
    match run(effect) {
        Exit::Failure(cause) => {
            assert!(cause.defects().is_empty(), "a none is a typed failure, not a defect");
            assert!(!cause.is_interrupted(), "a none is not an interrupt");
        }
        Exit::Success(_) => panic!("expected a typed failure"),
    }
}

#[test]
fn from_result_success_succeeds() {
    let effect: Effect<&str, String, ()> = Effect::from_result(Ok("A"));
    assert!(matches!(run(effect), Exit::Success("A")));
}

#[test]
fn from_result_failure_fails_with_its_error() {
    let effect: Effect<&str, String, ()> = Effect::from_result(Err("error".to_string()));
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"error".to_string())),
        Exit::Success(_) => panic!("expected a typed failure"),
    }
}

#[test]
fn transpose_option_none_succeeds_with_none() {
    let effect: Effect<Option<&str>, String, ()> = Effect::transpose_option(None);
    assert!(matches!(run(effect), Exit::Success(Option::None)));
}

#[test]
fn transpose_option_some_of_success_succeeds_with_some() {
    let inner: Effect<&str, String, ()> = Effect::succeed("A");
    let effect = Effect::transpose_option(Some(inner));
    assert!(matches!(run(effect), Exit::Success(Some("A"))));
}

#[test]
fn transpose_option_some_of_failure_keeps_the_error() {
    // The inner effect is not run until interpreted; its failure is not
    // swallowed by the transpose.
    let inner: Effect<&str, String, ()> = Effect::fail("error".to_string());
    let effect = Effect::transpose_option(Some(inner));
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"error".to_string())),
        Exit::Success(_) => panic!("expected the inner failure"),
    }
}

#[test]
fn conversions_are_descriptions_not_results() {
    // L1: constructing and dropping these performs nothing. The counter
    // proves the error builder never ran for a `Some`.
    let calls = std::cell::Cell::new(0u32);
    let effect: Effect<&str, String, ()> =
        Effect::from_option(Some("A"), || {
            calls.set(calls.get() + 1);
            "missing".to_string()
        });
    drop(effect);
    assert_eq!(calls.get(), 0, "constructing a description runs nothing");
}

#[test]
fn typed_failure_stays_a_fail_reason() {
    // The row asserts the error channel type: a `Fail`, never a `Die`.
    let effect: Effect<&str, String, ()> = Effect::from_result(Err("e".to_string()));
    match run(effect) {
        Exit::Failure(cause) => {
            assert!(matches!(cause.reasons.as_slice(), [Reason::Fail(_)]));
        }
        Exit::Success(_) => panic!("expected a typed failure"),
    }
}
