//! Slice 5: decoding defaults, in all four spellings.
//!
//! `withDecodingDefault(Key)` takes a wire value, `withDecodingDefaultType(Key)`
//! one already decoded; the `Key` spellings are field-level (a missing key
//! fills, an explicit null still decodes) and the others are schema-level (a
//! null counts as absent too). Binds Schema.test.ts rows 9673-10050.
//!
//! One spelling note: a struct field here is a `Schema<Unknown>`, and a
//! re-typed schema's Unknown value is its wire form, so a `FiniteFromString`
//! field reads back as `Text("1")` rather than the number it decoded to. The
//! assertions below are written in that spelling; the pin's `1` there is this
//! port's `Text("1")`.

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
            .collect(),
    )
}

fn map(pairs: Vec<(&str, Unknown)>) -> BTreeMap<String, Unknown> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// `Schema.FiniteFromString`, which is the schema every one of these rows
/// defaults, with the re-typing a struct field needs.
fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

fn struct_of(fields: Vec<Field>) -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(fields)
}

// The four spellings, named so a nested row can put the default on `b` and the
// outer field on `a` exactly as the pin does.

fn wire_key(name: &str) -> Field {
    Field::decoding_default(name, finite(), || Ok(text("1")))
}

fn typed_key(name: &str) -> Field {
    Field::decoding_default_type(name, finite(), || Ok(text("1")))
}

fn wire_schema(name: &str) -> Field {
    Field::required(
        name,
        Schema::as_unknown(Schema::with_decoding_default(Schema::finite_from_string(), || {
            Ok(text("1"))
        })),
    )
}

fn typed_schema(name: &str) -> Field {
    Field::required(
        name,
        Schema::as_unknown(Schema::with_decoding_default_type(
            Schema::finite_from_string(),
            || Ok(1.0),
        )),
    )
}

// ------------------------------------------- withDecodingDefaultKey (wire) ---

/// Schema.test.ts:9673 `should return a decoding default value if the key is
/// missing`.
#[test]
fn wire_key_default_fills_a_missing_key() {
    let schema = struct_of(vec![wire_key("a")]);

    assert_eq!(schema.decode(obj(vec![])).unwrap(), map(vec![("a", text("1"))]));
    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
    // An explicit null is a value, not an absence: the wire schema refuses it.
    assert!(schema.decode(obj(vec![("a", Unknown::Null)])).is_err());
}

/// Schema.test.ts:9689 `by default should pass through the value`: the default
/// is a decoding affair, so encoding leaves the field alone.
#[test]
fn wire_key_default_passes_the_value_through_on_encode() {
    let schema = struct_of(vec![wire_key("a")]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        obj(vec![("a", text("1"))])
    );
}

/// Schema.test.ts:9699 `should omit the value if the encoding strategy is set
/// to omit`.
#[test]
fn wire_key_default_omit_encoding_drops_the_key() {
    let schema = struct_of(vec![wire_key("a").omit_encoding()]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        Unknown::Object(BTreeMap::new())
    );
}

/// Schema.test.ts:9711 `nested default values`. The outer default is the wire
/// `{}`, so decoding it through the inner struct is what fills `b`.
#[test]
fn wire_key_default_nested_defaults() {
    let inner = Schema::as_unknown(struct_of(vec![wire_key("b")]));
    let schema = struct_of(vec![Field::decoding_default("a", inner, || {
        Ok(Unknown::Object(BTreeMap::new()))
    })]);

    let filled = map(vec![("a", obj(vec![("b", text("1"))]))]);
    assert_eq!(schema.decode(obj(vec![])).unwrap(), filled);
    assert_eq!(schema.decode(obj(vec![("a", obj(vec![]))])).unwrap(), filled);
    assert_eq!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", text("2"))]))]))
            .unwrap(),
        map(vec![("a", obj(vec![("b", text("2"))]))])
    );
    assert!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", Unknown::Null)]))]))
            .is_err()
    );
}

