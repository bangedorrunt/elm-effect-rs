//! Law L2: the cause model owns the failure vocabulary at the crate edge.
//!
//! A cause is ordinary data: a vector of reasons. A defect is never a typed
//! error value, and an exit is data, not an effect.

use elm_effect::{
    Cause, Defect, Exit, FiberId, Reason, is_die_reason, is_fail_reason, is_interrupt_reason,
};
use std::convert::Infallible;

#[test]
fn cause_reasons_classify() {
    let fail = Cause::<String>::fail("boom".to_string());
    let die = Cause::<String>::die(Defect::new("kaboom"));
    let interrupt = Cause::<String>::interrupt(None);
    let interrupt_known = Cause::<String>::interrupt(Some(FiberId(7)));

    assert!(matches!(fail.reasons.as_slice(), [Reason::Fail(_)]));
    assert!(matches!(die.reasons.as_slice(), [Reason::Die(_)]));
    assert!(matches!(
        interrupt.reasons.as_slice(),
        [Reason::Interrupt { fiber_id: None }]
    ));
    assert!(matches!(
        interrupt_known.reasons.as_slice(),
        [Reason::Interrupt { fiber_id: Some(_) }]
    ));
}

#[test]
fn exit_to_string() {
    assert_eq!(Exit::<i32, String>::succeed(1).to_string(), "Success(1)");
    assert_eq!(
        Exit::<(), String>::fail("error".to_string()).to_string(),
        "Failure(Cause([Fail(\"error\")]))"
    );
    assert_eq!(
        Exit::<(), String>::die(Defect::new("error")).to_string(),
        "Failure(Cause([Die(\"error\")]))"
    );
    assert_eq!(
        Exit::<(), String>::Failure(Cause::interrupt(Some(FiberId(1)))).to_string(),
        "Failure(Cause([Interrupt(1)]))"
    );
    assert_eq!(
        Exit::<(), String>::Failure(Cause::interrupt(None)).to_string(),
        "Failure(Cause([Interrupt(undefined)]))"
    );
}

#[test]
fn cause_map_preserves_annotations() {
    // CauseMapAnnotations.test.ts:5 — annotations survive mapping.
    let cause = Cause::<String>::fail("error".to_string())
        .annotate("RequestId", "request-1")
        .map(|error| error.to_uppercase());
    assert_eq!(cause.failure(), Some(&"ERROR".to_string()));
    assert_eq!(cause.annotation("RequestId"), Some("request-1"));
    assert_eq!(cause.annotation("Missing"), None);
}

#[test]
fn cause_structural_eq() {
    let a = Cause::fail("x".to_string());
    let b = Cause::fail("x".to_string());
    assert_eq!(a, b);

    let c = Cause::fail("y".to_string());
    assert_ne!(a, c);

    // A Die with the same text is still not a Fail: the variants cannot collapse.
    // Struct literal (not `new`) keeps the backtrace deterministic.
    let d: Cause<String> = Cause::die(Defect {
        message: "x".to_string(),
        backtrace: String::new(),
    });
    assert_ne!(a, d);
    assert_eq!(
        d,
        Cause::die(Defect {
            message: "x".to_string(),
            backtrace: String::new(),
        })
    );
}

#[test]
fn cause_empty_has_no_reasons() {
    let empty: Cause<String> = Cause::empty();
    assert!(empty.reasons.is_empty());
    assert_eq!(empty.reasons, Vec::new());
    assert_eq!(empty, Cause::empty());

    // `never` is `Infallible`, not `()`: an empty cause inhabits any channel.
    let never: Cause<Infallible> = Cause::empty();
    assert!(never.reasons.is_empty());
}

#[test]
fn defect_is_not_an_error_value() {
    // A Die carries no typed error: typed handlers see nothing.
    let die = Cause::<String>::die(Defect::new("boom"));
    assert!(die.failure().is_none());
    assert_eq!(die.defects().len(), 1);
    assert_eq!(die.defects()[0].message, "boom");

    // A Fail carries a typed error and no defect.
    let fail = Cause::fail("e".to_string());
    assert_eq!(fail.failure(), Some(&"e".to_string()));
    assert!(fail.defects().is_empty());

    // Defect implements std Error (reportable) but never unifies with E.
    fn is_std_error<T: std::error::Error>(_: &T) {}
    is_std_error(&Defect::new("boom"));
}

