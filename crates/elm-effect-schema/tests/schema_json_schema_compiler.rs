//! Phase-1 schema→JSON-Schema compiler core (epic schema-compiler-openapi).
//! RED test: `json_schema` module does not exist yet.

use elm_effect_schema::json_schema::{compile, compile_multi};
use elm_effect_schema::{Schema, Unknown};

fn checked(base: Schema<Unknown>, name: &str, args: Vec<&str>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::checked(base, name, args, vec![("expected", "refined")]))
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

#[test]
fn compiles_scalar_types() {
    assert_eq!(compile(&string()), serde_json::json!({"type": "string"}));
    assert_eq!(compile(&number()), serde_json::json!({"type": "number"}));
    assert_eq!(
        compile(&Schema::as_unknown(Schema::boolean())),
        serde_json::json!({"type": "boolean"})
    );
    assert_eq!(
        compile(&Schema::as_unknown(Schema::null())),
        serde_json::json!({"type": "null"})
    );
    assert_eq!(
        compile(&Schema::as_unknown(Schema::json())),
        serde_json::json!({})
    );
}

#[test]
fn compiles_integer_check_as_type_integer() {
    assert_eq!(
        compile(&Schema::as_unknown(Schema::int())),
        serde_json::json!({"type": "integer"})
    );
}

#[test]
fn compiles_string_constraints() {
    let schema = checked(
        checked(string(), "isMinLength", vec!["2"]),
        "isMaxLength",
        vec!["5"],
    );
    assert_eq!(
        compile(&schema),
        serde_json::json!({"type": "string", "minLength": 2, "maxLength": 5})
    );
}

#[test]
fn compiles_number_constraints() {
    let schema = checked(
        checked(number(), "isGreaterThanOrEqualTo", vec!["1"]),
        "isLessThan",
        vec!["10"],
    );
    assert_eq!(
        compile(&schema),
        serde_json::json!({"type": "number", "minimum": 1, "exclusiveMaximum": 10})
    );
}

#[test]
fn compiles_closed_struct_with_required() {
    let schema = Schema::as_unknown(Schema::struct_of(vec![
        ("a".to_string(), string()),
        (
            "b".to_string(),
            Schema::optional_key_shape(string()),
        ),
    ]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false
        })
    );
}

#[test]
fn compiles_enum_const_and_unions() {
    let schema = Schema::<Unknown>::literal(Unknown::Text("red".into()));
    assert_eq!(compile(&schema), serde_json::json!({"const": "red"}));
    let schema = Schema::<Unknown>::literals(vec![
        Unknown::Text("a".into()),
        Unknown::Number(1.0),
    ]);
    assert_eq!(
        compile(&schema),
        serde_json::json!({"enum": ["a", 1.0]})
    );
    let schema = Schema::union_with_mode(vec![string(), number()], "oneOf");
    assert_eq!(
        compile(&schema),
        serde_json::json!({"oneOf": [{"type": "string"}, {"type": "number"}]})
    );
}

#[test]
fn compiles_tuple_with_prefix_items() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![string(), number()]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}, {"type": "number"}],
            "minItems": 2,
            "maxItems": 2
        })
    );
}

#[test]
fn hoists_identifier_annotations_into_defs() {
    let named = Schema::annotated_with(string(), vec![("identifier", "Name")]);
    let schema = Schema::as_unknown(Schema::tuple_of(vec![named.clone(), named]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"$ref": "#/$defs/Name"}, {"$ref": "#/$defs/Name"}],
            "minItems": 2,
            "maxItems": 2,
            "$defs": {"Name": {"type": "string"}}
        })
    );
}

#[test]
fn shares_definitions_across_roots() {
    let named = Schema::annotated_with(string(), vec![("identifier", "Shared")]);
    let doc = compile_multi(&[named.clone(), named]);
    assert_eq!(doc.schemas.len(), 2);
    assert_eq!(
        doc.schemas[0],
        serde_json::json!({"$ref": "#/$defs/Shared"})
    );
    assert_eq!(
        doc.definitions.get("Shared"),
        Some(&serde_json::json!({"type": "string"}))
    );
}

