//! Slice 3: template-literal anchors with greedy backtracking.
//! Ports Schema.test.ts anchor rows by exact call.

use elm_effect_schema::{Schema, TemplatePart, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

#[test]
fn greedy_segmentation_at_repeated_anchors() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        TemplatePart::Anchor(":".to_string()),
        TemplatePart::AnyString,
    ]);
    assert_eq!(
        schema.decode(text("a:b:c")),
        Ok(vec![text("a:b"), text(":"), text("c")])
    );
}

#[test]
fn backtracks_from_an_invalid_literal_anchor() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        TemplatePart::Anchor(":".to_string()),
        TemplatePart::NonEmptyString,
        TemplatePart::Anchor("x".to_string()),
    ]);
    assert_eq!(
        schema.decode(text("a:b:x")),
        Ok(vec![text("a"), text(":"), text("b:"), text("x")])
    );
}

#[test]
fn backtracks_across_an_empty_literal_anchor() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        TemplatePart::Anchor(String::new()),
        TemplatePart::NonEmptyString,
    ]);
    assert_eq!(
        schema.decode(text("a")),
        Ok(vec![text(""), text(""), text("a")])
    );
}

#[test]
fn adjacent_spans_split_greedily() {
    let schema = Schema::template(vec![
        TemplatePart::NonEmptyString,
        TemplatePart::AnyString,
    ]);
    assert_eq!(schema.decode(text("a")), Ok(vec![text("a"), text("")]));
}

#[test]
fn single_anchor_matches_whole() {
    let schema = Schema::template(vec![TemplatePart::Anchor("a".to_string())]);
    assert_eq!(schema.decode(text("a")), Ok(vec![text("a")]));
    assert!(schema.decode(text("b")).is_err());
}

#[test]
fn anchors_with_spaces_roundtrip() {
    let schema = Schema::template(vec![
        TemplatePart::Anchor("a".to_string()),
        TemplatePart::Anchor(" ".to_string()),
        TemplatePart::Anchor("b".to_string()),
    ]);
    let captures = vec![text("a"), text(" "), text("b")];
    assert_eq!(schema.decode(text("a b")), Ok(captures.clone()));
    assert_eq!(schema.encode(&captures), text("a b"));
}

#[test]
fn int_span_captures_integer_prefix() {
    let schema = Schema::template(vec![
        TemplatePart::Int,
        TemplatePart::Anchor("a".to_string()),
    ]);
    assert_eq!(
        schema.decode(text("1a")),
        Ok(vec![Unknown::Number(1.0), text("a")])
    );
    assert!(schema.decode(text("a")).is_err());
    assert_eq!(schema.encode(&vec![Unknown::Number(1.0), text("a")]), text("1a"));
}
