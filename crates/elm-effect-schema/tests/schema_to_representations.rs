//! `toRepresentations` shared documents
//! (packages/effect/test/schema/representation/toRepresentations.test.ts).
//!
//! Each test pins the exact multi-root document the converter emits for one
//! row. Roots share one reference map in input order: the first root claims
//! each identifier, later roots reuse the entry for a clone of the same schema
//! and take a suffixed name otherwise.
//!
//! Carried: `:6` root order, `:23` shared named reference, `:113` checked
//! derivative, `:362`/`:389` identifier collisions, `:416` fallback and
//! explicit collisions.
//!
//! Excused, with reasons:
//! - `:42 fallback across contextual copies of one encoded AST`: needs
//!   `fromJsonString`/`toCodecJson` plus the encoded `~identifier` projection
//!   (`contentMediaType`/`expected` annotations); the port has no encoded side.
//! - `:68`/`:90 Context copies and owner chains`: need `SchemaAST.Context`,
//!   `replaceContext` and `getContextOwner`; the port has no context machinery
//!   (`:68`'s observable sharing is already pinned by `:23`).
//! - `:145 contextual projections with $name` (24 combinations): needs
//!   `toType`/`toEncoded`/`flip` projections, `annotateEncoded`, and key
//!   `mutable`/`optional` contexts; the port projects one side only.
//! - `:191 recursive references across contextual copies`: needs `mutableKey`
//!   plus a type-side projection; neither exists here.
//! - `:224 type references without losing key annotations`: needs `toType`
//!   projection, `annotateKey` and `withConstructorDefault`/`make`.
//! - `:246 projected checks and value annotations`: needs `toType` projection.
//! - `:274`/`:301`/`:323` reference policies (3 rows): need the
//!   `referencePolicy` option, which the port does not take.
//! - `:337 identifiers sharing one encoded AST owner`: needs `referencePolicy`
//!   plus the `NumberFromString` encoded AST.

use elm_effect_schema::representation_doc::to_representations;
use elm_effect_schema::{Schema, Unknown};
use serde_json::json;

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn identified(schema: Schema<Unknown>, id: &str) -> Schema<Unknown> {
    Schema::annotated_with(schema, vec![("identifier".to_string(), id.to_string())])
}

fn fallback(schema: Schema<Unknown>, name: &str) -> Schema<Unknown> {
    Schema::annotated_with(schema, vec![("~identifier".to_string(), name.to_string())])
}

// 6: roots convert in input order against an empty reference map.
#[test]
fn preserves_root_order() {
    let documents = to_representations(&[string(), number(), boolean()]).expect("converts");
    assert_eq!(
        serde_json::to_value(&documents.representations).expect("json"),
        json!([
            { "_tag": "String", "checks": [] },
            { "_tag": "Number", "checks": [] },
            { "_tag": "Boolean", "checks": [] },
        ])
    );
    assert!(documents.references.is_empty());
}

fn boolean() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

// 23: clones of one named schema share its reference.
#[test]
fn shares_a_named_reference_between_roots() {
    let shared = identified(string(), "Shared");
    let documents = to_representations(&[shared.clone(), shared.clone()]).expect("converts");
    assert_eq!(
        serde_json::to_value(&documents.representations).expect("json"),
        json!([
            { "_tag": "Reference", "$ref": "Shared" },
            { "_tag": "Reference", "$ref": "Shared" },
        ])
    );
    assert_eq!(
        serde_json::to_value(&documents.references).expect("json"),
        json!({
            "Shared": {
                "_tag": "String",
                "annotations": { "identifier": "Shared" },
                "checks": [],
            },
        })
    );
}