/// Schema.test.ts:9730 `Effect failing with SchemaError propagates as decode
/// failure`.
#[test]
fn wire_key_default_failure_is_a_decode_failure() {
    let failing = Field::decoding_default("a", finite(), || {
        Err(SchemaError::DefaultFailed {
            path: String::new(),
            message: "decoding default failed".to_string(),
        })
    });
    let schema = struct_of(vec![failing]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
    let error = schema.decode(obj(vec![])).unwrap_err();
    assert!(error.to_string().contains("decoding default failed"));
}

// --------------------------------------------- withDecodingDefault (wire) ---

/// Schema.test.ts:9793 `should return a decoding default value if the key is
/// missing`.
#[test]
fn wire_default_fills_a_missing_key_and_a_null() {
    let schema = struct_of(vec![wire_schema("a")]);

    assert_eq!(schema.decode(obj(vec![])).unwrap(), map(vec![("a", text("1"))]));
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        map(vec![("a", text("1"))])
    );
    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
}

/// Schema.test.ts:9805 `should return a decoding default value if the schema is
/// used as standalone and the input is undefined`: the port stands null in for
/// the pin's undefined.
#[test]
fn wire_default_absorbs_a_standalone_null() {
    let schema = Schema::with_decoding_default(Schema::string(), || Ok(text("a")));

    assert_eq!(schema.decode(Unknown::Null).unwrap(), "a".to_string());
    assert_eq!(schema.decode(text("b")).unwrap(), "b".to_string());
}

/// Schema.test.ts:9814 `by default should pass through the value`.
#[test]
fn wire_default_passes_the_value_through_on_encode() {
    let schema = struct_of(vec![wire_schema("a")]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        obj(vec![("a", text("1"))])
    );
}

/// Schema.test.ts:9824 `should omit the value if the encoding strategy is set
/// to omit`. The strategy rides the schema here, so the walk reads it off the
/// field's schema rather than off the field.
#[test]
fn wire_default_omit_encoding_drops_the_key() {
    let schema = struct_of(vec![Field::required(
        "a",
        Schema::as_unknown(
            Schema::with_decoding_default(Schema::finite_from_string(), || Ok(text("1")))
                .omit_encoding(),
        ),
    )]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        Unknown::Object(BTreeMap::new())
    );
}

/// Schema.test.ts:9834 `nested default values`: five spellings of absence, all
/// landing on `{ a: { b: 1 } }`.
#[test]
fn wire_default_nested_defaults() {
    let inner = Schema::as_unknown(struct_of(vec![wire_schema("b")]));
    let schema = struct_of(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::with_decoding_default(inner, || {
            Ok(Unknown::Object(BTreeMap::new()))
        })),
    )]);

    let filled = map(vec![("a", obj(vec![("b", text("1"))]))]);
    assert_eq!(schema.decode(obj(vec![])).unwrap(), filled);
    assert_eq!(schema.decode(obj(vec![("a", obj(vec![]))])).unwrap(), filled);
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        filled
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", Unknown::Null)]))]))
            .unwrap(),
        filled
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", text("2"))]))]))
            .unwrap(),
        map(vec![("a", obj(vec![("b", text("2"))]))])
    );
}

// ------------------------------------- withDecodingDefaultTypeKey (decoded) ---

/// Schema.test.ts:9895 `should return a decoding default value if the key is
/// missing`.
#[test]
fn typed_key_default_fills_a_missing_key() {
    let schema = struct_of(vec![typed_key("a")]);

    assert_eq!(schema.decode(obj(vec![])).unwrap(), map(vec![("a", text("1"))]));
    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
    assert!(schema.decode(obj(vec![("a", Unknown::Null)])).is_err());
}

/// Schema.test.ts:9911 `by default should pass through the value`.
#[test]
fn typed_key_default_passes_the_value_through_on_encode() {
    let schema = struct_of(vec![typed_key("a")]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        obj(vec![("a", text("1"))])
    );
}

/// Schema.test.ts:9921 `should omit the value if the encoding strategy is set
/// to omit`.
#[test]
fn typed_key_default_omit_encoding_drops_the_key() {
    let schema = struct_of(vec![typed_key("a").omit_encoding()]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        Unknown::Object(BTreeMap::new())
    );
}

/// Schema.test.ts:9933 `nested default values`. The outer default is decoded
/// already (`{ b: 1 }`), so it is inserted as it is rather than decoded again.
#[test]
fn typed_key_default_nested_defaults() {
    let inner = Schema::as_unknown(struct_of(vec![typed_key("b")]));
    let schema = struct_of(vec![Field::decoding_default_type("a", inner, || {
        Ok(obj(vec![("b", text("1"))]))
    })]);

    let filled = map(vec![("a", obj(vec![("b", text("1"))]))]);
    assert_eq!(schema.decode(obj(vec![])).unwrap(), filled);
    assert_eq!(schema.decode(obj(vec![("a", obj(vec![]))])).unwrap(), filled);
    assert_eq!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", text("2"))]))]))
            .unwrap(),
        map(vec![("a", obj(vec![("b", text("2"))]))])
    );
    assert!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", Unknown::Null)]))]))
            .is_err()
    );
}