#[test]
fn exit_is_data_not_effect() {
    // Clone + structural equality + matching: data semantics, no runtime.
    let x: Exit<i32, String> = Exit::succeed(1_i32);
    assert_eq!(x, x.clone());
    assert!(x.is_success());

    let f: Exit<i32, String> = Exit::fail("nope".to_string());
    assert!(!f.is_success());
    assert_eq!(f, f.clone());

    let d: Exit<i32, String> = Exit::die(Defect::new("kaboom"));
    assert!(!d.is_success());

    match f {
        Exit::Failure(c) => assert!(matches!(c.reasons.as_slice(), [Reason::Fail(_)])),
        Exit::Success(_) => panic!("expected failure"),
    }
}

#[test]
#[cfg(feature = "tokio")]
fn five_outcomes_do_not_collapse() {
    use elm_effect::{Effect, Runtime};

    // Success classifies as success.
    let ok = Runtime::run(Effect::<u32, String>::succeed(1));
    assert!(matches!(ok, Exit::Success(1)));

    // Fail classifies as a typed failure: no defects, no interruption.
    let fail = Runtime::run(Effect::<u32, String>::fail("boom".to_string()));
    match &fail {
        Exit::Failure(c) => {
            assert_eq!(c.failure(), Some(&"boom".to_string()));
            assert!(c.defects().is_empty());
            assert!(!c.is_interrupted());
        }
        Exit::Success(_) => panic!("expected failure"),
    }

    // Die classifies as a defect: never a typed error value.
    let die = Runtime::run(Effect::<u32, String>::die(Defect::new("kaput")));
    match &die {
        Exit::Failure(c) => {
            assert_eq!(c.failure(), None);
            assert_eq!(c.defects().len(), 1);
            assert!(!c.is_interrupted());
        }
        Exit::Success(_) => panic!("expected failure"),
    }

    // Interrupt classifies as interruption: not a failure at all.
    let intr = Runtime::run(Effect::<u32, String>::interrupt());
    match &intr {
        Exit::Failure(c) => {
            assert!(c.is_interrupted());
            assert_eq!(c.failure(), None);
            assert!(c.defects().is_empty());
        }
        Exit::Success(_) => panic!("expected failure"),
    }

    // `catch` on the error channel recovers typed failures but leaves
    // Die and Interrupt in place.
    let recovered =
        Runtime::run(Effect::<u32, String>::fail("x".into()).catch(|_| Effect::succeed(99)));
    assert!(matches!(recovered, Exit::Success(99)));
    let die_kept =
        Runtime::run(Effect::<u32, String>::die(Defect::new("k")).catch(|_| Effect::succeed(99)));
    assert!(matches!(die_kept, Exit::Failure(_)));
    let intr_kept = Runtime::run(Effect::<u32, String>::interrupt().catch(|_| Effect::succeed(99)));
    assert!(matches!(intr_kept, Exit::Failure(_)));

    // `catch_defect` recovers defects but leaves typed failures and
    // interrupts alone.
    let drecovered = Runtime::run(
        Effect::<u32, String>::die(Defect::new("k")).catch_defect(|_| Effect::succeed(7)),
    );
    assert!(matches!(drecovered, Exit::Success(7)));
    let fleaves =
        Runtime::run(Effect::<u32, String>::fail("x".into()).catch_defect(|_| Effect::succeed(7)));
    assert!(matches!(fleaves, Exit::Failure(_)));
    let ileaves =
        Runtime::run(Effect::<u32, String>::interrupt().catch_defect(|_| Effect::succeed(7)));
    assert!(matches!(ileaves, Exit::Failure(_)));

    // `catch_cause` is the only combinator that sees all three.
    for (effect, code) in [
        (Effect::<u32, String>::fail("x".into()), 1),
        (Effect::<u32, String>::die(Defect::new("k")), 2),
        (Effect::<u32, String>::interrupt(), 130),
    ] {
        let seen = Runtime::run(effect.catch_cause(move |cause| {
            let code = if cause.failure().is_some() {
                1
            } else if !cause.defects().is_empty() {
                2
            } else {
                130
            };
            Effect::succeed(code)
        }));
        assert!(matches!(seen, Exit::Success(c) if c == code));
    }

    // Exit codes do not collapse: 0 success, 1 typed, 2 defect, 130 interrupt.
    assert_eq!(Runtime::exit_code(&ok), 0);
    assert_eq!(Runtime::exit_code(&fail), 1);
    assert_eq!(Runtime::exit_code(&die), 2);
    assert_eq!(Runtime::exit_code(&intr), 130);
}

