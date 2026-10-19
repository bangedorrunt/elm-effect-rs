//! Defaults composed through a field transform: the pin writes these rows as
//! `decodeTo(target, { decode: withDefault(...) })`, and the port's spelling of
//! "an absent key means this value" is a field decoding default, so the two
//! rows land on that mechanism.
//!
//!     cargo test -p elm-effect-schema --test schema_nested_defaults
//!
//! Schema.test.ts:2453 is the nesting case: the default on the outer field
//! produces `{}`, which the inner struct's own field default then fills, so a
//! value the wire omits entirely still arrives with the innermost default.
//! Schema.test.ts:2533 is the single-level case on a field the source schema
//! required.

use elm_effect_schema::{Field, Schema, Unknown};
use std::collections::BTreeMap;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

/// A struct decodes to a key map, so the field comparison is on that shape.
fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// Schema.test.ts:2453 "nested defaults": `a` defaults to `{}` when absent, and
/// `b` inside it defaults to `"default-b"`, so both an absent `a` and a `{}`
/// `a` decode to the same nested value.
#[test]
fn a_default_inside_a_default_fills_both_levels() {
    let inner = Schema::as_unknown(Schema::struct_shape(vec![Field::decoding_default(
        "b",
        string(),
        || Ok(text("default-b")),
    )]));
    let schema = Schema::struct_shape(vec![Field::decoding_default("a", inner, || {
        Ok(obj(Vec::new()))
    })]);

    assert_eq!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", text("b"))]))]))
            .unwrap(),
        map(vec![("a", obj(vec![("b", text("b"))]))]),
        "a present value is left alone"
    );
    assert_eq!(
        schema.decode(obj(vec![("a", obj(Vec::new()))])).unwrap(),
        map(vec![("a", obj(vec![("b", text("default-b"))]))]),
        "an empty inner object still gets the inner default"
    );
    assert_eq!(
        schema.decode(obj(Vec::new())).unwrap(),
        map(vec![("a", obj(vec![("b", text("default-b"))]))]),
        "and an absent outer key gets both"
    );
}

/// Schema.test.ts:2533 "required to optionalKey": a default on a field the
/// source schema required makes decoding accept the missing key, while a
/// present value still travels and encoding passes it through.
#[test]
fn a_required_field_can_default_itself_on_decode() {
    let schema = Schema::struct_shape(vec![Field::decoding_default("a", string(), || {
        Ok(text("default"))
    })]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("a"))])).unwrap(),
        map(vec![("a", text("a"))])
    );
    assert_eq!(
        schema.decode(obj(Vec::new())).unwrap(),
        map(vec![("a", text("default"))])
    );
    assert_eq!(
        schema.try_encode(&map(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))]),
        "the default is a decoding affair, so the value passes through"
    );
}
