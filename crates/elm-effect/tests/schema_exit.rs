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

fn obj<const N: usize>(pairs: [(&str, Unknown); N]) -> Unknown {
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

/// Schema.test.ts:3538 `Exit(FiniteFromString, String, Defect)`. The defect
/// side is a real schema: the wire's `{ name, message }` becomes the runtime's
/// defect, so a cause holding a die reason decodes instead of being refused.
#[test]
fn exit_decodes_a_defect_through_the_defect_schema() {
    let schema = exit_schema(
        Schema::finite_from_string(),
        Schema::string(),
        elm_effect::defect_schema(),
    );
    let defect = obj([("name", text("boom")), ("message", text("boom message"))]);
    let wire = exit_wire(
        "Failure",
        "cause",
        Unknown::Array(vec![obj([("_tag", text("Die")), ("defect", defect)])]),
    );

    assert_eq!(
        schema.decode(wire).unwrap(),
        Exit::die(elm_effect::Defect {
            message: "boom message".to_string(),
            backtrace: String::new(),
        })
    );
}

/// Schema.test.ts:3501 `Exit` `should expose the values`: the three schemas the
/// constructor was given stay reachable, and an annotation over it keeps them.
#[test]
fn the_exit_schema_exposes_the_values_it_was_built_from() {
    use elm_effect::ExitSchema;

    let schema = ExitSchema::new(
        Schema::string(),
        Schema::number(),
        Schema::as_unknown(Schema::boolean()),
    );
    assert_eq!(schema.value().to_code(), Schema::string().to_code());
    assert_eq!(schema.error().to_code(), Schema::number().to_code());
    assert_eq!(
        schema.defect().to_code(),
        Schema::as_unknown(Schema::boolean()).to_code()
    );

    let annotated = schema.annotate(vec![("title", "exit")]);
    assert_eq!(annotated.value().to_code(), Schema::string().to_code());
    assert_eq!(annotated.error().to_code(), Schema::number().to_code());
    assert_eq!(
        annotated.defect().to_code(),
        Schema::as_unknown(Schema::boolean()).to_code()
    );

    // The annotated codec is still the same codec.
    let wire = exit_wire("Success", "value", text("a"));
    assert_eq!(
        annotated.schema().decode(wire.clone()).unwrap(),
        schema.schema().decode(wire).unwrap()
    );
}

/// Schema.test.ts:3305 `Defect`, the half this port carries: the wire's
/// `message` is required, `name` is advisory, and `stack` is the backtrace.
///
/// The row's other half, decoding an arbitrary non-object value as the defect
/// itself, has no carrier in a [`elm_effect::Defect`] struct, which is why the
/// row itself stays excused rather than ported.
#[test]
fn a_defect_reads_message_name_and_stack() {
    let schema = elm_effect::defect_schema();

    let message_only = schema.decode(obj([("message", text("a"))])).unwrap();
    assert_eq!(message_only.message, "a");
    assert!(message_only.backtrace.is_empty());

    let stacked = schema
        .decode(obj([
            ("name", text("b")),
            ("message", text("a")),
            ("stack", text("c")),
        ]))
        .unwrap();
    assert_eq!(stacked.message, "a");
    assert_eq!(stacked.backtrace, "c");

    // No message, no defect: the path names the field the pin names.
    let refused = schema
        .decode(obj([("name", text("b"))]))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("$.message"), "{refused}");
}