// 113: a check outside the identifier keeps the derivative inline while the
// source still hoists, in either root order.
#[test]
fn keeps_a_checked_derivative_distinct_from_its_identified_source() {
    let base = identified(string(), "Text");
    let refined = Schema::checked(base.clone(), "isMinLength", vec!["1"], vec![]);
    let forward = to_representations(&[base.clone(), refined.clone()]).expect("converts");
    let reversed = to_representations(&[refined.clone(), base.clone()]).expect("converts");
    let inline = json!({
        "_tag": "String",
        "annotations": { "identifier": "Text" },
        "checks": [{
            "_tag": "Filter",
            "representation": {
                "id": "effect/schema/isMinLength",
                "payload": { "minLength": 1 },
            },
            "aborted": false,
        }],
    });
    assert_eq!(
        serde_json::to_value(&forward.representations).expect("json"),
        json!([{ "_tag": "Reference", "$ref": "Text" }, inline])
    );
    assert_eq!(
        serde_json::to_value(&reversed.representations).expect("json"),
        json!([inline, { "_tag": "Reference", "$ref": "Text" }])
    );
    assert_eq!(forward.references, reversed.references);
    assert_eq!(
        serde_json::to_value(&forward.references).expect("json"),
        json!({
            "Text": {
                "_tag": "String",
                "annotations": { "identifier": "Text" },
                "checks": [],
            },
        })
    );
}

// 362: different schemas sharing one identifier suffix in encounter order,
// with the stored bodies keeping their own descriptions.
#[test]
fn suffixes_different_schemas_with_the_same_identifier() {
    let first = Schema::annotated_with(
        string(),
        vec![
            ("identifier".to_string(), "Value".to_string()),
            ("description".to_string(), "first".to_string()),
        ],
    );
    let second = Schema::annotated_with(
        number(),
        vec![
            ("identifier".to_string(), "Value".to_string()),
            ("description".to_string(), "second".to_string()),
        ],
    );
    let documents = to_representations(&[first, second]).expect("converts");
    assert_eq!(
        serde_json::to_value(&documents.representations).expect("json"),
        json!([
            { "_tag": "Reference", "$ref": "Value" },
            { "_tag": "Reference", "$ref": "Value_1" },
        ])
    );
    assert_eq!(
        serde_json::to_value(&documents.references).expect("json"),
        json!({
            "Value": {
                "_tag": "String",
                "annotations": { "identifier": "Value", "description": "first" },
                "checks": [],
            },
            "Value_1": {
                "_tag": "Number",
                "annotations": { "identifier": "Value_1", "description": "second" },
                "checks": [],
            },
        })
    );
}

// 389: separately built schemas suffix even when their bodies match.
#[test]
fn suffixes_referentially_distinct_asts_with_equal_representations() {
    let first = identified(string(), "Value");
    let second = identified(string(), "Value");
    let documents = to_representations(&[first, second]).expect("converts");
    assert_eq!(
        serde_json::to_value(&documents.representations).expect("json"),
        json!([
            { "_tag": "Reference", "$ref": "Value" },
            { "_tag": "Reference", "$ref": "Value_1" },
        ])
    );
    assert_eq!(
        serde_json::to_value(&documents.references).expect("json"),
        json!({
            "Value": {
                "_tag": "String",
                "annotations": { "identifier": "Value" },
                "checks": [],
            },
            "Value_1": {
                "_tag": "String",
                "annotations": { "identifier": "Value_1" },
                "checks": [],
            },
        })
    );
}

// 416: fallback names reserve first, so the explicit identifier takes the
// next suffix in encounter order.
#[test]
fn suffixes_fallback_and_explicit_identifier_collisions_in_encounter_order() {
    let documents = to_representations(&[
        fallback(string(), "Person"),
        fallback(number(), "Person"),
        identified(boolean(), "PersonEncoded"),
    ])
    .expect("converts");
    assert_eq!(
        serde_json::to_value(&documents.representations).expect("json"),
        json!([
            { "_tag": "Reference", "$ref": "PersonEncoded" },
            { "_tag": "Reference", "$ref": "PersonEncoded_1" },
            { "_tag": "Reference", "$ref": "PersonEncoded_2" },
        ])
    );
    assert_eq!(
        serde_json::to_value(&documents.references).expect("json"),
        json!({
            "PersonEncoded": {
                "_tag": "String",
                "annotations": { "~identifier": "Person" },
                "checks": [],
            },
            "PersonEncoded_1": {
                "_tag": "Number",
                "annotations": { "~identifier": "Person" },
                "checks": [],
            },
            "PersonEncoded_2": {
                "_tag": "Boolean",
                "annotations": { "identifier": "PersonEncoded_2" },
                "checks": [],
            },
        })
    );
}
