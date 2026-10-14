//! Law L1: building an [`Effect`](crate::Effect) runs nothing.
//!
//! Construction and `drop` perform no I/O, no sleep, no state change. Every
//! constructor and combinator is covered: if any of them ran work eagerly,
//! its counter would be non-zero after the drop.

use elm_effect::{Cause, Defect, Effect, Exit, Layer, ManagedRuntime, Reason, Runtime, ServiceTag, use_service};
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

// ---------------------------------------------------------------------------
// predicate- and filter-gated catch (Effect.test.ts :3424-:3482)
// ---------------------------------------------------------------------------

/// Effect.test.ts :3424 "first argument as success" — `catchCauseFilter`
/// over a succeeding effect never consults the filter, so the value passes
/// through and a handler that would fail with "e2" never runs.
#[test]
fn catch_cause_filter_first_argument_as_success() {
    let effect = Effect::<i32, String>::succeed(1).catch_cause_filter(
        |_cause: &Cause<String>| -> std::result::Result<(), ()> { Err(()) },
        |_pass, _cause| Effect::fail("e2".to_string()),
    );
    assert_eq!(Runtime::run(effect), Exit::Success(1));
}

/// Effect.test.ts :3429 "first argument as failure and predicate return
/// false" — a `Result::Err` from the filter declines the recovery, so the
/// original cause is re-raised untouched. (`Effect.flip` in the pin is the
/// assertion device; the port asserts the failure directly.)
#[test]
fn catch_cause_filter_declined_cause_is_re_raised() {
    let effect = Effect::<i32, String>::fail("e1".to_string()).catch_cause_filter(
        |_cause: &Cause<String>| -> std::result::Result<(), ()> { Err(()) },
        |_pass, _cause| Effect::fail("e2".to_string()),
    );
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the original failure"),
    }
}

/// Effect.test.ts :3436 "first argument as failure and predicate return
/// true" — a `Result::Ok` from the filter routes the cause to the handler,
/// whose effect replaces the failure.
#[test]
fn catch_cause_filter_accepted_cause_recovers_through_the_handler() {
    let effect = Effect::<i32, String>::fail("e1".to_string()).catch_cause_filter(
        |_cause: &Cause<String>| -> std::result::Result<(), ()> { Ok(()) },
        |_pass, _cause| Effect::fail("e2".to_string()),
    );
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e2".to_string())),
        Exit::Success(_) => panic!("expected the handler's failure"),
    }
}

/// Effect.test.ts :3450 "predicate match recovers" — `catchIf` recovers a
/// clean typed failure whose error passes the predicate, handing the error
/// to the handler.
#[test]
fn catch_if_predicate_match_recovers() {
    let effect = Effect::<String, String>::fail("e1".to_string()).catch_if(
        |e: &String| !e.is_empty(),
        |e| Effect::succeed(format!("recovered: {e}")),
    );
    assert_eq!(Runtime::run(effect), Exit::Success("recovered: e1".to_string()));
}

/// Effect.test.ts :3459 "predicate no match preserves error" — a predicate
/// that rejects the error leaves `catchIf` out of the way, so the original
/// failure is the outcome.
#[test]
fn catch_if_predicate_no_match_preserves_error() {
    let effect = Effect::<String, String>::fail("e1".to_string()).catch_if(
        |_e: &String| false,
        |_e| Effect::succeed("recovered".to_string()),
    );
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the original error"),
    }
}

/// The unhappy path `catchIf` inherits from `catch`: a defect is not a
/// typed error, so a predicate that would accept the value must not
/// recover it.
#[test]
fn catch_if_ignores_a_defect_even_when_the_predicate_accepts() {
    let effect = Effect::<String, String>::die(Defect::new("boom")).catch_if(
        |_e: &String| true,
        |e| Effect::succeed(format!("recovered: {e}")),
    );
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.defects()[0].message, "boom"),
        Exit::Success(_) => panic!("a defect is not a recoverable typed error"),
    }
}

/// Effect.test.ts :3473 "predicate match recovers" — `catchCauseIf` recovers
/// any failure whose whole cause passes `Cause.hasFails`, and the handler
/// sees the cause it recovered from.
#[test]
fn catch_cause_if_predicate_match_recovers() {
    let effect = Effect::<String, String>::fail("e1".to_string()).catch_cause_if(
        |cause: &Cause<String>| cause.has_fails(),
        |cause| {
            let e = cause.failure().cloned().expect("has_fails holds");
            Effect::succeed(format!("recovered: {e}"))
        },
    );
    assert_eq!(Runtime::run(effect), Exit::Success("recovered: e1".to_string()));
}

