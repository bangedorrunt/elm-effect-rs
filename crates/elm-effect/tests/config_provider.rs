//! ConfigProvider.test.ts's `constantCase` family.
//!
//! `constantCase` maps a REQUEST path into the SCREAMING_SNAKE_CASE key an env
//! var carries, so `["constant.case"]` finds `CONSTANT_CASE`. Each case below is
//! one whole case: the source under test, the requested path, and the value that
//! must come back.

#[allow(dead_code, clippy::type_complexity)]
#[path = "../src/config_provider.rs"]
mod config_provider;

// The path-included config modules name `crate::Defect`, so the host has to
// provide it in the test crate's root namespace.
pub use elm_effect::Defect;

use config_provider::{Lookup, Provider};

fn s(text: &str) -> String {
    text.to_string()
}

/// A path is a list of segments, and Effect 4's `constantCase` cases pass ONE
/// segment that itself contains punctuation — `"constant.case"`, `"api-v2 xml"`,
/// `"field2Value"`. Splitting on `.` here would turn one key into two and the
/// lookup would miss, which is exactly what happened before this comment.
fn path(segment: &str) -> Vec<String> {
    vec![s(segment)]
}

#[test]
fn constant_case_converts_string_segments_to_config_case() {
    // :203 — the shape every case in this family follows.
    let provider = Provider::from_pairs([("CONSTANT_CASE".to_string(), "value1".to_string())]);
    let provider = provider.constant_case();
    assert_eq!(
        provider.get(&path("constant.case")).as_deref(),
        Some("value1")
    );
}

#[test]
fn constant_case_preserves_numeric_word_groups() {
    // :211 — a digit is its own word boundary, so `V2` stays one word while
    // `field2` and `Value` split.
    let provider = Provider::from_pairs([
        ("API_V2_XML".to_string(), "value1".to_string()),
        ("FIELD2_VALUE".to_string(), "value2".to_string()),
    ]);
    let provider = provider.constant_case();
    assert_eq!(provider.get(&path("api-v2 xml")).as_deref(), Some("value1"));
    assert_eq!(
        provider.get(&path("field2Value")).as_deref(),
        Some("value2")
    );
}

#[test]
fn or_else_preserves_each_operands_transformations() {
    // :113 — an `orElse` of two transformed providers keeps BOTH transforms:
    // the primary is constant-cased, the fallback is nested under a prefix.
    let primary_source =
        Provider::from_pairs([("DATABASE_HOST".to_string(), "from-env".to_string())]);
    let primary = primary_source.constant_case();
    let fallback_source = Provider::from_pairs([("APP.PORT".to_string(), "3000".to_string())]);
    let fallback = fallback_source.nested("APP");
    let joined = primary.or_else(&fallback);
    // The primary answers through its own transform.
    assert_eq!(
        joined.get(&path("databaseHost")).as_deref(),
        Some("from-env")
    );
    // The fallback answers through its own prefix, not the primary's transform.
    assert_eq!(joined.get(&path("PORT")).as_deref(), Some("3000"));
}
