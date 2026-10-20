//! `Schema.test.ts` "extendTo" rows: a struct extended with fields derived from
//! its own decoded value, dropped again on the way out.
//!
//! Row `:9401` ("Union") is not carried here and stays open. A union member is
//! erased to its wire form in this port (`as_unknown` re-encodes), so an
//! extended member's derived field is not observable through `Union([...])`:
//! the row's decode assertion has nothing to see. Its encode half does hold,
//! but half a row is not a row.
//!
//!     cargo test -p elm-effect-schema --test schema_extend_to

use std::collections::BTreeMap;
use std::sync::Arc;

use elm_effect_schema::{Field, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn base(fields: Vec<Field>) -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(fields)
}

/// `Schema.test.ts:9383` — "Struct": the derived field is filled from the
/// value the base decoded, and neither direction carries it on the wire.
#[test]
fn extend_to_fills_a_derived_field_and_drops_it_on_encode() {
    let schema = Schema::extend_to(
        base(vec![
            Field::required("a", string()),
            Field::required("b", number()),
        ]),
        vec![Field::required("c", string())],
        vec![(
            "c".to_string(),
            Arc::new(|value: &BTreeMap<String, Unknown>| {
                let a = match value.get("a")? {
                    Unknown::Text(text) => text.clone(),
                    _ => return None,
                };
                let b = match value.get("b")? {
                    Unknown::Number(number) => *number,
                    _ => return None,
                };
                Some(Unknown::Text(format!("{a}c{b}")))
            }) as elm_effect_schema::DeriveFn,
        )],
    );

    assert_eq!(
        schema
            .decode(obj(vec![("a", text("1")), ("b", Unknown::Number(2.0))]))
            .expect("decodes"),
        map(vec![
            ("a", text("1")),
            ("b", Unknown::Number(2.0)),
            ("c", text("1c2")),
        ])
    );

    assert_eq!(
        schema
            .try_encode(&map(vec![
                ("a", text("1")),
                ("b", Unknown::Number(2.0)),
                ("c", text("1c2")),
            ]))
            .expect("encodes"),
        obj(vec![("a", text("1")), ("b", Unknown::Number(2.0))]),
        "the derived key does not travel back out"
    );

    // A getter that answers `None` leaves the key absent rather than null.
    let optional = Schema::extend_to(
        base(vec![Field::required("a", string())]),
        vec![Field::required("note", string())],
        vec![(
            "note".to_string(),
            Arc::new(|_value: &BTreeMap<String, Unknown>| None) as elm_effect_schema::DeriveFn,
        )],
    );
    assert_eq!(
        optional
            .decode(obj(vec![("a", text("1"))]))
            .expect("decodes"),
        map(vec![("a", text("1"))])
    );
}
