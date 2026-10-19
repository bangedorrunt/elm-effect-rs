//! `Schema.test.ts` "ArrayEnsure" rows: one value or an array of them.
//!
//!     cargo test -p elm-effect-schema --test schema_array_ensure
//!
//! The pin builds `ArrayEnsure` as `Union([one, many])` lowered through a
//! `decodeTo`; this port keeps the order the union tries (the inner schema
//! first, then element-wise) and the inverse encode. Values are wire values
//! here, as everywhere in this port, so a `FiniteFromString` element appears as
//! the text it travels as; the pinned rows that assert decoded numbers are read
//! at that level.

use elm_effect_schema::{Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn array(items: Vec<Unknown>) -> Unknown {
    Unknown::Array(items)
}

fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

/// Schema.test.ts:1210 — "ArrayEnsure". A single value becomes a one-element
/// array, an array decodes element-wise, and silence (`null`) is neither.
#[test]
fn one_value_or_many() {
    let schema = Schema::array_ensure(finite());

    assert_eq!(schema.decode(text("1")).unwrap(), array(vec![text("1")]));
    assert_eq!(
        schema.decode(array(vec![text("1"), text("2")])).unwrap(),
        array(vec![text("1"), text("2")])
    );
    assert_eq!(schema.decode(array(vec![])).unwrap(), array(vec![]));

    // Neither member accepts `null`, and the inner schema is the one that
    // says so; the pin prints the union's joined type here instead.
    assert_eq!(
        schema.decode(Unknown::Null).unwrap_err().to_string(),
        "expected a finite number string at $, got null"
    );
    assert_eq!(
        schema.decode(text("a")).unwrap_err().to_string(),
        "expected a finite number string at $, got string"
    );
    assert_eq!(
        schema
            .decode(array(vec![text("a")]))
            .unwrap_err()
            .to_string(),
        "expected a finite number string at $[0], got string"
    );

    // Encoding is the inverse: a one-element array unwraps, everything else
    // stays an array.
    assert_eq!(schema.try_encode(&array(vec![])).unwrap(), array(vec![]));
    assert_eq!(
        schema.try_encode(&array(vec![text("1")])).unwrap(),
        text("1")
    );
    assert_eq!(
        schema
            .try_encode(&array(vec![text("1"), text("2")]))
            .unwrap(),
        array(vec![text("1"), text("2")])
    );
}

/// Schema.test.ts:1232 — "ArrayEnsure roundtrips array-valued elements". The
/// inner schema is tried first, so a one-element array of an array is one
/// tuple and not two numbers.
#[test]
fn an_array_valued_element_round_trips() {
    let schema = Schema::array_ensure(Schema::as_unknown(Schema::tuple2(
        Schema::number(),
        Schema::number(),
    )));

    let encoded = schema
        .try_encode(&array(vec![array(vec![number(1.0), number(2.0)])]))
        .expect("a one-element array unwraps to the tuple");
    assert_eq!(encoded, array(vec![number(1.0), number(2.0)]));
    assert_eq!(
        schema.decode(encoded).unwrap(),
        array(vec![array(vec![number(1.0), number(2.0)])])
    );
}

/// Schema.test.ts:1239 — "ArrayEnsure preserves outer-array encoding". A
/// multi-element array encodes element-wise even when the element schema has
/// its own wire form.
#[test]
fn many_values_encode_element_wise() {
    let schema = Schema::array_ensure(Schema::from_json_string(Schema::unknown()));

    assert_eq!(
        schema
            .try_encode(&array(vec![number(1.0), number(2.0)]))
            .unwrap(),
        array(vec![text("1"), text("2")])
    );
}