#[test]
fn library_crate_has_no_run() {
    // `Runtime::run` / `block_on` appear only in `elm-effect` itself,
    // `elm-effect-test`, and `examples/` binaries.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let allowed = ["crates/elm-effect", "crates/elm-effect-test", "examples"];
    let mut stack = vec![root.join("crates")];
    if root.join("examples").exists() {
        stack.push(root.join("examples"));
    }
    let mut offenders = Vec::new();
    while let Some(dir) = stack.pop() {
        let rel = dir
            .strip_prefix(&root)
            .unwrap_or(&dir)
            .to_string_lossy()
            .replace('\\', "/");
        let is_allowed = allowed
            .iter()
            .any(|a| rel == *a || rel.starts_with(&format!("{a}/")));
        let entries = std::fs::read_dir(&dir).unwrap();
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().map(|n| n == "target").unwrap_or(false) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                if is_allowed {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                if src.contains("Runtime::run")
                    || src.contains("block_on")
                    || src.contains("run_main")
                {
                    offenders.push(
                        path.strip_prefix(&root)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "run/block_on outside the edge: {offenders:?}"
    );
}

#[test]
fn cause_narrows_channels() {
    use elm_effect::{Cause, Reason};
    let cause = Cause {
        reasons: vec![
            Reason::Fail("e1"),
            Reason::Die(elm_effect::Defect::new("d1")),
            Reason::Interrupt { fiber_id: None },
            Reason::Fail("e2"),
        ],
        annotations: Vec::new(),
    };
    assert_eq!(cause.failures(), [&"e1", &"e2"]);
    assert_eq!(cause.interruptors(), [None]);
    assert!(cause.is_interrupted());
    let fails = cause.strip_dies_and_interrupts();
    assert_eq!(fails.failures().len(), 2);
    assert!(!fails.is_interrupted());
    let dies = cause.strip_failures_and_interrupts();
    assert!(dies.failures().is_empty());
    assert_eq!(dies.defects().len(), 1);
    let ints = cause.strip_failures_and_dies();
    assert!(ints.failures().is_empty());
    assert!(ints.is_interrupted());
    let clean = Cause::<&str>::empty();
    assert!(clean.failures().is_empty());
    assert!(clean.interruptors().is_empty());
    assert!(!clean.is_interrupted());
}

#[test]
fn cause_type_ids_match_wire() {
    // Cause.test.ts:15, 21 — string constants.
    assert_eq!(Cause::<String>::TYPE_ID, "~effect/Cause");
    assert_eq!(Reason::<String>::REASON_TYPE_ID, "~effect/Cause/Reason");
}

#[test]
fn cause_fail_reason_shape() {
    // Cause.test.ts:106 — empty has no reasons.
    let empty: Cause<String> = Cause::empty();
    assert_eq!(empty.reasons.len(), 0);
    // Cause.test.ts:116 — fail creates a single Fail reason.
    let cause = Cause::fail("error");
    assert_eq!(cause.reasons.len(), 1);
    assert!(is_fail_reason(&cause.reasons[0]));
    // Cause.test.ts:122 — preserves the error value.
    assert!(matches!(&cause.reasons[0], Reason::Fail(e) if *e == "error"));
    // Cause.test.ts:130 — works with various error types.
    let obj = Cause::fail(vec!["key".to_owned()]);
    assert!(matches!(&obj.reasons[0], Reason::Fail(v) if v == &vec!["key".to_owned()]));
}

#[test]
fn cause_reason_narrowers_reject() {
    let fail = Cause::fail("error");
    let die: Cause<String> = Cause::die(Defect::new("defect"));
    let interrupt: Cause<String> = Cause::interrupt(None);
    // Cause.test.ts:64, 69 — Fail narrows; Die and Interrupt rejected.
    assert!(is_fail_reason(&fail.reasons[0]));
    assert!(!is_fail_reason(&die.reasons[0]));
    assert!(!is_fail_reason(&interrupt.reasons[0]));
    // Cause.test.ts:78, 83 — Die narrows; Fail and Interrupt rejected.
    assert!(is_die_reason(&die.reasons[0]));
    assert!(!is_die_reason(&fail.reasons[0]));
    assert!(!is_die_reason(&interrupt.reasons[0]));
    // Cause.test.ts:92, 97 — Interrupt narrows; Fail and Die rejected.
    assert!(is_interrupt_reason(&interrupt.reasons[0]));
    assert!(!is_interrupt_reason(&fail.reasons[0]));
    assert!(!is_interrupt_reason(&die.reasons[0]));
}

// ---------------------------------------------------------------------------
// Cause combinators at the pin (describe fromReasons/has*/squash)
// ---------------------------------------------------------------------------

#[test]
fn from_reasons_builds_a_cause_from_an_array() {
    // Cause.fromReasons([Fail("error"), Die("defect"), Interrupt(1)])
    let cause = Cause::from_reasons(vec![
        Reason::Fail("error".to_string()),
        Reason::Die(Defect::new("defect")),
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
    ]);
    assert_eq!(cause.reasons.len(), 3);
}

#[test]
fn from_reasons_empty_is_empty_cause() {
    // An empty reason list is Cause.empty, not a distinct state.
    let cause = Cause::<String>::from_reasons(Vec::new());
    assert!(cause.reasons.is_empty());
    assert!(!cause.has_fails());
}

#[test]
fn has_fails_requires_a_fail_reason() {
    assert!(Cause::fail("e".to_string()).has_fails());
    assert!(!Cause::<String>::die(Defect::new("d")).has_fails());
    assert!(!Cause::<String>::interrupt(Some(elm_effect::FiberId(1))).has_fails());
    assert!(!Cause::<String>::empty().has_fails());
}

#[test]
fn has_fails_is_true_for_a_combined_cause() {
    // squash-order pinning: a Die plus a Fail still has fails.
    let cause = Cause::from_reasons(vec![
        Reason::Die(Defect::new("defect")),
        Reason::Fail("error".to_string()),
    ]);
    assert!(cause.has_fails());
}

#[test]
fn has_dies_requires_a_die_reason() {
    assert!(Cause::<String>::die(Defect::new("d")).has_dies());
    assert!(!Cause::fail("e".to_string()).has_dies());
    assert!(!Cause::<String>::interrupt(Some(elm_effect::FiberId(1))).has_dies());
    assert!(!Cause::<String>::empty().has_dies());
}

#[test]
fn has_interrupts_requires_an_interrupt_reason() {
    assert!(Cause::<String>::interrupt(Some(elm_effect::FiberId(1))).has_interrupts());
    assert!(!Cause::fail("e".to_string()).has_interrupts());
    assert!(!Cause::<String>::die(Defect::new("d")).has_interrupts());
    assert!(!Cause::<String>::empty().has_interrupts());
}

#[test]
fn has_interrupts_only_is_true_when_every_reason_is_an_interrupt() {
    // One interrupt, and a cause combining two, are both interrupt-only.
    assert!(
        Cause::<String>::interrupt(Some(elm_effect::FiberId(1))).has_interrupts_only()
    );
    let two = Cause::<String>::from_reasons(vec![
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(2)) },
    ]);
    assert!(two.has_interrupts_only());
}

#[test]
fn has_interrupts_only_is_false_for_an_empty_cause() {
    // An empty cause is NOT interrupt-only: the empty case is its own answer.
    assert!(!Cause::<String>::empty().has_interrupts_only());
}

#[test]
fn has_interrupts_only_is_false_when_mixed() {
    let mixed = Cause::from_reasons(vec![
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
        Reason::Fail("error".to_string()),
    ]);
    assert!(!mixed.has_interrupts_only());
}

#[test]
fn squash_prefers_the_first_fail_even_after_interrupt_and_die() {
    let cause = Cause::from_reasons(vec![
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
        Reason::Die(Defect::new("defect")),
        Reason::Fail("first".to_string()),
        Reason::Fail("second".to_string()),
    ]);
    assert_eq!(cause.squash(), elm_effect::Squash::Fail("first".to_string()));
}

#[test]
fn squash_returns_the_first_die_when_there_are_no_fails() {
    let cause = Cause::<String>::from_reasons(vec![
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
        Reason::Die(Defect::new("first")),
        Reason::Die(Defect::new("second")),
    ]);
    assert_eq!(cause.squash(), elm_effect::Squash::Die(Defect::new("first")));
}

#[test]
fn squash_reports_an_interrupt_only_cause() {
    let cause = Cause::<String>::interrupt(Some(elm_effect::FiberId(1)));
    assert_eq!(cause.squash(), elm_effect::Squash::Interrupted);
}

#[test]
fn squash_reports_an_empty_cause() {
    let cause = Cause::<String>::empty();
    assert_eq!(cause.squash(), elm_effect::Squash::Empty);
}

// ---------------------------------------------------------------------------
// Cause.find* family at the pin (describe findFail..findInterrupt)
// ---------------------------------------------------------------------------

#[test]
fn find_fail_returns_the_first_fail_reason() {
    let cause = Cause::from_reasons(vec![
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
        Reason::Fail("error".to_string()),
        Reason::Fail("second".to_string()),
    ]);
    match cause.find_fail() {
        Some(Reason::Fail(e)) => assert_eq!(e, "error"),
        other => panic!("expected the first Fail, got {other:?}"),
    }
}

#[test]
fn find_fail_is_absent_without_a_fail_reason() {
    let none = Cause::<String>::die(Defect::new("defect"));
    assert!(none.find_fail().is_none());
    assert!(Cause::<String>::empty().find_fail().is_none());
}

#[test]
fn find_error_returns_the_error_value() {
    let cause = Cause::fail("error".to_string());
    assert_eq!(cause.find_error(), Some(&"error".to_string()));
    assert!(Cause::<String>::die(Defect::new("d")).find_error().is_none());
}

#[test]
fn find_error_option_is_none_without_a_fail_reason() {
    assert!(Cause::<String>::die(Defect::new("defect")).find_error_option().is_none());
    assert!(Cause::<String>::empty().find_error_option().is_none());
}

#[test]
fn find_die_returns_the_first_die_reason() {
    let cause = Cause::from_reasons(vec![
        Reason::Fail("error".to_string()),
        Reason::Die(Defect::new("first")),
        Reason::Die(Defect::new("second")),
    ]);
    match cause.find_die() {
        Some(Reason::Die(d)) => assert_eq!(d.message, "first"),
        other => panic!("expected the first Die, got {other:?}"),
    }
    assert!(Cause::fail("e".to_string()).find_die().is_none());
}

#[test]
fn find_defect_returns_the_defect_value() {
    let cause = Cause::<String>::die(Defect::new("defect"));
    assert_eq!(cause.find_defect().map(|d| d.message.clone()), Some("defect".to_string()));
    assert!(Cause::fail("e".to_string()).find_defect().is_none());
}

#[test]
fn find_interrupt_returns_the_first_interrupt_reason() {
    let cause = Cause::<String>::interrupt(Some(elm_effect::FiberId(42)));
    match cause.find_interrupt() {
        Some(Reason::Interrupt { fiber_id }) => {
            assert_eq!(*fiber_id, Some(elm_effect::FiberId(42)));
        }
        other => panic!("expected an Interrupt, got {other:?}"),
    }
    assert!(Cause::fail("e".to_string()).find_interrupt().is_none());
}

#[test]
fn interruptor_ids_is_a_deduplicated_ordered_set() {
    let cause = Cause::<String>::from_reasons(vec![
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(3)) },
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(1)) },
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(3)) },
    ]);
    assert_eq!(
        cause.interruptor_ids(),
        vec![elm_effect::FiberId(1), elm_effect::FiberId(3)],
        "a set: ordered and without the duplicate"
    );
}