/// Effect.test.ts :3482 "predicate no match preserves error" — a declined
/// cause is re-raised unchanged, never recovered.
#[test]
fn catch_cause_if_predicate_no_match_preserves_error() {
    let effect = Effect::<String, String>::fail("e1".to_string()).catch_cause_if(
        |_cause: &Cause<String>| false,
        |_cause| Effect::succeed("recovered".to_string()),
    );
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the original error"),
    }
}

/// Effect.test.ts :3711 "first argument as failure" — the mirror of :3706:
/// `catch` over a failing effect runs the handler, and the handler's
/// failure is the outcome. (`Effect.flip` is the pin's assertion device.)
#[test]
fn catch_first_argument_as_failure() {
    let effect = Effect::<i32, String>::fail("e1".to_string())
        .catch(|_e| Effect::fail("e2".to_string()));
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e2".to_string())),
        Exit::Success(_) => panic!("expected the handler's error"),
    }
}

/// Effect.test.ts :3768 "first argument as failure" — the same claim for
/// `catchCause`: the handler's failure replaces the original cause.
#[test]
fn catch_cause_first_argument_as_failure() {
    let effect = Effect::<i32, String>::fail("e1".to_string())
        .catch_cause(|_cause| Effect::fail("e2".to_string()));
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e2".to_string())),
        Exit::Success(_) => panic!("expected the handler's error"),
    }
}

// ---------------------------------------------------------------------------
// iterable constructors (Effect.test.ts :3677-:3693) and firstSuccessOf
// (:3719-:3749)
// ---------------------------------------------------------------------------

/// Effect.test.ts :3677 "predicate filters iterable" — `Effect.filter` keeps
/// the elements the predicate accepts, in iteration order.
#[test]
fn filter_keeps_the_elements_the_predicate_accepts() {
    let effect: Effect<Vec<i32>, Infallible> =
        Effect::<i32, Infallible>::filter(vec![1, 2, 3, 4, 5], |n| n % 2 == 0);
    assert_eq!(Runtime::run(effect), Exit::Success(vec![2, 4]));
}

/// A mixed element for the refinement row below.
#[derive(Clone, Debug, PartialEq)]
enum Mixed {
    Num(i32),
    Str(&'static str),
}

/// Effect.test.ts :3685 "refinement narrows iterable" — the pin's
/// `x is number` guard narrows the element type. Rust has no type predicate,
/// so the mixed iterable is an enum and the guard is the `Num` arm; the
/// row's value-level observable survives exactly: only the numbers, in
/// order.
#[test]
fn filter_refinement_keeps_only_the_narrowed_elements() {
    let items = vec![
        Mixed::Num(1),
        Mixed::Str("a"),
        Mixed::Num(2),
        Mixed::Str("b"),
        Mixed::Num(3),
    ];
    let effect: Effect<Vec<Mixed>, Infallible> =
        Effect::<Mixed, Infallible>::filter(items, |item| matches!(item, Mixed::Num(_)));
    assert_eq!(
        Runtime::run(effect),
        Exit::Success(vec![Mixed::Num(1), Mixed::Num(2), Mixed::Num(3)])
    );
}

/// Effect.test.ts :3693 "Filter.Filter overload maps pass values" — the
/// `Result`-returning selector: `Ok` keeps the mapped value, `Err` drops
/// the element.
#[test]
fn filter_map_keeps_the_mapped_pass_values() {
    let effect: Effect<Vec<String>, Infallible> = Effect::<i32, Infallible>::filter_map(
        vec![1, 2, 3, 4],
        |n| {
            if n % 2 == 0 {
                Ok(format!("n={n}"))
            } else {
                Err(n)
            }
        },
    );
    assert_eq!(
        Runtime::run(effect),
        Exit::Success(vec!["n=2".to_string(), "n=4".to_string()])
    );
}

/// Effect.test.ts :3719 "returns the first success and does not run later
/// effects" — the chain short-circuits on the first success, so the third
/// effect never runs.
#[test]
fn first_success_of_returns_the_first_success_and_skips_later_effects() {
    let executed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (mark1, mark2, mark3) = (executed.clone(), executed.clone(), executed.clone());
    let first = Effect::sync(move || {
        mark1.lock().unwrap().push("first".to_string());
    })
    .flat_map(|_| Effect::<&str, String>::fail("e1".to_string()));
    let second =
        Effect::sync(move || {
            mark2.lock().unwrap().push("second".to_string());
        })
        .map(|_| "success");
    let third = Effect::sync(move || {
        mark3.lock().unwrap().push("third".to_string());
    })
    .map(|_| "unreachable");
    let effect: Effect<&str, String> = Effect::first_success_of(vec![first, second, third]);
    assert_eq!(Runtime::run(effect), Exit::Success("success"));
    assert_eq!(*executed.lock().unwrap(), ["first".to_string(), "second".to_string()]);
}

/// Effect.test.ts :3738 "fails with the last failure when all effects fail"
/// — the rightmost failure is the outcome.
#[test]
fn first_success_of_fails_with_the_last_failure() {
    let effects: Vec<Effect<&str, String>> = vec![
        Effect::fail("e1".to_string()),
        Effect::fail("e2".to_string()),
        Effect::fail("e3".to_string()),
    ];
    let effect: Effect<&str, String> = Effect::first_success_of(effects);
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e3".to_string())),
        Exit::Success(_) => panic!("expected the last failure"),
    }
}

