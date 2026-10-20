//! Schema.test.ts rows for the decode/encode adapters, and what each carrier
//! can hold: `Exit` keeps a whole cause, and option, result, sync and promise
//! refuse one that holds a defect, handing the cause back on the refusal.
//!
//! The pin reaches these through an effectful `SchemaGetter`
//! (`SchemaGetter.transformOptionalEffect`); a synchronous decode in this port
//! cannot die, so these witnesses hand the adapter the cause directly, which is
//! the same contract one hop before the getter. The controls keep them honest:
//! a cause that is one bare issue still degrades the way the pin says.
//!
//!     cargo test -p elm-effect --features schema --test schema_adapters

use elm_effect::{
    AdapterFailure, Cause, Defect, Exit, OPTION_MESSAGE, PROMISE_MESSAGE, RESULT_MESSAGE,
    SYNC_MESSAGE,
};
use elm_effect_schema::SchemaError;

/// The pin's `new SchemaIssue.InvalidValue({ message: "schema issue" })`.
fn issue() -> SchemaError {
    SchemaError::FilterFailed {
        path: "$".to_string(),
        message: "schema issue".to_string(),
    }
}

/// The pin's `Cause.combine(Cause.fail(issue), Cause.die(new Error("defect")))`:
/// one cause holding a schema issue and a defect.
fn mixed() -> Cause<SchemaError> {
    Cause::combine(Cause::fail(issue()), Cause::die(Defect::new("defect")))
}

/// Schema.test.ts:8714 — "should preserve mixed schema issue and defect
/// causes". The exit adapter is the one carrier that can say both, so nothing
/// is refused and the cause arrives whole, on the decode side and the encode
/// side alike.
#[test]
fn an_exit_keeps_a_mixed_cause_whole() {
    // The pin asserts this for `decodeUnknownExit` and `encodeUnknownExit`;
    // this port's `Exit` carries no direction, so one cause proves both.
    for exit in [
        Exit::<String, SchemaError>::Failure(mixed()),
        Exit::<String, SchemaError>::Failure(mixed()),
    ] {
        assert!(!exit.is_success());
        let Exit::Failure(cause) = exit else {
            unreachable!("the exit failed");
        };
        assert!(cause.has_dies(), "the defect survives");
        assert_eq!(
            cause.find_error().map(ToString::to_string),
            Some("schema issue at $".to_string()),
            "and so does the issue beside it"
        );
    }
}

/// Schema.test.ts:8435 — "should reject with an error when the cause contains
/// both a schema issue and a defect": the promise adapter refuses, and the
/// rejection carries the whole cause.
#[test]
fn the_promise_adapter_refuses_a_mixed_cause() {
    let error = Exit::<String, SchemaError>::Failure(mixed())
        .into_promise()
        .expect_err("a cause with a defect is not a rejection the promise can carry");
    assert_eq!(error.to_string(), PROMISE_MESSAGE);
    let AdapterFailure::Refusal(refusal) = error else {
        panic!("a die must not be reported as the schema issue: {error:?}");
    };
    assert_eq!(refusal.message(), PROMISE_MESSAGE);
    assert!(refusal.cause().has_dies());
    assert!(refusal.cause().find_error().is_some());

    // The control: one bare issue rejects as itself.
    let plain = Exit::<String, SchemaError>::Failure(Cause::fail(issue()))
        .into_promise()
        .expect_err("a bare issue rejects");
    assert_eq!(plain.to_string(), "schema issue at $");
    assert!(matches!(plain, AdapterFailure::Issue(_)));
}

/// Schema.test.ts:8482 — "should throw an error when the cause is not a schema
/// issue": a bare defect is not a validation failure, so the option adapter
/// throws instead of answering none.
#[test]
fn the_option_adapter_refuses_a_bare_defect() {
    let cause = Cause::fail(issue());
    let error = Exit::<String, SchemaError>::Failure(Cause::die(Defect::new("decode defect")))
        .into_option()
        .expect_err("a die is not a schema issue");
    assert_eq!(error.to_string(), OPTION_MESSAGE);
    assert!(error.cause().has_dies());

    // The control: a bare issue is exactly what none means.
    assert_eq!(
        Exit::<String, SchemaError>::Failure(cause).into_option(),
        Ok(None)
    );
    assert_eq!(
        Exit::<String, SchemaError>::succeed("a".to_string()).into_option(),
        Ok(Some("a".to_string()))
    );
}

/// Schema.test.ts:8504 — "should throw an error when the cause contains both a
/// schema issue and a defect": the option adapter refuses the mixture rather
/// than reading the issue out of it.
#[test]
fn the_option_adapter_refuses_a_mixed_cause() {
    let error = Exit::<String, SchemaError>::Failure(mixed())
        .into_option()
        .expect_err("a mixed cause is not a schema issue");
    assert_eq!(error.to_string(), OPTION_MESSAGE);
    assert!(error.cause().has_dies());
    assert!(
        error.cause().find_error().is_some(),
        "the issue still rides on the refusal"
    );
}

/// Schema.test.ts:8566 — "should throw an error when the cause contains both a
/// schema issue and a defect", for the result adapter.
#[test]
fn the_result_adapter_refuses_a_mixed_cause() {
    let error = Exit::<String, SchemaError>::Failure(mixed())
        .into_result()
        .expect_err("a mixed cause is not a schema issue");
    assert_eq!(error.to_string(), RESULT_MESSAGE);
    assert!(error.cause().has_dies());

    // The control: a bare issue is the result's own failure.
    let plain = Exit::<String, SchemaError>::Failure(Cause::fail(issue()))
        .into_result()
        .expect("a bare issue is what the result adapter carries");
    assert_eq!(
        plain
            .expect_err("the issue travels as the failure")
            .to_string(),
        "schema issue at $"
    );
}

/// Schema.test.ts:8619 — "should throw an error when the cause contains both a
/// schema issue and a defect", for the sync adapter.
#[test]
fn the_sync_adapter_refuses_a_mixed_cause() {
    let error = Exit::<String, SchemaError>::Failure(mixed())
        .into_value()
        .expect_err("a mixed cause is not a schema issue");
    assert_eq!(error.to_string(), SYNC_MESSAGE);
    let AdapterFailure::Refusal(refusal) = error else {
        panic!("a die must not be reported as the schema issue: {error:?}");
    };
    assert!(refusal.cause().has_dies());

    // The control: success hands the value back, a bare issue is the throw.
    assert_eq!(
        Exit::<String, SchemaError>::succeed("a".to_string()).into_value(),
        Ok("a".to_string())
    );
    let plain = Exit::<String, SchemaError>::Failure(Cause::fail(issue()))
        .into_value()
        .expect_err("a bare issue is what the sync adapter throws");
    assert_eq!(plain.to_string(), "schema issue at $");
}
