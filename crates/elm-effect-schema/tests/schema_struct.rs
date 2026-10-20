//! `Schema.test.ts` "Struct" rows: a required field, a transformation field,
//! `optionalKey`, `optional`, and an `optionalKey` over a transformation.
//!
//!     cargo test -p elm-effect-schema --test schema_struct
//!
//! The pin asserts both axes. Here a decoded value is already its wire form
//! (`Unknown` carries no separate type side), so the encode half of each row
//! can only assert the key-presence rules: a wrong field type is unrepresentable
//! once the value exists, and each test says so where the pin's encode refusal
//! would sit.

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
            .collect::<BTreeMap<_, _>>(),
    )
}

fn empty() -> Unknown {
    obj(Vec::new())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

/// `Schema.test.ts:750` — `{ readonly "a": string }`.
#[test]
fn a_required_string_field_is_read_and_required() {
    let schema = Schema::struct_of(vec![("a".to_string(), string())]);

    // The field list is exposed, which is the pin's `schema.fields`.
    let value = schema
        .decode(obj(vec![("a", text("a"))]))
        .expect("decodes");
    assert_eq!(value.field_names(), vec!["a"]);

    let schema = Schema::as_unknown(schema);
    assert_eq!(
        schema.decode(obj(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))])
    );
    assert_eq!(
        schema.decode(empty()).unwrap_err().to_string(),
        "expected a declared field at $.a, got absent"
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", Unknown::Number(1.0))]))
            .unwrap_err()
            .to_string(),
        "expected string at $.a, got number"
    );

    // Encoding: a missing required key is refused by the shape's own encoder.
    // A wrong field type cannot reach it, since the value is already the wire
    // form.
    let shape = Schema::struct_shape(vec![Field::required("a", string())]);
    assert_eq!(
        shape
            .try_encode(&BTreeMap::new())
            .unwrap_err()
            .to_string(),
        "expected required field at $.a, got missing"
    );
}

/// `Schema.test.ts:790` — `{ readonly "a": <transformation> }`.
#[test]
fn a_transformation_field_reads_its_wire_spelling() {
    let schema = Schema::as_unknown(Schema::struct_of(vec![(
        "a".to_string(),
        finite(),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", text("a"))]))
            .unwrap_err()
            .to_string(),
        "expected a finite number string at $.a, got string"
    );
    // The encode direction of this row's transformation is carried by
    // `schema_checked_transform.rs`; here the field's presence is what the
    // shape adds.
    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}

/// `Schema.test.ts:823` — `Schema.optionalKey: { readonly "a"?: string }`.
#[test]
fn an_optional_key_field_may_be_absent_but_not_null() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional_key(
        "a",
        string(),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());
    assert_eq!(
        schema
            .decode(obj(vec![("a", Unknown::Number(1.0))]))
            .unwrap_err()
            .to_string(),
        "expected string at $.a, got number"
    );

    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))])
    );
    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
}

/// `Schema.test.ts:852` — `Schema.optional: { readonly "a"?: string | undefined }`.
#[test]
fn an_optional_field_may_be_absent_or_null() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional(
        "a",
        string(),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        empty(),
        "a null is the absence marker this spelling absorbs"
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", Unknown::Number(1.0))]))
            .unwrap_err()
            .to_string(),
        "expected string at $.a, got number"
    );

    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))])
    );
    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
}

/// `Schema.test.ts:884` — `{ readonly "a"?: <transformation> }`: an absent key
/// is allowed, but a present one still goes through the transformation, so a
/// null is refused by the field rather than absorbed.
#[test]
fn an_optional_transformation_field_still_decodes_a_present_key() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional_key(
        "a",
        finite(),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());
    assert_eq!(
        schema
            .decode(obj(vec![("a", Unknown::Null)]))
            .unwrap_err()
            .to_string(),
        "expected a finite number string at $.a, got null"
    );

    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}
