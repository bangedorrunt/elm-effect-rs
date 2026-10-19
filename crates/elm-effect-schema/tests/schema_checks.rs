//! `Schema.test.ts` "Checks" rows: named checks in a chain, an aborting check,
//! a filter group, and an `isMultipleOf` with an invalid divisor.
//!
//!     cargo test -p elm-effect-schema --test schema_checks

use elm_effect_schema::checks;
use elm_effect_schema::{Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// Every message a failure carries, one per collected issue.
fn messages(error: &SchemaError) -> Vec<String> {
    match error {
        SchemaError::FilterFailed { message, .. } => vec![message.clone()],
        SchemaError::Composite { issues } => issues.iter().flat_map(messages).collect(),
        other => vec![other.to_string()],
    }
}

/// `Schema.test.ts:1318` — "multiple checks".
#[test]
fn check_runs_every_named_check_in_order() {
    let chain = || vec![checks::is_min_length(3), checks::is_including("c")];

    let first_only = Schema::filtered(string(), chain(), false);
    assert!(first_only.decode(text("abc")).is_ok());
    assert_eq!(
        messages(&first_only.decode(text("ab")).unwrap_err()),
        vec!["Expected a value with a length of at least 3"],
        "the first failing check is the failure unless every one is collected"
    );

    let all = Schema::filtered(string(), chain(), true);
    assert_eq!(
        messages(&all.decode(text("ab")).unwrap_err()),
        vec![
            "Expected a value with a length of at least 3",
            "Expected a string including \"c\"",
        ],
        "collecting every check reports them in order"
    );
}

/// `Schema.test.ts:1339` — "aborting checks".
#[test]
fn an_aborting_named_check_stops_the_chain() {
    let schema = Schema::filtered(
        string(),
        vec![
            checks::is_min_length(2).aborting(),
            checks::is_including("b"),
        ],
        true,
    );

    assert_eq!(
        messages(&schema.decode(text("a")).unwrap_err()),
        vec!["Expected a value with a length of at least 2"],
        "an aborting check stops even a collecting chain"
    );
}

/// `Schema.test.ts:1353` — "makeFilterGroup".
#[test]
fn a_filter_group_is_a_chain_of_its_members() {
    let group = checks::make_filter_group(
        vec![
            checks::is_min_length(3),
            checks::is_pattern("^[a-zA-Z0-9]+$"),
            checks::is_trimmed(),
        ],
        "username",
        "a valid username",
    );
    let schema = Schema::filtered(string(), group, false);

    assert!(schema.decode(text("abc")).is_ok());
    assert_eq!(
        messages(&schema.decode(text("")).unwrap_err()),
        vec!["Expected a value with a length of at least 3"],
        "the group reports its first failing member"
    );
    assert_eq!(
        messages(&schema.decode(text("a b")).unwrap_err()),
        vec!["Expected a string matching the RegExp ^[a-zA-Z0-9]+$"],
        "a member past the length check still decides"
    );
}

/// `Schema.test.ts:1677` — "isMultipleOf rejects invalid divisors".
#[test]
fn is_multiple_of_rejects_invalid_divisors() {
    let invalid = [
        (0.0, "0"),
        (-0.0, "0"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
    ];
    for (divisor, printed) in invalid {
        let panicked = std::panic::catch_unwind(|| checks::is_multiple_of(divisor));
        let message = panicked
            .err()
            .and_then(|payload| {
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            })
            .unwrap_or_else(|| panic!("{printed} must be refused"));
        assert_eq!(
            message,
            format!("Expected a finite non-zero number, got {printed}")
        );
    }

    // A valid divisor still builds a working check.
    let even = Schema::filtered(
        Schema::as_unknown(Schema::number()),
        vec![checks::is_multiple_of(2.0)],
        false,
    );
    assert!(even.decode(Unknown::Number(4.0)).is_ok());
    assert!(even.decode(Unknown::Number(3.0)).is_err());
}
