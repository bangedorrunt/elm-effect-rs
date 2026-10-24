//! `SchemaParser.test.ts:531`, `:565`, `:598`, `:620`, `:650`, `:663`, `:700`,
//! `:713`, `:728`, `:750` and `:1176` — a step whose cause holds an issue beside
//! a defect is not a schema issue, so each adapter that can carry only an issue
//! refuses it under its own message and keeps the whole cause.
//!
//!     cargo test -p elm-effect --features schema --test schema_parser_mixed_causes
//!
//! The steps here answer on their first poll, which is what makes them
//! reachable from a sync adapter: the pin's `asExit` is `Effect.runSyncExit`,
//! which runs work that settles now and dies on work that has to wait.

#![cfg(feature = "schema")]

use std::collections::BTreeMap;
use std::future::ready;

use elm_effect::{
    ASSERTION_MESSAGE, AdapterFailure, Exit, GUARD_MESSAGE, OPTION_MESSAGE, PROMISE_MESSAGE,
    RESULT_MESSAGE, SYNC_ISSUE_MESSAGE, asserts, decode_unknown_exit, decode_unknown_option,
    decode_unknown_result, decode_unknown_sync, encode_unknown_exit, encode_unknown_option,
    encode_unknown_result, encode_unknown_sync, type_guard,
};
use elm_effect_schema::{Field, Schema, SchemaError, StepCause, StepFuture, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn issue() -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected: "a string",
        got: "number",
    }
}

/// A codec whose decode step answers `cause` and whose encode step passes its
/// input through.
fn decode_failure(cause: StepCause) -> Schema<Unknown> {
    Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        move |_input| Box::pin(ready(Err(cause.clone()))) as StepFuture,
        |input| Box::pin(ready(Ok(input))) as StepFuture,
    )
}

/// The same codec with the failure on the encode axis.
fn encode_failure(cause: StepCause) -> Schema<Unknown> {
    Schema::effect_codec(
        Schema::as_unknown(Schema::string()),
        |input| Box::pin(ready(Ok(input))) as StepFuture,
        move |_input| Box::pin(ready(Err(cause.clone()))) as StepFuture,
    )
}

/// `:531` (through `:1176`) — `Exit` is the one carrier that can hold both
/// reasons, and the issue beside the defect stays reachable.
#[test]
fn parser_exit_keeps_a_mixed_step_cause() {
    let decoded = decode_unknown_exit(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    );
    match decoded {
        Exit::Failure(cause) => {
            assert!(cause.has_fails(), "the issue rides: {cause:?}");
            assert!(cause.has_dies(), "the defect rides beside it: {cause:?}");
            assert!(
                matches!(cause.failure(), Some(SchemaError::TypeMismatch { .. })),
                "the issue is still findable: {cause:?}"
            );
        }
        Exit::Success(value) => panic!("expected the mixed cause, got {value:?}"),
    }

    let encoded = encode_unknown_exit(
        &encode_failure(StepCause::mixed(issue(), "encode defect")),
        &text("1"),
    );
    match encoded {
        Exit::Failure(cause) => {
            assert!(cause.has_fails(), "the issue rides: {cause:?}");
            assert!(cause.has_dies(), "the defect rides beside it: {cause:?}");
        }
        Exit::Success(value) => panic!("expected the mixed cause, got {value:?}"),
    }
}

/// `:531` — `getSchemaIssueOrThrow` refuses a cause that is not a bare issue,
/// on both axes.
#[test]
fn parser_sync_refuses_a_mixed_step_cause() {
    let decode = decode_unknown_sync(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    );
    match decode {
        Err(AdapterFailure::Refusal(refusal)) => {
            assert_eq!(refusal.message(), SYNC_ISSUE_MESSAGE);
            assert!(refusal.cause().has_fails(), "the issue rides");
            assert!(refusal.cause().has_dies(), "the defect rides");
        }
        other => panic!("expected the sync adapter's refusal, got {other:?}"),
    }

    let encode = encode_unknown_sync(
        &encode_failure(StepCause::mixed(issue(), "encode defect")),
        &text("1"),
    );
    match encode {
        Err(AdapterFailure::Refusal(refusal)) => {
            assert_eq!(refusal.message(), SYNC_ISSUE_MESSAGE);
            assert!(refusal.cause().has_dies(), "the defect rides");
        }
        other => panic!("expected the sync adapter's refusal, got {other:?}"),
    }
}

