//! `Config.schema`: a config decoded through a `Schema` from provider data.
//!
//! Config.test.ts's schema describe reads provider *nodes*: the pin's provider
//! keeps a string tree, so a struct reads declared properties, an array reads
//! indexed children, and a leaf reads its own co-located scalar. The port's
//! provider is flat (`path -> text`), so a node's shape is inferred the same
//! way from what is stored under its path: declared children make an object,
//! numeric children make an array, and text at the path itself is a scalar.
//!
//! `config` and `config_provider` are private modules, so they load by path
//! like the other wave tests. Most of their surface is unexercised here, which
//! is a statement about this test's coverage rather than a defect.
#![allow(dead_code)]
#![cfg(feature = "schema")]

#[path = "../src/config.rs"]
mod config;
#[path = "../src/config_provider.rs"]
mod config_provider;

use config::{Config, Value};
use config_provider::Provider;
use elm_effect_schema::{Field, Schema, SchemaError, StructValue, Unknown, transform::Transformation};
use std::collections::BTreeMap;

fn provider(pairs: &[(&str, &str)]) -> Provider {
    Provider::from_pairs(pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())))
}

/// A provider that keeps an empty string as a present value, which is how the
/// pin stages "the node exists but is not an array".
fn preserving(pairs: &[(&str, &str)]) -> Provider {
    Provider::from_pairs_preserving(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
    )
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect(),
    )
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// A struct of `Schema<Unknown>` fields. `Schema.Struct` reports its field
/// list through `kind()`, which is what the bridge walks.
fn structure(fields: Vec<(&str, Schema<Unknown>)>) -> Schema<StructValue> {
    Schema::struct_of(
        fields
            .into_iter()
            .map(|(name, schema)| (name.to_string(), schema))
            .collect(),
    )
}

/// The same struct erased to `Schema<Unknown>`, for a nested field.
fn shape(fields: Vec<(&str, Schema<Unknown>)>) -> Schema<Unknown> {
    Schema::as_unknown(structure(fields))
}

/// A struct shape with declared fields, which is where field optionality
/// lives. Its wire kind is the plain object kind, so the field list is what the
/// bridge walks.
fn field_shape(fields: Vec<Field>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::struct_shape(fields))
}

fn mismatch(expected: &'static str) -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected,
        got: "the encoded value",
    }
}

/// `Schema.String -> Schema.Struct({ value: String })`, the pin's
/// `scalarToStruct` at Config.test.ts:1732.
fn scalar_to_struct() -> Schema<Unknown> {
    Schema::decode_to(
        Schema::<String>::string(),
        Transformation::new(
            |source: String| {
                Ok(Unknown::Object(BTreeMap::from([(
                    "value".to_string(),
                    Unknown::Text(source),
                )])))
            },
            |value: Unknown| match value {
                Unknown::Object(fields) => match fields.get("value") {
                    Some(Unknown::Text(source)) => Ok(source.clone()),
                    _ => Err(mismatch("value: string")),
                },
                _ => Err(mismatch("an object")),
            },
        ),
    )
}

/// `Schema.Struct({ value: String }) -> Schema.String`, the pin's
/// `structToScalar` at the same line.
fn struct_to_scalar() -> Schema<String> {
    Schema::decode_to(
        Schema::as_unknown(structure(vec![("value", string())])),
        Transformation::new(
            |encoded: Unknown| match encoded {
                Unknown::Object(fields) => match fields.get("value") {
                    Some(Unknown::Text(source)) => Ok(source.clone()),
                    _ => Err(mismatch("value: string")),
                },
                _ => Err(mismatch("an object")),
            },
            |source: String| {
                Ok(Unknown::Object(BTreeMap::from([(
                    "value".to_string(),
                    Unknown::Text(source),
                )])))
            },
        ),
    )
}

