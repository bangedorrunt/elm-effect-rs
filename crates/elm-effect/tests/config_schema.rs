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

use config::{Config, ConfigError, Value};
use config_provider::{Node, Provider};
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

/// A provider that also records explicit container nodes. A flat map cannot:
/// `{ a: {} }` and `{}` store the same nothing, which is the state the pin's
/// provider distinguishes (Config.test.ts:729).
fn container(pairs: &[(&str, &str)], nodes: &[(&[&str], Node)]) -> Provider {
    let mut provider = provider(pairs);
    for (path, node) in nodes {
        provider = provider.with_node(path, *node);
    }
    provider
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

/// A schema that accepts only `undefined`, which is the pin's
/// `Schema.Undefined`.
fn undefined_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::undefined())
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
fn record_reads_object_input_by_literal_key() {
    // Config.test.ts:1650 — record keys keep their dots: they are keys, not
    // path segments.
    let config = Config::record_of(
        &Schema::string(),
        &Schema::string(),
        Some("OTEL_RESOURCE_ATTRIBUTES".to_string()),
    );
    let provider = Provider::record_at(
        vec!["OTEL_RESOURCE_ATTRIBUTES".to_string()],
        [
            ("service.name".to_string(), "my-service".to_string()),
            ("service.version".to_string(), "1.0.0".to_string()),
            ("custom.attribute".to_string(), "value".to_string()),
        ],
    );

    assert_eq!(
        config.load(&provider),
        // The provider lists keys in its own order; the assertion is about
        // which keys exist, not their order.
        Ok(obj(&[
            ("custom.attribute", Value::Str("value".to_string())),
            ("service.name", Value::Str("my-service".to_string())),
            ("service.version", Value::Str("1.0.0".to_string())),
        ]))
    );
}

#[test]
fn record_reads_a_separated_string_with_the_default_separators() {
    // Config.test.ts:1670.
    let config = Config::record_of(
        &Schema::string(),
        &Schema::string(),
        Some("OTEL_RESOURCE_ATTRIBUTES".to_string()),
    );
    let provider = provider(&[(
        "OTEL_RESOURCE_ATTRIBUTES",
        "service.name=my-service,service.version=1.0.0",
    )]);

    assert_eq!(
        config.load(&provider),
        Ok(obj(&[
            ("service.name", Value::Str("my-service".to_string())),
            ("service.version", Value::Str("1.0.0".to_string())),
        ]))
    );
}

#[test]
fn record_supports_custom_separators() {
    // Config.test.ts:1688 — both the path form and the root form.
    let input = "service.name==my-service&service.version==1.0.0";
    let expected = obj(&[
        ("service.name", Value::Str("my-service".to_string())),
        ("service.version", Value::Str("1.0.0".to_string())),
    ]);

    let root = Config::record_with(&Schema::string(), &Schema::string(), None, "&", "==");
    assert_eq!(
        root.load(&Provider::single(Vec::new(), Some(input.to_string()))),
        Ok(expected.clone())
    );

    let named = Config::record_with(
        &Schema::string(),
        &Schema::string(),
        Some("OTEL_RESOURCE_ATTRIBUTES".to_string()),
        "&",
        "==",
    );
    assert_eq!(
        named.load(&provider(&[("OTEL_RESOURCE_ATTRIBUTES", input)])),
        Ok(expected)
    );
}

#[test]
fn config_record_owns_separated_parsing() {
    // Config.test.ts:1857 — `Config.Record` reads `values` as a separated
    // string; the schema bridge would read its indexed children instead.
    let config = Config::record_of(
        &Schema::string(),
        &Schema::number(),
        Some("values".to_string()),
    );

    assert_eq!(
        config.load(&provider(&[("values", "first=1,second=2")])),
        Ok(obj(&[
            ("first", Value::Num(1.0)),
            ("second", Value::Num(2.0)),
        ]))
    );
}

