//! Schema core fence: decode unknown into typed values, encode back.
//! Derives from packages/effect/test/schema/ rows (core shapes only).

use elm_effect_schema::{Field, Schema, SchemaError, Unknown};
use std::collections::BTreeMap;

fn text(s: &str) -> Unknown {
    Unknown::Text(s.to_string())
}

#[test]
fn string_round_trips() {
    let schema = Schema::string();
    let value = schema.decode(Unknown::Text("hi".to_string())).unwrap();
    assert_eq!(value, "hi");
    assert_eq!(
        schema.try_encode(&value).unwrap(),
        Unknown::Text("hi".to_string())
    );
}

#[test]
fn number_round_trips() {
    let schema = Schema::number();
    let value = schema.decode(Unknown::Number(2.5)).unwrap();
    assert_eq!(value, 2.5);
    assert_eq!(schema.try_encode(&value).unwrap(), Unknown::Number(2.5));
}

#[test]
fn boolean_round_trips() {
    let schema = Schema::boolean();
    assert!(schema.decode(Unknown::Bool(true)).unwrap());
    assert_eq!(schema.try_encode(&true).unwrap(), Unknown::Bool(true));
}

#[test]
fn wrong_type_is_a_typed_error() {
    let schema = Schema::string();
    let error = schema.decode(Unknown::Number(1.0)).unwrap_err();
    assert!(matches!(error, SchemaError::TypeMismatch { .. }));
    assert!(error.to_string().contains("string"));
}

#[test]
fn arrays_decode_elementwise() {
    let schema = Schema::array(Schema::number());
    let value = schema
        .decode(Unknown::Array(vec![
            Unknown::Number(1.0),
            Unknown::Number(2.0),
        ]))
        .unwrap();
    assert_eq!(value, vec![1.0, 2.0]);
}

#[test]
fn array_element_errors_carry_index_path() {
    let schema = Schema::array(Schema::number());
    let error = schema
        .decode(Unknown::Array(vec![
            Unknown::Number(1.0),
            Unknown::Text("no".to_string()),
        ]))
        .unwrap_err();
    assert!(error.to_string().contains("[1]"));
}

#[test]
fn optional_accepts_missing_and_null() {
    let schema = Schema::optional(Schema::string());
    assert_eq!(
        schema.decode(Unknown::Null).unwrap(),
        None,
        "null decodes to None"
    );
    assert_eq!(
        schema.decode(Unknown::Text("x".to_string())).unwrap(),
        Some("x".to_string())
    );
    assert_eq!(schema.try_encode(&None).unwrap(), Unknown::Null);
}

#[test]
fn structs_decode_fieldwise() {
    #[derive(Debug, PartialEq)]
    struct Point {
        x: f64,
        y: f64,
    }
    let schema = Schema::convert(
        Schema::object(),
        |mut fields: std::collections::BTreeMap<String, Unknown>| {
            let x = Schema::number().decode(fields.remove("x").unwrap_or(Unknown::Null))?;
            let y = Schema::number().decode(fields.remove("y").unwrap_or(Unknown::Null))?;
            Ok(Point { x, y })
        },
        |point: &Point| {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("x".to_string(), Unknown::Number(point.x));
            fields.insert("y".to_string(), Unknown::Number(point.y));
            fields
        },
    );
    let mut input = std::collections::BTreeMap::new();
    input.insert("x".to_string(), Unknown::Number(1.0));
    input.insert("y".to_string(), Unknown::Number(2.0));
    let point = schema.decode(Unknown::Object(input)).unwrap();
    assert_eq!(point, Point { x: 1.0, y: 2.0 });
    let Unknown::Object(back) = schema.try_encode(&point).unwrap() else {
        panic!("struct encodes to an object");
    };
    assert_eq!(back["x"], Unknown::Number(1.0));
}

#[test]
fn missing_field_names_the_path() {
    let schema = Schema::field("name", Schema::string());
    let error = schema
        .decode(Unknown::Object(std::collections::BTreeMap::new()))
        .unwrap_err();
    assert!(error.to_string().contains("name"));
}

