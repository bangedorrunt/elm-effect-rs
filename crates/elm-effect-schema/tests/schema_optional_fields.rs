//! `Schema.test.ts` "Optional Fields" rows that turn only on values this port
//! has: an absent key and a null one.
//!
//! Two rows in that block (`:8111`, `:8145`) also assert that a present
//! `undefined` key is treated as absent, and `Unknown` has no undefined, so
//! they stay open. These two do not need it.
//!
//! Spellings: a struct field is a `Schema<Unknown>`, and a re-typed schema's
//! Unknown value is its wire form, so a `FiniteFromString` field reads back as
//! `Text("1")` where the pin shows `1`. The assertions below use this port's
//! spelling, and a value the transform refuses is asserted too, so the transform
//! is proven to run rather than skipped.
//!
//!     cargo test -p elm-effect-schema --test schema_optional_fields

use std::collections::BTreeMap;

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

fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

fn struct_of(fields: Vec<Field>) -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(fields)
}

/// `Schema.test.ts:8096` — "Exact Optional Property": the key may be absent,
/// decodes to its own type when present, and an absent key stays absent.
#[test]
fn an_exact_optional_key_is_absent_or_decoded() {
    let schema = struct_of(vec![Field::optional_key("a", finite())]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).expect("decodes"),
        map(vec![("a", text("1"))])
    );
    assert!(
        schema.decode(obj(vec![("a", text("x"))])).is_err(),
        "the field's own transform still runs"
    );
    assert_eq!(
        schema.decode(obj(vec![])).expect("decodes"),
        map(vec![]),
        "an absent key stays absent rather than filling a value"
    );

    assert_eq!(
        schema
            .try_encode(&map(vec![("a", text("1"))]))
            .expect("encodes"),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(
        schema.try_encode(&map(vec![])).expect("encodes"),
        obj(vec![])
    );
}

/// `Schema.test.ts:8128` — "Exact Optional Property with Nullability": the key
/// may be absent, and when present it is either null or the inner value.
#[test]
fn an_optional_key_admits_null_as_a_value() {
    let schema = struct_of(vec![Field::optional_key("a", Schema::null_or(finite()))]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).expect("decodes"),
        map(vec![("a", text("1"))])
    );
    assert!(
        schema.decode(obj(vec![("a", text("x"))])).is_err(),
        "the field's own transform still runs"
    );
    assert_eq!(schema.decode(obj(vec![])).expect("decodes"), map(vec![]));
    assert_eq!(
        schema
            .decode(obj(vec![("a", Unknown::Null)]))
            .expect("decodes"),
        map(vec![("a", Unknown::Null)]),
        "a null is a value the key carries, not an absence"
    );

    assert_eq!(
        schema
            .try_encode(&map(vec![("a", text("1"))]))
            .expect("encodes"),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(
        schema.try_encode(&map(vec![])).expect("encodes"),
        obj(vec![])
    );
    assert_eq!(
        schema
            .try_encode(&map(vec![("a", Unknown::Null)]))
            .expect("encodes"),
        obj(vec![("a", Unknown::Null)])
    );
}