#[test]
fn leaf_schema_uses_its_own_scalar_over_object_children() {
    // Config.test.ts:2142 — a Number leaf reads `a`, not `a_b`.
    let schema = structure(vec![("a", number())]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1"), ("a.b", "2")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
}

#[test]
fn leaf_schema_uses_its_own_scalar_over_array_children() {
    // Config.test.ts:2149 — the same rule against indexed children.
    let schema = structure(vec![("a", number())]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1"), ("a.0", "2")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
}

#[test]
fn struct_schema_prefers_object_children_over_a_co_located_scalar() {
    // Config.test.ts:2156 — `a` is a struct, so `a=1` is ignored and `a.b=2`
    // decides the value; `a_0` is unrelated to a struct.
    let schema = structure(vec![("a", shape(vec![("b", number())]))]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1"), ("a.b", "2"), ("a.0", "3")])),
        Ok(obj(&[("a", obj(&[("b", Value::Num(2.0))]))]))
    );
}

#[test]
fn null_schema_decodes_null_and_reports_absence() {
    // Config.test.ts:2166.
    let schema = Schema::as_unknown(Schema::<()>::null());
    let config = Config::schema(&schema, Some("a".to_string()));

    assert_eq!(config.load(&provider(&[("a", "null")])), Ok(Value::Null));
    assert!(config.load(&Provider::empty()).is_err());
    assert!(
        config.load(&provider(&[("a", "a")])).is_err(),
        "a present non-null value is refused"
    );
}

#[test]
fn struct_schema_decodes_required_properties() {
    // Config.test.ts:2246.
    let schema = structure(vec![("a", number())]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
}

#[test]
fn struct_schema_omits_absent_optional_key_properties() {
    // Config.test.ts:2253 — an `optionalKey` field is absent, not null; the
    // unrelated key is what makes the node an object rather than nothing.
    let schema = field_shape(vec![Field::optional_key("a", number())]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
    assert_eq!(
        config.load(&provider(&[("unrelated", "value")])),
        Ok(obj(&[]))
    );
}

#[test]
fn struct_schema_omits_absent_optional_properties() {
    // Config.test.ts:2261.
    let schema = field_shape(vec![Field::optional("a", number())]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "2")])),
        Ok(obj(&[("a", Value::Num(2.0))]))
    );
    assert_eq!(
        config.load(&provider(&[("unrelated", "value")])),
        Ok(obj(&[]))
    );
}

#[test]
fn struct_schema_decodes_literal_properties() {
    // Config.test.ts:2270.
    let schema = structure(vec![(
        "a",
        Schema::<Unknown>::literals(vec![text("b"), text("c")]),
    )]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "b")])),
        Ok(obj(&[("a", Value::Str("b".to_string()))]))
    );
    assert_eq!(
        config.load(&provider(&[("a", "c")])),
        Ok(obj(&[("a", Value::Str("c".to_string()))]))
    );
}

#[test]
fn array_properties_decode_from_indexed_children_only() {
    // Config.test.ts:2278 — a co-located scalar at an array path is a type
    // mismatch, never an array; only indexed children build one.
    let schema = structure(vec![(
        "a",
        Schema::as_unknown(Schema::array(Schema::number())),
    )]);
    let config = Config::schema(&schema, None);

    assert!(
        config.load(&provider(&[("a", "")])).is_err(),
        "an empty value reads as a missing key"
    );
    assert!(
        config.load(&preserving(&[("a", "")])).is_err(),
        "a preserved empty scalar is not an array"
    );
    assert!(
        config.load(&provider(&[("a", "1")])).is_err(),
        "a scalar is not an array"
    );
    assert_eq!(
        config.load(&provider(&[("a.0", "1")])),
        Ok(obj(&[("a", Value::List(vec![Value::Num(1.0)]))]))
    );
    assert_eq!(
        config.load(&provider(&[("a.0", "1"), ("a.1", "2")])),
        Ok(obj(&[(
            "a",
            Value::List(vec![Value::Num(1.0), Value::Num(2.0)])
        )]))
    );
    // The scalar is ignored when indexed children exist.
    assert_eq!(
        config.load(&provider(&[("a", "1"), ("a.0", "2")])),
        Ok(obj(&[("a", Value::List(vec![Value::Num(2.0)]))]))
    );
    assert!(
        config.load(&Provider::empty()).is_err(),
        "an absent parent object is a failure"
    );
}

#[test]
fn number_schema_decodes_numeric_text_and_rejects_other_text() {
    // Config.test.ts:2507.
    let schema = Schema::number();
    let config = Config::schema(&schema, Some("a".to_string()));

    assert_eq!(config.load(&provider(&[("a", "1")])), Ok(Value::Num(1.0)));
    assert!(config.load(&provider(&[("a", "a")])).is_err());
}

