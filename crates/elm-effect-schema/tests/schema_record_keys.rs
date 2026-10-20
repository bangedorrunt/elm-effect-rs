//! Slice 10: what a record says about its keys.
//!
//! An index signature (`Record(String, value)`) has no keys of its own, so the
//! pin refuses an `optionalKey` value there: nothing could be absent, and
//! `optional` is the spelling that means it. A literal-key record
//! (`Record(Literals([...]), value)`) fixes a key set instead, and every key in
//! it is required unless its value is the optional one.
//! Binds Schema.test.ts rows 4028-4212.

use elm_effect_schema::{Schema, Unknown};
use std::collections::BTreeMap;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
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

fn number_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn literal_keys(names: &[&str]) -> Schema<Unknown> {
    Schema::literals(names.iter().map(|name| text(name)).collect())
}

/// The message a refused key names, so the test can assert which key is missing
/// rather than only that something failed.
fn refusal(schema: &Schema<BTreeMap<String, Unknown>>, value: Unknown) -> String {
    schema
        .decode(value)
        .expect_err("the key is not in the object")
        .to_string()
}

/// Schema.test.ts:4028 `Record(String, optionalKey(Number)) should throw`: an
/// index signature has no key to be absent, so the shape is refused where it is
/// built rather than quietly decoding one.
#[test]
#[should_panic(expected = "Cannot use `Schema.optionalKey` with index signatures")]
fn index_signature_refuses_an_optional_key_value() {
    let _ = Schema::record(Schema::optional_key_shape(number_schema()));
}

/// The refusal is narrow: `Schema.optional` is exactly the spelling an index
/// signature accepts, and it means the value may be absent.
#[test]
fn index_signature_accepts_an_optional_value() {
    let schema = Schema::record(Schema::as_unknown(Schema::optional(number_schema())));

    assert_eq!(schema.decode(obj(vec![])).unwrap(), map(vec![]));
    assert_eq!(
        schema.decode(obj(vec![("a", number(1.0))])).unwrap(),
        map(vec![("a", number(1.0))])
    );
}

/// Schema.test.ts:4194 `Record(Literals, Number)`: every declared key is
/// required, and the refusal names the one that is missing. The ledger cited
/// `record_string_number_decodes_every_value` for this row, which is a witness
/// for a plain index signature and proves nothing about a key set.
#[test]
fn literal_key_record_requires_every_declared_key() {
    let schema = Schema::record2(literal_keys(&["a", "b"]), number_schema());

    assert_eq!(
        schema
            .decode(obj(vec![("a", number(1.0)), ("b", number(2.0))]))
            .unwrap(),
        map(vec![("a", number(1.0)), ("b", number(2.0))])
    );
    assert_eq!(
        refusal(&schema, obj(vec![("a", number(1.0))])),
        "expected a declared key at $.b, got missing"
    );
    assert_eq!(
        refusal(&schema, obj(vec![("b", number(2.0))])),
        "expected a declared key at $.a, got missing"
    );
}

/// Schema.test.ts:4189 `deduplicates repeated literal keys`: a key listed twice
/// is one key, so the object is refused once for the key nobody declared rather
/// than once per repetition.
#[test]
fn literal_key_record_counts_a_repeated_key_once() {
    let schema = Schema::record2(literal_keys(&["a", "a", "b"]), number_schema());

    assert_eq!(
        schema
            .decode(obj(vec![("a", number(1.0)), ("b", number(2.0))]))
            .unwrap(),
        map(vec![("a", number(1.0)), ("b", number(2.0))])
    );
    assert_eq!(
        refusal(&schema, obj(vec![("a", number(1.0))])),
        "expected a declared key at $.b, got missing"
    );
}

/// Schema.test.ts:4212 `Record(Literals, optionalKey(Number))`: with an
/// `optionalKey` value the key set is not required, so every subset decodes,
/// the empty object included. This is why the constructor refuses the same
/// value under an index signature and accepts it here.
#[test]
fn literal_key_record_with_optional_values_requires_no_key() {
    let schema = Schema::record2(
        literal_keys(&["a", "b"]),
        Schema::optional_key_shape(number_schema()),
    );

    for value in [
        obj(vec![]),
        obj(vec![("a", number(1.0))]),
        obj(vec![("b", number(2.0))]),
        obj(vec![("a", number(1.0)), ("b", number(2.0))]),
    ] {
        assert!(
            schema.decode(value.clone()).is_ok(),
            "{value:?} has no key outside the set"
        );
    }
}

