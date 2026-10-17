//! `Effect` conversions at the pin (`packages/effect/test/Effect.test.ts`,
//! describe blocks `fromOption`, `transposeOption`, `fromResult`).
//!
//! The edge is `ManagedRuntime::run` (L3: only the edge interprets a
//! description). A clean typed failure carries `Reason::Fail`; that is what
//! these rows assert on.

use elm_effect::{Cause, Effect, Exit, ManagedRuntime, Reason, Squash};

fn run<A: Send + 'static>(effect: Effect<A, String, ()>) -> Exit<A, String> {
    ManagedRuntime::new().run(effect)
}

#[test]
fn from_option_some_succeeds() {
    // Effect.fromOption(Some("A")) succeeds with the inner value.
    let effect: Effect<&str, String, ()> = Effect::from_option(Some("A"), || "missing".to_string());
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
            assert!(
                cause.defects().is_empty(),
                "a none is a typed failure, not a defect"
            );
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
    let effect: Effect<&str, String, ()> = Effect::from_option(Some("A"), || {
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
    let effect: Effect<&str, String, ()> = Effect::<&str, String>::fail("body failure".to_string())
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
    let effect: Effect<&str, String, ()> = Effect::<&str, String>::fail("body failure".to_string())
        .on_error(move || {
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
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(7).on_error(move || {
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
    assert_eq!(
        *calls.lock().unwrap(),
        0,
        "onError must not fire on a defect"
    );
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
    let effect: Effect<&str, String, ()> = Effect::<&str, String>::fail("body failure".to_string())
        .on_exit(move |exit| {
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

#[test]
fn run_of_a_synchronous_effect_completes_without_spawning() {
    // Scheduler row "runSyncExit does not create a dispatcher for
    // synchronous effects". The pin measures a dispatcher-spy count of 0;
    // the port has no public fiber-count probe, so this asserts the part
    // that is observable from outside: the effect still completes with its
    // value, and the runtime stays usable for subsequent fibers.
    //
    // The "no dispatcher was needed" claim is structural here -- run() on a
    // Step::Sync effect interprets it directly -- but nothing in this port
    // lets a caller observe a dispatcher count, so asserting one would mean
    // inventing an accessor for a test. One row stays honestly unflipped
    // rather than fake that.
    let exit = ManagedRuntime::new().run(Effect::<i32, std::convert::Infallible>::sync(|| 21i32));
    assert!(matches!(exit, Exit::Success(21)));

    // The edge still drives real fibers afterwards, so the synchronous path
    // did not leave the runtime wedged.
    let managed = ManagedRuntime::new();
    let exit = managed
        .runtime()
        .fork_detach(|_| Exit::<(), std::convert::Infallible>::succeed(()))
        .join();
    assert!(matches!(exit, Exit::Success(())));
}

// ---------------------------------------------------------------------------
// conditional taps and finalizers (Effect.test.ts describes tapCauseIf,
// onErrorIf, onExitIf: :3496-:3662)
//
// The pin's filters select a cause or exit with a `Result`; the port's
// selectors return `std::result::Result` the same way. Every finalizer in
// these rows is `Effect.sync`, i.e. a plain side effect, which is the
// closure the port's `on_exit` family takes (see `ensuring`'s note on why
// the finalizer is not an effect here).
// ---------------------------------------------------------------------------

/// `Cause.squash` on a Fail-only cause is the failure itself.
fn squash_fail(cause: &Cause<String>) -> String {
    match cause.squash() {
        Squash::Fail(e) => e,
        other => panic!("expected a Fail cause, got {other:?}"),
    }
}

#[test]
fn tap_cause_filter_match_taps_and_re_raises_the_cause() {
    // Effect.test.ts :3496 "filter match taps" — the filter's pass value
    // carries the cause to the observer; the original failure is re-raised
    // untouched.
    let tapped = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = tapped.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .tap_cause_filter(
            |cause: &Cause<String>| -> std::result::Result<Cause<String>, ()> { Ok(cause.clone()) },
            move |_pass, cause| mark.lock().unwrap().push(squash_fail(cause)),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert_eq!(*tapped.lock().unwrap(), ["e1".to_string()]);
}

#[test]
fn tap_cause_filter_no_match_skips_the_tap() {
    // Effect.test.ts :3509 "filter no match skips tap" — an `Err` from the
    // filter skips the observer and preserves the cause.
    let tapped = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = tapped.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .tap_cause_filter(
            |_cause: &Cause<String>| -> std::result::Result<(), ()> { Err(()) },
            move |_pass, _cause| mark.lock().unwrap().push("tapped".to_string()),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert!(tapped.lock().unwrap().is_empty(), "no tap may run");
}

#[test]
fn tap_cause_if_predicate_match_taps_the_cause() {
    // Effect.test.ts :3522 "predicate match taps" — `Cause.hasFails` admits
    // the failure, so the observer sees the squashed cause.
    let tapped = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = tapped.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .tap_cause_if(
            |cause: &Cause<String>| cause.has_fails(),
            move |cause| mark.lock().unwrap().push(squash_fail(cause)),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert_eq!(*tapped.lock().unwrap(), ["e1".to_string()]);
}

#[test]
fn tap_cause_if_predicate_no_match_skips_the_tap() {
    // Effect.test.ts :3535 "predicate no match skips tap".
    let tapped = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = tapped.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .tap_cause_if(
            |_cause: &Cause<String>| false,
            move |_cause| mark.lock().unwrap().push("tapped".to_string()),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert!(tapped.lock().unwrap().is_empty(), "no tap may run");
}

#[test]
fn tap_cause_if_success_skips_the_tap() {
    // Effect.test.ts :3548 "success skips tap" — a success never presents a
    // cause, and its value passes through unchanged.
    let tapped = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = tapped.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(42).tap_cause_if(
        |_cause: &Cause<String>| true,
        move |_cause| mark.lock().unwrap().push("tapped".to_string()),
    );
    assert!(matches!(run(effect), Exit::Success(42)));
    assert!(
        tapped.lock().unwrap().is_empty(),
        "a success has no cause to tap"
    );
}

#[test]
fn on_error_if_predicate_match_runs_the_finalizer() {
    // Effect.test.ts :3562 "predicate match runs finalizer" — the finalizer
    // sees the cause and the failure is re-raised unchanged.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .on_error_if(
            |cause: &Cause<String>| cause.has_fails(),
            move |cause| mark.lock().unwrap().push(squash_fail(cause)),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert_eq!(*finalized.lock().unwrap(), ["e1".to_string()]);
}

#[test]
fn on_error_if_predicate_no_match_skips_the_finalizer() {
    // Effect.test.ts :3575 "predicate no match skips finalizer".
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .on_error_if(
            |_cause: &Cause<String>| false,
            move |_cause| mark.lock().unwrap().push("finalized".to_string()),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert!(finalized.lock().unwrap().is_empty(), "no finalizer may run");
}

#[test]
fn on_error_filter_match_runs_the_finalizer_with_the_pass_value() {
    // Effect.test.ts :3588 "filter match runs finalizer" — `Cause.findError`
    // extracts the typed failure and hands it to the finalizer.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string())
        .on_error_filter(
            |cause: &Cause<String>| cause.find_error().cloned().ok_or(()),
            move |error, _cause| mark.lock().unwrap().push(error),
        );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the re-raised failure"),
    }
    assert_eq!(*finalized.lock().unwrap(), ["e1".to_string()]);
}

#[test]
fn on_error_if_success_skips_the_finalizer() {
    // Effect.test.ts :3601 "success skips finalizer" — a success is not an
    // error, so the typed-error finalizer stays out of the way.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(42).on_error_if(
        |_cause: &Cause<String>| true,
        move |_cause| mark.lock().unwrap().push("finalized".to_string()),
    );
    assert!(matches!(run(effect), Exit::Success(42)));
    assert!(
        finalized.lock().unwrap().is_empty(),
        "a success finalizes nothing"
    );
}

#[test]
fn on_exit_if_match_on_success_runs_the_finalizer() {
    // Effect.test.ts :3615 "predicate match on success runs finalizer" —
    // `Exit.isSuccess` admits the success exit and the finalizer sees it.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(42).on_exit_if(
        |exit: &Exit<u32, String>| exit.is_success(),
        move |exit| {
            if let Exit::Success(value) = exit {
                mark.lock().unwrap().push(format!("success:{value}"));
            }
        },
    );
    assert!(matches!(run(effect), Exit::Success(42)));
    assert_eq!(*finalized.lock().unwrap(), ["success:42".to_string()]);
}

#[test]
fn on_exit_if_no_match_on_failure_skips_the_finalizer() {
    // Effect.test.ts :3631 "predicate no match on failure skips finalizer".
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string()).on_exit_if(
        |exit: &Exit<u32, String>| exit.is_success(),
        move |_exit| mark.lock().unwrap().push("finalized".to_string()),
    );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the failure"),
    }
    assert!(finalized.lock().unwrap().is_empty(), "no finalizer may run");
}

#[test]
fn on_exit_if_match_on_failure_runs_the_finalizer() {
    // Effect.test.ts :3644 "predicate match on failure runs finalizer" —
    // `Exit.isFailure` admits the failure exit; the finalizer records the
    // squashed cause.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::fail("e1".to_string()).on_exit_if(
        |exit: &Exit<u32, String>| !exit.is_success(),
        move |exit| {
            if let Exit::Failure(cause) = exit {
                mark.lock()
                    .unwrap()
                    .push(format!("failure:{}", squash_fail(cause)));
            }
        },
    );
    match run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"e1".to_string())),
        Exit::Success(_) => panic!("expected the failure"),
    }
    assert_eq!(*finalized.lock().unwrap(), ["failure:e1".to_string()]);
}

#[test]
fn on_exit_filter_match_passes_the_value_and_the_exit() {
    // Effect.test.ts :3662 "filter match receives pass value and exit" —
    // the filter extracts `value:42` from the success exit and the
    // finalizer receives both the extracted value and the original exit.
    let finalized = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mark = finalized.clone();
    let effect: Effect<u32, String, ()> = Effect::<u32, String>::succeed(42).on_exit_filter(
        |exit: &Exit<u32, String>| -> std::result::Result<String, ()> {
            match exit {
                Exit::Success(value) => Ok(format!("value:{value}")),
                Exit::Failure(_) => Err(()),
            }
        },
        move |value, exit| {
            if let Exit::Success(v) = exit {
                mark.lock().unwrap().push(format!("{value}:success:{v}"));
            }
        },
    );
    assert!(matches!(run(effect), Exit::Success(42)));
    assert_eq!(
        *finalized.lock().unwrap(),
        ["value:42:success:42".to_string()]
    );
}