#[test]
fn struct_schema_requires_and_validates_every_required_property() {
    // Config.test.ts:2554.
    let schema = structure(vec![("a", number())]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
    assert!(
        config.load(&Provider::empty()).is_err(),
        "a missing required key fails"
    );
    assert!(
        config.load(&provider(&[("a", "value")])).is_err(),
        "a present but invalid key fails"
    );
}

#[test]
fn tuple_schema_reads_positions_and_rejects_a_scalar() {
    // Config.test.ts:2638 — root-level positional input.
    let schema = Schema::tuple_of(vec![number()]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("0", "1")])),
        Ok(Value::List(vec![Value::Num(1.0)]))
    );
    assert!(
        config.load(&Provider::single(Vec::new(), Some("1".to_string())))
            .is_err(),
        "a scalar is not a tuple"
    );
}

#[test]
fn tuple_schema_requires_and_validates_every_element() {
    // Config.test.ts:2646.
    let schema = Schema::tuple_of(vec![string(), number()]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("0", "a"), ("1", "2")])),
        Ok(Value::List(vec![
            Value::Str("a".to_string()),
            Value::Num(2.0),
        ]))
    );
    assert!(
        config.load(&provider(&[("0", "a")])).is_err(),
        "a missing element fails"
    );
    assert!(
        config.load(&provider(&[("0", "a"), ("1", "value")])).is_err(),
        "an invalid element fails"
    );
}

#[test]
fn a_bare_literal_schema_decodes_its_members() {
    // Config.test.ts:2682 — the alternation itself, at the root.
    let schema = Schema::<Unknown>::literals(vec![text("a"), text("b")]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&Provider::single(Vec::new(), Some("a".to_string()))),
        Ok(Value::Str("a".to_string()))
    );
    assert_eq!(
        config.load(&Provider::single(Vec::new(), Some("b".to_string()))),
        Ok(Value::Str("b".to_string()))
    );
}

#[test]
fn array_schema_decodes_indexed_input_and_rejects_a_scalar() {
    // Config.test.ts:2364.
    let schema = Schema::array(Schema::number());
    let config = Config::schema(&schema, Some("a".to_string()));

    assert_eq!(
        config.load(&provider(&[("a.0", "1"), ("a.1", "2")])),
        Ok(Value::List(vec![Value::Num(1.0), Value::Num(2.0)]))
    );
    assert!(config.load(&provider(&[("a", "1")])).is_err());
    assert!(
        config
            .load(&provider(&[("a.0", "1"), ("a.1", "value")]))
            .is_err(),
        "an invalid element fails"
    );
}

#[test]
fn literal_union_decodes_only_its_members() {
    // Config.test.ts:2391 — a struct field holding the literal alternation.
    let schema = structure(vec![(
        "a",
        Schema::<Unknown>::literals(vec![text("a"), text("b")]),
    )]);
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "a")])),
        Ok(obj(&[("a", Value::Str("a".to_string()))]))
    );
    assert_eq!(
        config.load(&provider(&[("a", "b")])),
        Ok(obj(&[("a", Value::Str("b".to_string()))]))
    );
}

fn two_struct_members(mode: &'static str) -> Schema<Unknown> {
    Schema::union_with_mode(
        vec![
            shape(vec![("a", string())]),
            shape(vec![("b", number())]),
        ],
        mode,
    )
}

#[test]
fn union_uses_first_match_semantics() {
    // Config.test.ts:2399 — the first member that fits wins, so a provider
    // holding both keys still reads as `{ a }`.
    let config = Config::schema(&two_struct_members("anyOf"), None);

    assert_eq!(
        config.load(&provider(&[("a", "a")])),
        Ok(obj(&[("a", Value::Str("a".to_string()))]))
    );
    assert_eq!(
        config.load(&provider(&[("b", "1")])),
        Ok(obj(&[("b", Value::Num(1.0))]))
    );
    assert_eq!(
        config.load(&provider(&[("a", "a"), ("b", "1")])),
        Ok(obj(&[("a", Value::Str("a".to_string()))]))
    );
}

#[test]
fn one_of_mode_requires_exactly_one_match() {
    // Config.test.ts:2411 — both members fit, so `oneOf` refuses.
    let config = Config::schema(&two_struct_members("oneOf"), None);

    assert_eq!(
        config.load(&provider(&[("a", "a")])),
        Ok(obj(&[("a", Value::Str("a".to_string()))]))
    );
    assert_eq!(
        config.load(&provider(&[("b", "1")])),
        Ok(obj(&[("b", Value::Num(1.0))]))
    );
    assert!(
        config
            .load(&provider(&[("a", "a"), ("b", "1")]))
            .is_err(),
        "two matching members is ambiguous in oneOf mode"
    );
}

