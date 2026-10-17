//! TemplateLiteral rows: conversion, revival, compilation, record keys.
//!
//! Covers toRepresentation :59, fromRepresentation :151/:157/:168, and the
//! schema compiler :2400/:2985 — every row the missing first-class template
//! kind had excused.

use elm_effect_schema::json_schema::compile;
use elm_effect_schema::representation_doc::{
    from_json, from_representation, to_json, to_representation,
};
use elm_effect_schema::{Schema, SchemaKind, TemplatePart, Unknown};

fn anchor(text: &str) -> SchemaKind {
    SchemaKind::Literal(Unknown::Text(text.to_string()))
}

fn roundtrip(schema: &Schema<Unknown>) -> Schema<Unknown> {
    let document = to_representation(schema).expect("converts");
    from_representation(&document, &[]).expect("revives")
}

fn is_ok(schema: &Schema<Unknown>, value: Unknown) -> bool {
    schema.decode(value).is_ok()
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

#[test]
fn template_literal_converts_to_parts() {
    // toRepresentation :59 — parts convert in order with literal values kept.
    let schema = Schema::as_unknown(Schema::template_literal(vec![
        TemplatePart::Anchor("prefix-".to_string()),
        TemplatePart::AnyString,
        TemplatePart::Number,
    ]));
    assert_eq!(
        to_representation(&schema).expect("converts").representation,
        serde_json::json!({
            "_tag": "TemplateLiteral",
            "parts": [
                {"_tag": "Literal", "literal": "prefix-", "checks": []},
                {"_tag": "String", "checks": []},
                {"_tag": "Number", "checks": []}
            ],
            "checks": []
        })
    );
}

#[test]
fn template_literal_revives_and_validates() {
    // fromRepresentation :151 — the revived schema keeps prefix matches out.
    let schema = Schema::as_unknown(Schema::template_literal(vec![
        TemplatePart::Anchor("prefix-".to_string()),
        TemplatePart::AnyString,
    ]));
    let revived = roundtrip(&schema);
    assert!(is_ok(&revived, text("prefix-value")));
    assert!(!is_ok(&revived, text("value")));
    assert!(matches!(
        revived.kind(),
        elm_effect_schema::SchemaKind::TemplateLiteral(_)
    ));
}

#[test]
fn template_literal_revives_an_identified_part() {
    // fromRepresentation :157 — the identified String part survives.
    let part = Schema::annotated_with(
        Schema::as_unknown(Schema::string()),
        vec![("identifier", "Part")],
    );
    let schema = Schema::as_unknown(
        Schema::template_literal_from_kinds(vec![
            anchor("prefix-"),
            part.kind(),
        ])
        .expect("parts spell a template"),
    );
    let representation = to_representation(&schema).expect("converts");
    // The identified part hoists to a reference, exactly like any other
    // identified node; the union-of-literals case below pins the same shape.
    let parts = representation.representation["parts"]
        .as_array()
        .expect("parts array")
        .clone();
    assert_eq!(
        parts[1],
        serde_json::json!({"_tag": "Reference", "$ref": "Part"})
    );
    let revived = roundtrip(&schema);
    assert!(is_ok(&revived, text("prefix-value")));
    assert!(!is_ok(&revived, text("value")));
    // The pin asserts the schema AST keeps a String part; ours keeps the
    // identifier annotation on it (the representation hoists it instead).
    match revived.kind() {
        SchemaKind::TemplateLiteral(parts) => {
            assert!(matches!(
                &parts[1],
                SchemaKind::Annotated { inner, .. }
                    if matches!(**inner, SchemaKind::String)
            ));
        }
        other => panic!("expected a TemplateLiteral, got {other:?}"),
    }
}

#[test]
fn template_literal_revives_nested_identified_union_parts() {
    // fromRepresentation :168 — a union of identified literals as one part.
    let union = Schema::as_unknown(Schema::union_with_mode(
        vec![
            Schema::annotated_with(
                Schema::as_unknown(Schema::literal(Unknown::Text("a".to_string()))),
                vec![("identifier", "A")],
            ),
            Schema::annotated_with(
                Schema::as_unknown(Schema::literal(Unknown::Text("b".to_string()))),
                vec![("identifier", "B")],
            ),
        ],
        "anyOf",
    ));
    let part = Schema::annotated_with(union, vec![("identifier", "Part")]);
    let schema = Schema::as_unknown(
        Schema::template_literal_from_kinds(vec![anchor("prefix-"), part.kind()])
            .expect("parts spell a template"),
    );
    let revived = roundtrip(&schema);
    assert!(is_ok(&revived, text("prefix-a")));
    assert!(is_ok(&revived, text("prefix-b")));
    assert!(!is_ok(&revived, text("prefix-c")));
}

#[test]
fn template_literal_compiles_to_a_pattern() {
    // toJsonSchemaDocument :2400 — closed pattern plus description passthrough.
    let schema = Schema::as_unknown(Schema::template_literal(vec![
        TemplatePart::Anchor("a".to_string()),
        TemplatePart::AnyString,
    ]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({"type": "string", "pattern": "^a[\\s\\S]*?$"})
    );
    let annotated = Schema::annotated_with(schema, vec![("description", "a")]);
    assert_eq!(
        compile(&annotated),
        serde_json::json!({
            "type": "string",
            "pattern": "^a[\\s\\S]*?$",
            "description": "a"
        })
    );
}

#[test]
fn template_keyed_records_compile_to_pattern_properties() {
    // toJsonSchemaDocument :2985 — template keys become patternProperties.
    let record = Schema::as_unknown(Schema::record2(
        Schema::as_unknown(Schema::template_literal(vec![
            TemplatePart::Anchor("a".to_string()),
            TemplatePart::AnyString,
        ])),
        Schema::as_unknown(Schema::number()),
    ));
    assert_eq!(
        compile(&record),
        serde_json::json!({
            "type": "object",
            "patternProperties": {"^a[\\s\\S]*?$": {"type": "number"}},
            "additionalProperties": false
        })
    );
}

#[test]
fn template_documents_round_trip_through_json() {
    // The wire form survives JSON: to_json/from_json carry the parts.
    let schema = Schema::as_unknown(Schema::template_literal(vec![
        TemplatePart::Anchor("prefix-".to_string()),
        TemplatePart::AnyString,
    ]));
    let document = to_representation(&schema).expect("converts");
    let json = to_json(&document).expect("serializes");
    let back = elm_effect_schema::representation_doc::from_json(&json).expect("parses");
    assert_eq!(back.representation, document.representation);
}
