//! `Schema.test.ts:8647` and `:8680` — a schema carrying an Effect step cannot
//! be run by the sync adapters.
//!
//!     cargo test -p elm-effect --features schema --test schema_parser
//!
//! The step itself has already been resolved by whoever built the schema (this
//! port has no Effect runtime inside `elm-effect-schema`), so `decode` runs it.
//! What the adapters must not do is run it as if it were synchronous: `Result`
//! has no die channel, so the result adapter refuses with its own message, and
//! the exit adapter answers a defect.

use elm_effect::{
    Exit, RESULT_MESSAGE, Schema, SchemaError, Unknown, decode_unknown_exit, decode_unknown_result,
};
use elm_effect_schema::transform::Transformation;

/// `String.pipe(decode({ decode: transformOptionalEffect(...) }))` from the pin,
/// with the effect already resolved: the step hands its input through.
fn async_string() -> Schema<String> {
    Schema::decode_to_effect(
        Schema::<String>::string(),
        Transformation::effect(|value: String| Ok(value), |value: String| Ok(value)),
    )
}

/// `:8647` — `decodeUnknownResult` throws rather than blocking on the step.
#[test]
fn the_result_adapter_refuses_an_async_schema() {
    let refused = decode_unknown_result(&async_string(), Unknown::Text("1".to_string()))
        .expect_err("the result adapter cannot carry a die");

    assert_eq!(refused.message(), RESULT_MESSAGE);
    assert!(
        refused.cause().has_dies(),
        "the refusal carries the defect the result could not"
    );
}

/// `:8680` — `decodeUnknownExit` answers a defect, because `Exit` is the one
/// carrier that can say so.
#[test]
fn the_exit_adapter_dies_on_an_async_schema() {
    match decode_unknown_exit(&async_string(), Unknown::Text("1".to_string())) {
        Exit::Success(value) => panic!("the sync adapter ran an async step: {value:?}"),
        Exit::Failure(cause) => assert!(
            cause.has_dies(),
            "the exit carries the defect, got {cause:?}"
        ),
    }
}

/// The other half of the same rule: a schema whose steps are ordinary closures
/// runs, and its own issue travels as a typed failure rather than a refusal.
#[test]
fn a_sync_schema_still_runs_through_the_same_adapters() {
    let schema = Schema::<f64>::finite_from_string();

    let exit = decode_unknown_exit(&schema, Unknown::Text("1.5".to_string()));
    assert_eq!(exit, Exit::Success(1.5));

    let error = match decode_unknown_exit(&schema, Unknown::Text("nope".to_string())) {
        Exit::Success(value) => panic!("not a finite number: {value}"),
        Exit::Failure(cause) => cause,
    };
    assert!(!error.has_dies(), "a refusal is not a die");
    assert!(error.has_fails());

    let result = decode_unknown_result(&schema, Unknown::Text("nope".to_string()))
        .expect("a bare schema issue is what the result adapter reports")
        .expect_err("text is not a finite number");
    assert!(matches!(result, SchemaError::TypeMismatch { .. }), "{result}");
}