/// Effect.test.ts :3749 "defects on an empty collection" — an empty input
/// is a defect at interpretation, never a typed failure.
#[test]
fn first_success_of_defects_on_an_empty_collection() {
    let effect: Effect<&str, String> = Effect::first_success_of(Vec::new());
    match Runtime::run(effect) {
        Exit::Failure(cause) => {
            assert_eq!(cause.defects()[0].message, "Received an empty collection of effects");
        }
        Exit::Success(_) => panic!("expected a defect"),
    }
}

/// The unhappy path `firstSuccessOf` inherits from `catch`: a defect is
/// never a typed failure, so fall-through must not swallow it. A dying
/// effect ends the chain even with a succeeding effect queued next.
#[test]
fn first_success_of_propagates_a_defect_without_falling_through() {
    let effect: Effect<&str, String> = Effect::first_success_of(vec![
        Effect::die(Defect::new("boom")),
        Effect::succeed("unreachable"),
    ]);
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.defects()[0].message, "boom"),
        Exit::Success(_) => panic!("a defect must not be recovered by the next effect"),
    }
}

/// Law L1 for the iterable constructors: the predicate runs at
/// interpretation, not at construction or drop.
#[test]
fn filter_runs_nothing_until_interpreted() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let effect: Effect<Vec<i32>, Infallible> =
        Effect::<i32, Infallible>::filter(vec![1, 2, 3], move |n| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            n % 2 == 1
        });
    drop(effect);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

// `catchReason`, `catchReasons`, and `unwrapReason` (Effect.test.ts
// :4086-:4280). The pin discriminates nested tagged errors by `_tag`
// strings on both the parent and its `reason`; this port's typed channel
// is a Rust enum, so the same discrimination is an exhaustive match and
// the recovery itself is the `catch` family. `Ai` is the pin's
// `AiError`, `Nested` the union of its nested reasons, `Other` the
// non-matching parent tag.
#[derive(Debug, Clone, PartialEq)]
enum Nested {
    RateLimit(u64),
    Quota(u64),
}

#[derive(Debug, Clone, PartialEq)]
enum Problem {
    Ai(Nested),
    Other(String),
}

/// `catchReason` on the matching nested reason: the handler sees the
/// reason and recovers with its field.
#[test]
fn catch_reason_matches_nested_reason_and_handler_succeeds() {
    let effect = Effect::<String, Problem>::fail(Problem::Ai(Nested::RateLimit(60))).catch(|e| {
        match e {
            Problem::Ai(Nested::RateLimit(retry)) => {
                Effect::<String, Problem>::succeed(format!("retry: {retry}"))
            }
            other => Effect::fail(other),
        }
    });
    assert_eq!(
        Runtime::run(effect),
        Exit::Success("retry: 60".to_string())
    );
}

/// `catchReason`'s `orElse`: the parent tag matches but the nested reason
/// does not, so the fallback handler runs.
#[test]
fn catch_reason_or_else_handles_the_second_reason_type() {
    let effect = Effect::<String, Problem>::fail(Problem::Ai(Nested::Quota(100))).catch(|e| {
        match e {
            Problem::Ai(Nested::RateLimit(retry)) => {
                Effect::<String, Problem>::succeed(format!("retry: {retry}"))
            }
            Problem::Ai(_) => Effect::succeed("quota".to_string()),
            other => Effect::fail(other),
        }
    });
    assert_eq!(Runtime::run(effect), Exit::Success("quota".to_string()));
}

