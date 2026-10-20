//! `Schema.test.ts` "Checks" rows: named checks in a chain, an aborting check,
//! a filter group, an `isMultipleOf` with an invalid divisor, and the two
//! string-format checks (`isStringBigInt`, `isStringSymbol`).
//!
//!     cargo test -p elm-effect-schema --test schema_named_checks

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

/// `Schema.test.ts:2298` — "FiniteFromString & isGreaterThan": the check lands
/// on the number the transform produced, in both directions, so the text `"1"`
/// is refused as a number and not as a string.
#[test]
fn a_check_on_a_transforming_schema_reads_the_decoded_number() {
    let schema = Schema::<Unknown>::checked(
        Schema::as_unknown(Schema::finite_from_string()),
        "isGreaterThan",
        vec!["2"],
        vec![("expected", "a value greater than 2")],
    );

    assert_eq!(schema.decode(text("3")).expect("decodes"), text("3"));
    // The refusal names the check's own expected text AND reports the value as
    // a number, which is what proves the check read the transform's output.
    let refused = messages(&schema.decode(text("1")).unwrap_err());
    assert_eq!(refused.len(), 1);
    assert!(refused[0].contains("a value greater than 2"), "{refused:?}");
    assert!(refused[0].ends_with("got number"), "{refused:?}");

    // The encoding direction is checked too, against the value's own number.
    assert_eq!(schema.try_encode(&text("3")).expect("encodes"), text("3"));
    let refused = messages(&schema.try_encode(&text("1")).unwrap_err());
    assert!(refused[0].contains("a value greater than 2"), "{refused:?}");
    assert!(refused[0].ends_with("got number"), "{refused:?}");
}

/// `Schema.test.ts:10319` — "isStringBigInt". The row itself asserts the pin's
/// representation id, which this port does not carry; what is asserted here is
/// the predicate that id stands for, including the two edges the annotation
/// cannot show: no leading `+`, and no limit on size.
#[test]
fn is_string_big_int_matches_signed_digits_only() {
    let schema = Schema::filtered(string(), vec![checks::is_string_big_int()], false);

    for accepted in ["0", "123", "-7", "1234567890123456789012345678901234567890"] {
        assert!(schema.decode(text(accepted)).is_ok(), "{accepted}");
    }
    for refused in ["", "-", "+1", "1.0", "1e3", " 1", "1 ", "0x10", "abc"] {
        assert_eq!(
            messages(&schema.decode(text(refused)).unwrap_err()),
            vec!["Expected a string representing a bigint"],
            "{refused:?}"
        );
    }
}

/// `Schema.test.ts:10328` — "isStringSymbol". Same reading as the row above:
/// the shape `Symbol(description)` is what is checked, and the inner text runs
/// to the last `)` and may be empty or span lines.
#[test]
fn is_string_symbol_matches_the_symbol_spelling() {
    let schema = Schema::filtered(string(), vec![checks::is_string_symbol()], false);

    for accepted in ["Symbol()", "Symbol(x)", "Symbol(a\nb)", "Symbol(a))"] {
        assert!(schema.decode(text(accepted)).is_ok(), "{accepted:?}");
    }
    for refused in [
        "",
        "Symbol(",
        "Symbol(a",
        "Symbol(a)x",
        "xSymbol(a)",
        "symbol(a)",
    ] {
        assert_eq!(
            messages(&schema.decode(text(refused)).unwrap_err()),
            vec!["Expected a string representing a symbol"],
            "{refused:?}"
        );
    }
}
