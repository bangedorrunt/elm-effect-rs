//! `resolveAnnotations` / `resolveAnnotationsKey` (Schema.test.ts :10690-:10740).
//!
//! The pin reads two slots: `resolveAnnotations` answers with the LAST check's
//! annotations when a schema has checks and with the node's own otherwise, and
//! `resolveAnnotationsKey` answers from the AST's `context` slot, which is what
//! `annotateKey` writes.

use std::collections::BTreeMap;

use elm_effect_schema::{Field, Schema, Unknown};

#[test]
fn resolve_annotations_is_none_for_a_schema_without_annotations() {
    // :10713 — `Schema.String` carries no annotations, so the answer is
    // undefined.
    assert_eq!(Schema::string().resolve_annotations(), None);
}

#[test]
fn resolve_annotations_returns_the_base_annotations() {
    // :10717 — an annotated schema answers with its own pairs.
    let schema = Schema::annotated(Schema::string(), Some("my string"), None);
    assert_eq!(
        schema.resolve_annotations(),
        Some(vec![("title".to_string(), "my string".to_string())])
    );
}

#[test]
fn resolve_annotations_returns_the_last_checks_annotations() {
    // :10722 — a base annotated `title: base` with a check annotated
    // `title: check` resolves to the check's, because a schema that has checks
    // never consults the base.
    let base = Schema::annotated(Schema::string(), Some("base"), None);
    let checked = Schema::checked(
        Schema::as_unknown(base),
        "isNonEmpty",
        vec![],
        vec![("title", "check")],
    );
    assert_eq!(
        checked.resolve_annotations(),
        Some(vec![("title".to_string(), "check".to_string())])
    );
}

#[test]
fn a_check_without_annotations_hides_the_base_annotations() {
    // The same rule read the other way: the pin's `resolve` is
    // `ast.checks ? checks[last].annotations : ast.annotations`, so an
    // unannotated check answers undefined even though the base is annotated.
    let base = Schema::annotated(Schema::string(), Some("base"), None);
    let checked = Schema::checked(Schema::as_unknown(base), "isNonEmpty", vec![], vec![]);
    assert_eq!(checked.resolve_annotations(), None);
}

#[test]
fn the_last_check_wins_over_an_earlier_one() {
    // "Last" is the most recently applied check, which is the outermost one.
    let base = Schema::as_unknown(Schema::string());
    let first = Schema::checked(base, "isNonEmpty", vec![], vec![("title", "first")]);
    let second = Schema::checked(first, "isMaxLength", vec!["3"], vec![("title", "second")]);
    assert_eq!(
        second.resolve_annotations(),
        Some(vec![("title".to_string(), "second".to_string())])
    );
}

#[test]
fn resolve_annotations_key_is_none_without_key_annotations() {
    // :10731.
    assert_eq!(Schema::string().resolve_annotations_key(), None);
}

#[test]
fn resolve_annotations_key_returns_the_key_annotations() {
    // :10735.
    let schema = Schema::annotate_key(Schema::string(), vec![("messageMissingKey", "required")]);
    assert_eq!(
        schema.resolve_annotations_key(),
        Some(vec![(
            "messageMissingKey".to_string(),
            "required".to_string()
        )])
    );
}

#[test]
fn key_annotations_survive_the_re_typing_a_struct_field_goes_through() {
    // Every struct field is re-typed to `Unknown`, and the key annotation is
    // read at that occurrence, so the re-typing must carry it.
    let field = Schema::annotate_key(Schema::string(), vec![("messageMissingKey", "required")]);
    let re_typed = Schema::as_unknown(field);
    assert_eq!(
        re_typed.resolve_annotations_key(),
        Some(vec![(
            "messageMissingKey".to_string(),
            "required".to_string()
        )])
    );
    // ...and the key slot is separate from the node's own annotations: the
    // node still carries none, and the re-typing kept the string codec.
    assert_eq!(re_typed.resolve_annotations(), None);
    assert_eq!(
        re_typed.decode(Unknown::Text("a".to_string())),
        Ok(Unknown::Text("a".to_string()))
    );
}

/// Schema.test.ts:8746 — the `messageMissingKey` annotation is the error message
/// for the key it guards, at the field's own path.
#[test]
fn a_missing_key_reports_the_message_its_key_schema_annotated() {
    let guarded = Schema::annotate_key(
        Schema::string(),
        vec![("messageMissingKey", "this field is required")],
    );
    let schema = Schema::struct_shape(vec![Field::required("a", Schema::as_unknown(guarded))]);
    let error = schema.decode(Unknown::Object(BTreeMap::new())).unwrap_err();
    assert_eq!(error.to_string(), "this field is required at $.a");
    assert!(matches!(
        error,
        elm_effect_schema::SchemaError::MissingKey { .. }
    ));

    // The control: without the annotation the port's own refusal stands, and it
    // is a different shape of message (expected/got), which is the whole point
    // of the annotation replacing it.
    let bare = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    assert_eq!(
        bare.decode(Unknown::Object(BTreeMap::new()))
            .unwrap_err()
            .to_string(),
        "expected required field at $.a, got missing"
    );
}

/// The same row's Tuple half: the element guards its own position.
#[test]
fn a_missing_tuple_position_reports_its_element_annotated_message() {
    let element = Schema::annotate_key(
        Schema::string(),
        vec![("messageMissingKey", "this element is required")],
    );
    let schema = Schema::tuple_of(vec![Schema::as_unknown(element)]);
    assert_eq!(
        schema
            .decode(Unknown::Array(Vec::new()))
            .unwrap_err()
            .to_string(),
        "this element is required at $[0]"
    );

    // A nested occurrence keeps the outer path and the annotated message.
    let outer = Schema::struct_shape(vec![Field::required(
        "inner",
        Schema::as_unknown(Schema::tuple_of(vec![Schema::as_unknown(
            Schema::annotate_key(
                Schema::string(),
                vec![("messageMissingKey", "this element is required")],
            ),
        )])),
    )]);
    assert_eq!(
        outer
            .decode(Unknown::Object(BTreeMap::from([(
                "inner".to_string(),
                Unknown::Array(Vec::new()),
            )])))
            .unwrap_err()
            .to_string(),
        "this element is required at $.inner[0]"
    );
}
