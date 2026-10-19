//! `Schema.test.ts` Option schemas: the boxed `Option`, its `.value`
//! accessor, and the four field-level spellings that read an absent or nullish
//! key as `None`.
//!
//!     cargo test -p elm-effect-schema --test schema_option
//!
//! Two wire conventions are at work here, and they are this port's own:
//!
//! * The boxed `Schema.Option(inner)` travels as the tag object the pin's
//!   codec links to, `{ "_tag": "None" }` or `{ "_tag": "Some", value }`,
//!   because JSON is this port's wire. The pin's *encoded* type is a JS
//!   `Option` instance, which is the same information in a form JSON cannot
//!   carry.
//! * `undefined` has no value in `Unknown`, so `Unknown::Null` stands in for
//!   it everywhere in this crate (see `Schema::void`). That collapse is why
//!   `OptionFromUndefinedOr` and `OptionFromNullishOr` answer exactly what
//!   `OptionFromNullOr` does, and `OptionFromOptional` exactly what
//!   `OptionFromOptionalNullOr`'s default does: the rows that turn only on the
//!   null/undefined distinction are excused in the ledger with that cause
//!   rather than witnessed twice.
//!
//! A struct field's decoded value here is a wire value, so the `None` of an
//! Option field is the `Unknown::Null` marker: absence decodes to it, and
//! `omit_none_encoding` drops it again on the way out.

use elm_effect_schema::{Field, Schema, SchemaError, Unknown};
use std::collections::BTreeMap;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// The `(path, expected)` pair a refusal carries.
fn refusal(error: &SchemaError) -> (String, String) {
    match error {
        SchemaError::TypeMismatch { path, expected, .. } => (path.clone(), (*expected).to_string()),
        SchemaError::TypeMismatchDynamic { path, expected, .. } => (path.clone(), expected.clone()),
        other => panic!("not a shape refusal: {other}"),
    }
}

/// `Schema.FiniteFromString`, the schema every Option row wraps.
fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

/// Schema.test.ts:3034 — "should expose the value". The box hands back the
/// schema it wraps, and an annotation keeps it reachable.
#[test]
fn option_exposes_the_wrapped_schema() {
    let schema = Schema::option(Schema::string());
    let value = schema.value().expect("an Option keeps the schema it wraps");
    assert_eq!(value.to_code(), "Schema.String");
    assert_eq!(value.decode(text("a")), Ok(text("a")));
    assert!(value.decode(Unknown::Number(1.0)).is_err());

    let annotated = Schema::annotated_with(schema, vec![("title", "a maybe")]);
    assert_eq!(
        annotated
            .value()
            .expect("an annotation keeps the wrapped schema")
            .to_code(),
        "Schema.String"
    );
}

/// Schema.test.ts:3040 — `Option(FiniteFromString)`. `None` is a value, so a
/// bare null is refused; the tag object is the only shape either axis reads.
#[test]
fn option_decodes_and_encodes_the_tag_object() {
    let schema = Schema::option(Schema::finite_from_string());

    assert_eq!(
        schema.decode(obj(vec![("_tag", text("None"))])).unwrap(),
        None
    );
    assert_eq!(
        schema
            .decode(obj(vec![("_tag", text("Some")), ("value", text("123"))]))
            .unwrap(),
        Some(123.0)
    );
    // Not an Option: this is what keeps the boxed schema apart from
    // `Schema::optional`, which reads the null as `None`.
    assert_eq!(
        refusal(&schema.decode(Unknown::Null).unwrap_err()),
        ("$".to_string(), "Option".to_string())
    );
    assert!(schema.decode(obj(vec![])).is_err());
    assert!(
        schema.decode(obj(vec![("_tag", text("Some"))])).is_err(),
        "Some without a value is not a shape the box has"
    );
    // The wrapped schema's refusal lands on the value, not the box.
    assert_eq!(
        refusal(
            &schema
                .decode(obj(vec![("_tag", text("Some")), ("value", Unknown::Null)]))
                .unwrap_err()
        )
        .0,
        "$.value"
    );

    assert_eq!(
        schema.try_encode(&None).unwrap(),
        obj(vec![("_tag", text("None"))])
    );
    assert_eq!(
        schema.try_encode(&Some(123.0)).unwrap(),
        obj(vec![("_tag", text("Some")), ("value", text("123"))])
    );
}

