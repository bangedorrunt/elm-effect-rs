//! Wave corpus: literal / union / boundary / optional-field shapes.
//! Ports packages/effect/test/schema/Schema.test.ts rows by exact behavior.

use elm_effect_schema::{Schema, Unknown};

#[test]
fn literal_exposes_its_value() {
    let schema = Schema::literal(Unknown::Text("a".to_string()));
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
}

#[test]
fn literal_rejects_other_values() {
    let schema = Schema::literal(Unknown::Number(1.0));
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
    assert!(schema.decode(Unknown::Text("1".to_string())).is_err());
}

#[test]
fn union_tries_first_then_second() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::literal(Unknown::Text("b".to_string())),
    );
    assert_eq!(
        schema.decode(Unknown::Text("b".to_string())).unwrap(),
        Unknown::Text("b".to_string())
    );
    assert!(schema.decode(Unknown::Text("c".to_string())).is_err());
}

#[test]
fn struct_shape_validates_fields() {
    use elm_effect_schema::Field;
    let schema = Schema::struct_shape(vec![
        Field::required("name", Schema::as_unknown(Schema::string())),
        Field::optional("nickname", Schema::as_unknown(Schema::string())),
    ]);
    let mut input = std::collections::BTreeMap::new();
    input.insert("name".to_string(), Unknown::Text("Al".to_string()));
    let out = schema.decode(Unknown::Object(input)).unwrap();
    assert_eq!(out["name"], Unknown::Text("Al".to_string()));
    assert!(!out.contains_key("nickname"));
    let error = schema
        .decode(Unknown::Object(std::collections::BTreeMap::new()))
        .unwrap_err();
    assert!(error.to_string().contains("$.name"));
}

#[test]
fn pick_decodes_a_subset() {
    use elm_effect_schema::Field;
    let fields = vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(Schema::number())),
    ];
    let schema = Schema::pick(fields, &["b"]);
    let mut input = std::collections::BTreeMap::new();
    input.insert("b".to_string(), Unknown::Number(1.0));
    let out = schema.decode(Unknown::Object(input)).unwrap();
    assert_eq!(out["b"], Unknown::Number(1.0));
    // The picked-away field is not required anymore.
    assert!(!out.contains_key("a"));
}

#[test]
fn tuple_checks_length_and_order() {
    let schema = Schema::tuple2(Schema::string(), Schema::number());
    assert_eq!(
        schema
            .decode(Unknown::Array(vec![
                Unknown::Text("a".to_string()),
                Unknown::Number(1.0)
            ]))
            .unwrap(),
        ("a".to_string(), 1.0)
    );
    let error = schema
        .decode(Unknown::Array(vec![Unknown::Number(1.0)]))
        .unwrap_err();
    assert!(error.to_string().contains("$[0]"));
}

#[test]
fn refine_rejects_failing_values() {
    let schema = Schema::number().refine("positive", |n: &f64| *n > 0.0);
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), 1.0);
    let error = schema.decode(Unknown::Number(-1.0)).unwrap_err();
    assert!(error.to_string().contains("positive"));
}

#[test]
fn refine_is_int() {
    let schema = Schema::number().refine("an integer", |n: &f64| n.fract() == 0.0);
    assert_eq!(schema.decode(Unknown::Number(3.0)).unwrap(), 3.0);
    assert!(schema.decode(Unknown::Number(3.5)).is_err());
}

#[test]
fn refine_is_between() {
    let schema = Schema::number().refine("between 1 and 5", |n: &f64| (1.0..=5.0).contains(n));
    assert_eq!(schema.decode(Unknown::Number(4.0)).unwrap(), 4.0);
    assert!(schema.decode(Unknown::Number(6.0)).is_err());
}

#[test]
fn refine_is_min_length() {
    let schema = Schema::string().refine("min length 2", |s: &String| s.len() >= 2);
    assert!(schema.decode(Unknown::Text("ab".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("a".to_string())).is_err());
}

#[test]
fn null_accepts_only_null() {
    assert_eq!(Schema::null().decode(Unknown::Null).unwrap(), ());
    assert!(Schema::null().decode(Unknown::Number(0.0)).is_err());
}

#[test]
fn any_accepts_everything() {
    assert_eq!(
        Schema::any().decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert_eq!(Schema::any().decode(Unknown::Null).unwrap(), Unknown::Null);
}

#[test]
fn never_rejects_everything() {
    assert!(Schema::never().decode(Unknown::Null).is_err());
}

#[test]
fn option_from_null_or_value() {
    let schema = Schema::optional(Schema::string());
    assert_eq!(schema.decode(Unknown::Null).unwrap(), None);
    assert_eq!(
        schema.decode(Unknown::Text("x".to_string())).unwrap(),
        Some("x".to_string())
    );
}

#[test]
fn optional_field_missing_is_none() {
    let schema = Schema::optional_field("nickname", Schema::string());
    assert_eq!(
        schema
            .decode(Unknown::Object(std::collections::BTreeMap::new()))
            .unwrap(),
        None
    );
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("nickname".to_string(), Unknown::Text("Al".to_string()));
    assert_eq!(
        schema.decode(Unknown::Object(fields)).unwrap(),
        Some("Al".to_string())
    );
}