#[test]
fn json_bytes_decode_and_encode() {
    let schema = Schema::array(Schema::string());
    let value = schema.decode_json(r#"["a", "b"]"#).unwrap();
    assert_eq!(value, vec!["a".to_string(), "b".to_string()]);
    let back = schema.encode_json(&value).unwrap();
    assert_eq!(back, r#"["a","b"]"#);
}

#[test]
fn malformed_json_is_a_schema_error() {
    let schema = Schema::string();
    assert!(schema.decode_json(r#"{"oops"#).is_err());
}

#[test]
fn ffs_decodes_plain_numbers() {
    let schema = Schema::finite_from_string();
    assert_eq!(schema.decode(Unknown::Text("1.5".to_string())), Ok(1.5));
    assert_eq!(schema.decode(Unknown::Text("-0.5".to_string())), Ok(-0.5));
    assert_eq!(schema.decode(Unknown::Text("1e3".to_string())), Ok(1000.0));
    assert_eq!(schema.decode(Unknown::Text("  42  ".to_string())), Ok(42.0));
}

#[test]
fn ffs_rejects_non_numeric_text() {
    let schema = Schema::finite_from_string();
    for text in [
        "",
        "   ",
        "abc",
        "12px",
        "NaN",
        "Infinity",
        "-Infinity",
        "0x10",
    ] {
        assert!(
            schema.decode(Unknown::Text(text.to_string())).is_err(),
            "{text:?} must not decode"
        );
    }
}

#[test]
fn ffs_rejects_non_text() {
    let schema = Schema::finite_from_string();
    assert!(schema.decode(Unknown::Number(1.5)).is_err());
    assert!(schema.decode(Unknown::Null).is_err());
    assert!(schema.decode(Unknown::Bool(true)).is_err());
}

#[test]
fn ffs_encodes_numbers_to_text() {
    let schema = Schema::finite_from_string();
    assert_eq!(
        schema.try_encode(&1.5).unwrap(),
        Unknown::Text("1.5".to_string())
    );
}

#[test]
fn ffs_round_trips() {
    let schema = Schema::finite_from_string();
    let value = schema.decode(Unknown::Text("42".to_string())).unwrap();
    assert_eq!(value, 42.0);
    assert_eq!(
        schema.try_encode(&value).unwrap(),
        Unknown::Text("42".to_string())
    );
}

#[test]
fn ffs_composes_with_optional() {
    let schema = Schema::optional(Schema::finite_from_string());
    assert_eq!(schema.decode(Unknown::Null).unwrap(), None);
    assert_eq!(
        schema.decode(Unknown::Text("2.5".to_string())).unwrap(),
        Some(2.5)
    );
    assert!(schema.decode(Unknown::Text("nope".to_string())).is_err());
}

#[test]
fn ffs_tuples_union_by_shape() {
    let left = Schema::as_unknown(Schema::tuple2(
        Schema::finite_from_string(),
        Schema::string(),
    ));
    let right = Schema::as_unknown(Schema::tuple2(Schema::boolean(), Schema::string()));
    let schema = Schema::union(left, right);
    assert_eq!(
        schema.decode(Unknown::Array(vec![
            Unknown::Text("1".to_string()),
            Unknown::Text("x".to_string())
        ])),
        Ok(Unknown::Array(vec![
            Unknown::Text("1".to_string()),
            Unknown::Text("x".to_string())
        ])),
        "numeric-first tuple takes the left branch"
    );
    assert_eq!(
        schema.decode(Unknown::Array(vec![
            Unknown::Bool(true),
            Unknown::Text("x".to_string())
        ])),
        Ok(Unknown::Array(vec![
            Unknown::Bool(true),
            Unknown::Text("x".to_string())
        ])),
        "boolean-first tuple falls to the right branch"
    );
    assert!(schema.decode(Unknown::Array(vec![Unknown::Null])).is_err());
}

// Schema.test.ts keyword rows: the scalar and keyword schemas the pin exposes,
// plus the two struct-input rules they exercise.
//
// The ledger numbers below are the tsv's own row numbers, which run behind the
// pin's `it(` lines, so they are what a reader should look up.

#[test]
fn literal_refuses_a_non_finite_number() {
    // Schema.test.ts:219 — a numeric literal that is not finite is refused at
    // construction with the value spelled the way JavaScript spells it.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for (value, expected) in [
        (
            f64::INFINITY,
            "A numeric literal must be finite, got Infinity",
        ),
        (
            f64::NEG_INFINITY,
            "A numeric literal must be finite, got -Infinity",
        ),
        (f64::NAN, "A numeric literal must be finite, got NaN"),
    ] {
        let caught = std::panic::catch_unwind(|| Schema::literal(Unknown::Number(value)));
        let message = caught
            .err()
            .and_then(|payload| payload.downcast_ref::<String>().cloned())
            .expect("a non-finite literal is refused");
        assert_eq!(message, expected);
    }
    std::panic::set_hook(hook);
    // A finite literal still builds.
    assert_eq!(
        Schema::literal(Unknown::Number(1.5)).decode(Unknown::Number(1.5)),
        Ok(Unknown::Number(1.5))
    );
}

#[test]
fn literal_accepts_either_signed_zero_and_keeps_the_input_sign() {
    // Schema.test.ts:277 — a zero literal admits both signed zeros, and the
    // decoded and encoded values keep the sign the input carried.
    for literal in [0.0_f64, -0.0_f64] {
        let schema = Schema::literal(Unknown::Number(literal));
        for input in [0.0_f64, -0.0_f64] {
            let decoded = schema.decode(Unknown::Number(input)).expect("either zero");
            let Unknown::Number(number) = decoded.clone() else {
                panic!("a number decoded to {decoded:?}");
            };
            assert_eq!(
                number.is_sign_negative(),
                input.is_sign_negative(),
                "{literal} {input}"
            );
            let Unknown::Number(encoded) = schema.try_encode(&decoded).unwrap() else {
                panic!("a number encoded to something else");
            };
            assert_eq!(encoded.is_sign_negative(), input.is_sign_negative());
        }
    }
}

#[test]
fn unknown_admits_every_value() {
    // Schema.test.ts:362 — Unknown decodes and encodes any value unchanged.
    let schema = Schema::unknown();
    for value in [
        Unknown::Text("a".to_string()),
        Unknown::Number(1.0),
        Unknown::Bool(false),
        Unknown::Null,
        Unknown::Array(vec![Unknown::Null]),
        Unknown::Object(std::collections::BTreeMap::new()),
    ] {
        assert_eq!(schema.decode(value.clone()), Ok(value.clone()));
        assert_eq!(schema.try_encode(&value).unwrap(), value);
    }
}

#[test]
fn string_for_literal_autocomplete_is_the_string_schema() {
    // Schema.test.ts:408 — the autocomplete string has the string AST and the
    // string behavior: text decodes, anything else does not.
    let schema = Schema::string_for_literal_autocomplete();
    assert_eq!(schema.json_schema(), Schema::string().json_schema());
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())),
        Ok("a".to_string())
    );
    for (refused, got) in [(Unknown::Null, "null"), (Unknown::Number(1.0), "number")] {
        assert_eq!(
            schema.decode(refused).unwrap_err().to_string(),
            format!("expected string at $, got {got}")
        );
    }
}