/// Schema.test.ts:3070 — `OptionFromNullOr`. The null is the `None`; every
/// other value goes through the wrapped schema.
#[test]
fn option_from_null_or_maps_null_to_none() {
    let schema = Schema::optional(Schema::finite_from_string());

    assert_eq!(schema.decode(Unknown::Null).unwrap(), None);
    assert_eq!(schema.decode(text("1")).unwrap(), Some(1.0));
    assert_eq!(
        refusal(&schema.decode(text("a")).unwrap_err()).0,
        "$".to_string()
    );

    assert_eq!(schema.try_encode(&None).unwrap(), Unknown::Null);
    assert_eq!(schema.try_encode(&Some(1.0)).unwrap(), text("1"));
}

/// Schema.test.ts:3146 — `OptionFromOptionalKey`. An absent key is `None`, but
/// a key that is there must hold a value the wrapped schema accepts: the
/// port's null stand-in for undefined is not skipped.
#[test]
fn option_from_optional_key_fills_an_absent_key() {
    let schema = Schema::struct_shape(vec![Field::option_from_optional_key("a", finite())]);

    assert_eq!(
        schema.decode(obj(vec![])).unwrap(),
        map(vec![("a", Unknown::Null)])
    );
    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        map(vec![("a", text("1"))])
    );

    let (path, expected) = refusal(&schema.decode(obj(vec![("a", Unknown::Null)])).unwrap_err());
    assert_eq!(path, "$.a");
    assert!(expected.contains("finite number"), "{expected}");
    assert_eq!(
        refusal(&schema.decode(obj(vec![("a", text("a"))])).unwrap_err()).0,
        "$.a"
    );

    // `None` leaves the key out; a present value travels.
    assert_eq!(
        schema.try_encode(&map(vec![("a", Unknown::Null)])).unwrap(),
        obj(vec![])
    );
    assert_eq!(
        schema.try_encode(&map(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}

/// Schema.test.ts:3175 — `OptionFromOptional`. Absence and the null stand-in
/// for undefined are both `None`, and a `None` is omitted on encode.
#[test]
fn option_from_optional_fills_absent_and_null() {
    let schema = Schema::struct_shape(vec![Field::option_from_optional("a", finite())]);

    assert_eq!(
        schema.decode(obj(vec![])).unwrap(),
        map(vec![("a", Unknown::Null)])
    );
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        map(vec![("a", Unknown::Null)])
    );
    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        map(vec![("a", text("1"))])
    );
    assert_eq!(
        refusal(&schema.decode(obj(vec![("a", text("a"))])).unwrap_err()).0,
        "$.a"
    );

    assert_eq!(
        schema.try_encode(&map(vec![("a", Unknown::Null)])).unwrap(),
        obj(vec![])
    );
    assert_eq!(
        schema.try_encode(&map(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}

/// Schema.test.ts:3232 — `OptionFromOptionalNullOr` with `onNoneEncoding:
/// null`. The None travels back out as an explicit null instead of leaving the
/// key out.
#[test]
fn option_from_optional_null_or_encodes_none_as_null() {
    let schema = Schema::struct_shape(vec![Field::option_from_optional_null_or(
        "a",
        finite(),
        false,
    )]);

    for absent in [obj(vec![]), obj(vec![("a", Unknown::Null)])] {
        assert_eq!(
            schema.decode(absent).unwrap(),
            map(vec![("a", Unknown::Null)])
        );
    }
    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        map(vec![("a", text("1"))])
    );

    assert_eq!(
        schema.try_encode(&map(vec![("a", Unknown::Null)])).unwrap(),
        obj(vec![("a", Unknown::Null)])
    );
    assert_eq!(
        schema.try_encode(&map(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}
