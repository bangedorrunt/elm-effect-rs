//! `toStandardJSONSchemaV1` rows (Effect v4.0.0
//! packages/effect/test/schema/toStandardJSONSchemaV1.test.ts, 8 rows).
//! Input vs output documents differ only for transforms (`FiniteFromString`:
//! input `string`, output `number`); `draft-07` spells the tuple prefix as
//! `items` with `definitions`, `draft-2020-12` as `prefixItems` with `$defs`.

use elm_effect_schema::{
    Schema, StandardJsonSchemaTarget, Unknown, to_standard_json_schema_v1, to_standard_schema_v1,
};

fn input(schema: &Schema<Unknown>, target: StandardJsonSchemaTarget) -> serde_json::Value {
    to_standard_json_schema_v1(schema.clone()).input(target)
}

fn output(schema: &Schema<Unknown>, target: StandardJsonSchemaTarget) -> serde_json::Value {
    to_standard_json_schema_v1(schema.clone()).output(target)
}

fn finite_tuple() -> Schema<Unknown> {
    Schema::tuple_of(vec![Schema::as_unknown(Schema::finite_from_string())])
}

fn identified_tuple() -> Schema<Unknown> {
    let s = Schema::as_unknown(Schema::annotated_with(
        Schema::string(),
        vec![("identifier", "id")],
    ));
    Schema::tuple_of(vec![s.clone(), s])
}

// :25 — the wrapped schema is still a schema (decodes/encodes as before).
#[test]
fn standard_json_schema_keeps_codec() {
    let std = to_standard_json_schema_v1(Schema::finite_from_string());
    assert_eq!(std.schema().decode(Unknown::Text("1.5".into())), Ok(1.5));
    assert_eq!(
        std.schema().try_encode(&1.5).unwrap(),
        Unknown::Text("1.5".into())
    );
}

// :31 — composing both standards still yields the input document.
#[test]
fn standard_json_schema_supports_both_standards() {
    let both = to_standard_schema_v1(to_standard_json_schema_v1(Schema::string()));
    assert_eq!(
        both.input(StandardJsonSchemaTarget::Draft2020_12),
        serde_json::json!({"type": "string"})
    );
}

// :40 — draft-2020-12 input: tuple of FiniteFromString is string items.
#[test]
fn standard_json_schema_draft2020_input() {
    assert_eq!(
        input(&finite_tuple(), StandardJsonSchemaTarget::Draft2020_12),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

// :51 — draft-2020-12 output: same tuple is number items.
#[test]
fn standard_json_schema_draft2020_output() {
    assert_eq!(
        output(&finite_tuple(), StandardJsonSchemaTarget::Draft2020_12),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "number"}],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

// :62 — draft-2020-12 identifier: repeated id hoists to $defs with $refs.
#[test]
fn standard_json_schema_draft2020_identifier() {
    assert_eq!(
        input(&identified_tuple(), StandardJsonSchemaTarget::Draft2020_12),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"$ref": "#/$defs/id"}, {"$ref": "#/$defs/id"}],
            "minItems": 2,
            "maxItems": 2,
            "$defs": {"id": {"type": "string"}}
        })
    );
}

// :84 — draft-07 input: prefix becomes items.
#[test]
fn standard_json_schema_draft07_input() {
    assert_eq!(
        input(&finite_tuple(), StandardJsonSchemaTarget::Draft07),
        serde_json::json!({
            "type": "array",
            "items": [{"type": "string"}],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

// :95 — draft-07 output: items with numbers.
#[test]
fn standard_json_schema_draft07_output() {
    assert_eq!(
        output(&finite_tuple(), StandardJsonSchemaTarget::Draft07),
        serde_json::json!({
            "type": "array",
            "items": [{"type": "number"}],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

// :106 — draft-07 identifier: definitions with #/definitions refs.
#[test]
fn standard_json_schema_draft07_identifier() {
    assert_eq!(
        input(&identified_tuple(), StandardJsonSchemaTarget::Draft07),
        serde_json::json!({
            "type": "array",
            "items": [{"$ref": "#/definitions/id"}, {"$ref": "#/definitions/id"}],
            "minItems": 2,
            "maxItems": 2,
            "definitions": {"id": {"type": "string"}}
        })
    );
}