/// `:565` — the promise adapter rejects with its own message, and its `cause`
/// is the underlying cause with the defect in it.
#[test]
fn parser_promise_refuses_a_mixed_step_cause() {
    let exit = decode_unknown_exit(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    );
    match exit.into_promise() {
        Err(AdapterFailure::Refusal(refusal)) => {
            assert_eq!(refusal.message(), PROMISE_MESSAGE);
            assert!(refusal.cause().has_fails(), "the issue rides");
            assert!(refusal.cause().has_dies(), "the defect rides");
        }
        other => panic!("expected the promise adapter's refusal, got {other:?}"),
    }
}

/// `:598` — a step that dies is not a schema issue, so the option adapter
/// refuses instead of answering `None`.
#[test]
fn parser_option_refuses_a_dying_step() {
    let refused = decode_unknown_option(
        &decode_failure(StepCause::defect("decode defect")),
        text("1"),
    )
    .expect_err("a die is not `None`");
    assert_eq!(refused.message(), OPTION_MESSAGE);
    assert!(refused.cause().has_dies(), "the defect rides");
    assert!(!refused.cause().has_fails(), "a die alone is not an issue");
}

/// `:620` — and it refuses the mixed cause the same way.
#[test]
fn parser_option_refuses_a_mixed_step_cause() {
    let refused = decode_unknown_option(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    )
    .expect_err("a mixed cause is not `None`");
    assert_eq!(refused.message(), OPTION_MESSAGE);
    assert!(refused.cause().has_fails(), "the issue rides");
    assert!(refused.cause().has_dies(), "the defect rides");

    let encoded = encode_unknown_option(
        &encode_failure(StepCause::mixed(issue(), "encode defect")),
        &text("1"),
    )
    .expect_err("a mixed cause is not `None`");
    assert_eq!(encoded.message(), OPTION_MESSAGE);
    assert!(encoded.cause().has_dies(), "the defect rides");
}

/// `:728` — the result adapter refuses a die rather than wrapping it in an
/// `Err` it cannot represent.
#[test]
fn parser_result_refuses_a_dying_step() {
    let refused = decode_unknown_result(
        &decode_failure(StepCause::defect("decode defect")),
        text("1"),
    )
    .expect_err("a die is not a schema issue");
    assert_eq!(refused.message(), RESULT_MESSAGE);
    assert!(refused.cause().has_dies(), "the defect rides");
    assert!(!refused.cause().has_fails(), "a die alone is not an issue");
}

/// `:750` — and it refuses the mixed cause the same way, on both axes.
#[test]
fn parser_result_refuses_a_mixed_step_cause() {
    let decode = decode_unknown_result(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    );
    match decode {
        Err(refusal) => {
            assert_eq!(refusal.message(), RESULT_MESSAGE);
            assert!(refusal.cause().has_fails(), "the issue rides");
            assert!(refusal.cause().has_dies(), "the defect rides");
        }
        other => panic!("expected the result adapter's refusal, got {other:?}"),
    }

    let encode = encode_unknown_result(
        &encode_failure(StepCause::mixed(issue(), "encode defect")),
        &text("1"),
    );
    match encode {
        Err(refusal) => {
            assert_eq!(refusal.message(), RESULT_MESSAGE);
            assert!(refusal.cause().has_dies(), "the defect rides");
        }
        other => panic!("expected the result adapter's refusal, got {other:?}"),
    }
}

/// `:1176` — a union candidate whose step died is not a candidate that
/// refused. The cause is preserved rather than the next candidate being tried,
/// in both the plain and the tagged union the row drives.
#[test]
fn parser_union_stops_at_a_mixed_step_cause() {
    let plain = Schema::union_with_mode(
        vec![
            decode_failure(StepCause::mixed(issue(), "decode defect")),
            Schema::<Unknown>::literal(text("a")),
        ],
        "anyOf",
    );
    match decode_unknown_exit(&plain, text("a")) {
        Exit::Failure(cause) => {
            assert!(cause.has_fails(), "the issue rides: {cause:?}");
            assert!(cause.has_dies(), "the defect rides: {cause:?}");
        }
        Exit::Success(value) => panic!("the union tried the next candidate: {value:?}"),
    }

    let tagged = Schema::union_with_mode(
        vec![
            Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
                Field::required(
                    "kind",
                    Schema::as_unknown(Schema::<Unknown>::literal(text("a"))),
                ),
                Field::required(
                    "value",
                    decode_failure(StepCause::mixed(issue(), "decode defect")),
                ),
            ])),
            Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
                Field::required(
                    "kind",
                    Schema::as_unknown(Schema::<Unknown>::literal(text("b"))),
                ),
                Field::required("value", Schema::as_unknown(Schema::string())),
            ])),
        ],
        "anyOf",
    );
    let input = Unknown::Object(
        [
            ("kind".to_string(), text("a")),
            ("value".to_string(), text("a")),
        ]
        .into_iter()
        .collect(),
    );
    match decode_unknown_exit(&tagged, input) {
        Exit::Failure(cause) => assert!(cause.has_dies(), "the defect rides: {cause:?}"),
        Exit::Success(value) => panic!("the tagged union tried the next candidate: {value:?}"),
    }
}

