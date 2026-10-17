//! `packages/effect/test/ai/OpenAiStructuredOutputRepresentation.test.ts` witnesses.
//!
//! The pin's `toCodecOpenAI` is two layers: a codec transform (encoded-side
//! projection, custom filter callbacks, Record-to-entry-array lowering) and a
//! JSON Schema rewrite to the OpenAI strict subset. This crate ports the
//! rewrite layer as [`rewrite_openai_schema`](elm_effect_unstable::ai::rewrite_openai_schema)
//! plus the rows whose schema-side machinery already exists here. Rows needing
//! custom `toJsonSchema` filter callbacks (TS:87), callback-after-structural
//! lowering (TS:111), or Record entry-array size-bound transport (TS:238) have
//! no representable target and are not carried.

use elm_effect_schema::{Schema, StandardJsonSchemaTarget, Unknown};
use elm_effect_unstable::ai::rewrite_openai_schema;
use serde_json::json;

#[test]
fn projects_the_encoded_side_before_json_schema_generation() {
    // OpenAiStructuredOutputRepresentation.test.ts:15 — FiniteFromString is
    // text on the wire, so the input (encode-side) document is a string.
    let doc = Schema::<f64>::finite_from_string()
        .standard_json_schema_input(StandardJsonSchemaTarget::Draft2020_12);
    assert_eq!(doc.get("type"), Some(&json!("string")));
}

#[test]
fn keeps_supported_pattern_with_a_description() {
    // OpenAiStructuredOutputRepresentation.test.ts:19 — `pattern` is in the
    // OpenAI subset, so it and the description survive the rewrite.
    let doc = json!({"type": "string", "description": "starts with a", "pattern": "^a"});
    assert_eq!(rewrite_openai_schema(&doc), doc);
}

#[test]
fn drops_unsupported_min_length_but_keeps_description_and_validation() {
    // OpenAiStructuredOutputRepresentation.test.ts:36 — `minLength` is outside
    // the OpenAI subset so the rewrite drops it, keeps the description, and
    // the checked codec still rejects the short value on decode.
    let doc = json!({"type": "string", "description": "at least two characters", "minLength": 2});
    assert_eq!(
        rewrite_openai_schema(&doc),
        json!({"type": "string", "description": "at least two characters"})
    );
    let schema = Schema::checked(
        Schema::json(),
        "isMinLength",
        vec!["2"],
        vec![("description", "at least two characters")],
    );
    assert!(schema.decode(Unknown::Text("a".to_owned())).is_err());
    assert!(schema.decode(Unknown::Text("ab".to_owned())).is_ok());
}

#[test]
fn keeps_supported_pattern_without_a_description() {
    // OpenAiStructuredOutputRepresentation.test.ts:58.
    let doc = json!({"type": "string", "pattern": "^a"});
    assert_eq!(rewrite_openai_schema(&doc), doc);
}

#[test]
fn drops_unsupported_min_length_without_a_description() {
    // OpenAiStructuredOutputRepresentation.test.ts:73.
    let doc = json!({"type": "string", "minLength": 2});
    assert_eq!(rewrite_openai_schema(&doc), json!({"type": "string"}));
}

#[test]
fn keeps_property_names_that_match_unsupported_keywords() {
    // OpenAiStructuredOutputRepresentation.test.ts:133 — `minLength` as a
    // *property name* is data, not a keyword, so both the struct document and
    // the rewrite keep it.
    let doc = Schema::struct_([("minLength", Schema::json())]).json_schema();
    assert_eq!(doc["properties"]["minLength"], json!({}));
    assert_eq!(doc["required"], json!(["minLength"]));
    let rewritten = rewrite_openai_schema(&doc);
    assert_eq!(rewritten["properties"]["minLength"], json!({}));
    assert_eq!(rewritten["required"], json!(["minLength"]));
}

#[test]
fn drops_pattern_properties_returned_by_custom_callbacks() {
    // OpenAiStructuredOutputRepresentation.test.ts:150 — `patternProperties`
    // is outside the OpenAI subset.
    let doc = json!({
        "type": "object",
        "properties": {"fixed": {"type": "string"}},
        "required": ["fixed"],
        "additionalProperties": false,
        "patternProperties": {"^x": {"type": "string"}}
    });
    assert_eq!(
        rewrite_openai_schema(&doc),
        json!({
            "type": "object",
            "properties": {"fixed": {"type": "string"}},
            "required": ["fixed"],
            "additionalProperties": false
        })
    );
}

#[test]
fn does_not_merge_structural_constraints_from_all_of() {
    // OpenAiStructuredOutputRepresentation.test.ts:171 — `properties` and
    // `required` arriving via `allOf` are dropped, never merged.
    let doc = json!({
        "type": "object",
        "properties": {"fixed": {"type": "string"}},
        "required": ["fixed"],
        "additionalProperties": false,
        "allOf": [{"properties": {"extra": {"type": "string"}}, "required": ["extra"]}]
    });
    assert_eq!(
        rewrite_openai_schema(&doc),
        json!({
            "type": "object",
            "properties": {"fixed": {"type": "string"}},
            "required": ["fixed"],
            "additionalProperties": false
        })
    );
}

#[test]
fn leaves_an_already_strict_document_unchanged() {
    // OpenAiStructuredOutputRepresentation.test.ts:193 — the pin keeps the
    // original codec on its fast path; the port's fixpoint is a rewrite that
    // returns an already-strict document equal.
    let doc = json!({
        "type": "object",
        "properties": {"value": {"type": "string"}},
        "required": ["value"],
        "additionalProperties": false
    });
    assert_eq!(rewrite_openai_schema(&doc), doc);
}

#[test]
fn keeps_declaration_fallback_validation_in_the_codec() {
    // OpenAiStructuredOutputRepresentation.test.ts:198 — an opaque declaration
    // emits a non-empty document and still refuses values on decode.
    let schema = Schema::<Unknown>::never();
    assert!(
        schema
            .json_schema()
            .as_object()
            .is_some_and(|map| !map.is_empty())
    );
    assert!(schema.decode(Unknown::Object(Default::default())).is_err());
}

#[test]
fn merges_repeated_lower_bounds_conservatively() {
    // OpenAiStructuredOutputRepresentation.test.ts:210 — greaterThan(1),
    // greaterThanOrEqualTo(2), greaterThan(2) collapse to exclusiveMinimum 2.
    let doc = json!({
        "type": "number",
        "description": "a value greater than 1 and a value greater than or equal to 2 and a value greater than 2",
        "allOf": [{"minimum": 1}, {"minimum": 2}, {"exclusiveMinimum": 2}]
    });
    assert_eq!(
        rewrite_openai_schema(&doc),
        json!({
            "type": "number",
            "description": "a value greater than 1 and a value greater than or equal to 2 and a value greater than 2",
            "exclusiveMinimum": 2
        })
    );
}

#[test]
fn merges_repeated_array_bounds_conservatively() {
    // OpenAiStructuredOutputRepresentation.test.ts:225 — the tighter bound
    // wins on each side.
    let doc = json!({
        "type": "array",
        "allOf": [{"minItems": 1}, {"minItems": 2}, {"maxItems": 5}, {"maxItems": 4}]
    });
    let rewritten = rewrite_openai_schema(&doc);
    assert_eq!(rewritten.get("minItems"), Some(&json!(2)));
    assert_eq!(rewritten.get("maxItems"), Some(&json!(4)));
}
