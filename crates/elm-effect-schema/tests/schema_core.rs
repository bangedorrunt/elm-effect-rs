//! Schema core fence: decode unknown into typed values, encode back.
//! Derives from packages/effect/test/schema/ rows (core shapes only).

use elm_effect_schema::{Schema, SchemaError, Unknown};

#[test]
fn string_round_trips() {
    let schema = Schema::string();
    let value = schema.decode(Unknown::Text("hi".to_string())).unwrap();
    assert_eq!(value, "hi");
    assert_eq!(schema.encode(&value), Unknown::Text("hi".to_string()));
}

#[test]
fn number_round_trips() {
    let schema = Schema::number();
    let value = schema.decode(Unknown::Number(2.5)).unwrap();
    assert_eq!(value, 2.5);
    assert_eq!(schema.encode(&value), Unknown::Number(2.5));
}

#[test]
fn boolean_round_trips() {
    let schema = Schema::boolean();
    assert!(schema.decode(Unknown::Bool(true)).unwrap());
    assert_eq!(schema.encode(&true), Unknown::Bool(true));
}

#[test]
fn wrong_type_is_a_typed_error() {
    let schema = Schema::string();
    let error = schema.decode(Unknown::Number(1.0)).unwrap_err();
    assert!(matches!(error, SchemaError::TypeMismatch { .. }));
    assert!(error.to_string().contains("string"));
}

#[test]
fn arrays_decode_elementwise() {
    let schema = Schema::array(Schema::number());
    let value = schema
        .decode(Unknown::Array(vec![
            Unknown::Number(1.0),
            Unknown::Number(2.0),
        ]))
        .unwrap();
    assert_eq!(value, vec![1.0, 2.0]);
}

#[test]
fn array_element_errors_carry_index_path() {
    let schema = Schema::array(Schema::number());
    let error = schema
        .decode(Unknown::Array(vec![
            Unknown::Number(1.0),
            Unknown::Text("no".to_string()),
        ]))
        .unwrap_err();
    assert!(error.to_string().contains("[1]"));
}

#[test]
fn optional_accepts_missing_and_null() {
    let schema = Schema::optional(Schema::string());
    assert_eq!(
        schema.decode(Unknown::Null).unwrap(),
        None,
        "null decodes to None"
    );
    assert_eq!(
        schema.decode(Unknown::Text("x".to_string())).unwrap(),
        Some("x".to_string())
    );
    assert_eq!(schema.encode(&None), Unknown::Null);
}

#[test]
fn structs_decode_fieldwise() {
    #[derive(Debug, PartialEq)]
    struct Point {
        x: f64,
        y: f64,
    }
    let schema = Schema::convert(
        Schema::object(),
        |mut fields: std::collections::BTreeMap<String, Unknown>| {
            let x = Schema::number().decode(fields.remove("x").unwrap_or(Unknown::Null))?;
            let y = Schema::number().decode(fields.remove("y").unwrap_or(Unknown::Null))?;
            Ok(Point { x, y })
        },
        |point: &Point| {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("x".to_string(), Unknown::Number(point.x));
            fields.insert("y".to_string(), Unknown::Number(point.y));
            fields
        },
    );
    let mut input = std::collections::BTreeMap::new();
    input.insert("x".to_string(), Unknown::Number(1.0));
    input.insert("y".to_string(), Unknown::Number(2.0));
    let point = schema.decode(Unknown::Object(input)).unwrap();
    assert_eq!(point, Point { x: 1.0, y: 2.0 });
    let Unknown::Object(back) = schema.encode(&point) else {
        panic!("struct encodes to an object");
    };
    assert_eq!(back["x"], Unknown::Number(1.0));
}

#[test]
fn missing_field_names_the_path() {
    let schema = Schema::field("name", Schema::string());
    let error = schema
        .decode(Unknown::Object(std::collections::BTreeMap::new()))
        .unwrap_err();
    assert!(error.to_string().contains("name"));
}

#[test]
fn json_bytes_decode_and_encode() {
    let schema = Schema::array(Schema::string());
    let value = schema.decode_json(r#"["a", "b"]"#).unwrap();
    assert_eq!(value, vec!["a".to_string(), "b".to_string()]);
    let back = schema.encode_json(&value).unwrap();
    assert_eq!(back, r#"["a","b"]"#);
}

#[test]
fn malformed_json_is_a_schema_error() {
    let schema = Schema::string();
    assert!(schema.decode_json(r#"{"oops"#).is_err());
}

#[test]
fn facade_reexports_schema() {
    let schema = elm_effect::Schema::number();
    assert_eq!(
        schema.decode(elm_effect::Unknown::Number(1.0)).unwrap(),
        1.0
    );
}