#[test]
fn a_numeric_scalar_decodes_as_a_number_in_either_order() {
    // Config.test.ts:2427 and :2435 — numeric text is a number whether Number
    // or String is declared first, while non-numeric text stays text.
    let number_first = Schema::union_with_mode(vec![number(), string()], "anyOf");
    let string_first = Schema::union_with_mode(vec![string(), number()], "anyOf");

    for schema in [&number_first, &string_first] {
        let config = Config::schema(schema, Some("a".to_string()));
        assert_eq!(
            config.load(&provider(&[("a", "1")])),
            Ok(Value::Num(1.0)),
            "numeric text reads as a number"
        );
        assert_eq!(
            config.load(&provider(&[("a", "a")])),
            Ok(Value::Str("a".to_string())),
            "non-numeric text stays text"
        );
    }
}

#[test]
fn incompatible_container_representations_read_as_absent() {
    // Config.test.ts:1286 — an array node where a struct is expected, and an
    // object node where an array is expected, both read as absent, so a
    // default and an option recover them.
    let struct_config = Config::schema(
        &field_shape(vec![Field::optional_key("value", string())]),
        Some("value".to_string()),
    )
    .with_default(obj(&[("value", Value::Str("default".to_string()))]));
    assert_eq!(
        struct_config.load(&provider(&[("value", "[]")])),
        Ok(obj(&[("value", Value::Str("default".to_string()))]))
    );

    let array_config = Config::schema(&Schema::array(Schema::string()), Some("value".to_string()))
        .option();
    assert_eq!(
        array_config.load(&provider(&[("value", "{}")])),
        Ok(Value::None)
    );
}

#[test]
fn present_containers_with_incompatible_shapes_still_fail() {
    // Config.test.ts:1321 — the same two mismatches without a default or an
    // option are failures, so absence is not silently swallowed.
    let struct_config = Config::schema(
        &field_shape(vec![Field::optional_key("value", string())]),
        Some("value".to_string()),
    );
    assert!(struct_config.load(&provider(&[("value", "[]")])).is_err());

    let array_config = Config::schema(&Schema::array(Schema::string()), Some("value".to_string()));
    assert!(array_config.load(&provider(&[("value", "{}")])).is_err());
}

#[test]
fn a_refused_value_is_not_recovered_by_a_default() {
    // `withDefault` recovers absence, not a value the schema refused. The
    // pin's row builds this refusal with `Schema.check`; the port's `refine`
    // reports an opaque kind the bridge cannot read, so the refusal is spelled
    // as a failing decode over the same Number wire.
    let schema = Schema::decode_to(
        Schema::number(),
        Transformation::new(
            |number: f64| {
                if number > 0.0 {
                    Ok(number)
                } else {
                    Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: "a positive number",
                        got: "a value failing the check",
                    })
                }
            },
            |number: f64| Ok(number),
        ),
    );
    let config = Config::schema(&schema, Some("a".to_string())).with_default(Value::Num(0.0));

    assert_eq!(config.load(&provider(&[("a", "1")])), Ok(Value::Num(1.0)));
    assert!(
        config.load(&provider(&[("a", "-1")])).is_err(),
        "an invalid present value must not fall back to the default"
    );
}

#[test]
fn a_schema_loads_the_encoded_shape_when_it_differs_from_the_decoded_one() {
    // Config.test.ts:1732 — the provider is read through the *encoded* side of
    // the transformation, then decoded into the struct.
    let config = Config::schema(&scalar_to_struct(), Some("config".to_string()));
    assert_eq!(
        config.load(&provider(&[("config", "value")])),
        Ok(obj(&[("value", Value::Str("value".to_string()))]))
    );

    let config = Config::schema(&struct_to_scalar(), Some("config".to_string()));
    assert_eq!(
        config.load(&provider(&[("config.value", "value")])),
        Ok(Value::Str("value".to_string()))
    );
}

#[test]
fn a_schema_reads_root_flat_and_nested_paths() {
    // Config.test.ts:2118 — a root read, a dotted path, and an absent root.
    let root = Config::schema(&Schema::string(), None);
    assert!(
        root.load(&Provider::empty()).is_err(),
        "an empty provider has no root scalar"
    );

    let nested = Config::schema(&Schema::string(), Some("a.b".to_string()));
    assert_eq!(
        nested.load(&provider(&[("a.b", "value")])),
        Ok(Value::Str("value".to_string()))
    );
}