#[test]
fn union_of_autocomplete_string_and_literals_accepts_every_string() {
    // Schema.test.ts:427 — a union of the autocomplete string with two literals
    // admits every string, both literals included, and nothing else. The pin
    // also reads `members[1].literals`; the port's union kind is opaque, so the
    // acceptance is what carries.
    let members = Schema::union(
        Schema::literal(Unknown::Text("GET".to_string())),
        Schema::literal(Unknown::Text("POST".to_string())),
    );
    let schema = Schema::union(
        Schema::as_unknown(Schema::string_for_literal_autocomplete()),
        members,
    );
    for accepted in ["GET", "POST", "PATCH"] {
        assert_eq!(
            schema.decode(Unknown::Text(accepted.to_string())),
            Ok(Unknown::Text(accepted.to_string())),
            "{accepted}"
        );
    }
    for refused in [Unknown::Null, Unknown::Number(1.0)] {
        assert!(schema.decode(refused).is_err());
    }
}

#[test]
fn void_yields_the_unit_value_for_any_input() {
    // Schema.test.ts:537 — Void decodes anything at all and yields the value
    // that has no content. The port encodes that as null, the stand-in for
    // JavaScript's undefined.
    let schema = Schema::void();
    for input in [
        Unknown::Null,
        Unknown::Text("a".to_string()),
        Unknown::Object(std::collections::BTreeMap::new()),
    ] {
        assert_eq!(schema.decode(input), Ok(()));
    }
    assert_eq!(schema.try_encode(&()).unwrap(), Unknown::Null);
}

#[test]
fn object_keyword_admits_objects_and_arrays() {
    // Schema.test.ts:565 — ObjectKeyword takes an object or an array and refuses
    // a scalar.
    let schema = Schema::object_keyword();
    assert_eq!(
        schema.decode(Unknown::Object(std::collections::BTreeMap::new())),
        Ok(Unknown::Object(std::collections::BTreeMap::new()))
    );
    assert_eq!(
        schema.decode(Unknown::Array(Vec::new())),
        Ok(Unknown::Array(Vec::new()))
    );
    let error = schema.decode(Unknown::Text("1".to_string())).unwrap_err();
    assert_eq!(
        error.to_string(),
        "expected object | array at $, got string"
    );
    assert_eq!(schema.try_encode(&Unknown::Null).unwrap(), Unknown::Null);
}

#[test]
fn struct_decode_ignores_undeclared_keys() {
    // Schema.test.ts:666 — decoding with undeclared keys dropped: the result
    // holds the declared fields only, whatever else arrived. The pin's row also
    // sends a symbol key, which has no counterpart in a string-keyed object.
    use elm_effect_schema::Field;
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let mut input = std::collections::BTreeMap::new();
    input.insert("a".to_string(), Unknown::Text("a".to_string()));
    input.insert("b".to_string(), Unknown::Text("b".to_string()));
    input.insert("c".to_string(), Unknown::Text("c".to_string()));
    let mut expected = std::collections::BTreeMap::new();
    expected.insert("a".to_string(), Unknown::Text("a".to_string()));
    assert_eq!(schema.decode(Unknown::Object(input)), Ok(expected));
}

