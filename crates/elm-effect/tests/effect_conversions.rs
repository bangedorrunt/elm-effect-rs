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

// ---------------------------------------------------------------------------
// finalization at the pin (describe "finalization")
// ---------------------------------------------------------------------------

#[test]
fn ensuring_runs_on_a_typed_failure_and_preserves_it() {
    // Effect.fail("body failure").pipe(Effect.ensuring(sync), Effect.exit)
    // === Exit.fail("body failure") with the finalizer having run.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(false));
    let seen = finalized.clone();
    let effect: Effect<&str, String, ()> =
        Effect::<&str, String>::fail("body failure".to_string())
            .ensuring(move || *seen.lock().unwrap() = true);
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"body failure".to_string())),
        Exit::Success(_) => panic!("expected the typed failure"),
    }
    assert!(*finalized.lock().unwrap(), "the finalizer must have run");
}

#[test]
fn ensuring_runs_on_success() {
    // A success exit keeps its value and still runs the finalizer.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(false));
    let seen = finalized.clone();
    let effect: Effect<u32, String, ()> =
        Effect::<u32, String>::succeed(1).ensuring(move || *seen.lock().unwrap() = true);
    assert!(matches!(run(effect), Exit::Success(1)));
    assert!(*finalized.lock().unwrap(), "the finalizer must have run");
}

#[test]
fn on_error_runs_only_on_a_typed_failure() {
    // Effect.fail("body failure").pipe(Effect.onError(sync), Effect.exit)
    // === Exit.fail("body failure") with the finalizer having run.
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    let seen = calls.clone();
    let effect: Effect<&str, String, ()> =
        Effect::<&str, String>::fail("body failure".to_string()).on_error(move || {
            *seen.lock().unwrap() += 1;
        });
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"body failure".to_string())),
        Exit::Success(_) => panic!("expected the typed failure"),
    }
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn on_error_does_not_run_on_success() {
    // A success never fires the typed-error finalizer.
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    let seen = calls.clone();
    let effect: Effect<u32, String, ()> =
        Effect::<u32, String>::succeed(7).on_error(move || {
            *seen.lock().unwrap() += 1;
        });
    assert!(matches!(run(effect), Exit::Success(7)));
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn on_error_does_not_run_on_a_defect() {
    // A defect is not a typed failure: onError must leave it alone.
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    let seen = calls.clone();
    let effect: Effect<u32, String, ()> =
        Effect::<u32, String>::die(elm_effect::Defect::new("boom")).on_error(move || {
            *seen.lock().unwrap() += 1;
        });
    match run(effect) {
        Exit::Failure(cause) => assert!(!cause.defects().is_empty(), "the defect survives"),
        Exit::Success(_) => panic!("expected the defect"),
    }
    assert_eq!(*calls.lock().unwrap(), 0, "onError must not fire on a defect");
}

#[test]
fn ensuring_does_not_change_the_cause() {
    // The finalizer is a plain side effect, so the original cause is passed
    // through byte-for-byte: a defect and an interrupt survive untouched.
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    let seen = calls.clone();
    let effect: Effect<u32, String, ()> =
        Effect::<u32, String>::die(elm_effect::Defect::new("boom")).ensuring(move || {
            *seen.lock().unwrap() += 1;
        });
    match run(effect) {
        Exit::Failure(cause) => {
            assert!(cause.is_interrupted() == false);
            assert_eq!(cause.defects().len(), 1);
        }
        Exit::Success(_) => panic!("expected the defect"),
    }
    assert_eq!(*calls.lock().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// onExit / onResult at the pin (describe "finalization")
// ---------------------------------------------------------------------------

#[test]
fn on_exit_observes_a_defect_and_passes_it_through() {
    // Effect.die("boom").pipe(Effect.onExit(exit => hasDies(exit) ? ... ))
    // The cleanup runs and the defect survives untouched.
    let observed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mark = observed.clone();
    let effect: Effect<&str, String, ()> =
        Effect::<&str, String>::die(elm_effect::Defect::new("boom")).on_exit(move |exit| {
            *mark.lock().unwrap() = match exit {
                Exit::Failure(cause) if !cause.defects().is_empty() => "die".to_string(),
                Exit::Failure(cause) if cause.is_interrupted() => "interrupt".to_string(),
                Exit::Failure(_) => "fail".to_string(),
                Exit::Success(_) => "success".to_string(),
            };
        });
    match run(effect) {
        Exit::Failure(cause) => {
            assert_eq!(cause.defects().len(), 1, "the defect survives");
            assert_eq!(cause.defects()[0].message, "boom");
        }
        Exit::Success(_) => panic!("expected the defect"),
    }
    assert_eq!(*observed.lock().unwrap(), "die");
}

#[test]
fn on_exit_observes_a_typed_failure() {
    let observed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mark = observed.clone();
    let effect: Effect<&str, String, ()> =
        Effect::<&str, String>::fail("body failure".to_string()).on_exit(move |exit| {
            *mark.lock().unwrap() = match exit {
                Exit::Failure(cause) if !cause.defects().is_empty() => "die".to_string(),
                Exit::Failure(_) => "fail".to_string(),
                Exit::Success(_) => "success".to_string(),
            };
        });
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"body failure".to_string())),
        Exit::Success(_) => panic!("expected the typed failure"),
    }
    assert_eq!(*observed.lock().unwrap(), "fail");
}

#[test]
fn on_exit_observes_success() {
    // A success exit reaches the callback with the success outcome.
    let observed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mark = observed.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(1).on_exit(move |exit| {
        *mark.lock().unwrap() = match exit {
            Exit::Success(_) => "success".to_string(),
            Exit::Failure(_) => "failure".to_string(),
        };
    });
    assert!(matches!(run(effect), Exit::Success(1)));
    assert_eq!(*observed.lock().unwrap(), "success");
}

#[test]
fn on_exit_runs_exactly_once() {
    // The shared slot is taken in whichever arm runs: two runs, one call each.
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    let mark = calls.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(1).on_exit(move |_| {
        *mark.lock().unwrap() += 1;
    });
    assert!(matches!(run(effect), Exit::Success(1)));
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn on_result_runs_the_cleanup_when_the_effect_dies() {
    // onResult's cleanup runs for a defect. The pin's test branches on
    // Exit.hasDies; the observable claim is that the cleanup runs.
    use std::sync::Arc as StdArc;
    use std::sync::Mutex as StdMutex;
    let cleaned = StdArc::new(StdMutex::new(false));
    let mark = StdArc::clone(&cleaned);
    let effect: Effect<&str, String, ()> =
        Effect::<&str, String>::die(elm_effect::Defect::new("boom")).on_exit(move |exit| {
            if let Exit::Failure(cause) = exit {
                if !cause.defects().is_empty() {
                    *mark.lock().unwrap() = true;
                }
            }
        });
    let _ = run(effect);
    assert!(*cleaned.lock().unwrap(), "the cleanup must run on a defect");
}

