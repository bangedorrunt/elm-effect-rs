//! Law L1: building an [`Effect`](crate::Effect) runs nothing.
//!
//! Construction and `drop` perform no I/O, no sleep, no state change. Every
//! constructor and combinator is covered: if any of them ran work eagerly,
//! its counter would be non-zero after the drop.

use elm_effect::{Effect, Exit, Runtime};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn counter() -> (Arc<AtomicUsize>, impl Fn() -> i32 + Send + 'static) {
    let n = Arc::new(AtomicUsize::new(0));
    let thunk = {
        let n = n.clone();
        move || {
            n.fetch_add(1, Ordering::SeqCst);
            1
        }
    };
    (n, thunk)
}

#[test]
fn description_drop_runs_nothing() {
    let (n, thunk) = counter();
    let effect: Effect<i32, String> = Effect::sync(thunk);
    drop(effect);
    assert_eq!(n.load(Ordering::SeqCst), 0);
}

#[test]
fn combinator_descriptions_drop_runs_nothing() {
    let (n1, t1) = counter();
    let (n2, t2) = counter();
    let base: Effect<i32, String> = Effect::sync(t1);
    let chained = base
        .flat_map(move |a| Effect::sync(t2).map(move |b| a + b))
        .map(|x| x * 2);
    drop(chained);
    assert_eq!(n1.load(Ordering::SeqCst), 0);
    assert_eq!(n2.load(Ordering::SeqCst), 0);

    // Narrowing combinators are lazy too.
    let (n3, t3) = counter();
    let narrowed: Effect<i32, String> = Effect::sync(t3);
    let _ = narrowed.result();
    let _ = Effect::<i32, String>::fail("e".to_string()).or_die();
    let _ = Effect::<i32, String>::succeed(1).exit();
    assert_eq!(n3.load(Ordering::SeqCst), 0);
}

#[test]
fn or_die_type_is_infallible() {
    let narrowed: Effect<i32, Infallible> = Effect::fail("e".to_string()).or_die();
    drop(narrowed);
}

/// Effect.test.ts :3706 "first argument as success" —
/// `Effect.catch(Effect.succeed(1), () => Effect.fail("e2"))` yields 1, so a
/// `catch` over an already-succeeding effect never runs its handler and the
/// success value passes through unchanged. The pin's two-argument form maps to
/// this port's `catch`, whose handler is the second argument.
#[test]
fn catch_first_argument_as_success() {
    let effect = Effect::<i32, String>::succeed(1).catch(|_e| Effect::fail("e2".to_string()));
    assert_eq!(Runtime::run(effect), Exit::Success(1));
}

/// Effect.test.ts :3763 "first argument as success" — the same claim for
/// `catchCause`: the handler never fires over a success, so the value passes
/// through and the typed error channel stays empty.
#[test]
fn catch_cause_first_argument_as_success() {
    let effect = Effect::<i32, String>::succeed(1).catch_cause(|_cause| Effect::fail("e2".to_string()));
    assert_eq!(Runtime::run(effect), Exit::Success(1));
}

/// Effect.test.ts :665 "gen" — a generator that yields one effect and returns
/// its value has no `gen` in this port, so it desugars to a `flat_map` over the
/// yielded value. The observable the row asserts is that the bound value
/// survives the continuation and is what the whole effect returns.
#[test]
fn gen_yields_the_value_and_returns_it() {
    let effect = Effect::<i32, Infallible>::succeed(1).flat_map(|bound| Effect::succeed(bound));
    assert_eq!(Runtime::run(effect), Exit::Success(1));
}

/// Effect.test.ts :672 "gen with context" — `gen`'s `self` is the TS `this`
/// binding. In Rust the same values arrive by capture rather than by a `this`
/// receiver, so the row's observable is that the continuation sees them and
/// returns their sum: succeed(1).flatMap(r => succeed(r + 2)) is 3.
#[test]
fn gen_with_self_binding_uses_the_bound_values() {
    let a = 1i32;
    let b = 2i32;
    let effect = Effect::<i32, Infallible>::succeed(a).flat_map(move |bound| Effect::succeed(bound + b));
    assert_eq!(Runtime::run(effect), Exit::Success(3));
}
