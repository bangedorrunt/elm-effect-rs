//! `Schema.test.ts` boxes: `Redacted`, `RedactedFromValue` and the `declare`
//! primitive they sit on.
//!
//!     cargo test -p elm-effect-schema --test schema_redacted
//!
//! The pin boxes a value in a `Redacted` instance; a port whose values are all
//! wire values boxes it as `{ value }`, plus `label` when the schema declares
//! one. Seven rows are carried here: `:2850` (`declare`), `:2863` (a box
//! exposes its schema), `:2869` (a raw value is not a box), `:2880` and `:2917`
//! (`Redacted` over a plain number and over a transform), `:2970` (a declared
//! label has to match) and `:3008` (`RedactedFromValue`).
//!
//! The pin passes *decoded* values into its boxes and asserts decoded values
//! back out (`Redacted.make(123)` holds a number). Here a box holds wire values
//! (`{ value: "123" }` for the same schema), which is this port's rule
//! everywhere: `Unknown` is the wire form. Every assertion below is the pin's
//! own, read at that level; where the pin's input is a decoded value, the wire
//! value the port would carry is what stands in its place.
//!
//! Two refusals the pin makes are now reachable and are asserted:
//! `"1.2"` under `FiniteFromString.check(isInt())` (the check reads the number
//! the transform produced, see `schema_checked_transform.rs`) and a box holding
//! a value the schema cannot read (the box validates what it holds on the way
//! out, not only on the way in).

use elm_effect_schema::{Schema, SchemaError, Unknown, checks, redacted_value};
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

/// `Schema.NonEmptyString`: the label row's wrapped schema.
fn non_empty_string() -> Schema<Unknown> {
    Schema::filtered(
        Schema::as_unknown(Schema::string()),
        vec![checks::is_min_length(1)],
        false,
    )
}

/// `Schema.Int`: a whole number, with the pin's own expected text.
fn int() -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isInt",
        Vec::new(),
        vec![("expected", "an integer")],
    )
}

/// `Schema.FiniteFromString.check(Schema.isInt())`: text in, whole number out,
/// which is the pair the boxed rows wrap.
fn int_from_string() -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::finite_from_string()),
        "isInt",
        Vec::new(),
        vec![("expected", "an integer")],
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
                .decode(boxed(vec![
                    ("value", number(1.0)),
                    ("label", text("password"))
                ]))
                .unwrap_err()
        ),
        ("$.value".to_string(), "a valid value".to_string())
    );
    assert_eq!(
        refusal(
            &schema
                .try_encode(&boxed(vec![
                    ("value", text("")),
                    ("label", text("password"))
                ]))
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
            .decode(boxed(vec![
                ("value", text("123")),
                ("label", text("secret"))
            ]))
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
        refusal(
            &schema
                .decode(boxed(vec![("value", Unknown::Null)]))
                .unwrap_err()
        ),
        ("$.value".to_string(), "a valid value".to_string())
    );
    assert!(
        redacted_value(&schema)
            .expect("a box exposes its wrapped schema")
            .decode(text("123"))
            .is_ok()
    );

    // A boxed value the wrapped check refuses, which it can only do by reading
    // the number the transform produced.
    assert_eq!(
        refusal(
            &schema
                .decode(boxed(vec![("value", text("1.2"))]))
                .unwrap_err()
        ),
        ("$.value".to_string(), "a valid value".to_string())
    );
}