/// `:644` on the adapter itself: a bare schema issue is what the guard answers
/// `false` for, and a value the schema takes answers `true`.
#[test]
fn parser_type_guard_answers_false_for_a_bare_issue() {
    let schema = Schema::string();
    assert_eq!(type_guard(&schema, text("a")), Ok(true));
    assert_eq!(type_guard(&schema, Unknown::Bool(true)), Ok(false));
}

/// `:650` — a step that dies is not a value that failed validation, so the
/// guard refuses instead of answering `false`.
#[test]
fn parser_type_guard_refuses_a_dying_step() {
    let refused = type_guard(
        &decode_failure(StepCause::defect("decode defect")),
        text("1"),
    )
    .expect_err("a die is not `false`");
    assert_eq!(refused.message(), GUARD_MESSAGE);
    assert!(refused.cause().has_dies(), "the defect rides");
    assert!(!refused.cause().has_fails(), "a die alone is not an issue");
}

/// `:663` — and the mixed cause the same way.
#[test]
fn parser_type_guard_refuses_a_mixed_step_cause() {
    let refused = type_guard(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    )
    .expect_err("a mixed cause is not `false`");
    assert_eq!(refused.message(), GUARD_MESSAGE);
    assert!(refused.cause().has_fails(), "the issue rides");
    assert!(refused.cause().has_dies(), "the defect rides");
}

/// `:678` and `:690` on the adapter itself: the default options decide the
/// assertion, and the schema's own issue comes back as data.
#[test]
fn parser_asserts_reports_a_bare_issue() {
    let schema = Schema::struct_shape(vec![
        Field::required("a", Schema::as_unknown(Schema::<String>::string())),
        Field::required("b", Schema::as_unknown(Schema::<String>::string())),
    ]);
    let valid = Unknown::Object(
        [("a".to_string(), text("a")), ("b".to_string(), text("b"))]
            .into_iter()
            .collect(),
    );
    assert_eq!(asserts(&schema, valid), Ok(()));

    // `errors: "all"` would name both keys; the default reports the first.
    let invalid = Unknown::Object(
        [
            ("a".to_string(), Unknown::Bool(true)),
            ("b".to_string(), text("b")),
        ]
        .into_iter()
        .collect(),
    );
    match asserts(&schema, invalid) {
        Err(AdapterFailure::Issue(issue)) => assert!(
            matches!(&issue, SchemaError::TypeMismatch { path, .. } if path == "$.a"),
            "the first key is the issue: {issue:?}"
        ),
        other => panic!("expected the schema issue, got {other:?}"),
    }
}

/// `:700` — the assertion adapter refuses a die rather than reporting it as a
/// validation failure.
#[test]
fn parser_asserts_refuses_a_dying_step() {
    let refused = asserts(
        &decode_failure(StepCause::defect("decode defect")),
        text("1"),
    )
    .expect_err("a die is not a validation failure");
    match refused {
        AdapterFailure::Refusal(refusal) => {
            assert_eq!(refusal.message(), ASSERTION_MESSAGE);
            assert!(refusal.cause().has_dies(), "the defect rides");
            assert!(!refusal.cause().has_fails(), "a die alone is not an issue");
        }
        AdapterFailure::Issue(issue) => panic!("a die became an issue: {issue:?}"),
    }
}

/// `:713` — and it refuses the mixed cause the same way.
#[test]
fn parser_asserts_refuses_a_mixed_step_cause() {
    let refused = asserts(
        &decode_failure(StepCause::mixed(issue(), "decode defect")),
        text("1"),
    )
    .expect_err("a mixed cause is not a validation failure");
    match refused {
        AdapterFailure::Refusal(refusal) => {
            assert_eq!(refusal.message(), ASSERTION_MESSAGE);
            assert!(refusal.cause().has_fails(), "the issue rides");
            assert!(refusal.cause().has_dies(), "the defect rides");
        }
        AdapterFailure::Issue(issue) => panic!("a mixed cause became an issue: {issue:?}"),
    }
}