/// `catchReason` when the handler itself fails: the handler's failure is
/// the exit, the original cause is not preserved.
#[test]
fn catch_reason_handler_failure_becomes_the_exit() {
    let effect = Effect::<(), Problem>::fail(Problem::Ai(Nested::RateLimit(60)))
        .catch(|_| Effect::fail(Problem::Other("handled".to_string())));
    match Runtime::run(effect) {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), Some(&Problem::Other("handled".to_string())));
        }
        Exit::Success(_) => panic!("the handler's failure must be the exit"),
    }
}

/// `catchReason` on a non-matching nested reason: the original error is
/// re-raised, so the combinator never recovers an error it did not name.
#[test]
fn catch_reason_ignores_non_matching_reason() {
    let effect = Effect::<(), Problem>::fail(Problem::Ai(Nested::Quota(100))).catch(|e| match e {
        Problem::Ai(Nested::RateLimit(_)) => Effect::succeed(()),
        other => Effect::fail(other),
    });
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&Problem::Ai(Nested::Quota(100)))
        ),
        Exit::Success(_) => panic!("a non-matching reason must not be recovered"),
    }
}

/// `catchReason` with a scope: a failing finalizer's defect appends to the
/// cause, and the combined cause passes through the non-recovering
/// `catchReason` untouched (`catch` only fires on a clean single `Fail`).
#[test]
fn catch_reason_preserves_the_cause_for_a_non_matching_reason() {
    let rt = Runtime::new();
    let child = rt.fork_detach(|ctx| -> Exit<(), Problem> {
        ctx.acquire_release(
            Effect::<(), String>::succeed(()),
            |()| Effect::<(), String>::die(Defect::new("finalizer failure")),
        )
        .expect("acquire succeeds");
        ctx.spawn_effect(
            Effect::<(), Problem>::fail(Problem::Ai(Nested::Quota(100))).catch(|e| match e {
                Problem::Ai(Nested::RateLimit(_)) => Effect::succeed(()),
                other => Effect::fail(other),
            }),
        )
        .join()
    });
    match child.join() {
        Exit::Failure(cause) => {
            assert!(matches!(
                cause.reasons.as_slice(),
                [Reason::Fail(_), Reason::Die(_)]
            ));
            assert_eq!(
                cause.failure(),
                Some(&Problem::Ai(Nested::Quota(100)))
            );
            assert!(cause.defects()[0].message.contains("finalizer failure"));
        }
        Exit::Success(_) => panic!("the failing effect and its finalizer must stay a failure"),
    }
}

/// `catchReason` on a non-matching parent tag: a failure of a different
/// constructor passes through with its cause intact.
#[test]
fn catch_reason_ignores_non_matching_parent_tag() {
    let effect = Effect::<(), Problem>::fail(Problem::Other("test".to_string())).catch(|e| match e {
        Problem::Ai(Nested::RateLimit(_)) => Effect::succeed(()),
        other => Effect::fail(other),
    });
    match Runtime::run(effect) {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), Some(&Problem::Other("test".to_string())));
        }
        Exit::Success(_) => panic!("a non-matching parent tag must not be recovered"),
    }
}

/// `catchReasons` with an object of handlers: the handler for the nested
/// reason's tag runs.
#[test]
fn catch_reasons_object_handlers_select_by_nested_tag() {
    let effect = Effect::<String, Problem>::fail(Problem::Ai(Nested::RateLimit(60))).catch(|e| {
        match e {
            Problem::Ai(Nested::RateLimit(retry)) => {
                Effect::<String, Problem>::succeed(format!("rate: {retry}"))
            }
            Problem::Ai(Nested::Quota(limit)) => Effect::succeed(format!("quota: {limit}")),
            other => Effect::fail(other),
        }
    });
    assert_eq!(Runtime::run(effect), Exit::Success("rate: 60".to_string()));
}

/// `catchReasons`' `orElse`: a nested reason with no handler in the
/// object falls back.
#[test]
fn catch_reasons_or_else_covers_unlisted_reasons() {
    let effect = Effect::<String, Problem>::fail(Problem::Ai(Nested::RateLimit(60))).catch(|e| {
        match e {
            Problem::Ai(Nested::Quota(limit)) => Effect::<String, Problem>::succeed(format!("quota: {limit}")),
            Problem::Ai(_) => Effect::succeed("orElse".to_string()),
            other => Effect::fail(other),
        }
    });
    assert_eq!(Runtime::run(effect), Exit::Success("orElse".to_string()));
}