// --- Schema.test.ts: the union rows, whose law is which member a value
// resolves to and what the whole union refuses ---

/// Schema.test.ts:2038 — `Record(String, Finite).check(isPropertyNames(String
/// .check(isPattern(/^[A-Z]/))))`: a property-names check decides every key of
/// the object it is attached to, so a key that fails the key schema fails the
/// whole record and names itself.
#[test]
fn property_names_check_decides_every_key() {
    let key = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isPattern",
        vec![r#"new RegExp("^[A-Z]", "u")"#],
        vec![("expected", "a string matching the RegExp ^[A-Z]")],
    );
    let record: Schema<Unknown> =
        Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::number())));
    let schema = Schema::checked_property_names(
        record,
        key,
        vec![(
            "expected",
            "an object with property names matching the schema",
        )],
    );

    let ok = std::collections::BTreeMap::from([("Ab".to_string(), Unknown::Number(1.0))]);
    assert!(
        schema.decode(Unknown::Object(ok)).is_ok(),
        "a matching key decodes"
    );

    let bad = std::collections::BTreeMap::from([("ab".to_string(), Unknown::Number(1.0))]);
    let error = schema
        .decode(Unknown::Object(bad))
        .expect_err("a key outside the pattern is refused");
    let printed = error.to_string();
    assert!(
        printed.contains("ab"),
        "the failing key is named: {printed}"
    );
}

/// Schema.test.ts:2053 — `isPropertyNames(Schema.Never)`: a key schema that
/// admits no key makes the empty object the only admissible one.
#[test]
fn property_names_check_with_never_admits_only_the_empty_object() {
    let record: Schema<Unknown> =
        Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::number())));
    let schema = Schema::checked_property_names(
        record,
        Schema::as_unknown(Schema::never()),
        vec![(
            "expected",
            "an object with property names matching the schema",
        )],
    );
    assert!(
        schema
            .decode(Unknown::Object(std::collections::BTreeMap::new()))
            .is_ok()
    );
    let error = schema
        .decode(Unknown::Object(std::collections::BTreeMap::from([(
            "a".to_string(),
            Unknown::Number(1.0),
        )])))
        .expect_err("a never key schema admits no key");
    assert!(error.to_string().contains("never"), "{error}");
    assert!(error.to_string().contains("a"), "{error}");
}

/// Schema.test.ts:4053 — `Record(String.check(isPattern(/^a/)), Number)`: the
/// key schema selects which entries the record describes, so an entry whose
/// key fails it is dropped rather than failing the record, and an entry whose
/// key passes still has its value decoded.
#[test]
fn record_with_a_checked_key_selects_its_entries() {
    let key = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isPattern",
        vec![r#"new RegExp("^a", "u")"#],
        vec![("expected", "a string matching the RegExp ^a")],
    );
    let schema = Schema::record2(key, Schema::as_unknown(Schema::number()));

    let input = std::collections::BTreeMap::from([
        ("a".to_string(), Unknown::Number(1.0)),
        ("ab".to_string(), Unknown::Number(2.0)),
        ("b".to_string(), Unknown::Text("ignored".to_string())),
    ]);
    let decoded = schema.decode(Unknown::Object(input)).expect("decodes");
    assert_eq!(
        decoded,
        std::collections::BTreeMap::from([
            ("a".to_string(), Unknown::Number(1.0)),
            ("ab".to_string(), Unknown::Number(2.0))
        ]),
        "the entry whose key fails the pattern is dropped"
    );

    let bad = std::collections::BTreeMap::from([
        ("a".to_string(), Unknown::Text("bad".to_string())),
        ("b".to_string(), Unknown::Number(1.0)),
    ]);
    let error = schema
        .decode(Unknown::Object(bad))
        .expect_err("a selected entry's value still decodes");
    assert!(error.to_string().contains("number"), "{error}");
    assert!(error.to_string().contains("$.a"), "{error}");
}

/// Schema.test.ts:4246 — `Record(Number, String)`: a numeric key schema
/// admits the keys whose text is a number, so `{ "1": "a" }` decodes and
/// `{ a: "b" }` describes no entry at all. The port's number-from-string is
/// the key schema that spells this, because a wire key arrives as text.
#[test]
fn record_with_a_number_key_selects_numeric_keys() {
    let schema = Schema::record2(
        Schema::as_unknown(Schema::number_from_string()),
        Schema::as_unknown(Schema::string()),
    );
    let input =
        std::collections::BTreeMap::from([("1".to_string(), Unknown::Text("a".to_string()))]);
    let decoded = schema.decode(Unknown::Object(input)).expect("decodes");
    assert_eq!(
        decoded,
        std::collections::BTreeMap::from([("1".to_string(), Unknown::Text("a".to_string()))]),
        "the numeric key is selected"
    );
    let other =
        std::collections::BTreeMap::from([("a".to_string(), Unknown::Text("b".to_string()))]);
    let decoded = schema.decode(Unknown::Object(other)).expect("decodes");
    assert!(
        decoded.is_empty(),
        "a non-numeric key describes no entry: {decoded:?}"
    );
}

