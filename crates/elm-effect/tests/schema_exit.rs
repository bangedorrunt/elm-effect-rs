//! Schema.test.ts:3511 `Exit(FiniteFromString, String, Unknown)`.
//!
//! Both wire shapes travel the caller's schemas: a success value and a
//! failure's cause, each reported at the field it failed.

use std::collections::BTreeMap;

use elm_effect::Exit;
use elm_effect::cause::{Cause, Reason};
use elm_effect::exit_schema;
use elm_effect_schema::{Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: [(&str, Unknown); 2]) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn exit_wire(tag: &str, field: &str, value: Unknown) -> Unknown {
    obj([("_tag", text(tag)), (field, value)])
}

fn cause_wire(error: Unknown) -> Unknown {
    Unknown::Array(vec![obj([("_tag", text("Fail")), ("error", error)])])
}

#[test]
fn exit_round_trips_through_three_schemas() {
    let schema = exit_schema(
        Schema::finite_from_string(),
        Schema::string(),
        Schema::<Unknown>::any(),
    );

    // A success value runs the success schema: text on the wire, number inside.
    let success = schema
        .decode(exit_wire("Success", "value", text("123")))
        .unwrap();
    assert_eq!(success, Exit::Success(123.0));

    // A failure runs the cause schema, which runs the error schema.
    let failure = schema
        .decode(exit_wire("Failure", "cause", cause_wire(text("boom"))))
        .unwrap();
    assert_eq!(
        failure,
        Exit::Failure(Cause::from_reasons(vec![Reason::Fail("boom".to_string())]))
    );

    // Anything that is not a tagged object is not an Exit.
    assert!(schema.decode(Unknown::Null).is_err());

    // A number where the success schema wants text fails at the value field.
    let bad_value = schema
        .decode(exit_wire("Success", "value", Unknown::Number(123.0)))
        .unwrap_err()
        .to_string();
    assert!(bad_value.contains("$.value"), "{bad_value}");

    // A failure whose cause error is a number fails inside the cause.
    let bad_cause = schema
        .decode(exit_wire(
            "Failure",
            "cause",
            cause_wire(Unknown::Number(123.0)),
        ))
        .unwrap_err()
        .to_string();
    assert!(bad_cause.contains("$.cause[0].error"), "{bad_cause}");

    // Encoding runs the same schemas the other way.
    assert_eq!(
        schema.try_encode(&success).unwrap(),
        exit_wire("Success", "value", text("123"))
    );
    assert_eq!(
        schema.try_encode(&failure).unwrap(),
        exit_wire("Failure", "cause", cause_wire(text("boom")))
    );
}
