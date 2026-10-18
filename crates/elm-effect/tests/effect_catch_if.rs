//! `Effect.catchIf` refinement rows at the effect@4.0.0 pin.
//!
//! The pin's `catchIf` takes a refinement plus an optional `orElse`. Rust has
//! no refinement types and no overloading, so the refinement is a predicate
//! over the error and the two-argument form re-raises a declined error while
//! `catch_if_else` runs the fallback instead.

use elm_effect::{Effect, Exit, Runtime};

#[derive(Clone, Debug, PartialEq)]
enum Error {
    A,
    B,
}

#[test]
fn catch_if_with_refinement_reraises_a_declined_error() {
    // Pin :3205 — the refinement only admits `ErrorA`, so the `ErrorB` failure
    // comes out of `catchIf` untouched.
    let effect: Effect<i64, Error> = Effect::fail(Error::B).catch_if(
        |error| *error == Error::A,
        |_| Effect::succeed(0),
    );
    assert_eq!(Runtime::run(effect), Exit::Failure(elm_effect::Cause::fail(Error::B)));
}

#[test]
fn catch_if_with_refinement_or_else() {
    // Pin :3222 — the same declined error, but `orElse` turns it into a
    // success instead of re-raising.
    let effect: Effect<i64, Error> = Effect::fail(Error::B).catch_if_else(
        |error| *error == Error::A,
        |_| Effect::succeed(0),
        |error| Effect::succeed(if error == Error::B { 1 } else { -1 }),
    );
    assert_eq!(Runtime::run(effect), Exit::Success(1));
}

#[test]
fn catch_if_with_refinement_handles_an_accepted_error() {
    // The accepting branch, so the test above cannot pass by never calling a
    // handler at all.
    let effect: Effect<i64, Error> = Effect::fail(Error::A).catch_if_else(
        |error| *error == Error::A,
        |_| Effect::succeed(7),
        |_| Effect::succeed(0),
    );
    assert_eq!(Runtime::run(effect), Exit::Success(7));
}