/// Schema.test.ts:4322 — `Schema.Union([])`: a union with no members admits
/// nothing, and the refusal names what there is nothing of.
#[test]
fn union_with_no_members_admits_nothing() {
    let schema = Schema::union_with_mode(Vec::new(), "anyOf");
    let error = schema
        .decode(Unknown::Null)
        .expect_err("no member can match");
    assert!(error.to_string().contains("never"), "{error}");
}

/// Schema.test.ts:4330 — `Schema.Union([Schema.String])`: a one-member union
/// is that member, so the member's own refusal is what a non-member sees
/// rather than a union-shaped one.
#[test]
fn union_of_one_member_decodes_as_that_member() {
    let schema = Schema::union_with_mode(vec![Schema::as_unknown(Schema::string())], "anyOf");
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    let error = schema
        .decode(Unknown::Null)
        .expect_err("null is not a string");
    assert!(error.to_string().contains("string"), "{error}");
}

/// Schema.test.ts:4339 — `Schema.Union([Schema.Void])`: the void member
/// admits every value and yields the absent one, so the union admits
/// everything too.
#[test]
fn union_of_void_admits_every_value() {
    let schema = Schema::union_with_mode(vec![Schema::as_unknown(Schema::void())], "anyOf");
    for value in [
        Unknown::Null,
        Unknown::Bool(true),
        Unknown::Number(1.0),
        Unknown::Text("a".to_string()),
        Unknown::Array(vec![]),
        Unknown::Object(std::collections::BTreeMap::new()),
    ] {
        assert!(schema.decode(value).is_ok(), "void admits every value");
    }
}

/// Schema.test.ts:4352 — `Schema.Union([Schema.Void, Schema.String])`: the
/// void member comes first, so every value resolves to it rather than to the
/// string member behind it.
#[test]
fn union_of_void_then_string_resolves_to_the_void_member() {
    let schema = Schema::union_with_mode(
        vec![
            Schema::as_unknown(Schema::void()),
            Schema::as_unknown(Schema::string()),
        ],
        "anyOf",
    );
    let decoded = schema.decode(Unknown::Text("a".to_string())).unwrap();
    assert_eq!(decoded, Unknown::Null, "the first member wins");
}

/// Schema.test.ts:4360 — `Schema.Union([Schema.String, Schema.Number])`: each
/// member takes its own values, and the members are exposed in the order they
/// were declared.
#[test]
fn union_of_string_and_number_takes_each_members_values() {
    let schema = Schema::union_with_mode(
        vec![
            Schema::as_unknown(Schema::string()),
            Schema::as_unknown(Schema::number()),
        ],
        "anyOf",
    );
    let members = schema.members().expect("a union exposes its members");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].to_code(), "Schema.String");
    assert_eq!(members[1].to_code(), "Schema.Number");
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    let error = schema.decode(Unknown::Null).expect_err("null is neither");
    assert!(error.to_string().contains("string | number"), "{error}");
}

/// Schema.test.ts:4375 — `Schema.Union([Schema.String, Schema.Never])`: a
/// never member contributes no values, so the union is the other member and
/// the refusal still names both.
#[test]
fn union_of_string_and_never_is_the_other_member() {
    let schema = Schema::union_with_mode(
        vec![
            Schema::as_unknown(Schema::string()),
            Schema::as_unknown(Schema::never()),
        ],
        "anyOf",
    );
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert!(schema.decode(Unknown::Null).is_err());
}

/// Schema.test.ts:4384 — a union of checked members: each member's own check
/// decides its own values, so the member that matched is the one whose message
/// a failing value sees.
#[test]
fn union_of_checked_members_decides_by_that_members_check() {
    let non_empty = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isMinLength",
        vec!["1"],
        vec![("expected", "a value with a length of at least 1")],
    );
    let positive = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isGreaterThan",
        vec!["0"],
        vec![("expected", "a value greater than 0")],
    );
    let schema = Schema::union_with_mode(vec![non_empty, positive], "anyOf");
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert!(
        schema.decode(Unknown::Text(String::new())).is_err(),
        "an empty string is too short"
    );
    assert!(
        schema.decode(Unknown::Number(-1.0)).is_err(),
        "a negative number is too small"
    );
}

