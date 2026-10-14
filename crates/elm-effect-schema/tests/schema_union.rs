//! Union dispatch: member order, and what `mode: "oneOf"` counts. Derives from
//! Schema.test.ts:4484 and :4762.

use std::collections::BTreeMap;

use elm_effect_schema::{Field, Schema, Unknown};

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

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// A struct member that decodes to a fixed string, the shape the pin builds with
/// `Struct({...}).pipe(decodeTo(String, { decode: transform(...) }))`.
fn struct_to(text_value: &'static str, fields: Vec<Field>) -> Schema<Unknown> {
    Schema::convert(
        Schema::struct_shape(fields),
        move |_map: BTreeMap<String, Unknown>| Ok(text(text_value)),
        |_value: &Unknown| BTreeMap::new(),
    )
}

#[test]
fn member_order_is_preserved_after_sentinel_dispatch() {
    // :4484 "preserves member order after sentinel dispatch": the fallback member
    // comes first and takes the input, sentinel and all, so the member carrying
    // the `kind` literal does not get to jump the queue.
    let fallback = struct_to("fallback", vec![Field::required("value", string())]);
    let discriminated = struct_to(
        "discriminated",
        vec![
            Field::required("kind", Schema::as_unknown(Schema::literal(text("a")))),
            Field::required("value", string()),
        ],
    );
    let schema = Schema::union_with_mode(vec![fallback, discriminated], "anyOf");
    assert_eq!(
        schema.decode(obj(vec![("kind", text("a")), ("value", text("value"))])),
        Ok(text("fallback"))
    );
}

#[test]
fn a_void_member_makes_every_one_of_union_ambiguous() {
    // :4762 `mode: "oneOf" with Void`: `Void` admits anything, so a union of it
    // and a string has two members matching a string rather than one.
    let schema =
        Schema::union_with_mode(vec![Schema::as_unknown(Schema::void()), string()], "oneOf");
    assert_eq!(
        schema.decode(text("a")).unwrap_err().to_string(),
        "expected exactly one member to match at $, got string"
    );
    // Under `anyOf` the first member is enough, which is the difference the two
    // modes exist for.
    let any_of =
        Schema::union_with_mode(vec![Schema::as_unknown(Schema::void()), string()], "anyOf");
    assert!(any_of.decode(text("a")).is_ok());
}