#[test]
fn config_array_reads_a_separated_string_before_indexed_children() {
    // Config.test.ts:1892 — `Config.Array` owns the scalar form, so `values`
    // wins over `values_0`, while the schema bridge reads the index.
    let config = Config::array_of(&Schema::number(), Some("values".to_string()));
    let provider = provider(&[("values", "1,2"), ("values.0", "3")]);

    assert_eq!(
        config.load(&provider),
        Ok(Value::List(vec![Value::Num(1.0), Value::Num(2.0)]))
    );

    let schema = Config::schema(
        &Schema::array(Schema::number()),
        Some("values".to_string()),
    );
    assert_eq!(
        schema.load(&provider),
        Ok(Value::List(vec![Value::Num(3.0)]))
    );
}

#[test]
fn config_array_supports_options_with_and_without_a_path() {
    // Config.test.ts:1911.
    let root = Config::array_with(&Schema::number(), None, ";");
    assert_eq!(
        root.load(&Provider::single(Vec::new(), Some("1;2".to_string()))),
        Ok(Value::List(vec![Value::Num(1.0), Value::Num(2.0)]))
    );

    let named = Config::array_with(&Schema::number(), Some("values".to_string()), ";");
    assert_eq!(
        named.load(&provider(&[("values", "1;2")])),
        Ok(Value::List(vec![Value::Num(1.0), Value::Num(2.0)]))
    );
}

