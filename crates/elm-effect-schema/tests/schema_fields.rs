//! Slice 6: struct derivations that rewrite the field list.
//!
//! The pin's `Struct.mapFields` takes a field transformer, and each of its
//! helpers (`Struct.evolve`, `Struct.map`, `Struct.mapPick`, ...) builds one.
//! These tests assert the derived *field list*, which is what the pin's rows
//! compare, so the helper reads the fields back through `Schema::fields`.
//! Binds Schema.test.ts rows 8793-9172.

use elm_effect_schema::{
    Field, Optionality, Schema, Unknown, field_evolve, field_evolve_entries, field_map,
    field_map_omit, field_map_pick,
};
use std::collections::BTreeMap;

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

fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn str_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn bool_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

/// `FiniteFromString`, whose Unknown spelling is its wire text.
fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

fn struct_of(fields: Vec<Field>) -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(fields)
}

/// The derived shape, in the pin's terms: key, optionality, and the code of the
/// schema each key decodes with.
fn fields_of(schema: &Schema<BTreeMap<String, Unknown>>) -> Vec<(String, Optionality, String)> {
    schema
        .fields()
        .expect("a derived schema is still a struct shape")
        .iter()
        .map(|field| {
            (
                field.name().to_string(),
                field.optionality(),
                field.inner().to_code(),
            )
        })
        .collect()
}

fn pair(name: &str, optionality: Optionality, code: &str) -> (String, Optionality, String) {
    (name.to_string(), optionality, code.to_string())
}

fn two_field_struct() -> Schema<BTreeMap<String, Unknown>> {
    struct_of(vec![
        Field::required("a", str_schema()),
        Field::required("b", number_schema()),
    ])
}

// ------------------------------------------------------------- mapFields ---

/// Schema.test.ts:8793 `evolve`: one named field through one function.
#[test]
fn evolve_maps_a_named_field() {
    let schema =
        two_field_struct().map_fields(field_evolve(vec![("a", |field| field.make_optional_key())]));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::OptionalKey, "Schema.String"),
            pair("b", Optionality::Required, "Schema.Number"),
        ]
    );
}

/// Schema.test.ts:8805 `evolveKeys`: the function receives the key and returns
/// the new one.
#[test]
fn evolve_keys_renames_a_named_field() {
    let schema = two_field_struct().map_fields(field_evolve(vec![("a", |field| {
        let upper = field.name().to_uppercase();
        field.renamed(&upper)
    })]));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("A", Optionality::Required, "Schema.String"),
            pair("b", Optionality::Required, "Schema.Number"),
        ]
    );
}

/// Schema.test.ts:8817 `renameKeys`: a table of old key to new key.
#[test]
fn rename_keys_rewrites_two_of_three() {
    let schema = struct_of(vec![
        Field::required("a", str_schema()),
        Field::required("b", number_schema()),
        Field::required("c", bool_schema()),
    ])
    .map_fields(field_evolve(vec![
        ("a", |field| field.renamed("A")),
        ("b", |field| field.renamed("B")),
    ]));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("A", Optionality::Required, "Schema.String"),
            pair("B", Optionality::Required, "Schema.Number"),
            pair("c", Optionality::Required, "Schema.Boolean"),
        ]
    );
}

/// Schema.test.ts:8831 `evolveEntries`: the whole entry through one function, so
/// a rename and a codec change happen together. The pin returns `[key, value]`;
/// the port returns the field with both replaced.
#[test]
fn evolve_entries_renames_and_rewraps() {
    let schema = two_field_struct().map_fields(field_evolve_entries(|field| {
        if field.name() == "a" {
            let upper = field.name().to_uppercase();
            field.renamed(&upper).make_optional_key()
        } else {
            field
        }
    }));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("A", Optionality::OptionalKey, "Schema.String"),
            pair("b", Optionality::Required, "Schema.Number"),
        ]
    );
}

/// Schema.test.ts:8843 `optionalKey`: every field's key may be absent.
#[test]
fn optional_key_applies_to_every_field() {
    let schema = two_field_struct().map_fields(field_map(|field| field.make_optional_key()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::OptionalKey, "Schema.String"),
            pair("b", Optionality::OptionalKey, "Schema.Number"),
        ]
    );
}

