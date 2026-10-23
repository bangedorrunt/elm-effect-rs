//! `SchemaParser` parse options: the pin's second argument to every adapter.
//!
//! `ParseOptions` decides how many issues a parse reports (`errors`), what
//! happens to keys the schema does not model (`onExcessProperty`), whether
//! refinements run (`disableChecks`), and whether a refusal carries the value it
//! rejected (`reportInput`). The pin passes it at creation
//! (`decodeUnknownExit(schema, options)`) and per call
//! (`parser(input, override)`); here the bound form is `Decoder`/`Encoder` and
//! the per-call form is the `_with` entry points.

#![cfg(feature = "schema")]

use std::collections::BTreeMap;

use elm_effect::{
    AdapterFailure, Exit, decode_unknown_exit, decode_unknown_exit_with,
    decode_unknown_option_with, decode_unknown_sync_with, encode_unknown_sync_with,
};
use elm_effect_schema::transform::{Errors, ParseOptions};
use elm_effect_schema::{ExcessProperty, Field, Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn object(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    )
}

/// One option set, spelled once so each witness reads as a decision.
fn options(errors: Errors, on_excess_property: ExcessProperty) -> ParseOptions {
    ParseOptions {
        errors,
        on_excess_property,
        ..ParseOptions::default()
    }
}

/// Two required fields, so "how many issues" has an answer bigger than one.
fn two_required() -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(Schema::string())),
    ])
}

fn paths(error: &SchemaError) -> Vec<String> {
    match error {
        SchemaError::Composite { issues } => issues.iter().map(ToString::to_string).collect(),
        other => vec![other.to_string()],
    }
}

/// The pin's defaults: first issue, excess keys dropped, checks running, no
/// reported input.
#[test]
fn parser_options_default_to_first_and_ignore() {
    let defaults = ParseOptions::default();
    assert_eq!(defaults.errors, Errors::First);
    assert_eq!(defaults.on_excess_property, ExcessProperty::Ignore);
    assert!(!defaults.disable_checks);
    assert!(!defaults.report_input);

    // A call that names no options is the default parse: one issue, the first.
    let undeclared = decode_unknown_exit(&two_required(), object(&[]))
        .into_result()
        .expect("a bare issue is the result adapter's own")
        .unwrap_err();
    let messages = paths(&undeclared);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("at $.a"), "{messages:?}");
}

/// `errors: "all"` reaches the facade: both misses, from one call.
#[test]
fn parser_options_all_reports_every_issue() {
    let error = decode_unknown_exit_with(
        &two_required(),
        object(&[]),
        options(Errors::All, ExcessProperty::Ignore),
    )
    .into_result()
    .expect("a bare issue is the result adapter's own")
    .unwrap_err();

    let messages = paths(&error);
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("at $.a"), "{messages:?}");
    assert!(messages[1].contains("at $.b"), "{messages:?}");
}

/// `disableChecks` skips the refinement and still applies the parser's own
/// work: a decoding default fills, a check does not run.
#[test]
fn parser_options_disable_checks_skips_refinements() {
    let checked = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isInt",
        Vec::new(),
        Vec::new(),
    );
    assert!(
        decode_unknown_exit(&checked, number(1.5))
            .into_result()
            .expect("a check failure is a bare issue")
            .is_err(),
        "the check runs by default"
    );
    assert!(
        matches!(
            decode_unknown_exit_with(
                &checked,
                number(1.5),
                ParseOptions {
                    disable_checks: true,
                    ..ParseOptions::default()
                },
            ),
            Exit::Success(Unknown::Number(value)) if value == 1.5
        ),
        "disableChecks takes the check out of the parse"
    );

    // Defaults are not checks: the field still fills under disableChecks.
    let defaulted = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::with_decoding_default(Schema::string(), || {
            Ok(text("filled"))
        })),
    )]);
    let decoded = decode_unknown_exit_with(
        &defaulted,
        object(&[]),
        ParseOptions {
            disable_checks: true,
            ..ParseOptions::default()
        },
    );
    assert!(
        matches!(decoded, Exit::Success(value) if value.get("a") == Some(&text("filled"))),
        "a decoding default is not a check"
    );
}

