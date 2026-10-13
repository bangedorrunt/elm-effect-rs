//! Codec corpus: `toCodec` round-trips plus annotation document rows.
//!
//! Every test decodes an [`Unknown`] wire value and encodes back; the
//! codec is the two-way shape, so idempotence is `decode(encode(x)) == x`
//! and `encode(decode(u)) == u`.

use elm_effect_schema::{Field, Schema, Unknown};
use std::collections::BTreeMap;

fn text(s: &str) -> Unknown {
    Unknown::Text(s.to_string())
}

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

/// Schema.test.ts:287 `Literal.transform`: a one-pair map, so decoding the
/// source literal yields the target and encoding does the reverse. The pin's
/// expected text names the literal; this port's refusal carries the same list.
#[test]
fn literal_transform_maps_one_pair_both_ways() {
    let schema = Schema::literal_transform(vec![(Unknown::Number(0.0), text("a"))]);

    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), text("a"));
    assert!(schema.decode(Unknown::Number(1.0)).is_err());

    assert_eq!(schema.try_encode(&text("a")).unwrap(), Unknown::Number(0.0));
    let refused = schema.try_encode(&text("b")).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "cannot encode at $: expected one of \"a\""
    );
}

/// Schema.test.ts:315 `Literals.transform`: one pair per member, so each source
/// literal maps to its own target and every other value is refused in both
/// directions.
#[test]
fn literal_transform_maps_member_pairs_both_ways() {
    let schema = Schema::literal_transform(vec![
        (Unknown::Number(0.0), text("a")),
        (Unknown::Number(1.0), text("b")),
    ]);

    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), text("a"));
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), text("b"));
    assert!(schema.decode(Unknown::Number(2.0)).is_err());

    assert_eq!(schema.try_encode(&text("a")).unwrap(), Unknown::Number(0.0));
    assert_eq!(schema.try_encode(&text("b")).unwrap(), Unknown::Number(1.0));
    let refused = schema.try_encode(&text("c")).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "cannot encode at $: expected one of \"a\" | \"b\""
    );
}

#[test]
fn codec_union_of_literals_covers_enum() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::literal(Unknown::Text("b".to_string())),
    );
    for member in ["a", "b"] {
        let wire = Unknown::Text(member.to_string());
        let decoded = schema.decode(wire.clone()).unwrap();
        assert_eq!(schema.encode(&decoded), wire);
    }
    assert!(schema.decode(Unknown::Text("z".to_string())).is_err());
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

#[test]
fn codec_optional_date_json_shapes() {
    use elm_effect_schema::transform::date_schema;
    const ISO: &str = "2021-01-01T00:00:00.000Z";
    const MILLIS: i64 = 1_609_459_200_000;
    // Bare NullOr/UndefinedOr/optional(Date).
    let nullable = Schema::optional(date_schema());
    assert_eq!(
        nullable.decode(Unknown::Text(ISO.to_string())).unwrap(),
        Some(MILLIS)
    );
    assert_eq!(nullable.decode(Unknown::Null).unwrap(), None);
    assert!(nullable.decode(Unknown::Number(1.0)).is_err());
    assert_eq!(
        nullable.encode(&Some(MILLIS)),
        Unknown::Text(ISO.to_string())
    );
    assert_eq!(nullable.encode(&None), Unknown::Null);
    // Struct NullOr/UndefinedOr/optional(Date): present, null, absent.
    let strukt = Schema::struct_shape(vec![Field::optional(
        "a",
        Schema::as_unknown(Schema::optional(date_schema())),
    )]);
    let mut present = BTreeMap::new();
    present.insert("a".to_string(), Unknown::Text(ISO.to_string()));
    assert_eq!(
        strukt.decode(Unknown::Object(present.clone())).unwrap()["a"],
        Unknown::Text(ISO.to_string())
    );
    assert_eq!(strukt.encode(&present), Unknown::Object(present));
    let mut nullish = BTreeMap::new();
    nullish.insert("a".to_string(), Unknown::Null);
    // Explicit null normalizes to absent (TS decodes it to undefined).
    assert!(!strukt
        .decode(Unknown::Object(nullish))
        .unwrap()
        .contains_key("a"));
    assert!(!strukt
        .decode(Unknown::Object(BTreeMap::new()))
        .unwrap()
        .contains_key("a"));
    // Struct optionalKey(Date): absent ok, explicit null decodes inner.
    let keyed = Schema::struct_shape(vec![Field::optional_key(
        "a",
        Schema::as_unknown(Schema::optional(date_schema())),
    )]);
    assert!(!keyed
        .decode(Unknown::Object(BTreeMap::new()))
        .unwrap()
        .contains_key("a"));
    let mut null_wire = BTreeMap::new();
    null_wire.insert("a".to_string(), Unknown::Null);
    assert_eq!(
        keyed.decode(Unknown::Object(null_wire)).unwrap()["a"],
        Unknown::Null
    );
}

#[test]
fn codec_unknown_round_trips_any_json() {
    let schema = Schema::unknown();
    for value in [
        Unknown::Text("a".to_string()),
        Unknown::Number(1.0),
        Unknown::Bool(true),
        Unknown::Null,
        Unknown::Object([("a".to_string(), Unknown::Text("a".to_string()))].into()),
        Unknown::Array(vec![Unknown::Text("a".to_string()), Unknown::Number(1.0)]),
    ] {
        let encoded = schema.encode(&value);
        assert_eq!(encoded, value);
        assert_eq!(schema.decode(value.clone()).unwrap(), value);
    }
}

#[test]
fn codec_object_keyword_takes_objects_only() {
    let schema = Schema::object();
    let fields: BTreeMap<String, Unknown> =
        [("a".to_string(), Unknown::Text("a".to_string()))].into();
    assert_eq!(
        schema.decode(Unknown::Object(fields.clone())).unwrap(),
        fields
    );
    assert_eq!(schema.encode(&fields), Unknown::Object(fields));
    assert!(schema.decode(Unknown::Array(vec![])).is_err());
    assert!(schema.decode(Unknown::Null).is_err());
}

#[test]
fn codec_null_and_string_are_identity() {
    let null = Schema::<()>::null();
    assert!(null.decode(Unknown::Null).is_ok());
    assert!(null.decode(Unknown::Text("a".to_string())).is_err());

    let string = Schema::string();
    assert_eq!(
        string.decode(Unknown::Text("a".to_string())).unwrap(),
        "a".to_string()
    );
    assert_eq!(
        string.encode(&"a".to_string()),
        Unknown::Text("a".to_string())
    );
    assert!(string.decode(Unknown::Number(1.0)).is_err());
}

#[test]
fn codec_void_and_undefined_map_to_null() {
    // TS Void/Undefined hold no data and travel as JSON null; the Rust
    // wire representation is the same literal.
    let void = Schema::<Unknown>::literal(Unknown::Null);
    assert_eq!(void.encode(&Unknown::Null), Unknown::Null);
    assert_eq!(void.decode(Unknown::Null).unwrap(), Unknown::Null);
    assert!(void.decode(Unknown::Text("a".to_string())).is_err());
}
