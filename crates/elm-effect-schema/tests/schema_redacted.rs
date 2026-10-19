//! `Schema.test.ts` boxes: `Redacted`, `RedactedFromValue` and the `declare`
//! primitive they sit on.
//!
//!     cargo test -p elm-effect-schema --test schema_redacted
//!
//! The pin boxes a value in a `Redacted` instance; a port whose values are all
//! wire values boxes it as `{ value }`, plus `label` when the schema declares
//! one. Four rows are carried here: `:2850` (`declare`), `:2863` (a box
//! exposes its schema), `:2869` (a raw value is not a box) and `:2970` (a
//! declared label has to match).
//!
//! Three rows of the same family are NOT carried, and the two reasons are worth
//! naming because they are port-wide rather than local to this file:
//!
//! - `:2880`/`:2917`/`:3008`: the pin's rows refuse a boxed value like
//!   `Redacted.make(1.2)` under `FiniteFromString.check(isInt())`, because its
//!   checks read the decoded value. This port's `checked` reads the value it was
//!   handed, so `isInt` cannot see through a transform: it passes a wire text
//!   through unenforced (`check_holds`'s unknown-name answer) instead of
//!   refusing it.
//! - the same rows refuse a boxed value of the wrong type on the *encoding*
//!   side. Here a re-typed schema (`as_unknown`) encodes by identity, so a box
//!   holding wire text is never type-checked on the way out; only checks and
//!   filters that run on the encode side (which is why the label row, whose
//!   wrapped schema is a filter chain, does cover it).
//!
//! `a_boxed_transform_decodes_inside_the_box` below shows exactly how far the
//! port gets, and is deliberately not cited by the ledger.

use elm_effect_schema::{checks, redacted_value, Schema, SchemaError, Unknown};
use std::collections::BTreeMap;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