#[test]
fn interruptor_ids_is_empty_for_non_interrupt_causes() {
    assert!(Cause::fail("e".to_string()).interruptor_ids().is_empty());
    assert!(Cause::<String>::empty().interruptor_ids().is_empty());
}

#[test]
fn interruptor_ids_excludes_undefined_fiber_ids() {
    // A `None` fiber ID is not a member of the set.
    let cause = Cause::<String>::from_reasons(vec![
        Reason::Interrupt { fiber_id: None },
        Reason::Interrupt { fiber_id: Some(elm_effect::FiberId(7)) },
        Reason::Interrupt { fiber_id: None },
    ]);
    assert_eq!(cause.interruptor_ids(), vec![elm_effect::FiberId(7)]);
    assert_eq!(cause.interruptors().len(), 3, "the raw list keeps every entry");
}

// ---------------------------------------------------------------------------
// Cause.combine / map / constructors at the pin
// ---------------------------------------------------------------------------

#[test]
fn combine_merges_two_causes() {
    let combined = Cause::fail("a".to_string()).combine(Cause::fail("b".to_string()));
    assert_eq!(combined.reasons.len(), 2);
}

#[test]
fn combine_with_empty_returns_the_other_cause() {
    let one = Cause::fail("error".to_string());
    assert_eq!(one.clone().combine(Cause::empty()).reasons.len(), 1);
    assert_eq!(Cause::empty().combine(one).reasons.len(), 1);
}