/// Schema.test.ts:9952 `Effect failing with SchemaError propagates as decode
/// failure`.
#[test]
fn typed_key_default_failure_is_a_decode_failure() {
    let failing = Field::decoding_default_type("a", finite(), || {
        Err(SchemaError::DefaultFailed {
            path: String::new(),
            message: "decoding default failed".to_string(),
        })
    });
    let schema = struct_of(vec![failing]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
    let error = schema.decode(obj(vec![])).unwrap_err();
    assert!(error.to_string().contains("decoding default failed"));
}

// ------------------------------------------ withDecodingDefaultType (typed) ---

/// Schema.test.ts:9991 `should return a decoding default value if the key is
/// missing`.
#[test]
fn typed_default_fills_a_missing_key_and_a_null() {
    let schema = struct_of(vec![typed_schema("a")]);

    assert_eq!(schema.decode(obj(vec![])).unwrap(), map(vec![("a", text("1"))]));
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        map(vec![("a", text("1"))])
    );
    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
}

/// Schema.test.ts:10003 `should return a decoding default value if the schema
/// is used as standalone and the input is undefined`.
#[test]
fn typed_default_absorbs_a_standalone_null() {
    let schema = Schema::with_decoding_default_type(Schema::string(), || Ok("a".to_string()));

    assert_eq!(schema.decode(Unknown::Null).unwrap(), "a".to_string());
    assert_eq!(schema.decode(text("b")).unwrap(), "b".to_string());
}

/// Schema.test.ts:10012 `by default should pass through the value`.
#[test]
fn typed_default_passes_the_value_through_on_encode() {
    let schema = struct_of(vec![typed_schema("a")]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        obj(vec![("a", text("1"))])
    );
}

/// Schema.test.ts:10022 `should omit the value if the encoding strategy is set
/// to omit`.
#[test]
fn typed_default_omit_encoding_drops_the_key() {
    let schema = struct_of(vec![Field::required(
        "a",
        Schema::as_unknown(
            Schema::with_decoding_default_type(Schema::finite_from_string(), || Ok(1.0))
                .omit_encoding(),
        ),
    )]);

    assert_eq!(
        schema.encode(&map(vec![("a", text("1"))])),
        Unknown::Object(BTreeMap::new())
    );
}

/// Schema.test.ts:10034 `nested default values`.
#[test]
fn typed_default_nested_defaults() {
    let inner = Schema::as_unknown(struct_of(vec![typed_schema("b")]));
    let schema = struct_of(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::with_decoding_default_type(inner, || {
            Ok(obj(vec![("b", text("1"))]))
        })),
    )]);

    let filled = map(vec![("a", obj(vec![("b", text("1"))]))]);
    assert_eq!(schema.decode(obj(vec![])).unwrap(), filled);
    assert_eq!(schema.decode(obj(vec![("a", obj(vec![]))])).unwrap(), filled);
    assert_eq!(
        schema.decode(obj(vec![("a", Unknown::Null)])).unwrap(),
        filled
    );
    assert_eq!(
        schema
            .decode(obj(vec![("a", obj(vec![("b", Unknown::Null)]))]))
            .unwrap(),
        filled
    );
}

/// Schema.test.ts:10050 `Effect failing with SchemaError propagates as decode
/// failure`.
#[test]
fn typed_default_failure_is_a_decode_failure() {
    let schema = struct_of(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::with_decoding_default_type(
            Schema::finite_from_string(),
            || {
                Err(SchemaError::DefaultFailed {
                    path: String::new(),
                    message: "decoding default failed".to_string(),
                })
            },
        )),
    )]);

    assert_eq!(
        schema.decode(obj(vec![("a", text("2"))])).unwrap(),
        map(vec![("a", text("2"))])
    );
    let error = schema.decode(obj(vec![])).unwrap_err();
    assert!(error.to_string().contains("decoding default failed"));
}