/// Schema.test.ts:8855 `mapPick`: only the named fields are touched.
#[test]
fn map_pick_touches_the_named_field() {
    let schema =
        two_field_struct().map_fields(field_map_pick(vec!["a"], |field| field.make_optional_key()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::OptionalKey, "Schema.String"),
            pair("b", Optionality::Required, "Schema.Number"),
        ]
    );
}

/// Schema.test.ts:8867 `mapOmit`: every field except the named ones is touched.
#[test]
fn map_omit_skips_the_named_field() {
    let schema =
        two_field_struct().map_fields(field_map_omit(vec!["b"], |field| field.make_optional_key()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::OptionalKey, "Schema.String"),
            pair("b", Optionality::Required, "Schema.Number"),
        ]
    );
}

/// Schema.test.ts:8879 `optional`: the field's schema becomes the absent-or
/// codec, while the key itself stays required.
#[test]
fn optional_wraps_every_field_schema() {
    let schema = two_field_struct().map_fields(field_map(|field| field.make_nullable()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::Required, "Schema.optional(Schema.String)"),
            pair("b", Optionality::Required, "Schema.optional(Schema.Number)"),
        ]
    );
}

/// Schema.test.ts:8927 `NullOr`. The pin distinguishes null-or from
/// undefined-or; this port has one absent value, so both spellings derive the
/// same nullable field, and the assertions below are the ones that differ.
#[test]
fn null_or_wraps_every_field_schema() {
    let schema = two_field_struct().map_fields(field_map(|field| field.make_nullable()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::Required, "Schema.optional(Schema.String)"),
            pair("b", Optionality::Required, "Schema.optional(Schema.Number)"),
        ]
    );
}

/// Schema.test.ts:8939 `UndefinedOr`, same collapse as the row above.
#[test]
fn undefined_or_wraps_every_field_schema() {
    let schema = two_field_struct().map_fields(field_map(|field| field.make_nullable()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::Required, "Schema.optional(Schema.String)"),
            pair("b", Optionality::Required, "Schema.optional(Schema.Number)"),
        ]
    );
}

/// Schema.test.ts:8951 `NullishOr`, same collapse again: null-or and
/// undefined-or together are exactly what the port's one absent value is.
#[test]
fn nullish_or_wraps_every_field_schema() {
    let schema = two_field_struct().map_fields(field_map(|field| field.make_nullable()));

    assert_eq!(
        fields_of(&schema),
        vec![
            pair("a", Optionality::Required, "Schema.optional(Schema.String)"),
            pair("b", Optionality::Required, "Schema.optional(Schema.Number)"),
        ]
    );
}

// ------------------------------------------------------------ encodeKeys ---

/// Schema.test.ts:9110 `Struct`: decoding accepts the encoded spelling, and
/// encoding produces it.
#[test]
fn encode_keys_renames_on_both_sides() {
    let schema = struct_of(vec![
        Field::required("a", finite()),
        Field::required("b", finite()),
    ])
    .encode_keys(vec![("a", "c")]);

    assert_eq!(
        schema
            .decode(obj(vec![("c", text("1")), ("b", text("2"))]))
            .unwrap(),
        map(vec![("a", text("1")), ("b", text("2"))])
    );
    assert_eq!(
        schema
            .try_encode(&map(vec![("a", text("1")), ("b", text("2"))]))
            .unwrap(),
        obj(vec![("c", text("1")), ("b", text("2"))])
    );
}

/// Schema.test.ts:9159 `rejects duplicate destination keys`: two sources
/// pointing at one destination would give one wire key two meanings.
#[test]
#[should_panic(expected = "Duplicate encoded keys")]
fn encode_keys_refuses_duplicate_destinations() {
    let _ = two_field_struct().encode_keys(vec![("a", "c"), ("b", "c")]);
}

/// Schema.test.ts:9172 `rejects destination keys that collide with unmapped
/// fields`: renaming `a` onto `b` would overwrite a field nobody renamed.
#[test]
#[should_panic(expected = "Duplicate encoded keys")]
fn encode_keys_refuses_a_collision_with_an_unmapped_field() {
    let _ = two_field_struct().encode_keys(vec![("a", "b")]);
}
