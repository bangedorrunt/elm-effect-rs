//! A check on top of a transform reads what the transform produced: the pin's
//! `check` lands on the target side of a link, so
//! `FiniteFromString.check(isInt())` refuses `"1.2"` rather than reading the
//! text it arrived as.
//!
//!     cargo test -p elm-effect-schema --test schema_checked_transform
//!
//! The port expresses the projection as `check_view` on the transform, applied
//! by `Schema::checked` (and the filter chain) on both axes. This file is the
//! mechanism's own fence: the rows it unblocks are witnessed where they belong
//! (`schema_redacted.rs` for the boxed rows).

use elm_effect_schema::{Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// `Schema.FiniteFromString.check(Schema.isInt())` as a struct field is
/// written: the typed transform, re-typed for the shared field type.
fn int_from_string() -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::finite_from_string()),
        "isInt",
        Vec::new(),
        vec![("expected", "an integer")],
    )
}

/// The `(path, expected)` pair a refusal carries.
fn refusal(error: &SchemaError) -> (String, String) {
    match error {
        SchemaError::TypeMismatch { path, expected, .. } => (path.clone(), (*expected).to_string()),
        SchemaError::TypeMismatchDynamic { path, expected, .. } => (path.clone(), expected.clone()),
        other => panic!("not a shape refusal: {other}"),
    }
}

/// The check reads the number the transform produced, not the text it read.
#[test]
fn a_check_reads_what_the_transform_produced() {
    let schema = int_from_string();

    assert_eq!(schema.decode(text("123")).unwrap(), text("123"));
    assert_eq!(
        refusal(&schema.decode(text("1.2")).unwrap_err()),
        ("$".to_string(), "an integer".to_string()),
        "1.2 is a number this check has to refuse, and it can only see it as one"
    );
    // A value the transform cannot read at all is still the transform's own
    // refusal, not the check's.
    assert_eq!(
        refusal(&schema.decode(text("a")).unwrap_err()),
        ("$".to_string(), "a finite number string".to_string())
    );
}

/// The same rule holds on the way out: the wire value a caller hands in is
/// projected before the check sees it.
#[test]
fn the_encode_side_reads_the_same_value() {
    let schema = int_from_string();

    assert_eq!(schema.try_encode(&text("123")).unwrap(), text("123"));
    assert_eq!(
        refusal(&schema.try_encode(&text("1.2")).unwrap_err()),
        ("$".to_string(), "an integer".to_string())
    );
}

/// A second check stacked on the first reads the same produced value, because a
/// refinement of a transform is still a refinement of that transform.
///
/// The port's stack is a chain of wrappers, so the outermost check runs first
/// (the pin appends both to one list and runs the first-added first); the
/// witness picks values that tell the two checks apart without depending on
/// that order.
#[test]
fn a_stacked_check_reads_the_same_value() {
    let schema = Schema::checked(
        int_from_string(),
        "isGreaterThan",
        vec!["100"],
        vec![("expected", "a value greater than 100")],
    );

    assert_eq!(schema.decode(text("123")).unwrap(), text("123"));
    // Below the bound: the outer check refuses it, reading the number.
    assert_eq!(
        refusal(&schema.decode(text("50")).unwrap_err()),
        ("$".to_string(), "a value greater than 100".to_string())
    );
    // Above the bound but not whole: the outer check passes it and the inner
    // one refuses it, which it can only do by reading the number.
    assert_eq!(
        refusal(&schema.decode(text("100.5")).unwrap_err()),
        ("$".to_string(), "an integer".to_string())
    );
}

/// `NumberFromString` reads a wider set of spellings, and its check view reads
/// the same numbers its decode does — including the non-finite ones, which no
/// check over the text could decide.
#[test]
fn a_wider_transform_projects_the_same_way() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::number_from_string()),
        "isInt",
        Vec::new(),
        vec![("expected", "an integer")],
    );

    assert_eq!(schema.decode(text("3")).unwrap(), text("3"));
    for not_whole in ["3.5", "Infinity", "-Infinity", "NaN"] {
        assert_eq!(
            refusal(&schema.decode(text(not_whole)).unwrap_err()),
            ("$".to_string(), "an integer".to_string()),
            "{not_whole}"
        );
    }
}

/// A schema whose decoded value is its wire value projects nothing, so a check
/// reads what arrived — the behaviour every check had before the projection
/// existed, kept here so the two rules stay distinguishable.
#[test]
fn a_plain_schema_reads_the_wire_value() {
    let plain = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isInt",
        Vec::new(),
        vec![("expected", "an integer")],
    );
    assert_eq!(
        plain.decode(Unknown::Number(2.0)).unwrap(),
        Unknown::Number(2.0)
    );
    assert_eq!(
        refusal(&plain.decode(Unknown::Number(1.5)).unwrap_err()),
        ("$".to_string(), "an integer".to_string())
    );
    // A text value is not projected anywhere: the check cannot decide it, and
    // the base schema is what refuses it.
    assert_eq!(
        refusal(&plain.decode(text("1")).unwrap_err()),
        ("$".to_string(), "number".to_string())
    );
}