/// Schema.test.ts:5182 — `Schema.NullishOr(String)`: null and the absent value
/// both decode, and the inner schema still decides everything else. The port
/// stands `Unknown::Null` in for the pin's `undefined`, so one wire value
/// carries both of the row's absent cases.
#[test]
fn nullish_or_admits_the_absent_values_and_the_inner_schema() {
    let inner = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isMinLength",
        vec!["1"],
        vec![("expected", "a value with a length of at least 1")],
    );
    let schema = Schema::union_with_mode(vec![inner, Schema::as_unknown(Schema::null())], "anyOf");
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert!(schema.decode(Unknown::Null).is_ok(), "null is admitted");
    assert!(
        schema.decode(Unknown::Text(String::new())).is_err(),
        "the inner check still decides a string"
    );
    assert!(
        schema.decode(Unknown::Number(1.0)).is_err(),
        "a number is neither"
    );
}

// --- Schema.test.ts: the tuple element-ordering rule ---

/// Schema.test.ts:1027 — a required element cannot follow an optional one, in
/// either spelling of "optional": `optionalKey` keeps the position
/// absent-or-present and `optional` also accepts an explicit null, and neither
/// can be followed by a required element. The refusal is at construction, so
/// the panic is caught the way the literal row catches its own.
#[test]
fn tuple_refuses_a_required_element_after_an_optional_one() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for optional in [
        Schema::optional_key_shape(Schema::as_unknown(Schema::string())),
        Schema::as_unknown(Schema::optional(Schema::as_unknown(Schema::string()))),
    ] {
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Schema::tuple_of(vec![optional, Schema::as_unknown(Schema::string())])
        }));
        // `assert!` with a plain literal panics with a `&'static str` payload
        // and a formatted one with a `String`, so both are read.
        let message = caught
            .err()
            .and_then(|payload| {
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
            })
            .expect("the tuple is refused at construction");
        assert_eq!(
            message,
            "A required element cannot follow an optional element. ts(1257)"
        );
    }
    std::panic::set_hook(hook);
    // The order the rule permits still builds and decodes.
    let ordered = Schema::tuple_of(vec![
        Schema::as_unknown(Schema::string()),
        Schema::optional_key_shape(Schema::as_unknown(Schema::boolean())),
    ]);
    assert!(
        ordered
            .decode(Unknown::Array(vec![Unknown::Text("a".to_string())]))
            .is_ok()
    );
}

/// Schema.test.ts:4872 — the same rule over a tuple that carries a rest: the
/// leading positions are still positional, so a required one may not follow an
/// optional one there either. The port's `tuple_with_rest` takes one rest
/// schema where the pin takes a list of trailing elements, so the rule is
/// checked over the leading positions, which is the part both signatures can
/// express.
#[test]
fn tuple_with_rest_refuses_a_required_element_after_an_optional_one() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Schema::tuple_with_rest(
            vec![
                Schema::optional_key_shape(Schema::as_unknown(Schema::string())),
                Schema::as_unknown(Schema::boolean()),
            ],
            Schema::as_unknown(Schema::number()),
        )
    }));
    let message = caught
        .err()
        .and_then(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        })
        .expect("the tuple is refused at construction");
    assert_eq!(
        message,
        "A required element cannot follow an optional element. ts(1257)"
    );
    std::panic::set_hook(hook);
    // The permitted order still builds and decodes.
    let ordered = Schema::tuple_with_rest(
        vec![Schema::as_unknown(Schema::string())],
        Schema::as_unknown(Schema::number()),
    );
    assert!(
        ordered
            .decode(Unknown::Array(vec![
                Unknown::Text("a".to_string()),
                Unknown::Number(1.0),
            ]))
            .is_ok()
    );
}

// --- Schema.test.ts: the tuple element rows, where the element schema is
// what decides each position and the empty case is the optional one ---

/// Schema.test.ts:1058 — `Schema.Tuple([Schema.NonEmptyString])`: the element
/// schema decides the position, and a failing element names the index it failed
/// at while a non-array names the tuple's own shape.
#[test]
fn tuple_of_checked_elements_names_the_index_a_failing_element_failed_at() {
    let element = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isMinLength",
        vec!["1"],
        vec![("expected", "a value with a length of at least 1")],
    );
    let schema = Schema::tuple_of(vec![element]);
    assert!(
        schema
            .decode(Unknown::Array(vec![Unknown::Text("a".to_string())]))
            .is_ok()
    );
    let error = schema
        .decode(Unknown::Array(vec![Unknown::Text(String::new())]))
        .expect_err("an empty string is too short for the element");
    // The pin's own text for the element's check, at the element's index.
    assert_eq!(
        error.to_string(),
        "expected a value with a length of at least 1 at $[0], got string",
        "the index is named"
    );
    let error = schema
        .decode(Unknown::Null)
        .expect_err("null is not an array");
    assert!(error.to_string().contains("array"), "{error}");
}

