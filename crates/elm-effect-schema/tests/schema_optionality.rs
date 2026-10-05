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
    assert_eq!(
        out.get("a"),
        Some(&Unknown::Text("x".to_string()))
    );
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
        assert!(schema
            .decode(obj(vec![("a", Unknown::Text("x".to_string()))]))
            .is_ok());
    }
}

#[test]
fn require_all_makes_optional_keys_required() {
    let schema = Schema::struct_shape(Field::require_all(vec![
        Field::optional("a", str_schema()),
        Field::optional_key("b", str_schema()),
    ]));
    assert!(schema.decode(obj(vec![])).is_err());
    assert!(schema
        .decode(obj(vec![
            ("a", Unknown::Text("x".to_string())),
            ("b", Unknown::Text("y".to_string())),
        ]))
        .is_ok());
}
