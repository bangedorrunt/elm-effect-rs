//! `SchemaParser.test.ts` adapter rows: construction, the sync carrier, and the
//! lazy `Suspend` factory.
//!
//! The effect-driven rows (concurrency, interruption, mixed decode causes) are
//! excused in the ledger on the mechanism named in `schema_parser.rs`: this
//! port's schema resolves an effect step before it attaches it, so there is no
//! step graph for a parser to schedule.

#![cfg(all(feature = "schema", feature = "tokio"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use elm_effect::{
    AdapterFailure, Exit, Runtime, decode_unknown_effect, decode_unknown_exit,
    decode_unknown_sync, encode_unknown_sync, make, make_option,
};
use elm_effect_schema::{
    AdapterError, CONSTRUCTOR_DIE, ConstructError, Field, OPTION_DIE, Schema, SchemaError, Unknown,
};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn object(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    )
}

fn issue() -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$.a".to_string(),
        expected: "string",
        got: "number",
    }
}

/// A struct whose `a` field has a constructor default that fails with `error`.
fn defaulted(error: ConstructError) -> Schema<BTreeMap<String, Unknown>> {
    let error = Arc::new(Mutex::new(Some(error)));
    Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()).with_constructor_default(move || {
            let mut slot = error.lock().unwrap();
            match slot.take() {
                Some(error) => Err(error),
                // The default fills once; a second occurrence has no error left.
                None => Ok(text("fallback")),
            }
        }),
    )])
}

#[test]
fn parser_make_keeps_a_mixed_cause() {
    // :380 - a default that died with an issue beside it is not a schema issue.
    // The adapter reports its own message, keeps the defect, and still hands
    // the issue back (the pin's `Cause.findError`).
    let schema = defaulted(ConstructError::Die {
        defect: text("defect"),
        issues: vec![issue()],
    });

    match make(&schema, object(&[])) {
        Err(AdapterError::Die {
            message,
            defect,
            issues,
        }) => {
            assert_eq!(message, CONSTRUCTOR_DIE);
            assert_eq!(defect, text("defect"));
            // The issue rides beside the defect; its path is re-rooted at the
            // field occurrence, so only its presence is stable here.
            assert_eq!(issues.len(), 1);
        }
        other => panic!("expected a constructor refusal, got {other:?}"),
    }
}

#[test]
fn parser_make_option_refuses_a_die() {
    // :394 - `makeOption` answers none for a schema issue, never for a defect.
    let schema = defaulted(ConstructError::die(text("make defect")));

    match make_option(&schema, object(&[])) {
        Err(AdapterError::Die { message, defect, .. }) => {
            assert_eq!(message, OPTION_DIE);
            assert_eq!(defect, text("make defect"));
        }
        other => panic!("expected an option refusal, got {other:?}"),
    }
}

#[test]
fn parser_make_option_keeps_a_mixed_cause() {
    // :406 - the option adapter refuses a cause that holds both, and the issue
    // beside the defect is still reachable.
    let schema = defaulted(ConstructError::Die {
        defect: text("defect"),
        issues: vec![issue()],
    });

    match make_option(&schema, object(&[])) {
        Err(error @ AdapterError::Die { message, .. }) => {
            assert_eq!(message, OPTION_DIE);
            assert!(error.issue().is_some());
        }
        other => panic!("expected an option refusal, got {other:?}"),
    }
}

#[test]
fn parser_sync_refuses_invalid_input() {
    // :521 - the sync adapter refuses a bad input on the decode side. The
    // encode half of the pin's row cannot be stated here: `encode_unknown_sync`
    // takes a `&String`, so an ill-typed value is a compile error, not a
    // runtime refusal.
    let schema = Schema::string();
    match decode_unknown_sync(&schema, Unknown::Bool(true)) {
        Err(AdapterFailure::Issue(error)) => {
            assert!(matches!(error, SchemaError::TypeMismatch { .. }));
        }
        other => panic!("expected a schema issue, got {other:?}"),
    }
    assert_eq!(
        encode_unknown_sync(&schema, &"hi".to_string()),
        Ok(text("hi"))
    );
}