/// Schema.test.ts:1101 — `Schema.Tuple([String.pipe(optionalKey)])`: an
/// optional element makes the empty tuple a value of its own, in both
/// directions, rather than a length failure.
#[test]
fn tuple_with_an_optional_element_accepts_the_empty_tuple() {
    let schema = Schema::tuple_of(vec![Schema::optional_key_shape(Schema::as_unknown(
        Schema::string(),
    ))]);
    for wire in [
        Unknown::Array(vec![Unknown::Text("a".to_string())]),
        Unknown::Array(vec![]),
    ] {
        let decoded = schema.decode(wire.clone()).expect("both shapes decode");
        assert_eq!(
            schema.try_encode(&decoded).unwrap(),
            wire,
            "and both encode back"
        );
    }
}

/// Schema.test.ts:1126 — `Schema.Array(Schema.String)`: the element schema
/// decides every element, and the array itself accepts the empty case.
#[test]
fn array_of_strings_decides_each_element() {
    let schema: Schema<Unknown> =
        Schema::as_unknown(Schema::array(Schema::as_unknown(Schema::string())));
    assert_eq!(
        schema.decode(Unknown::Array(vec![Unknown::Text("a".to_string())])),
        Ok(Unknown::Array(vec![Unknown::Text("a".to_string())]))
    );
    assert_eq!(
        schema.decode(Unknown::Array(vec![])),
        Ok(Unknown::Array(vec![]))
    );
    let error = schema
        .decode(Unknown::Array(vec![Unknown::Number(1.0)]))
        .expect_err("a non-string element is refused");
    assert!(error.to_string().contains("string"), "{error}");
    let error = schema
        .decode(Unknown::Null)
        .expect_err("null is not an array");
    assert!(error.to_string().contains("array"), "{error}");
}

// --- Schema.test.ts: the mapFields rows, where a field list is transformed
// and the struct that comes out decides its own keys ---

fn one_field_struct() -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )])
}

fn two_field_struct() -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(Schema::string())),
    ])
}

/// Schema.test.ts:942 — `from.mapFields(Struct.assign({ b: String }))`: the
/// assigned field joins the declared ones, so the struct that comes out
/// requires both and names each missing one.
#[test]
fn map_fields_assign_joins_the_declared_fields() {
    let schema = one_field_struct().map_fields(|mut fields| {
        fields.push(Field::required("b", Schema::as_unknown(Schema::string())));
        fields
    });
    let ok = BTreeMap::from([
        ("a".to_string(), Unknown::Text("a".to_string())),
        ("b".to_string(), Unknown::Text("b".to_string())),
    ]);
    assert_eq!(
        schema
            .decode(Unknown::Object(ok))
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["a".to_string(), "b".to_string()]
    );
    for (wire, missing) in [
        (
            BTreeMap::from([("b".to_string(), Unknown::Text("b".to_string()))]),
            "a",
        ),
        (
            BTreeMap::from([("a".to_string(), Unknown::Text("a".to_string()))]),
            "b",
        ),
    ] {
        let error = schema
            .decode(Unknown::Object(wire))
            .expect_err("each assigned field is required");
        assert!(error.to_string().contains(missing), "{error}");
    }
}

/// Schema.test.ts:963 — overlapping fields: an assigned field that repeats a
/// declared name replaces it, so the value decodes with the NEW schema rather
/// than the declared one.
#[test]
fn map_fields_assign_replaces_an_overlapping_field() {
    let schema = two_field_struct().map_fields(|mut fields| {
        for field in &mut fields {
            if field.name() == "b" {
                *field = Field::required("b", Schema::as_unknown(Schema::number()));
            }
        }
        fields
    });
    let ok = BTreeMap::from([
        ("a".to_string(), Unknown::Text("a".to_string())),
        ("b".to_string(), Unknown::Number(1.0)),
    ]);
    assert!(schema.decode(Unknown::Object(ok)).is_ok());
    let bad = BTreeMap::from([
        ("a".to_string(), Unknown::Text("a".to_string())),
        ("b".to_string(), Unknown::Text("b".to_string())),
    ]);
    let error = schema
        .decode(Unknown::Object(bad))
        .expect_err("the replaced field decodes as a number now");
    assert!(error.to_string().contains("number"), "{error}");
    assert!(error.to_string().contains("b"), "{error}");
}

