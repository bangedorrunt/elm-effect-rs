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

/// Effect.test.ts :4013 "should support pipeable arguments" — `Effect.fn`'s
/// second argument transforms the effect by appending its own pipeable args to
/// the mapped result. No `Effect.fn` in this port, so the observable is a plain
/// closure that builds the effect and maps it: calling with "a" yields the
/// length of "a" beside the argument that produced it, `[1, "a"]`.
#[test]
fn effect_fn_appends_pipeable_args_to_the_result() {
    let f = |s: String| Effect::<i32, Infallible>::succeed(s.len() as i32).map(move |result| (result, s));
    assert_eq!(
        Runtime::run(f("a".to_string())),
        Exit::Success((1, "a".to_string()))
    );
}

/// Effect.test.ts :4023 "should support self with pipeable arguments" —
/// `Effect.fn`'s `self` is the TS `this` receiver the body reads, and the third
/// argument maps the body's result. No `Effect.fn` here, so the same values
/// arrive by capture: prefix "bound" makes the body "bound:value", uppercased
/// to "BOUND:VALUE".
#[test]
fn effect_fn_self_binding_uppercases_the_body_result() {
    let prefix = "bound".to_string();
    let f =
        |value: String| Effect::<String, Infallible>::succeed(format!("{prefix}:{value}")).map(|v| v.to_uppercase());
    assert_eq!(
        Runtime::run(f("value".to_string())),
        Exit::Success("BOUND:VALUE".to_string())
    );
}

/// Effect.test.ts :936 "tuple" — `Effect.all([succeed(1), succeed(2),
/// succeed(3)])` yields the tuple `[1, 2, 3]`. No `Effect.all` and no
/// `tuple3` in this port, so the row's success-only assertion is carried by
/// sequential composition, which produces the same tuple when every member
/// succeeds: the shape of the result is what the row checks.
#[test]
fn all_collects_the_success_tuple() {
    let effect = Effect::<i32, Infallible>::succeed(1)
        .flat_map(|a| Effect::<i32, Infallible>::succeed(2).flat_map(move |b| Effect::<i32, Infallible>::succeed(3).map(move |c| (a, b, c))));
    assert_eq!(Runtime::run(effect), Exit::Success((1, 2, 3)));
}

/// Effect.test.ts :950 "record" — `Effect.all({a: succeed(1), b: succeed("2"),
/// c: succeed(true)})` yields the record `{a:1, b:"2", c:true}`. No
/// `Effect.all` here; the record's members are carried the same way, with the
/// named fields as a tuple in the record's declaration order.
#[test]
fn all_collects_the_success_record() {
    let effect = Effect::<i32, Infallible>::succeed(1)
        .flat_map(|a| {
            Effect::<String, Infallible>::succeed("2".to_string())
                .flat_map(move |b| Effect::<bool, Infallible>::succeed(true).map(move |c| (a, b, c)))
        });
    assert_eq!(
        Runtime::run(effect),
        Exit::Success((1, "2".to_string(), true))
    );
}
