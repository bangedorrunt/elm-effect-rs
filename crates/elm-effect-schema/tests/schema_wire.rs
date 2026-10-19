//! `Schema.test.ts` wire-text rows: the JSON-string adapters and `Trim`.
//!
//! One test per ledger row, named for the pin's own `it(...)` title, so the
//! row's `rust` column can point at it. Run from the crate:
//!
//!     cargo test -p elm-effect-schema --test schema_wire

use elm_effect_schema::{Field, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn object(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    )
}

/// A struct whose one field `b` is a number, the shape the pin's
/// `Schema.fromJsonString(Schema.Struct({ b: Schema.Number }))` rows use.
fn b_is_a_number() -> Schema<std::collections::BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![Field::required(
        "b",
        Schema::as_unknown(Schema::number()),
    )])
}

/// `Schema.test.ts:5991` — "use case: Unknown <-> JSON string".
#[test]
fn unknown_from_json_string_decodes_and_encodes() {
    let schema = Schema::<Unknown>::unknown_from_json_string();

    assert_eq!(
        schema.decode(text("{\"a\":1}")).expect("valid JSON text"),
        object(&[("a", Unknown::Number(1.0))])
    );
    assert!(
        schema.decode(text("{\"a\"")).is_err(),
        "truncated JSON text must not decode"
    );

    assert_eq!(
        schema
            .try_encode(&object(&[("a", Unknown::Number(1.0))]))
            .expect("encodes"),
        text("{\"a\":1}")
    );
}

/// `Schema.test.ts:6011` — "use case: create a JSON string serializer for an
/// existing schema".
#[test]
fn from_json_string_serializes_an_existing_schema() {
    let schema = Schema::from_json_string(b_is_a_number());

    let decoded = schema.decode(text("{\"b\":1}")).expect("decodes");
    assert_eq!(decoded.get("b"), Some(&Unknown::Number(1.0)));

    assert!(
        schema.decode(text("{\"a\":null}")).is_err(),
        "the wrapped struct still rejects a document missing its key"
    );
}

// `Schema.test.ts:6030` — "use case: parse / stringify a nested schema" — is
// NOT witnessed here. Its shape is `Struct({ a: fromJsonString(Struct) })`, and
// a struct field can only be spelled `Schema<Unknown>`, so it has to be re-typed
// with `as_unknown` — whose decode is the inner ENCODE. A decoding transform
// therefore comes back as its own text, and the row's `a: { b: 2 }` cannot be
// observed. That is the port's wire-level `Unknown`, not this schema's doing:
// `as_unknown(NumberFromString)` behaves the same way.

/// `Schema.test.ts:6047` — "reviver option".
#[test]
fn from_json_string_reviver_runs_on_each_value() {
    let schema = Schema::from_json_string_with(
        b_is_a_number(),
        |key: &str, value: Unknown| match (key, value) {
            ("b", Unknown::Text(text)) => Unknown::Number(text.parse().unwrap_or_default()),
            (_, value) => value,
        },
        |_: &str, value: Unknown| Some(value),
        None,
    );

    let decoded = schema.decode(text("{\"b\":\"1\"}")).expect("revived");
    assert_eq!(decoded.get("b"), Some(&Unknown::Number(1.0)));
}

/// `Schema.test.ts:6056` — "replacer and space options".
#[test]
fn from_json_string_replacer_and_space_shape_the_output() {
    let two_numbers: Schema<std::collections::BTreeMap<String, Unknown>> =
        Schema::struct_shape(vec![
            Field::required("a", Schema::as_unknown(Schema::number())),
            Field::required("b", Schema::as_unknown(Schema::number())),
        ]);
    let schema = Schema::from_json_string_with(
        two_numbers.clone(),
        |_: &str, value: Unknown| value,
        |key: &str, value: Unknown| (key != "b").then_some(value),
        Some(2),
    );
    let value = two_numbers
        .decode(object(&[
            ("a", Unknown::Number(1.0)),
            ("b", Unknown::Number(2.0)),
        ]))
        .expect("decodes");

    assert_eq!(
        schema.try_encode(&value).expect("encodes"),
        text("{\n  \"a\": 1\n}")
    );
}

/// `Schema.test.ts:6172` — "Trim".
#[test]
fn trim_decodes_trimmed_and_refuses_untrimmed_on_encode() {
    let schema = Schema::<String>::trim();

    for input in ["a", "a ", " a", " a ", "a\n"] {
        assert_eq!(
            schema.decode(text(input)).expect("decodes"),
            "a".to_string(),
            "{input:?} trims to \"a\""
        );
    }

    assert_eq!(
        schema.try_encode(&"a".to_string()).expect("encodes"),
        text("a")
    );
    assert!(
        schema.try_encode(&"a ".to_string()).is_err(),
        "a value that is not already trimmed cannot travel back out"
    );
}