/// `catchReasons` on the second reason type in the object.
#[test]
fn catch_reasons_catches_second_reason_type() {
    let effect = Effect::<String, Problem>::fail(Problem::Ai(Nested::Quota(100))).catch(|e| {
        match e {
            Problem::Ai(Nested::RateLimit(retry)) => {
                Effect::<String, Problem>::succeed(format!("rate: {retry}"))
            }
            Problem::Ai(Nested::Quota(limit)) => Effect::succeed(format!("quota: {limit}")),
            other => Effect::fail(other),
        }
    });
    assert_eq!(
        Runtime::run(effect),
        Exit::Success("quota: 100".to_string())
    );
}

/// `catchReasons` with partial handlers: a nested reason the object does
/// not name passes the original error through.
#[test]
fn catch_reasons_partial_handlers_pass_unhandled_through() {
    let effect = Effect::<(), Problem>::fail(Problem::Ai(Nested::Quota(100))).catch(|e| match e {
        Problem::Ai(Nested::RateLimit(_)) => Effect::succeed(()),
        other => Effect::fail(other),
    });
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&Problem::Ai(Nested::Quota(100)))
        ),
        Exit::Success(_) => panic!("an unhandled reason must not be recovered"),
    }
}

/// `catchReasons` with a scope: the failing finalizer's defect appends and
/// the combined cause survives an unhandled reason.
#[test]
fn catch_reasons_preserves_the_cause_for_an_unhandled_reason() {
    let rt = Runtime::new();
    let child = rt.fork_detach(|ctx| -> Exit<(), Problem> {
        ctx.acquire_release(
            Effect::<(), String>::succeed(()),
            |()| Effect::<(), String>::die(Defect::new("finalizer failure")),
        )
        .expect("acquire succeeds");
        ctx.spawn_effect(
            Effect::<(), Problem>::fail(Problem::Ai(Nested::Quota(100))).catch(|e| match e {
                Problem::Ai(Nested::RateLimit(_)) => Effect::succeed(()),
                other => Effect::fail(other),
            }),
        )
        .join()
    });
    match child.join() {
        Exit::Failure(cause) => {
            assert!(matches!(
                cause.reasons.as_slice(),
                [Reason::Fail(_), Reason::Die(_)]
            ));
            assert_eq!(
                cause.failure(),
                Some(&Problem::Ai(Nested::Quota(100)))
            );
            assert!(cause.defects()[0].message.contains("finalizer failure"));
        }
        Exit::Success(_) => panic!("the failing effect and its finalizer must stay a failure"),
    }
}

/// `unwrapReason` narrows the error channel to the union of nested reasons
/// (the pin's `RateLimitError | QuotaExceededError | OtherError`), so the
/// port renders the narrowed channel as its own enum and the unwrap
/// itself as a channel transformation: `Ai(reason)` becomes the reason,
/// everything else keeps its rendering. Defects and interrupts are not
/// typed failures, so `map_err` leaves them untouched.
#[derive(Debug, Clone, PartialEq)]
enum Unwrapped {
    RateLimit(u64),
    Quota(u64),
    Other(String),
}

fn unwrap(problem: Problem) -> Unwrapped {
    match problem {
        Problem::Ai(Nested::RateLimit(retry)) => Unwrapped::RateLimit(retry),
        Problem::Ai(Nested::Quota(limit)) => Unwrapped::Quota(limit),
        Problem::Other(message) => Unwrapped::Other(message),
    }
}

/// `unwrapReason`: the nested reason replaces the parent in the error
/// channel.
#[test]
fn unwrap_reason_extracts_reason_into_error_channel() {
    let effect: Effect<(), Unwrapped> =
        Effect::<(), Problem>::fail(Problem::Ai(Nested::RateLimit(60))).map_err(unwrap);
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&Unwrapped::RateLimit(60))),
        Exit::Success(_) => panic!("the unwrapped reason must fail with it"),
    }
}

/// `unwrapReason` on the second reason type.
#[test]
fn unwrap_reason_extracts_second_reason_type() {
    let effect: Effect<(), Unwrapped> =
        Effect::<(), Problem>::fail(Problem::Ai(Nested::Quota(100))).map_err(unwrap);
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&Unwrapped::Quota(100))),
        Exit::Success(_) => panic!("the unwrapped reason must fail with it"),
    }
}

