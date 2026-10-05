//! Codec corpus: `toCodec` round-trips plus annotation document rows.
//!
//! Every test decodes an [`Unknown`] wire value and encodes back; the
//! codec is the two-way shape, so idempotence is `decode(encode(x)) == x`
//! and `encode(decode(u)) == u`.

use elm_effect_schema::{Field, Schema, Unknown};
use std::collections::BTreeMap;

fn name_schema() -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![Field::required(
        "name",
        Schema::as_unknown(Schema::string()),
    )])
}

fn wire_name(value: &str) -> Unknown {
    let mut fields = BTreeMap::new();
    fields.insert("name".to_string(), Unknown::Text(value.to_string()));
    Unknown::Object(fields)
}

#[test]
fn codec_struct_is_idempotent() {
    let schema = name_schema();
    let wire = wire_name("ada");
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.encode(&decoded), wire);
    assert_eq!(schema.decode(schema.encode(&decoded)).unwrap(), decoded);
}

#[test]
fn codec_record_round_trips_and_keeps_empty() {
    let schema = Schema::record(Schema::as_unknown(Schema::string()));
    let mut fields = BTreeMap::new();
    fields.insert("a".to_string(), Unknown::Text("1".to_string()));
    let wire = Unknown::Object(fields);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.encode(&decoded), wire);
    let empty = schema.decode(Unknown::Object(BTreeMap::new())).unwrap();
    assert_eq!(schema.encode(&empty), Unknown::Object(BTreeMap::new()));
}

#[test]
fn codec_tuple_round_trips_in_order() {
    let schema = Schema::tuple2(Schema::string(), Schema::number());
    let wire = Unknown::Array(vec![Unknown::Text("x".to_string()), Unknown::Number(2.0)]);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(decoded, ("x".to_string(), 2.0));
    assert_eq!(schema.encode(&decoded), wire);
}

#[test]
fn codec_optional_field_absent_and_null() {
    let schema = Schema::struct_shape(vec![
        Field::required("name", Schema::as_unknown(Schema::string())),
        Field::optional("nick", Schema::as_unknown(Schema::string())),
    ]);
    let mut bare = BTreeMap::new();
    bare.insert("name".to_string(), Unknown::Text("ada".to_string()));
    let decoded = schema.decode(Unknown::Object(bare)).unwrap();
    assert!(!decoded.contains_key("nick"));
    let mut nulled = BTreeMap::new();
    nulled.insert("name".to_string(), Unknown::Text("ada".to_string()));
    nulled.insert("nick".to_string(), Unknown::Null);
    let decoded = schema.decode(Unknown::Object(nulled)).unwrap();
    assert!(!decoded.contains_key("nick"));
    let missing = schema.decode(Unknown::Object(BTreeMap::new()));
    let error = missing.unwrap_err().to_string();
    assert!(error.contains("name"), "unexpected error: {error}");
}

#[test]
fn codec_union_falls_back_to_second_member() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::number()),
    );
    assert_eq!(
        schema.decode(Unknown::Text("s".to_string())).unwrap(),
        Unknown::Text("s".to_string())
    );
    assert_eq!(
        schema.decode(Unknown::Number(3.0)).unwrap(),
        Unknown::Number(3.0)
    );
    assert_eq!(
        schema.encode(&Unknown::Text("s".to_string())),
        Unknown::Text("s".to_string())
    );
}

#[test]
fn codec_json_text_round_trips_a_struct() {
    let schema = name_schema();
    let decoded = schema.decode_json(r#"{"name": "ada"}"#).unwrap();
    assert_eq!(schema.encode_json(&decoded).unwrap(), r#"{"name":"ada"}"#);
}

#[test]
fn doc_annotated_carries_title_and_description() {
    let schema = Schema::annotated(Schema::string(), Some("Name"), Some("A person's name"));
    let doc = schema.json_schema();
    assert_eq!(doc["type"], "string");
    assert_eq!(doc["title"], "Name");
    assert_eq!(doc["description"], "A person's name");
    // The wire shape is unchanged by annotations.
    let decoded = schema.decode(Unknown::Text("ada".to_string())).unwrap();
    assert_eq!(decoded, "ada".to_string());
}

#[test]
fn doc_union_marks_itself_opaque() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::number()),
    );
    assert_eq!(schema.json_schema()["$comment"], "union");
}