#[test]
fn combine_handles_mixed_reason_types() {
    let combined = Cause::fail("error".to_string())
        .combine(Cause::die(Defect::new("defect")).combine(Cause::<String>::interrupt(Some(
            elm_effect::FiberId(1),
        ))));
    assert_eq!(combined.reasons.len(), 3);
    assert!(combined.has_fails() && combined.has_dies() && combined.has_interrupts());
}

#[test]
fn die_creates_a_cause_with_a_single_die_reason() {
    let cause = Cause::<String>::die(Defect::new("defect"));
    assert_eq!(cause.reasons.len(), 1);
    assert!(is_die_reason(&cause.reasons[0]));
    match &cause.reasons[0] {
        Reason::Die(defect) => assert_eq!(defect.message, "defect"),
        other => panic!("expected a Die reason, got {other:?}"),
    }
}

#[test]
fn interrupt_creates_a_cause_with_a_single_interrupt_reason() {
    let cause = Cause::<String>::interrupt(Some(elm_effect::FiberId(123)));
    assert_eq!(cause.reasons.len(), 1);
    assert!(is_interrupt_reason(&cause.reasons[0]));
}

#[test]
fn map_transforms_fail_reasons_only() {
    let cause = Cause::fail(1u32).combine(Cause::<u32>::die(Defect::new("d")));
    let mapped = cause.map(|n| n + 10);
    assert_eq!(mapped.failures(), vec![&11u32]);
    assert_eq!(mapped.defects().len(), 1, "a Die is untouched");
}

