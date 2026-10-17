//! `schemaToJsonSchemaDocument` documents
//! (packages/effect/test/schema/representation/schemaToJsonSchemaDocument.test.ts).
//!
//! The document is the compiler's output wrapped with the dialect and a
//! top-level `definitions` slot. Rows needing machinery the port does not
//! have are excused with reasons instead of approximated:
//!
//! Excused, with reasons:
//! - `:5/:7 artificial JSON encoding` (2 rows): the compiler has no
//!   number-to-JSON-enum projection for non-finite values.
//! - `:19 reference policy`: `toJsonSchemaDocument` takes no reference-policy
//!   option here.
//! - `:114 output, references and generation options`: the Shared definition
//!   needs the pin's `allOf` wrapping and the count needs the expected-message
//!   plumbing `FiniteFromString` does not carry.
//! - `:177 custom compiler annotations`: a `toJsonSchema` callback is a
//!   custom hook the compiler degrades instead of running.
//! - `:196 JSON content media types`: no fromJsonString constructor, and the
//!   compiler drops the media-type annotation without one.
//! - `:207 declarations without a JSON codec`: no declare-with-predicate.
//! - The `:5` `toType` halves: one schema carries one side, so there is no
//!   type side to project.

use elm_effect_schema::representation_doc::{schema_to_json_schema_document, to_representation};
use elm_effect_schema::{Field, Schema, Unknown};
use serde_json::json;

fn finite_from_string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

// 5 (partial): the encoded side drives both the representation tag and the
// JSON Schema document. The `toType` projections need a second side the port
// does not keep, so only the encoded halves carry this row.
#[test]
fn uses_the_encoded_side_for_representations_and_json_schema() {
    let representation = to_representation(&finite_from_string()).expect("converts");
    assert_eq!(representation.representation["_tag"], json!("String"));
    assert_eq!(
        schema_to_json_schema_document(&finite_from_string()),
        json!({
            "dialect": "draft-2020-12",
            "schema": { "type": "string" },
            "definitions": {},
        })
    );
}

// 19: repeated anonymous schemas inline at every occurrence.
#[test]
fn inlines_repeated_anonymous_schemas_by_default() {
    let shared = || {
        Schema::as_unknown(Schema::struct_shape(vec![Field::required(
            "value",
            Schema::as_unknown(Schema::string()),
        )]))
    };
    let schema = Schema::as_unknown(Schema::struct_shape(vec![
        Field::required("first", shared()),
        Field::required("second", shared()),
    ]));
    assert_eq!(
        schema_to_json_schema_document(&schema),
        json!({
            "dialect": "draft-2020-12",
            "schema": {
                "type": "object",
                "properties": {
                    "first": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": true
                    },
                    "second": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": true
                    }
                },
                "required": ["first", "second"],
                "additionalProperties": true
            },
            "definitions": {},
        })
    );
}

// 72: encoded tuple elements project to their wire form.
#[test]
fn projects_encoded_tuple_elements_for_json_schema() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![Schema::as_unknown(
        Schema::number_from_string(),
    )]));
    assert_eq!(
        schema_to_json_schema_document(&schema)["schema"],
        json!({
            "type": "array",
            "prefixItems": [{ "type": "string" }],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

// 100: checks stay on the document when no artificial encoding intervenes.
#[test]
fn preserves_checks_when_no_artificial_json_encoding_is_needed() {
    let schema = Schema::checked(
        Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isFinite",
            vec![],
            vec![],
        ),
        "isGreaterThan",
        vec!["0"],
        vec![],
    );
    assert_eq!(
        schema_to_json_schema_document(&schema),
        json!({
            "dialect": "draft-2020-12",
            "schema": { "type": "number", "exclusiveMinimum": 0 },
            "definitions": {},
        })
    );
}