/// Schema.test.ts:2880 — "Redacted(Finite)". A boxed value the wrapped schema
/// accepts passes; one it refuses — the wrong shape or a number that is not
/// whole — is reported at the box, on both axes.
#[test]
fn a_boxed_int_decodes_and_refuses_at_the_box() {
    let schema = Schema::redacted(int(), None);
    let good = boxed(vec![("value", number(123.0))]);

    assert_eq!(schema.decode(good.clone()).unwrap(), good);
    assert_eq!(schema.try_encode(&good).unwrap(), good);
    assert_eq!(
        refusal(&schema.decode(Unknown::Null).unwrap_err()),
        ("$".to_string(), "Redacted".to_string())
    );
    assert_eq!(
        refusal(&schema.try_encode(&Unknown::Null).unwrap_err()),
        ("$".to_string(), "Redacted".to_string())
    );

    for refused in [text("a"), number(1.2)] {
        let refused_box = boxed(vec![("value", refused.clone())]);
        assert_eq!(
            refusal(&schema.decode(refused_box.clone()).unwrap_err()),
            ("$.value".to_string(), "a valid value".to_string()),
            "decoding {refused:?}"
        );
        assert_eq!(
            refusal(&schema.try_encode(&refused_box).unwrap_err()),
            ("$.value".to_string(), "a valid value".to_string()),
            "encoding {refused:?}"
        );
    }
}

/// Schema.test.ts:2917 — "Redacted(FiniteFromString)". The wrapped transform
/// runs inside the box, the label the input carried comes back out with it, and
/// `"1.2"` is refused because the wrapped check reads the number.
#[test]
fn a_boxed_transform_decodes_inside_the_box() {
    let schema = Schema::redacted(int_from_string(), None);
    let good = boxed(vec![("value", text("123"))]);

    assert_eq!(schema.decode(good.clone()).unwrap(), good);
    assert_eq!(schema.try_encode(&good).unwrap(), good);
    // The label rides through with the value.
    let labelled = boxed(vec![("value", text("123")), ("label", text("secret"))]);
    assert_eq!(schema.decode(labelled.clone()).unwrap(), labelled);

    for refused in [Unknown::Null, text("a"), text("1.2"), number(1.2)] {
        let refused_box = boxed(vec![("value", refused.clone())]);
        assert_eq!(
            refusal(&schema.decode(refused_box.clone()).unwrap_err()),
            ("$.value".to_string(), "a valid value".to_string()),
            "decoding {refused:?}"
        );
        assert_eq!(
            refusal(&schema.try_encode(&refused_box).unwrap_err()),
            ("$.value".to_string(), "a valid value".to_string()),
            "encoding {refused:?}"
        );
    }

    assert_eq!(
        redacted_value(&schema)
            .expect("a box exposes its wrapped schema")
            .decode(text("123"))
            .unwrap(),
        text("123")
    );
}

/// Schema.test.ts:3008 — "RedactedFromValue" / "should decode a value". The
/// box is the target, so decoding wraps a raw value and encoding unwraps one;
/// encoding refuses anything that was never boxed, and a box holding a value
/// the wrapped schema cannot read.
#[test]
fn a_box_wraps_on_the_way_in_and_unwraps_on_the_way_out() {
    let schema = Schema::redacted_from_value(int_from_string());

    assert_eq!(
        schema.decode(text("123")).unwrap(),
        boxed(vec![("value", text("123"))])
    );
    // The wrapped schema's own refusal, not the box's: on this side the value
    // is the value.
    assert_eq!(
        refusal(&schema.decode(Unknown::Null).unwrap_err()),
        ("$".to_string(), "a finite number string".to_string())
    );
    assert_eq!(
        refusal(&schema.decode(text("1.2")).unwrap_err()),
        ("$".to_string(), "an integer".to_string())
    );

    assert_eq!(
        schema
            .try_encode(&boxed(vec![("value", text("123"))]))
            .unwrap(),
        text("123")
    );
    assert_eq!(
        refusal(&schema.try_encode(&text("schema-secret")).unwrap_err()),
        ("$".to_string(), "Redacted".to_string())
    );
    for refused in [text("schema-secret"), number(1.2)] {
        assert_eq!(
            refusal(
                &schema
                    .try_encode(&boxed(vec![("value", refused.clone())]))
                    .unwrap_err()
            ),
            ("$.value".to_string(), "a valid value".to_string()),
            "{refused:?}"
        );
    }

    assert_eq!(
        redacted_value(&schema)
            .expect("a box exposes its wrapped schema")
            .decode(text("123"))
            .unwrap(),
        text("123")
    );
}