/// `unwrapReason` preserves an error whose parent tag does not match: it
/// stays in the channel, unchanged.
#[test]
fn unwrap_reason_preserves_other_errors() {
    let effect: Effect<(), Unwrapped> =
        Effect::<(), Problem>::fail(Problem::Other("test".to_string())).map_err(unwrap);
    match Runtime::run(effect) {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), Some(&Unwrapped::Other("test".to_string())));
        }
        Exit::Success(_) => panic!("a non-matching parent must pass through"),
    }
}

/// `map_err` narrows the typed failure channel and nothing else. A defect is
/// not a typed failure and neither is an interrupt, so both ride through
/// `Cause::map` verbatim: this is the half of the static-interpreter seam
/// (`provide_from` + `map_err`) that keeps the four outcomes apart, because
/// an interpreter translates its program's `Err` without ever re-labelling
/// the defect of a service that was never built.
#[test]
fn map_err_leaves_defects_and_interrupts_untouched() {
    let defect = Defect::new("boom");
    match Runtime::run(Effect::<(), Problem>::die(defect.clone()).map_err(unwrap)) {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), None, "a defect is not a typed failure");
            assert_eq!(cause.defects(), vec![&defect], "the defect survives verbatim");
        }
        other => panic!("a defect must stay a defect, got {other:?}"),
    }
    match Runtime::run(Effect::<(), Problem>::interrupt().map_err(unwrap)) {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), None, "an interrupt is not a typed failure");
            assert!(cause.defects().is_empty(), "an interrupt is not a defect");
            assert!(
                cause.reasons.iter().any(|reason| matches!(reason, Reason::Interrupt { .. })),
                "the interrupt survives verbatim, got {:?}",
                cause.reasons
            );
        }
        other => panic!("an interrupt must stay an interrupt, got {other:?}"),
    }
}

/// A tag for the seam tests: `use_service` needs one, and so does the
/// runtime whose layer builds it.
struct Seam;
struct Greeting(String);

impl Clone for Greeting {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl ServiceTag for Seam {
    type Service = Greeting;
    const ID: &'static str = "test/seam";
}

/// The other half of the wrong-runtime case: a layer that built a DIFFERENT
/// tag, so a program asking for `Seam` is asking for nothing.
struct Sibling;
struct Count(u32);

impl Clone for Count {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl ServiceTag for Sibling {
    type Service = Count;
    const ID: &'static str = "test/sibling";
}

/// What a program declares as its `Err`. A service read is `Infallible`, so
/// the widening has to reach for an unreachable match rather than a value:
/// the point is that a real declared channel accepts it at all.
fn unreachable(never: Infallible) -> String {
    match never {}
}

/// The widening the seam is for. A static interpreter's program declares an
/// `Err` channel, while `use_service` is `Infallible`, and the two do not
/// unify: `map_err` is the only door between them. This is that composition
/// over a service the runtime actually built.
#[test]
fn use_service_joins_a_declared_error_channel_through_map_err() {
    let layer = Layer::<Seam, Infallible, ()>::from_value(Greeting("from-layer".to_string()));
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let services = runtime.context().expect("built services");
    let program: Effect<String, String, ()> = use_service::<Seam>()
        .map(|greeting| greeting.0)
        .provide_from(services)
        .map_err(unreachable);
    match Runtime::run(program) {
        Exit::Success(message) => assert_eq!(message, "from-layer"),
        other => panic!("a built service must reach the widened channel, got {other:?}"),
    }
}

/// The fail-closed half of the same composition. A runtime booted for
/// `Sibling` must not fill a program's declared `Err` in with the defect of
/// a service it never built: `Cause::map` leaves the `Die` verbatim, so
/// `ProgramError` stays empty and the missing tag names itself.
#[test]
fn a_service_the_runtime_never_built_stays_a_defect_through_map_err() {
    let layer = Layer::<Sibling, Infallible, ()>::from_value(Count(1));
    let runtime = ManagedRuntime::make(&layer).expect("layer builds");
    let services = runtime.context().expect("built services");
    let program: Effect<String, String, ()> = use_service::<Seam>()
        .map(|greeting| greeting.0)
        .provide_from(services)
        .map_err(unreachable);
    match Runtime::run(program) {
        Exit::Failure(cause) => {
            assert_eq!(
                cause.failure(),
                None,
                "a missing service is a defect, not a typed failure"
            );
            let rendered = format!("{cause:?}");
            assert!(
                rendered.contains("missing service: test/seam"),
                "the missing tag must name itself, got: {rendered}"
            );
        }
        other => panic!("a missing tag must stay a failure, got {other:?}"),
    }
}
