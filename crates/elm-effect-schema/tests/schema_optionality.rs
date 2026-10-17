//! Slice 1: three-axis optionality (required / exact-optional / optional).
//! Ports packages/effect/test/schema/Schema.test.ts optionality rows.

use elm_effect_schema::{Field, Schema, Unknown};
use std::collections::BTreeMap;

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn str_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

#[test]
fn optional_key_absent_is_exact() {
    let schema = Schema::struct_shape(vec![Field::optional_key("a", str_schema())]);
    let out = schema.decode(obj(vec![])).unwrap();
    assert!(!out.contains_key("a"));
}

#[test]
fn optional_key_explicit_null_decodes_inner() {
    let schema = Schema::struct_shape(vec![Field::optional_key("a", str_schema())]);
    assert!(schema.decode(obj(vec![("a", Unknown::Null)])).is_err());
    let out = schema
        .decode(obj(vec![("a", Unknown::Text("x".to_string()))]))
        .unwrap();
    assert_eq!(out.get("a"), Some(&Unknown::Text("x".to_string())));
}

#[test]
fn optional_absent_or_null_skips_inner() {
    let schema = Schema::struct_shape(vec![Field::optional("a", str_schema())]);
    assert!(schema.decode(obj(vec![])).is_ok());
    assert!(schema.decode(obj(vec![("a", Unknown::Null)])).is_ok());
}

#[test]
fn require_reverses_optional_wrapping() {
    for field in [
        Field::optional("a", str_schema()).require(),
        Field::optional_key("a", str_schema()).require(),
    ] {
        let schema = Schema::struct_shape(vec![field]);
        assert!(schema.decode(obj(vec![])).is_err());
        assert!(
            schema
                .decode(obj(vec![("a", Unknown::Text("x".to_string()))]))
                .is_ok()
        );
    }
}

/// Schema.test.ts:2326 (`decodeTo`, "required to required"): a field declared
/// required on the source side is required on both axes, and the pin asserts
/// the encode one explicitly — `await encoding.fail({}, Missing key at ["a"])`.
/// A value that lacks the key is not something this schema describes, so it is
/// refused rather than passed through.
#[test]
fn a_required_field_refuses_a_missing_key_on_both_axes() {
    let schema = Schema::struct_shape(vec![Field::required("a", str_schema())]);

    let decode_error = schema
        .decode(obj(vec![]))
        .expect_err("a required key that is absent");
    assert_eq!(
        decode_error.to_string(),
        "expected required field at $.a, got missing"
    );

    let absent: BTreeMap<String, Unknown> = BTreeMap::new();
    let encode_error = schema
        .try_encode(&absent)
        .expect_err("a required key that is absent on the encode side too");
    assert_eq!(
        encode_error.to_string(),
        decode_error.to_string(),
        "the encode side refuses it the same way the decode side does"
    );

    let mut present: BTreeMap<String, Unknown> = BTreeMap::new();
    present.insert("a".to_string(), Unknown::Text("a".to_string()));
    assert_eq!(
        schema.try_encode(&present),
        Ok(Unknown::Object(present.clone()))
    );
}

/// Schema.test.ts:2381 (`decodeTo`, "optionalKey to required"): the declared
/// field is `optionalKey`, and a decode-side default is what fills the absent
/// key, so `{}` decodes to `{ a: "default" }` while a present key passes
/// through on both axes untouched.
#[test]
fn an_absent_optional_key_decodes_to_its_default() {
    let schema = Schema::struct_shape(vec![Field::decoding_default("a", str_schema(), || {
        Ok(Unknown::Text("default".to_string()))
    })]);
    let default = Unknown::Text("default".to_string());

    let filled = schema.decode(obj(vec![])).expect("the absent key fills");
    assert_eq!(filled.get("a"), Some(&default));

    let present = Unknown::Text("a".to_string());
    let decoded = schema
        .decode(obj(vec![("a", present.clone())]))
        .expect("a present key decodes through");
    assert_eq!(decoded.get("a"), Some(&present));

    let mut value: BTreeMap<String, Unknown> = BTreeMap::new();
    value.insert("a".to_string(), present);
    assert_eq!(
        schema.try_encode(&value),
        Ok(Unknown::Object(value.clone()))
    );
}

#[test]
fn require_all_makes_optional_keys_required() {
    let schema = Schema::struct_shape(Field::require_all(vec![
        Field::optional("a", str_schema()),
        Field::optional_key("b", str_schema()),
    ]));
    assert!(schema.decode(obj(vec![])).is_err());
    assert!(
        schema
            .decode(obj(vec![
                ("a", Unknown::Text("x".to_string())),
                ("b", Unknown::Text("y".to_string())),
            ]))
            .is_ok()
    );
}

// Record rows: every value decodes with the value schema, keys pass through.
#[test]
fn record_string_number_decodes_every_value() {
    let schema = Schema::record(Schema::as_unknown(Schema::number()));
    let decoded = schema
        .decode(Unknown::Object(
            [
                ("a".to_string(), Unknown::Number(1.0)),
                ("b".to_string(), Unknown::Number(2.0)),
            ]
            .into_iter()
            .collect(),
        ))
        .expect("record decodes");
    assert_eq!(decoded["a"], Unknown::Number(1.0));
    assert_eq!(decoded["b"], Unknown::Number(2.0));
    assert_eq!(
        schema.try_encode(&decoded).unwrap(),
        Unknown::Object(
            [
                ("a".to_string(), Unknown::Number(1.0)),
                ("b".to_string(), Unknown::Number(2.0)),
            ]
            .into_iter()
            .collect()
        ),
        "record round-trips keys and values"
    );
}

#[test]
fn record_rejects_non_object_and_names_the_key() {
    let schema = Schema::record(Schema::as_unknown(Schema::number()));
    assert!(schema.decode(Unknown::Null).is_err(), "not an object");
    let error = schema
        .decode(Unknown::Object(
            [("bad".to_string(), Unknown::Text("x".to_string()))]
                .into_iter()
                .collect(),
        ))
        .unwrap_err();
    assert!(
        error.to_string().contains("bad"),
        "error names the offending key: {error}"
    );
}

#[test]
fn record_empty_object_decodes() {
    let schema = Schema::record(Schema::as_unknown(Schema::number()));
    let decoded = schema
        .decode(Unknown::Object(BTreeMap::new()))
        .expect("empty record decodes");
    assert!(decoded.is_empty());
    assert_eq!(schema.try_encode(&decoded).unwrap(), Unknown::Object(BTreeMap::new()));
}

#[test]
fn record_optional_values_accept_null_but_reject_wrong_type() {
    // Record(String, optional(Number)): a null value decodes to None,
    // any present non-number still fails.
    let schema = Schema::record(Schema::as_unknown(Schema::optional(Schema::number())));
    let decoded = schema
        .decode(Unknown::Object(
            [
                ("here".to_string(), Unknown::Number(1.0)),
                ("gone".to_string(), Unknown::Null),
            ]
            .into_iter()
            .collect(),
        ))
        .expect("optional values decode");
    assert_eq!(decoded["here"], Unknown::Number(1.0));
    assert_eq!(decoded["gone"], Unknown::Null);
    assert!(
        schema
            .decode(Unknown::Object(
                [("bad".to_string(), Unknown::Text("x".to_string()))]
                    .into_iter()
                    .collect()
            ))
            .is_err()
    );
}
