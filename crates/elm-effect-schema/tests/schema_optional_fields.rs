//! `Schema.test.ts` "Optional Fields" rows: what an absent key, a null key and a
//! present key mean for `optionalKey` and `optional`, in both directions.
//!
//!     cargo test -p elm-effect-schema --test schema_optional_fields
//!
//! This port's `Unknown` has no `undefined`: absence is the only spelling of
//! "not there", and `null` is a value. `optionalKey` therefore means "the key
//! may be absent, but a present null still goes through the field's schema",
//! and `optional` means "absent or null, both skip the field's schema" — which
//! is the pin's `undefined`, carried by the two spellings the port has.

use elm_effect_schema::{Field, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn map(pairs: Vec<(&str, Unknown)>) -> std::collections::BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn empty() -> Unknown {
    Unknown::Object(Default::default())
}

fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

/// `Schema.test.ts:8111` `Optional Fields` / `Exact Optional Property`: an
/// absent key is allowed and a present one goes through the field's schema, so
/// the wire text is what a decode answers and what an encode accepts.
#[test]
fn an_exact_optional_property_may_be_absent_but_not_null() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional_key(
        "a",
        finite(),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());

    // `optionalKey` is exact: a present null still goes through the field.
    assert!(schema.decode(obj(vec![("a", Unknown::Null)])).is_err());

    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
}

/// `Schema.test.ts:8145` `Optional Property`: an `optional` field absorbs a
/// null as well as an absent key, in both directions.
#[test]
fn an_optional_property_may_be_absent_or_null() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional(
        "a",
        finite(),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        empty(),
        "a null is the absence marker this spelling absorbs"
    );

    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
}

/// `Schema.test.ts:8164` `Exact Optional Property with Nullability`: the field
/// is exact, but its schema admits null, so a present null travels as a value.
#[test]
fn an_exact_optional_property_can_still_admit_null() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional_key(
        "a",
        Schema::null_or(finite()),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        obj(vec![("a", Unknown::Null)]),
        "the field's own schema takes the null"
    );

    assert_eq!(
        schema.try_encode(&obj(vec![("a", Unknown::Null)])).unwrap(),
        obj(vec![("a", Unknown::Null)])
    );
    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
}

/// `Schema.test.ts:8183` `Optional Property with Nullability`: an `optional`
/// field over a nullable schema takes absent, null and a value alike, and a
/// value the field's schema refuses still fails at the field.
#[test]
fn an_optional_nullable_property_takes_all_three_shapes() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::optional(
        "a",
        Schema::null_or(finite()),
    )]));

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(schema.decode(empty()).unwrap(), empty());
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        empty(),
        "`optional` absorbs the null before the field's schema sees it"
    );

    // A present value still runs the field's schema, and a refused one names
    // the field.
    let refused = schema
        .decode(obj(vec![("a", text("not a number"))]))
        .expect_err("text is not a finite number");
    assert!(refused.to_string().contains("$.a"), "{refused}");

    assert_eq!(schema.try_encode(&empty()).unwrap(), empty());
    assert_eq!(
        schema.try_encode(&obj(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}

/// `Schema.test.ts:8207` `asOption` / `optionalKey -> Option`: the field reads
/// as an option — absent is `None`, present is the value — and a `None` leaves
/// the key out again.
///
/// This port spells `None` as `Unknown::Null`, the marker its option shapes
/// already use, and `Some(value)` as the value itself.
#[test]
fn an_optional_key_reads_as_an_option() {
    let schema = Schema::struct_shape(vec![Field::option_from_optional_key("a", finite())]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        map(vec![("a", text("1"))]),
        "a present value is `Some` that value"
    );
    assert_eq!(
        schema.decode(empty()).unwrap(),
        map(vec![("a", Unknown::Null)]),
        "an absent key is `None`"
    );

    assert_eq!(
        schema.try_encode(&map(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
    assert_eq!(
        schema.try_encode(&map(vec![("a", Unknown::Null)])).unwrap(),
        empty(),
        "a `None` leaves the key out"
    );
}

/// `Schema.test.ts:8230` `asOption` / `optional -> Option`: the same reading
/// when the field also absorbs a null, so absence and null are one `None`.
#[test]
fn an_optional_property_reads_as_an_option() {
    let schema = Schema::struct_shape(vec![Field::option_from_optional("a", finite())]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("1"))])).unwrap(),
        map(vec![("a", text("1"))])
    );
    assert_eq!(
        schema.decode(empty()).unwrap(),
        map(vec![("a", Unknown::Null)])
    );
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        map(vec![("a", Unknown::Null)]),
        "a null is the same `None`"
    );

    assert_eq!(
        schema.try_encode(&map(vec![("a", Unknown::Null)])).unwrap(),
        empty()
    );
    assert_eq!(
        schema.try_encode(&map(vec![("a", text("1"))])).unwrap(),
        obj(vec![("a", text("1"))])
    );
}