/// A box: the shape `Redacted` values have here.
fn boxed(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// The `(path, expected)` pair a refusal carries, which is the whole of what
/// the pin prints once its own layout is applied.
fn refusal(error: &SchemaError) -> (String, String) {
    match error {
        SchemaError::TypeMismatch { path, expected, .. } => (path.clone(), (*expected).to_string()),
        SchemaError::TypeMismatchDynamic { path, expected, .. } => (path.clone(), expected.clone()),
        other => panic!("not a shape refusal: {other}"),
    }
}

/// `Schema.NonEmptyString`: the label row's wrapped schema, and the one wrapped
/// schema whose checks do run on both axes (a filter chain, not a re-typing).
fn non_empty_string() -> Schema<Unknown> {
    Schema::filtered(
        Schema::as_unknown(Schema::string()),
        vec![checks::is_min_length(1)],
        false,
    )
}

/// Schema.test.ts:2850 — "declare". The pin's row declares a host class
/// (`u instanceof File`), which no wire value can be here; the witness proves
/// the shape with a predicate it can state, and the refusal text is still the
/// caller's own.
#[test]
fn declare_admits_what_its_predicate_admits() {
    let schema = Schema::declare(
        |value| matches!(value, Unknown::Object(map) if map.contains_key("payload")),
        "File",
    );

    let admitted = boxed(vec![("payload", text("x"))]);
    assert_eq!(schema.decode(admitted.clone()).unwrap(), admitted);
    assert_eq!(
        refusal(&schema.decode(text("a")).unwrap_err()),
        ("$".to_string(), "File".to_string())
    );
    assert_eq!(
        refusal(&schema.decode(Unknown::Null).unwrap_err()),
        ("$".to_string(), "File".to_string())
    );
    // `declare` has no encode side of its own: the value passes through.
    assert_eq!(schema.try_encode(&admitted).unwrap(), admitted);
}

/// Schema.test.ts:2863 — "should expose the value". The wrapped schema is
/// readable, and an annotation over the box keeps it readable.
#[test]
fn a_box_exposes_the_schema_it_wraps() {
    let schema = Schema::redacted(Schema::as_unknown(Schema::string()), None);
    let inner = redacted_value(&schema).expect("a box exposes its wrapped schema");
    assert_eq!(inner.to_code(), "Schema.String");
    assert!(inner.decode(text("a")).is_ok());
    assert!(inner.decode(number(1.0)).is_err());

    let annotated = Schema::annotated_with(schema.clone(), vec![("title", "a secret")]);
    let inner = redacted_value(&annotated).expect("an annotation keeps the wrapped schema");
    assert_eq!(inner.to_code(), "Schema.String");
}

/// Schema.test.ts:2869 — "does not expose unwrapped inputs". A raw value is
/// not a box, on either axis, and the refusal names the box rather than the
/// schema inside it.
#[test]
fn a_raw_value_is_not_a_box() {
    let schema = Schema::redacted(Schema::as_unknown(Schema::string()), None);

    assert_eq!(
        refusal(&schema.decode(text("secret")).unwrap_err()),
        ("$".to_string(), "Redacted".to_string())
    );
    assert_eq!(
        refusal(&schema.try_encode(&text("secret")).unwrap_err()),
        ("$".to_string(), "Redacted".to_string())
    );
}

/// Schema.test.ts:2970 — "with label". The declared label is checked before
/// the value is, so a wrong label is reported as the label whatever the value
/// is, and the expected side is the literal's own spelling, quotes included.
#[test]
fn a_declared_label_has_to_match() {
    let schema = Schema::redacted(non_empty_string(), Some("password"));

    let matched = boxed(vec![("value", text("a")), ("label", text("password"))]);
    assert_eq!(schema.decode(matched.clone()).unwrap(), matched);

    for wrong in [
        boxed(vec![("value", text("a")), ("label", text("API key"))]),
        boxed(vec![("value", number(1.0)), ("label", text("API key"))]),
        // A box with no label at all is a wrong label, not a missing one: the
        // pin decodes `undefined` through `Literal("password")`.
        boxed(vec![("value", text("a"))]),
    ] {
        assert_eq!(
            refusal(&schema.decode(wrong.clone()).unwrap_err()),
            ("$.label".to_string(), "\"password\"".to_string()),
            "{wrong:?}"
        );
        assert_eq!(
            refusal(&schema.try_encode(&wrong).unwrap_err()),
            ("$.label".to_string(), "\"password\"".to_string()),
            "{wrong:?}"
        );
    }

    // The right label with a value the wrapped schema refuses is the value's
    // failure, not the label's — and a filter chain does run on encode, so
    // both axes refuse the empty string.
    assert_eq!(
        refusal(
            &schema
                .decode(boxed(vec![("value", number(1.0)), ("label", text("password"))]))
                .unwrap_err()
        ),
        ("$.value".to_string(), "a valid value".to_string())
    );
    assert_eq!(
        refusal(
            &schema
                .try_encode(&boxed(vec![("value", text("")), ("label", text("password"))]))
                .unwrap_err()
        ),
        ("$.value".to_string(), "a valid value".to_string())
    );
}

/// The box plumbing over a transforming wrapped schema, which is what the
/// unported `:2917`/`:3008` rows are about: the box is rebuilt around the
/// decoded value, a carried label rides through, and the pin's own two
/// messages are used at the box. What this does NOT show is the transformed
/// *refusal* — see the module note.
#[test]
fn a_boxed_transform_keeps_the_box_and_the_label() {
    let int_from_string = Schema::checked(
        Schema::as_unknown(Schema::finite_from_string()),
        "isInt",
        Vec::new(),
        vec![("expected", "an integer")],
    );
    let schema = Schema::redacted(int_from_string.clone(), None);

    assert_eq!(
        schema.decode(boxed(vec![("value", text("123"))])).unwrap(),
        boxed(vec![("value", text("123"))])
    );
    assert_eq!(
        schema
            .decode(boxed(vec![("value", text("123")), ("label", text("secret"))]))
            .unwrap(),
        boxed(vec![("value", text("123")), ("label", text("secret"))])
    );
    assert_eq!(
        refusal(&schema.decode(Unknown::Null).unwrap_err()),
        ("$".to_string(), "Redacted".to_string())
    );
    // A boxed value the wrapped schema cannot read at all is the box's
    // failure, reported at the box.
    assert_eq!(
        refusal(&schema.decode(boxed(vec![("value", Unknown::Null)])).unwrap_err()),
        ("$.value".to_string(), "a valid value".to_string())
    );
    assert!(
        redacted_value(&schema)
            .expect("a box exposes its wrapped schema")
            .decode(text("123"))
            .is_ok()
    );

    // The wrapped-value refusals the port cannot make are these, and they are
    // the ones the pin's rows assert: `"1.2"` (a check that cannot see through
    // the transform) and `"a"` (a type refusal on the encode side).
    assert!(
        schema.decode(boxed(vec![("value", text("1.2"))])).is_ok(),
        "the port's isInt reads wire text, so it cannot refuse this"
    );
    assert!(
        Schema::redacted(int_from_string, None)
            .try_encode(&boxed(vec![("value", text("a"))]))
            .is_ok(),
        "a re-typing encodes by identity, so no type check happens there"
    );
}