/// Schema.test.ts:1000 — `mapFields(Struct.pick(["a"]))`: a picked field list
/// is the whole struct, so the field that was dropped is no longer required.
#[test]
fn map_fields_pick_keeps_only_the_named_fields() {
    let schema = Schema::pick(two_field_struct().fields().unwrap().to_vec(), &["a"]);
    let ok = BTreeMap::from([("a".to_string(), Unknown::Text("a".to_string()))]);
    assert!(schema.decode(Unknown::Object(ok)).is_ok());
    let bad = BTreeMap::from([
        ("a".to_string(), Unknown::Text("a".to_string())),
        ("b".to_string(), Unknown::Text("b".to_string())),
    ]);
    assert!(
        schema.decode(Unknown::Object(bad)).is_ok(),
        "b is not a declared key now"
    );
}

/// Schema.test.ts:1013 — `mapFields(Struct.omit(["b"]))`: the omitted field is
/// dropped from the struct rather than made optional.
#[test]
fn map_fields_omit_drops_the_named_field() {
    let kept: Vec<Field> = two_field_struct()
        .fields()
        .unwrap()
        .iter()
        .filter(|field| field.name() != "b")
        .cloned()
        .collect();
    let schema = Schema::struct_shape(kept);
    let ok = BTreeMap::from([("a".to_string(), Unknown::Text("a".to_string()))]);
    assert!(schema.decode(Unknown::Object(ok)).is_ok());
    let bad = BTreeMap::from([
        ("b".to_string(), Unknown::Text("b".to_string())),
        ("a".to_string(), Unknown::Text("a".to_string())),
    ]);
    assert!(
        schema.decode(Unknown::Object(bad)).is_ok(),
        "b is not declared"
    );
}

// --- Schema.test.ts: the StructWithRest rows, where a fixed key and a key the
// rest names are one identifier ---

/// Schema.test.ts:4997 — a `StructWithRest` whose rest names the same key as a
/// fixed property is a duplicate identifier: the key would be claimed twice, so
/// the shape is refused at construction rather than silently deciding one
/// lookup wins.
#[test]
fn struct_with_rest_refuses_a_fixed_key_the_rest_also_names() {
    let rest = Schema::record2(
        Schema::as_unknown(Schema::literals(vec![
            Unknown::Text("a".to_string()),
            Unknown::Text("b".to_string()),
        ])),
        Schema::as_unknown(Schema::number()),
    );
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Schema::struct_with_rest(
            vec![("a".to_string(), Schema::as_unknown(Schema::number()))],
            Schema::as_unknown(rest),
        )
    }));
    let message = caught
        .err()
        .and_then(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        })
        .expect("the duplicate is refused at construction");
    assert_eq!(message, "Duplicate identifier: \"a\". ts(2300)");
    std::panic::set_hook(hook);
    // A rest that names no key the fixed properties claim still builds.
    let plain: Schema<Unknown> =
        Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::number())));
    let schema = Schema::struct_with_rest(
        vec![("a".to_string(), Schema::as_unknown(Schema::number()))],
        plain,
    );
    let decoded = schema
        .decode(Unknown::Object(BTreeMap::from([
            ("a".to_string(), Unknown::Number(1.0)),
            ("z".to_string(), Unknown::Number(2.0)),
        ])))
        .expect("the fixed key and the rest's key both decode");
    assert_eq!(
        decoded,
        Unknown::Object(BTreeMap::from([
            ("a".to_string(), Unknown::Number(1.0)),
            ("z".to_string(), Unknown::Number(2.0)),
        ]))
    );
    // A value the rest's value schema refuses is refused at that key.
    let bad = BTreeMap::from([
        ("a".to_string(), Unknown::Number(1.0)),
        ("z".to_string(), Unknown::Text(String::new())),
    ]);
    let error = schema
        .decode(Unknown::Object(bad))
        .expect_err("the rest's value schema still decides each key");
    assert!(error.to_string().contains("number"), "{error}");
    assert!(error.to_string().contains("z"), "{error}");
}

// --- Schema.test.ts: the string checks, where the predicate is the law ---

/// Schema.test.ts:1288 — `Schema.Trimmed`: a trimmed string is a string with
/// no leading or trailing whitespace, so the middle of the word is fine and the
/// padding around it is refused.
#[test]
fn trimmed_string_refuses_padding_but_not_an_inner_space() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::string()),
        "isTrimmed",
        Vec::new(),
        vec![(
            "expected",
            "a string with no leading or trailing whitespace",
        )],
    );
    assert!(schema.decode(text("a")).is_ok(), "a bare word is trimmed");
    assert!(schema.decode(text("a b")).is_ok(), "an inner space is fine");
    for padded in [" a ", "a ", " a", "\ta", "a\n"] {
        let error = schema.decode(text(padded)).expect_err("padding is refused");
        assert_eq!(
            error.to_string(),
            "expected a string with no leading or trailing whitespace at $, got string",
            "{padded:?}"
        );
    }
    // A non-string is the string schema's refusal, not the check's.
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
}