#[test]
fn parser_effect_is_eager_for_a_sync_product() {
    // :304 - a product of sync schemas parses eagerly: the entry point answers
    // with a settled `Exit`, not a deferred description.
    let schema = Schema::array(Schema::string());
    let exit = decode_unknown_exit(
        &schema,
        Unknown::Array(vec![text("a"), text("b")]),
    );
    assert_eq!(
        exit,
        Exit::Success(vec!["a".to_string(), "b".to_string()])
    );

    // :872 - the same eagerness for a synchronous transformation.
    assert_eq!(
        decode_unknown_exit(&Schema::number_from_string(), text("1")),
        Exit::Success(1.0)
    );
}

#[test]
fn parser_effect_fails_with_the_schema_issue() {
    // :555 - the promise adapter's rejection is this port's effect failure: the
    // caller awaits the effect, and an invalid input fails with the issue.
    let exit = Runtime::run(decode_unknown_effect(
        &Schema::string(),
        Unknown::Bool(true),
    ));
    match exit {
        Exit::Failure(cause) => {
            assert!(matches!(
                cause.failure(),
                Some(SchemaError::TypeMismatch { .. })
            ));
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn parser_suspend_is_lazy_and_cached() {
    // :1045 - the factory runs on the first parse that needs it, once.
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let schema: Schema<String> = Schema::<String>::suspend(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Schema::string()
    });

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        decode_unknown_exit(&schema, text("a")),
        Exit::Success(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        decode_unknown_exit(&schema, text("b")),
        Exit::Success(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn parser_suspend_stays_lazy_through_a_structural_link() {
    // :1027 - a link over a suspended schema does not resolve the factory; the
    // first value that reaches it does, and only once.
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let inner: Schema<String> = Schema::<String>::suspend(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Schema::string()
    });
    let schema = Schema::array(inner);

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        decode_unknown_exit(&schema, Unknown::Array(vec![text("a")])),
        Exit::Success(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        decode_unknown_exit(&schema, Unknown::Array(vec![text("b")])),
        Exit::Success(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn parser_suspend_supports_mutual_recursion() {
    // :1060 - two suspended schemas resolve each other through shared cells.
    let a_slot: Arc<Mutex<Option<Schema<Unknown>>>> = Arc::new(Mutex::new(None));
    let b_slot: Arc<Mutex<Option<Schema<Unknown>>>> = Arc::new(Mutex::new(None));

    let a_ref: Schema<Unknown> = Schema::<Unknown>::suspend({
        let slot = Arc::clone(&a_slot);
        move || slot.lock().unwrap().clone().expect("A is built")
    });
    let b_ref: Schema<Unknown> = Schema::<Unknown>::suspend({
        let slot = Arc::clone(&b_slot);
        move || slot.lock().unwrap().clone().expect("B is built")
    });

    let a = Schema::struct_shape(vec![
        Field::required("_tag", Schema::literal(text("A"))),
        Field::required(
            "next",
            Schema::union(Schema::as_unknown(Schema::string()), b_ref),
        ),
    ]);
    let b = Schema::struct_shape(vec![
        Field::required("_tag", Schema::literal(text("B"))),
        Field::required(
            "next",
            Schema::union(Schema::as_unknown(Schema::string()), a_ref),
        ),
    ]);
    *a_slot.lock().unwrap() = Some(Schema::as_unknown(a));
    *b_slot.lock().unwrap() = Some(Schema::as_unknown(b));

    let input = object(&[
        ("_tag", text("A")),
        (
            "next",
            object(&[
                ("_tag", text("B")),
                ("next", object(&[("_tag", text("A")), ("next", text("end"))])),
            ]),
        ),
    ]);
    // Clone out before decoding: the suspend factory locks this same cell, so
    // decoding under the guard would deadlock on the first recursive step.
    let a_schema = a_slot.lock().unwrap().clone().expect("A is built");
    assert!(matches!(
        decode_unknown_exit(&a_schema, input),
        Exit::Success(_)
    ));
}

#[test]
fn parser_suspend_retries_after_a_factory_failure() {
    // :1157 - a factory that throws resolves no cell, so the next parse retries.
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let schema: Schema<String> = Schema::<String>::suspend(move || {
        let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt == 1 {
            panic!("thunk failure");
        }
        Schema::string()
    });

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_unknown_exit(&schema, text("a"))
    }));
    let message = caught.expect_err("the factory failure must escape");
    let message = message
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| message.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("");
    assert!(message.contains("thunk failure"), "message was {message:?}");

    assert!(matches!(
        decode_unknown_exit(&schema, text("b")),
        Exit::Success(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