#[test]
fn schema_record_decodes_and_validates_object_input() {
    // Config.test.ts:2305 and :2623 — a schema Record over the provider's keys.
    let schema = Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::number())));
    let config = Config::schema(&schema, None);

    assert_eq!(
        config.load(&provider(&[("a", "1")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
    assert_eq!(
        config.load(&provider(&[("a", "1"), ("b", "2")])),
        Ok(obj(&[
            ("a", Value::Num(1.0)),
            ("b", Value::Num(2.0)),
        ]))
    );
    assert!(
        config
            .load(&provider(&[("a", "1"), ("b", "value")]))
            .is_err(),
        "an invalid value fails at its key"
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

#[test]
fn an_empty_object_node_is_a_present_struct() {
    // Config.test.ts:729 — `{ a: {} }` is an existing object, so a struct
    // whose fields are all optional decodes to `{}`; a `Record` reads no keys
    // as `{}`; and an empty array node is an empty tuple. What the same row
    // asserts for an ABSENT path is the other half: the whole config falls
    // back to its default.
    let empty_object = container(&[], &[(&["a"], Node::Object)]);

    let optional = Config::schema(
        &field_shape(vec![Field::optional_key("value", string())]),
        Some("a".to_string()),
    );
    assert_eq!(optional.load(&empty_object), Ok(obj(&[])));

    let record = Config::schema(
        &Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::string()))),
        Some("a".to_string()),
    );
    assert_eq!(record.load(&empty_object), Ok(obj(&[])));

    let empty_tuple = Config::schema(&Schema::tuple_of(vec![]), Some("a".to_string()));
    assert_eq!(
        empty_tuple.load(&container(&[], &[(&["a"], Node::Array)])),
        Ok(Value::List(vec![]))
    );

    // The other half of the row: at an ABSENT path the same shapes fall back
    // to the whole default rather than decoding an empty value.
    let fallback = obj(&[("value", Value::Str("default".to_string()))]);
    assert_eq!(
        optional.with_default(fallback.clone()).load(&Provider::empty()),
        Ok(fallback.clone())
    );
    assert_eq!(
        record.with_default(fallback.clone()).load(&Provider::empty()),
        Ok(fallback.clone())
    );
    let tuple_default = Value::List(vec![Value::Str("default".to_string())]);
    let tuple = Config::schema(&Schema::tuple_of(vec![string()]), Some("a".to_string()))
        .with_default(tuple_default.clone());
    assert_eq!(tuple.load(&Provider::empty()), Ok(tuple_default));
}

#[test]
fn a_present_empty_container_is_validated_not_defaulted() {
    // Config.test.ts:877 — an existing empty object is a struct the schema
    // validates, so its missing `host` is a hard failure a default cannot
    // recover. The `all` group beside it reads its own lookups, has no such
    // node, and does fall back.
    let fallback = obj(&[
        ("host", Value::Str("fallback".to_string())),
        ("port", Value::Num(0.0)),
    ]);
    let schema_config = Config::schema(
        &field_shape(vec![
            Field::required("host", string()),
            Field::required("port", number()),
        ]),
        None,
    )
    .nested("database")
    .with_default(fallback.clone());
    let all_config = Config::record(vec![
        ("host".to_string(), Config::string(Some("host".to_string()))),
        ("port".to_string(), Config::number(Some("port".to_string()))),
    ])
    .nested("database")
    .with_default(fallback.clone());

    let empty_database = container(&[], &[(&["database"], Node::Object)]);
    assert!(
        schema_config.load(&empty_database).is_err(),
        "a present empty object is validated, not defaulted"
    );
    assert_eq!(all_config.load(&empty_database), Ok(fallback));
}

#[test]
fn a_partial_object_fails_instead_of_defaulting() {
    // Config.test.ts:897 — `host` present, `port` absent: the struct is
    // validated as a whole, so its default does not replace a half-filled
    // object; the `all` group has an absent child and does fall back.
    let fallback = obj(&[
        ("host", Value::Str("fallback".to_string())),
        ("port", Value::Num(0.0)),
    ]);
    let schema_config = Config::schema(
        &field_shape(vec![
            Field::required("host", string()),
            Field::required("port", number()),
        ]),
        None,
    )
    .nested("database")
    .with_default(fallback.clone());
    let all_config = Config::record(vec![
        ("host".to_string(), Config::string(Some("host".to_string()))),
        ("port".to_string(), Config::number(Some("port".to_string()))),
    ])
    .nested("database");

    let partial = provider(&[("database.host", "localhost")]);
    assert!(
        schema_config.load(&partial).is_err(),
        "a missing required field fails even with a default"
    );
    assert_eq!(
        all_config.clone().with_default(fallback.clone()).load(&partial),
        Ok(fallback),
        "the all group combines independent lookups, so it falls back"
    );
    assert_eq!(
        all_config.option().load(&partial),
        Ok(Value::None),
        "the same group is absent when one child is"
    );
}

#[test]
fn an_empty_object_node_is_not_a_string() {
    // Config.test.ts:2499 — `fromUnknown({})` hands an object node to a String
    // schema: the value is refused rather than read as an absent key.
    let config = Config::schema(&string(), None);
    assert!(
        config
            .load(&Provider::empty().with_node(&[], Node::Object))
            .is_err()
    );
}

#[test]
fn an_empty_root_object_omits_absent_optional_properties() {
    // Config.test.ts:2573 and :2581 — the second assertion in both rows reads
    // an empty object node as `{}` at the root, which a flat provider cannot
    // tell from an absent path.
    let empty_root = Provider::empty().with_node(&[], Node::Object);

    let optional_key = Config::schema(
        &field_shape(vec![Field::optional_key("a", number())]),
        None,
    );
    assert_eq!(optional_key.load(&empty_root), Ok(obj(&[])));
    assert_eq!(
        optional_key.load(&provider(&[("a", "1")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );

    let optional = Config::schema(&field_shape(vec![Field::optional("a", number())]), None);
    assert_eq!(optional.load(&empty_root), Ok(obj(&[])));
    assert_eq!(
        optional.load(&provider(&[("a", "1")])),
        Ok(obj(&[("a", Value::Num(1.0))]))
    );
}

#[test]
fn tuple_properties_reject_a_scalar_and_validate_every_element() {
    // Config.test.ts:2320, :2332, :2344 — a tuple property reads positions
    // only, so a co-located scalar is a type mismatch; a missing or invalid
    // element fails the tuple, and indexed children decode in order.
    let empty = structure(vec![("a", Schema::as_unknown(Schema::tuple_of(vec![])))]);
    let empty_config = Config::schema(&empty, None);
    assert!(
        empty_config.load(&preserving(&[("a", "")])).is_err(),
        "a preserved empty scalar is not an empty tuple"
    );

    let one = structure(vec![(
        "a",
        Schema::as_unknown(Schema::tuple_of(vec![number()])),
    )]);
    let one_config = Config::schema(&one, None);
    assert!(
        one_config.load(&provider(&[("a", "1")])).is_err(),
        "a scalar is not a tuple"
    );

    let pair = structure(vec![(
        "a",
        Schema::as_unknown(Schema::tuple_of(vec![string(), number()])),
    )]);
    let pair_config = Config::schema(&pair, None);
    assert_eq!(
        pair_config.load(&provider(&[("a.0", "a"), ("a.1", "2")])),
        Ok(obj(&[(
            "a",
            Value::List(vec![Value::Str("a".to_string()), Value::Num(2.0)])
        )]))
    );
    assert!(
        pair_config.load(&provider(&[("a.0", "a")])).is_err(),
        "a missing element fails"
    );
    assert!(
        pair_config
            .load(&provider(&[("a.0", "a"), ("a.1", "value")]))
            .is_err(),
        "an invalid element fails"
    );
}

#[test]
fn a_plain_array_schema_ignores_a_co_located_scalar() {
    // Config.test.ts:715 — `Schema.Array` reads structural input only, so a
    // scalar at the array's own path is absence and a default recovers it;
    // `Config.Array` is the constructor that splits text. Indexed children
    // are the array representation.
    let default = Value::List(vec![Value::Str("default".to_string())]);
    let config = Config::schema(&Schema::array(Schema::string()), Some("a".to_string()))
        .with_default(default.clone());

    assert_eq!(config.load(&provider(&[("a", "value")])), Ok(default.clone()));
    assert_eq!(config.load(&provider(&[("a", "")])), Ok(default.clone()));
    assert_eq!(config.load(&preserving(&[("a", "")])), Ok(default.clone()));
    assert_eq!(config.load(&Provider::empty()), Ok(default.clone()));
    assert_eq!(
        config.load(&provider(&[("a.0", "value")])),
        Ok(Value::List(vec![Value::Str("value".to_string())]))
    );
}

#[test]
fn a_refinement_reports_the_shape_it_refines() {
    // Config.test.ts:696 — a present value that fails the check is invalid
    // input, which a default does not recover; only absence takes the default.
    // The refinement's kind has to name the string underneath it for the bridge
    // to build a wire at all.
    let refined = Schema::as_unknown(
        Schema::string().refine("must be \"a\"", |value: &String| value == "a"),
    );
    let config = Config::schema(&refined, Some("a".to_string()))
        .with_default(Value::Str("fallback".to_string()));

    assert_eq!(
        config.load(&Provider::empty()),
        Ok(Value::Str("fallback".to_string()))
    );
    assert_eq!(
        config.load(&provider(&[("a", "a")])),
        Ok(Value::Str("a".to_string()))
    );
    match config.load(&provider(&[("a", "b")])) {
        Err(errors) => assert!(
            errors
                .iter()
                .all(|error| matches!(error, ConfigError::Invalid { .. })),
            "a failing check is invalid input, not absence: {errors:?}"
        ),
        Ok(value) => panic!("a failing refinement decoded to {value:?}"),
    }
}

#[test]
fn undefined_or_decodes_absence_as_undefined() {
    // Config.test.ts:796, :845, :2118 — absence decodes to `undefined`, which
    // a default leaves alone and an option keeps distinct from `None`.
    let schema = Schema::as_unknown(Schema::undefined_or(Schema::string()));

    let defaulted = Config::schema(&schema, Some("a".to_string()))
        .with_default(Value::Str("default".to_string()));
    assert_eq!(defaulted.load(&Provider::empty()), Ok(Value::Undefined));
    assert_eq!(
        defaulted.load(&provider(&[("a", "value")])),
        Ok(Value::Str("value".to_string()))
    );

    let optional = Config::schema(&schema, Some("a".to_string())).option();
    assert_eq!(optional.load(&Provider::empty()), Ok(Value::Undefined));
    assert_eq!(
        Config::string(Some("a".to_string()))
            .option()
            .load(&Provider::empty()),
        Ok(Value::None),
        "a decoded undefined is not the same state as an absent option"
    );

    let root = Config::schema(&schema, None);
    assert_eq!(root.load(&Provider::empty()), Ok(Value::Undefined));
}

#[test]
fn an_undefined_schema_decodes_absence() {
    // Config.test.ts:2483 — `Schema.Undefined` accepts the absent node and
    // refuses text at the same path.
    let config = Config::schema(&undefined_schema(), None);
    assert_eq!(config.load(&Provider::empty()), Ok(Value::Undefined));
    match config.load(&Provider::single(Vec::new(), Some("a".to_string()))) {
        Err(errors) => assert!(
            errors
                .iter()
                .all(|error| matches!(error, ConfigError::Invalid { .. })),
            "text is not undefined: {errors:?}"
        ),
        Ok(value) => panic!("`a` decoded to {value:?}"),
    }
}

#[test]
fn an_undefined_sibling_of_a_missing_field_keeps_its_undefined() {
    // Config.test.ts:910 — the undefined leg succeeds while the required leg
    // is missing, so the group defaults as a whole rather than failing on the
    // undefined leg.
    let undefined_or_string = Schema::as_unknown(Schema::undefined_or(Schema::string()));
    let group = Config::record(vec![
        (
            "optional".to_string(),
            Config::schema(&undefined_or_string, Some("optional".to_string())),
        ),
        (
            "required".to_string(),
            Config::string(Some("required".to_string())),
        ),
    ]);

    let fallback = obj(&[
        ("optional", Value::Str("fallback".to_string())),
        ("required", Value::Str("fallback".to_string())),
    ]);
    assert_eq!(
        group.clone().with_default(fallback.clone()).load(&Provider::empty()),
        Ok(fallback)
    );
    assert_eq!(group.option().load(&Provider::empty()), Ok(Value::None));
}

#[test]
fn a_union_with_an_undefined_member_decodes_the_undefined_arm() {
    // Config.test.ts:2042 — the struct member has no advertised child in a
    // flat source, so `Undefined` is the arm that materializes and the missing
    // sibling is what the group reports.
    let union = Schema::as_unknown(Schema::union_with_mode(
        vec![undefined_schema(), shape(vec![("child", string())])],
        "anyOf",
    ));

    let selected = Config::schema(&union, Some("value".to_string()));
    assert_eq!(
        selected.load(&provider(&[("value_child", "present")])),
        Ok(Value::Undefined)
    );

    let group = Config::record(vec![
        ("selected".to_string(), selected),
        (
            "required".to_string(),
            Config::string(Some("required".to_string())),
        ),
    ]);
    let fallback = obj(&[
        ("selected", Value::Undefined),
        ("required", Value::Str("default".to_string())),
    ]);
    assert_eq!(
        group
            .with_default(fallback.clone())
            .load(&provider(&[("value_child", "present")])),
        Ok(fallback)
    );
}

#[test]
fn a_recursive_suspended_schema_decodes_nested_arrays() {
    // Config.test.ts:2735 — `Struct({ a, as: Array(suspend(() => schema)) })`
    // is recursive: the bridge can only walk it by resolving the suspension to
    // the schema itself, then following `as` element by element.
    let cell: std::sync::Arc<std::sync::OnceLock<Schema<Unknown>>> =
        std::sync::Arc::new(std::sync::OnceLock::new());
    let factory = {
        let cell = std::sync::Arc::clone(&cell);
        move || {
            cell.get()
                .cloned()
                .expect("the recursive schema is set before the first walk")
        }
    };
    let array = Schema::as_unknown(Schema::array(Schema::<Unknown>::suspend(factory)));
    let tree = Schema::as_unknown(structure(vec![("a", string()), ("as", array)]));
    cell.set(tree.clone()).ok();

    let config = Config::schema(&tree, None);
    assert_eq!(
        config.load(&container(&[("a", "1")], &[(&["as"], Node::Array)])),
        Ok(obj(&[
            ("a", Value::Str("1".to_string())),
            ("as", Value::List(Vec::new())),
        ]))
    );
    assert_eq!(
        config.load(&container(
            &[("a", "1"), ("as.0.a", "2")],
            &[(&["as"], Node::Array), (&["as", "0", "as"], Node::Array)],
        )),
        Ok(obj(&[
            ("a", Value::Str("1".to_string())),
            (
                "as",
                Value::List(vec![obj(&[
                    ("a", Value::Str("2".to_string())),
                    ("as", Value::List(Vec::new())),
                ])]),
            ),
        ]))
    );
}