/// `reportInput` puts the rejected value on the issue; without it the issue
/// names only the expectation.
#[test]
fn parser_options_report_input_carries_the_rejected_value() {
    let refused = |options: ParseOptions| match decode_unknown_sync_with(
        &Schema::string(),
        number(1.0),
        options,
    ) {
        Err(AdapterFailure::Issue(error)) => error,
        other => panic!("expected a schema issue, got {other:?}"),
    };

    let plain = refused(ParseOptions::default());
    assert_eq!(plain.input(), None);
    assert_eq!(
        plain.to_string(),
        "expected string at $, got number",
        "the input is data, not message text"
    );

    let reported = refused(ParseOptions {
        report_input: true,
        ..ParseOptions::default()
    });
    assert_eq!(reported.input(), Some(&number(1.0)));
    assert!(
        matches!(reported, SchemaError::WithInput { .. }),
        "the carrier is the issue type itself, so a caller can match on it"
    );
}

/// The options given to one call do not outlive it: the thread-local scope is
/// restored, so the next call is back on the defaults.
#[test]
fn parser_options_scope_to_one_call() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let input = || object(&[("a", text("a")), ("b", text("b"))]);
    let strict = options(Errors::First, ExcessProperty::Reject);

    assert!(
        decode_unknown_sync_with(&schema, input(), strict).is_err(),
        "the strict call refuses the undeclared key"
    );
    assert!(
        decode_unknown_sync_with(&schema, input(), ParseOptions::default()).is_ok(),
        "the next call is back on the default"
    );
}

/// The carriers that can hand back only a bare issue answer none for an option
/// failure; the option adapter is not allowed to widen what it can carry.
#[test]
fn parser_options_reach_the_option_carrier() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let input = object(&[("a", text("a")), ("b", text("b"))]);

    let refused = decode_unknown_option_with(
        &schema,
        input.clone(),
        options(Errors::First, ExcessProperty::Reject),
    );
    assert_eq!(
        refused.expect("an excess key is a schema issue"),
        None,
        "the option adapter answers none for a bare issue"
    );
    assert!(
        decode_unknown_option_with(&schema, input, ParseOptions::default())
            .expect("the key is dropped")
            .is_some_and(|value| value.get("b").is_none()),
        "the default drops the undeclared key"
    );
}

/// Encoding reads the same options: strict refuses an undeclared key, the
/// default drops it. The two axes cannot disagree about one option.
#[test]
fn parser_options_reach_the_encoder() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let value: BTreeMap<String, Unknown> = [
        ("a".to_string(), text("a")),
        ("b".to_string(), text("b")),
    ]
    .into_iter()
    .collect();

    assert!(
        encode_unknown_sync_with(
            &schema,
            &value,
            options(Errors::First, ExcessProperty::Reject)
        )
        .is_err(),
        "encoding refuses the undeclared key under error"
    );
    let encoded = encode_unknown_sync_with(&schema, &value, ParseOptions::default())
        .expect("the default drops it");
    assert!(
        matches!(&encoded, Unknown::Object(map) if map.get("b").is_none()),
        "the undeclared key did not travel back out"
    );
}

/// The effect adapter carries the same options as the sync one, and is still
/// the answer for a schema whose steps are closures.
#[cfg(feature = "tokio")]
#[test]
fn parser_options_reach_the_effect_adapter() {
    use elm_effect::{Runtime, decode_unknown_effect_with};

    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let input = object(&[("a", text("a")), ("b", text("b"))]);

    let exit = Runtime::run(decode_unknown_effect_with(
        &schema,
        input,
        options(Errors::First, ExcessProperty::Reject),
    ));
    match exit {
        Exit::Failure(cause) => {
            assert!(
                cause.to_string().contains("no excess property"),
                "{cause}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}
