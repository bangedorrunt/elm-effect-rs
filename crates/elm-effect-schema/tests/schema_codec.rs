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

#[test]
fn codec_scalars_round_trip() {
    let text = Schema::string()
        .decode(Unknown::Text("s".to_string()))
        .unwrap();
    assert_eq!(
        Schema::string().encode(&text),
        Unknown::Text("s".to_string())
    );
    let number = Schema::number().decode(Unknown::Number(1.5)).unwrap();
    assert_eq!(Schema::number().encode(&number), Unknown::Number(1.5));
    let flag = Schema::boolean().decode(Unknown::Bool(true)).unwrap();
    assert_eq!(Schema::boolean().encode(&flag), Unknown::Bool(true));
}

#[test]
fn codec_null_round_trips_and_never_rejects() {
    Schema::null().decode(Unknown::Null).unwrap();
    assert_eq!(Schema::null().encode(&()), Unknown::Null);
    assert!(Schema::never().decode(Unknown::Null).is_err());
    assert!(
        Schema::never()
            .decode(Unknown::Text("x".to_string()))
            .is_err()
    );
}

#[test]
fn codec_any_accepts_everything() {
    for wire in [
        Unknown::Null,
        Unknown::Bool(false),
        Unknown::Number(0.0),
        Unknown::Text("u".to_string()),
    ] {
        let decoded = Schema::any().decode(wire.clone()).unwrap();
        assert_eq!(Schema::any().encode(&decoded), wire);
    }
}

#[test]
fn codec_literal_accepts_only_its_value() {
    let schema = Schema::literal(Unknown::Text("a".to_string()));
    let decoded = schema.decode(Unknown::Text("a".to_string())).unwrap();
    assert_eq!(schema.encode(&decoded), Unknown::Text("a".to_string()));
    assert!(schema.decode(Unknown::Text("b".to_string())).is_err());
}

#[test]
fn codec_array_of_literals_covers_enum() {
    let schema = Schema::array(Schema::literal(Unknown::Text("a".to_string())));
    let wire = Unknown::Array(vec![Unknown::Text("a".to_string())]);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.encode(&decoded), wire);
    let bad = Unknown::Array(vec![Unknown::Text("z".to_string())]);
    assert!(schema.decode(bad).is_err());
}