#[test]
fn map_leaves_interrupt_reasons_alone() {
    let cause = Cause::fail(1u32).combine(Cause::<u32>::interrupt(Some(elm_effect::FiberId(9))));
    let mapped = cause.map(|n| n + 10);
    assert!(mapped.has_interrupts(), "an Interrupt reason survives the map");
    assert_eq!(mapped.failures(), vec![&11u32]);
}

#[test]
fn map_of_an_empty_cause_is_empty() {
    let cause = Cause::<u32>::empty();
    let mapped = cause.map(|n| n + 10);
    assert!(mapped.reasons.is_empty());
}

#[test]
fn interrupt_preserves_the_fiber_id() {
    let cause = Cause::<String>::interrupt(Some(FiberId(42)));
    match &cause.reasons[0] {
        Reason::Interrupt { fiber_id } => {
            assert_eq!(*fiber_id, Some(FiberId(42)));
        }
        other => panic!("expected an Interrupt reason, got {other:?}"),
    }
}

#[test]
fn interrupt_allows_an_undefined_fiber_id() {
    let cause = Cause::<String>::interrupt(None);
    match &cause.reasons[0] {
        Reason::Interrupt { fiber_id } => assert!(fiber_id.is_none()),
        other => panic!("expected an Interrupt reason, got {other:?}"),
    }
}