/// A pattern key (`patternProperties`) names no fixed set, so the exact-key
/// rule must not reach it: the importer builds these, and requiring keys there
/// would refuse every object the pattern does not happen to spell out.
#[test]
fn pattern_key_record_requires_no_key() {
    let key = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isPattern",
        vec![r#"new RegExp("^a", "u")"#],
        vec![],
    );
    let schema = Schema::as_unknown(Schema::record2(key, number_schema()));

    assert!(schema.decode(obj(vec![])).is_ok());
    assert!(schema.decode(obj(vec![("anything", number(1.0))])).is_ok());
}

// --------------- StructWithRest (Schema.test.ts:4997-5136) ---------------

use elm_effect_schema::transform::Transformation;

/// A refusal's text, for a schema over the whole object. The file's `refusal`
/// helper is typed to the record shape; this one keeps the object shape.
fn refusal_of(schema: &Schema<Unknown>, value: Unknown) -> String {
    schema
        .decode(value)
        .expect_err("the value is refused")
        .to_string()
}

/// The pin's `Schema.String.pipe(Schema.decodeTo(Schema.String, t))` over this
/// port's `Schema::convert`: decode runs the transformation, encode is the
/// transformation's own encode direction.
fn transformed(t: Transformation<String, String>) -> Schema<Unknown> {
    let encoded = std::sync::Arc::new(t);
    let decoded = encoded.clone();
    Schema::as_unknown(Schema::convert(
        Schema::string(),
        move |value: String| decoded.forward(value),
        move |value: &String| {
            encoded
                .backward(value.clone())
                .expect("a one-way text transformation passes through on encode")
        },
    ))
}

/// Schema.test.ts:5121 `index signatures should not re-parse fixed properties`.
/// The fixed property decodes with its own schema and the catch-all must not
/// run over it a second time. The pin's trim cannot show the difference (trim
/// is idempotent), so the second half marks the text instead: a second pass
/// would mark it twice.
#[test]
fn struct_with_rest_does_not_re_parse_a_fixed_property() {
    let trimmed = Schema::struct_with_rest(
        vec![("a".to_string(), transformed(Transformation::trim()))],
        transformed(Transformation::trim()),
    );
    assert_eq!(
        trimmed
            .decode(obj(vec![("a", text("  x  ")), ("b", text("y"))]))
            .unwrap(),
        obj(vec![("a", text("x")), ("b", text("y"))])
    );

    // One pass over each key, so both values carry exactly one mark.
    let marked = Schema::struct_with_rest(
        vec![("a".to_string(), transformed(marked_once()))],
        transformed(marked_once()),
    );
    let decoded = marked
        .decode(obj(vec![("a", text("x")), ("b", text("y"))]))
        .unwrap();
    assert_eq!(decoded, obj(vec![("a", text("x!")), ("b", text("y!"))]));

    // The pin's encode half: encoding leaves the decoded object as it stands.
    assert_eq!(
        marked
            .try_encode(&obj(vec![("a", text("x!")), ("b", text("y!"))]))
            .unwrap(),
        obj(vec![("a", text("x!")), ("b", text("y!"))])
    );
}

/// One mark per pass, so a value that went through the transformation twice is
/// distinguishable from one that went through once.
fn marked_once() -> Transformation<String, String> {
    Transformation::new(
        |value: String| Ok(format!("{value}!")),
        |value: String| Ok(value),
    )
}

/// Schema.test.ts:5095 `should preserve both checks`: a check on the fixed part
/// and a check on the index signature are both kept and both enforced. The pin
/// spells them on the Struct and the Record; this port's `struct_with_rest`
/// takes the fixed fields plus one per-value shape for the catch-all, so the
/// same two checks sit on the field and on the rest value. Each refusal names
/// its own check, so a dropped check shows up as the wrong name or a pass.
#[test]
fn struct_with_rest_keeps_both_checks() {
    let positive = number_schema().refine(
        "agt(0)",
        |value: &Unknown| matches!(value, Unknown::Number(number) if *number > 0.0),
    );
    let above_one = number_schema().refine(
        "bgt(1)",
        |value: &Unknown| matches!(value, Unknown::Number(number) if *number > 1.0),
    );
    let schema = Schema::struct_with_rest(vec![("a".to_string(), positive)], above_one);

    assert_eq!(
        schema.decode(obj(vec![("a", number(1.0))])).unwrap(),
        obj(vec![("a", number(1.0))])
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", number(1.0)), ("b", number(2.0))]))
            .unwrap(),
        obj(vec![("a", number(1.0)), ("b", number(2.0))])
    );
    assert!(
        refusal_of(&schema, obj(vec![("a", number(0.0))])).contains("agt(0)"),
        "the fixed field's check is the one that refuses"
    );
    assert!(
        refusal_of(&schema, obj(vec![("a", number(1.0)), ("b", number(1.0))])).contains("bgt(1)"),
        "the catch-all's check is the one that refuses"
    );
}
