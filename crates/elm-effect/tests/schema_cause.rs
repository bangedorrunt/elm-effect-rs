//! Schema.test.ts:3420 `Cause(FiniteFromString, FiniteFromString)`.
//!
//! The pin's row drives both directions through the two value schemas: a text
//! wire decodes to numbers, and a cause holding numbers encodes back to text.

use std::collections::BTreeMap;

use elm_effect::cause::{Cause, FiberId, Reason};
use elm_effect::cause_schema;
use elm_effect_schema::transform::ParseOptions;
use elm_effect_schema::{ExcessProperty, Schema, Unknown};

fn reason(tag: &str, field: &str, value: Unknown) -> Unknown {
    Unknown::Object(BTreeMap::from([
        ("_tag".to_string(), Unknown::Text(tag.to_string())),
        (field.to_string(), value),
    ]))
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

#[test]
fn cause_round_trips_through_the_two_schemas() {
    let schema = cause_schema(Schema::finite_from_string(), Schema::finite_from_string());
    let wire = || {
        Unknown::Array(vec![
            reason("Fail", "error", text("1")),
            reason("Die", "defect", text("2")),
            reason("Interrupt", "fiberId", Unknown::Number(3.0)),
            reason("Interrupt", "fiberId", Unknown::Null),
        ])
    };

    let cause = schema.decode(wire()).unwrap();
    assert_eq!(
        cause,
        Cause::from_reasons(vec![
            Reason::Fail(1.0),
            Reason::Die(2.0),
            Reason::Interrupt {
                fiber_id: Some(FiberId(3))
            },
            Reason::Interrupt { fiber_id: None },
        ])
    );

    // A failing member names the reason and the field inside it.
    let bad_fail = schema
        .decode(Unknown::Array(vec![reason("Fail", "error", text("a"))]))
        .unwrap_err()
        .to_string();
    assert!(bad_fail.contains("$[0].error"), "{bad_fail}");
    let bad_die = schema
        .decode(Unknown::Array(vec![reason("Die", "defect", text("a"))]))
        .unwrap_err()
        .to_string();
    assert!(bad_die.contains("$[0].defect"), "{bad_die}");

    // The value side is the number the schema decoded; encoding runs it back
    // through the same schema, so the wire form is text again.
    assert_eq!(schema.try_encode(&cause).unwrap(), wire());

    // Encoding is total for a typed cause here: the pin's encoding failure
    // (`Cause.fail("a")` through a numeric schema) is a JS value this port
    // cannot construct, because `E` is `f64` and not `string | number`. What
    // matters for the defect side is that `Die` travels the caller's defect
    // schema, which the round trip above shows: the wire is text either way.
}

/// Schema.test.ts:3396 `strict encoding preserves the default wire format for
/// every reason variant`: asking the encoder to refuse excess properties
/// changes nothing about a cause's wire, because every field a reason carries
/// is one its schema declares. The interrupt with no fiber id is the case that
/// would otherwise be tempting to leave out.
#[test]
fn strict_encoding_keeps_the_reason_wire_format() {
    let schema = cause_schema(Schema::string(), Schema::number());
    let cause = Cause::from_reasons(vec![
        Reason::Fail("boom".to_string()),
        Reason::Die(42.0),
        Reason::Interrupt {
            fiber_id: Some(FiberId(7)),
        },
        Reason::Interrupt { fiber_id: None },
    ]);
    let wire = Unknown::Array(vec![
        Unknown::Object(BTreeMap::from([
            ("_tag".to_string(), text("Fail")),
            ("error".to_string(), text("boom")),
        ])),
        Unknown::Object(BTreeMap::from([
            ("_tag".to_string(), text("Die")),
            ("defect".to_string(), Unknown::Number(42.0)),
        ])),
        Unknown::Object(BTreeMap::from([
            ("_tag".to_string(), text("Interrupt")),
            ("fiberId".to_string(), Unknown::Number(7.0)),
        ])),
        Unknown::Object(BTreeMap::from([
            ("_tag".to_string(), text("Interrupt")),
            ("fiberId".to_string(), Unknown::Null),
        ])),
    ]);

    let strict = ParseOptions {
        on_excess_property: ExcessProperty::Reject,
        ..ParseOptions::default()
    };
    assert_eq!(schema.try_encode(&cause).unwrap(), wire);
    assert_eq!(strict.scoped(|| schema.try_encode(&cause)).unwrap(), wire);
    assert_eq!(schema.decode(wire.clone()).unwrap(), cause);
    assert_eq!(strict.scoped(|| schema.decode(wire.clone())).unwrap(), cause);
}


/// Schema.test.ts:3377 `CauseReason` `should expose the values`, and :3412 for
/// `Cause`: the two value schemas stay reachable on the value the constructor
/// answers, and an annotation over it keeps them.
///
/// "The same schema" is read through `to_code`, this port's rendering of a
/// schema; the accessors hand back the schemas the caller passed, not re-derived
/// copies, so the rendering matches exactly.
#[test]
fn the_cause_schemas_expose_the_values_they_were_built_from() {
    use elm_effect::{CauseReasonSchema, CauseSchema};

    let reason_schema = CauseReasonSchema::new(Schema::string(), Schema::number());
    assert_eq!(reason_schema.error().to_code(), Schema::string().to_code());
    assert_eq!(reason_schema.defect().to_code(), Schema::number().to_code());
    let annotated = reason_schema.clone().annotate(vec![("title", "reason")]);
    assert_eq!(annotated.error().to_code(), Schema::string().to_code());
    assert_eq!(annotated.defect().to_code(), Schema::number().to_code());

    let cause = CauseSchema::new(Schema::string(), Schema::number());
    assert_eq!(cause.error().to_code(), Schema::string().to_code());
    assert_eq!(cause.defect().to_code(), Schema::number().to_code());
    let annotated = cause.annotate(vec![("title", "cause")]);
    assert_eq!(annotated.error().to_code(), Schema::string().to_code());
    assert_eq!(annotated.defect().to_code(), Schema::number().to_code());

    // The annotated codec is still the same codec.
    let wire = Unknown::Array(vec![reason("Fail", "error", text("boom"))]);
    assert_eq!(
        annotated.schema().decode(wire.clone()).unwrap(),
        cause.schema().decode(wire).unwrap()
    );
}

/// Schema.test.ts:3385 `CauseReason(FiniteFromString, FiniteFromString)`: one
/// reason built over the two refined schemas reads text on the wire and holds
/// the numbers they decoded, in both directions.
///
/// The pin's body only runs a generation check, so the carrier this row
/// actually asserts is that the constructor accepts the pair and the pair then
/// runs for each non-interrupt variant.
#[test]
fn a_reason_over_two_refined_schemas_reads_and_writes_its_wire() {
    use elm_effect::cause_reason_schema;

    let schema = cause_reason_schema(Schema::finite_from_string(), Schema::finite_from_string());

    assert_eq!(
        schema.decode(reason("Fail", "error", text("1"))).unwrap(),
        Reason::Fail(1.0)
    );
    assert_eq!(
        schema.decode(reason("Die", "defect", text("2"))).unwrap(),
        Reason::Die(2.0)
    );

    assert_eq!(
        schema.try_encode(&Reason::Fail(1.0)).unwrap(),
        reason("Fail", "error", text("1"))
    );
    assert_eq!(
        schema.try_encode(&Reason::Die(2.0)).unwrap(),
        reason("Die", "defect", text("2"))
    );
}
