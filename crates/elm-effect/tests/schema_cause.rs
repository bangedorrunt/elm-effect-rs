//! Schema.test.ts:3420 `Cause(FiniteFromString, FiniteFromString)`.
//!
//! The pin's row drives both directions through the two value schemas: a text
//! wire decodes to numbers, and a cause holding numbers encodes back to text.

use std::collections::BTreeMap;

use elm_effect::cause::{Cause, FiberId, Reason};
use elm_effect::cause_schema;
use elm_effect_schema::{Schema, Unknown};

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
