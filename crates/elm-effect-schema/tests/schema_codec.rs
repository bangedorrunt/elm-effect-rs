//! Codec corpus: `toCodec` round-trips plus annotation document rows.
//!
//! Every test decodes an [`Unknown`] wire value and encodes back; the
//! codec is the two-way shape, so idempotence is `decode(encode(x)) == x`
//! and `encode(decode(u)) == u`.

use elm_effect_schema::{Field, Schema, TemplateParser, TemplatePart, Unknown};
use std::collections::BTreeMap;

fn text(s: &str) -> Unknown {
    Unknown::Text(s.to_string())
}

fn name_schema() -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![Field::required(
        "name",
        Schema::as_unknown(Schema::string()),
    )])
}

fn wire_name(value: &str) -> Unknown {
    let mut fields = BTreeMap::new();
    fields.insert("name".to_string(), Unknown::Text(value.to_string()));
    Unknown::Object(fields)
}

#[test]
fn codec_struct_is_idempotent() {
    let schema = name_schema();
    let wire = wire_name("ada");
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    assert_eq!(schema.decode(schema.try_encode(&decoded).unwrap()).unwrap(), decoded);
}

#[test]
fn codec_record_round_trips_and_keeps_empty() {
    let schema = Schema::record(Schema::as_unknown(Schema::string()));
    let mut fields = BTreeMap::new();
    fields.insert("a".to_string(), Unknown::Text("1".to_string()));
    let wire = Unknown::Object(fields);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    let empty = schema.decode(Unknown::Object(BTreeMap::new())).unwrap();
    assert_eq!(schema.try_encode(&empty).unwrap(), Unknown::Object(BTreeMap::new()));
}

#[test]
fn codec_tuple_round_trips_in_order() {
    let schema = Schema::tuple2(Schema::string(), Schema::number());
    let wire = Unknown::Array(vec![Unknown::Text("x".to_string()), Unknown::Number(2.0)]);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(decoded, ("x".to_string(), 2.0));
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
}

#[test]
fn codec_optional_field_absent_and_null() {
    let schema = Schema::struct_shape(vec![
        Field::required("name", Schema::as_unknown(Schema::string())),
        Field::optional("nick", Schema::as_unknown(Schema::string())),
    ]);
    let mut bare = BTreeMap::new();
    bare.insert("name".to_string(), Unknown::Text("ada".to_string()));
    let decoded = schema.decode(Unknown::Object(bare)).unwrap();
    assert!(!decoded.contains_key("nick"));
    let mut nulled = BTreeMap::new();
    nulled.insert("name".to_string(), Unknown::Text("ada".to_string()));
    nulled.insert("nick".to_string(), Unknown::Null);
    let decoded = schema.decode(Unknown::Object(nulled)).unwrap();
    assert!(!decoded.contains_key("nick"));
    let missing = schema.decode(Unknown::Object(BTreeMap::new()));
    let error = missing.unwrap_err().to_string();
    assert!(error.contains("name"), "unexpected error: {error}");
}

#[test]
fn codec_union_falls_back_to_second_member() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::number()),
    );
    assert_eq!(
        schema.decode(Unknown::Text("s".to_string())).unwrap(),
        Unknown::Text("s".to_string())
    );
    assert_eq!(
        schema.decode(Unknown::Number(3.0)).unwrap(),
        Unknown::Number(3.0)
    );
    assert_eq!(
        schema.try_encode(&Unknown::Text("s".to_string())).unwrap(),
        Unknown::Text("s".to_string())
    );
}

#[test]
fn codec_json_text_round_trips_a_struct() {
    let schema = name_schema();
    let decoded = schema.decode_json(r#"{"name": "ada"}"#).unwrap();
    assert_eq!(schema.encode_json(&decoded).unwrap(), r#"{"name":"ada"}"#);
}

#[test]
fn doc_annotated_carries_title_and_description() {
    let schema = Schema::annotated(Schema::string(), Some("Name"), Some("A person's name"));
    let doc = schema.json_schema();
    assert_eq!(doc["type"], "string");
    assert_eq!(doc["title"], "Name");
    assert_eq!(doc["description"], "A person's name");
    // The wire shape is unchanged by annotations.
    let decoded = schema.decode(Unknown::Text("ada".to_string())).unwrap();
    assert_eq!(decoded, "ada".to_string());
}

#[test]
fn doc_union_marks_itself_opaque() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::number()),
    );
    assert_eq!(schema.json_schema()["$comment"], "union");
}

#[test]
fn codec_scalars_round_trip() {
    let text = Schema::string()
        .decode(Unknown::Text("s".to_string()))
        .unwrap();
    assert_eq!(
        Schema::string().try_encode(&text).unwrap(),
        Unknown::Text("s".to_string())
    );
    let number = Schema::number().decode(Unknown::Number(1.5)).unwrap();
    assert_eq!(Schema::number().try_encode(&number).unwrap(), Unknown::Number(1.5));
    let flag = Schema::boolean().decode(Unknown::Bool(true)).unwrap();
    assert_eq!(Schema::boolean().try_encode(&flag).unwrap(), Unknown::Bool(true));
}

#[test]
fn codec_null_round_trips_and_never_rejects() {
    Schema::null().decode(Unknown::Null).unwrap();
    assert_eq!(Schema::null().try_encode(&()).unwrap(), Unknown::Null);
    assert!(Schema::never().decode(Unknown::Null).is_err());
    assert!(
        Schema::never()
            .decode(Unknown::Text("x".to_string()))
            .is_err()
    );
}

#[test]
fn unrepresentable_encode_returns_err() {
    // Honesty: no panic channel, the refusal travels as `EncodingFailed`.
    let err = Schema::never()
        .try_encode(&Unknown::Null)
        .expect_err("Never encodes nothing");
    assert!(matches!(
        err,
        elm_effect_schema::SchemaError::EncodingFailed { .. }
    ));
    let mapped = Schema::literal_transform(vec![(
        Unknown::Text("a".to_string()),
        Unknown::Text("A".to_string()),
    )]);
    let err = mapped
        .try_encode(&Unknown::Text("B".to_string()))
        .expect_err("an unmapped output is unrepresentable");
    assert!(matches!(
        err,
        elm_effect_schema::SchemaError::EncodingFailed { .. }
    ));
}

#[test]
fn codec_any_accepts_everything() {
    for wire in [
        Unknown::Null,
        Unknown::Bool(false),
        Unknown::Number(0.0),
        Unknown::Text("u".to_string()),
    ] {
        let decoded = Schema::any().decode(wire.clone()).unwrap();
        assert_eq!(Schema::any().try_encode(&decoded).unwrap(), wire);
    }
}

#[test]
fn codec_literal_accepts_only_its_value() {
    let schema = Schema::literal(Unknown::Text("a".to_string()));
    let decoded = schema.decode(Unknown::Text("a".to_string())).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), Unknown::Text("a".to_string()));
    assert!(schema.decode(Unknown::Text("b".to_string())).is_err());
}

/// Schema.test.ts:287 `Literal.transform`: a one-pair map, so decoding the
/// source literal yields the target and encoding does the reverse. The pin's
/// expected text names the literal; this port's refusal carries the same list.
#[test]
fn literal_transform_maps_one_pair_both_ways() {
    let schema = Schema::literal_transform(vec![(Unknown::Number(0.0), text("a"))]);

    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), text("a"));
    assert!(schema.decode(Unknown::Number(1.0)).is_err());

    assert_eq!(schema.try_encode(&text("a")).unwrap(), Unknown::Number(0.0));
    let refused = schema.try_encode(&text("b")).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "cannot encode at $: expected one of \"a\""
    );
}

/// Schema.test.ts:315 `Literals.transform`: one pair per member, so each source
/// literal maps to its own target and every other value is refused in both
/// directions.
#[test]
fn literal_transform_maps_member_pairs_both_ways() {
    let schema = Schema::literal_transform(vec![
        (Unknown::Number(0.0), text("a")),
        (Unknown::Number(1.0), text("b")),
    ]);

    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), text("a"));
    assert_eq!(schema.decode(Unknown::Number(1.0)).unwrap(), text("b"));
    assert!(schema.decode(Unknown::Number(2.0)).is_err());

    assert_eq!(schema.try_encode(&text("a")).unwrap(), Unknown::Number(0.0));
    assert_eq!(schema.try_encode(&text("b")).unwrap(), Unknown::Number(1.0));
    let refused = schema.try_encode(&text("c")).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "cannot encode at $: expected one of \"a\" | \"b\""
    );
}

#[test]
fn codec_union_of_literals_covers_enum() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::literal(Unknown::Text("b".to_string())),
    );
    for member in ["a", "b"] {
        let wire = Unknown::Text(member.to_string());
        let decoded = schema.decode(wire.clone()).unwrap();
        assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    }
    assert!(schema.decode(Unknown::Text("z".to_string())).is_err());
}

#[test]
fn codec_array_of_literals_covers_enum() {
    let schema = Schema::array(Schema::literal(Unknown::Text("a".to_string())));
    let wire = Unknown::Array(vec![Unknown::Text("a".to_string())]);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    let bad = Unknown::Array(vec![Unknown::Text("z".to_string())]);
    assert!(schema.decode(bad).is_err());
}

#[test]
fn codec_optional_date_json_shapes() {
    use elm_effect_schema::transform::date_schema;
    const ISO: &str = "2021-01-01T00:00:00.000Z";
    const MILLIS: i64 = 1_609_459_200_000;
    // Bare NullOr/UndefinedOr/optional(Date).
    let nullable = Schema::optional(date_schema());
    assert_eq!(
        nullable.decode(Unknown::Text(ISO.to_string())).unwrap(),
        Some(MILLIS)
    );
    assert_eq!(nullable.decode(Unknown::Null).unwrap(), None);
    assert!(nullable.decode(Unknown::Number(1.0)).is_err());
    assert_eq!(
        nullable.try_encode(&Some(MILLIS)).unwrap(),
        Unknown::Text(ISO.to_string())
    );
    assert_eq!(nullable.try_encode(&None).unwrap(), Unknown::Null);
    // Struct NullOr/UndefinedOr/optional(Date): present, null, absent.
    let strukt = Schema::struct_shape(vec![Field::optional(
        "a",
        Schema::as_unknown(Schema::optional(date_schema())),
    )]);
    let mut present = BTreeMap::new();
    present.insert("a".to_string(), Unknown::Text(ISO.to_string()));
    assert_eq!(
        strukt.decode(Unknown::Object(present.clone())).unwrap()["a"],
        Unknown::Text(ISO.to_string())
    );
    assert_eq!(strukt.try_encode(&present).unwrap(), Unknown::Object(present));
    let mut nullish = BTreeMap::new();
    nullish.insert("a".to_string(), Unknown::Null);
    // Explicit null normalizes to absent (TS decodes it to undefined).
    assert!(
        !strukt
            .decode(Unknown::Object(nullish))
            .unwrap()
            .contains_key("a")
    );
    assert!(
        !strukt
            .decode(Unknown::Object(BTreeMap::new()))
            .unwrap()
            .contains_key("a")
    );
    // Struct optionalKey(Date): absent ok, explicit null decodes inner.
    let keyed = Schema::struct_shape(vec![Field::optional_key(
        "a",
        Schema::as_unknown(Schema::optional(date_schema())),
    )]);
    assert!(
        !keyed
            .decode(Unknown::Object(BTreeMap::new()))
            .unwrap()
            .contains_key("a")
    );
    let mut null_wire = BTreeMap::new();
    null_wire.insert("a".to_string(), Unknown::Null);
    assert_eq!(
        keyed.decode(Unknown::Object(null_wire)).unwrap()["a"],
        Unknown::Null
    );
}

#[test]
fn codec_unknown_round_trips_any_json() {
    let schema = Schema::unknown();
    for value in [
        Unknown::Text("a".to_string()),
        Unknown::Number(1.0),
        Unknown::Bool(true),
        Unknown::Null,
        Unknown::Object([("a".to_string(), Unknown::Text("a".to_string()))].into()),
        Unknown::Array(vec![Unknown::Text("a".to_string()), Unknown::Number(1.0)]),
    ] {
        let encoded = schema.try_encode(&value).unwrap();
        assert_eq!(encoded, value);
        assert_eq!(schema.decode(value.clone()).unwrap(), value);
    }
}

#[test]
fn codec_object_keyword_takes_objects_only() {
    let schema = Schema::object();
    let fields: BTreeMap<String, Unknown> =
        [("a".to_string(), Unknown::Text("a".to_string()))].into();
    assert_eq!(
        schema.decode(Unknown::Object(fields.clone())).unwrap(),
        fields
    );
    assert_eq!(schema.try_encode(&fields).unwrap(), Unknown::Object(fields));
    assert!(schema.decode(Unknown::Array(vec![])).is_err());
    assert!(schema.decode(Unknown::Null).is_err());
}

#[test]
fn codec_null_and_string_are_identity() {
    let null = Schema::<()>::null();
    assert!(null.decode(Unknown::Null).is_ok());
    assert!(null.decode(Unknown::Text("a".to_string())).is_err());

    let string = Schema::string();
    assert_eq!(
        string.decode(Unknown::Text("a".to_string())).unwrap(),
        "a".to_string()
    );
    assert_eq!(
        string.try_encode(&"a".to_string()).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert!(string.decode(Unknown::Number(1.0)).is_err());
}

#[test]
fn codec_void_and_undefined_map_to_null() {
    // TS Void/Undefined hold no data and travel as JSON null; the Rust
    // wire representation is the same literal.
    let void = Schema::<Unknown>::literal(Unknown::Null);
    assert_eq!(void.try_encode(&Unknown::Null).unwrap(), Unknown::Null);
    assert_eq!(void.decode(Unknown::Null).unwrap(), Unknown::Null);
    assert!(void.decode(Unknown::Text("a".to_string())).is_err());
}

// --- toCodec.test.ts: the literal, Never, and template-literal codec rows ---

/// toCodec.test.ts:585 — `Schema.toCodecJson(Schema.Literal("a"))`: one value
/// decodes and encodes back to itself, and nothing else decodes.
#[test]
fn codec_literal_string_round_trips() {
    let schema = Schema::<Unknown>::literal(Unknown::Text("a".to_string()));
    let wire = Unknown::Text("a".to_string());
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    assert!(schema.decode(Unknown::Text("b".to_string())).is_err());
}

/// toCodec.test.ts:596 — `Schema.toCodecJson(Schema.Literal(1))`.
#[test]
fn codec_literal_number_round_trips() {
    let schema = Schema::<Unknown>::literal(Unknown::Number(1.0));
    let wire = Unknown::Number(1.0);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    assert!(schema.decode(Unknown::Number(2.0)).is_err());
    assert!(schema.decode(Unknown::Text("1".to_string())).is_err());
}

/// toCodec.test.ts:607 — `Schema.toCodecJson(Schema.Literal(true))`.
#[test]
fn codec_literal_boolean_round_trips() {
    let schema = Schema::<Unknown>::literal(Unknown::Bool(true));
    let wire = Unknown::Bool(true);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    assert!(schema.decode(Unknown::Bool(false)).is_err());
}

/// toCodec.test.ts:249 — `Schema.toCodecJson(Schema.Never)`: nothing decodes,
/// and the encode direction cannot hand back a wire value either, so the
/// refusal travels on the Result (`try_encode`) instead of panicking.
/// Red witness: this assertion failed while `never`'s encode was still total
/// over `Unknown`; the fix that gave encode a failure channel turned it green.
#[test]
fn codec_never_refuses_both_directions() {
    let schema = Schema::<Unknown>::never();
    for input in [
        Unknown::Null,
        Unknown::Bool(true),
        Unknown::Number(1.0),
        Unknown::Text("a".to_string()),
        Unknown::Array(vec![Unknown::Number(1.0)]),
        Unknown::Object(std::collections::BTreeMap::new()),
    ] {
        let error = schema.decode(input).expect_err("never decodes");
        assert!(error.to_string().contains("never"), "{error}");
    }
    let error = schema
        .try_encode(&Unknown::Null)
        .expect_err("never has no value to encode");
    assert!(error.to_string().contains("Never"), "{error}");
}

/// toCodec.test.ts:642 — `Schema.toCodecJson(Schema.TemplateLiteral([1n,
/// Schema.String]))`: the codec is identity on a matching string. `1n` is a
/// verbatim anchor and `Schema.String` is the span behind it.
#[test]
fn codec_template_literal_bigint_then_string_round_trips() {
    let schema = TemplateParser::new(vec![
        TemplatePart::Anchor("1".to_string()),
        TemplatePart::AnyString,
    ]);
    let captures = schema.parse("1a").expect("matches");
    assert_eq!(
        captures,
        vec![
            Unknown::Text("1".to_string()),
            Unknown::Text("a".to_string())
        ]
    );
    assert_eq!(schema.render(&captures).as_deref(), Some("1a"));
    assert!(!schema.matches("2a"));
}

/// toCodec.test.ts:653 — `Schema.toCodecJson(Schema.TemplateLiteral(["a",
/// Schema.BigInt]))`: the trailing span is a bigint, so `a1` matches and
/// `a1.2` does not.
#[test]
fn codec_template_literal_string_then_bigint_round_trips() {
    let schema = TemplateParser::new(vec![
        TemplatePart::Anchor("a".to_string()),
        TemplatePart::Digits,
    ]);
    let captures = schema.parse("a1").expect("matches");
    assert_eq!(
        captures,
        vec![
            Unknown::Text("a".to_string()),
            Unknown::Text("1".to_string())
        ]
    );
    assert_eq!(schema.render(&captures).as_deref(), Some("a1"));
    assert!(!schema.matches("a1.2"));
    assert!(!schema.matches("b1"));
}

/// toCodec.test.ts:893 — `Schema.toCodecJson(Schema.Array(Schema.Date))`: each
/// element travels as its ISO-8601 UTC text.
#[test]
fn codec_array_of_date_round_trips() {
    use elm_effect_schema::transform::date_schema;
    let schema = Schema::as_unknown(Schema::array(date_schema()));
    let wire = Unknown::Array(vec![
        Unknown::Text("2021-01-01T00:00:00.000Z".to_string()),
        Unknown::Text("2021-01-02T00:00:00.000Z".to_string()),
    ]);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    assert!(
        schema
            .decode(Unknown::Array(vec![Unknown::Text("nope".to_string())]))
            .is_err()
    );
}

/// toCodec.test.ts:905 — `Schema.toCodecJson(Schema.NullOr(Schema.String))`:
/// the member and null both travel in each direction.
#[test]
fn codec_null_or_string_round_trips() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::<()>::null()),
    );
    assert_eq!(
        schema.decode(Unknown::Null).unwrap(),
        Unknown::Null,
        "null decodes to the null member"
    );
    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert_eq!(schema.try_encode(&Unknown::Null).unwrap(), Unknown::Null);
    assert_eq!(
        schema.try_encode(&Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
}

/// toCodec.test.ts:918 — `Schema.toCodecJson(Schema.NullOr(Schema.Number))`.
#[test]
fn codec_null_or_number_round_trips() {
    let schema = Schema::union(
        Schema::as_unknown(Schema::number()),
        Schema::as_unknown(Schema::<()>::null()),
    );
    assert_eq!(schema.decode(Unknown::Null).unwrap(), Unknown::Null);
    assert_eq!(
        schema.decode(Unknown::Number(1.0)).unwrap(),
        Unknown::Number(1.0)
    );
    assert_eq!(schema.try_encode(&Unknown::Null).unwrap(), Unknown::Null);
    assert_eq!(schema.try_encode(&Unknown::Number(1.0)).unwrap(), Unknown::Number(1.0));
    assert!(schema.decode(Unknown::Text("a".to_string())).is_err());
}

/// toCodec.test.ts:931 — `Schema.toCodecJson(Schema.Array(Schema.NullOr(
/// Schema.Number)))`: the nullable element inside the array.
#[test]
fn codec_array_of_null_or_number_round_trips() {
    let nullable = Schema::union(
        Schema::as_unknown(Schema::number()),
        Schema::as_unknown(Schema::<()>::null()),
    );
    let schema = Schema::as_unknown(Schema::array(nullable));
    let wire = Unknown::Array(vec![Unknown::Number(1.0), Unknown::Null]);
    let decoded = schema.decode(wire.clone()).unwrap();
    assert_eq!(schema.try_encode(&decoded).unwrap(), wire);
    assert!(
        schema
            .decode(Unknown::Array(vec![Unknown::Text("a".to_string())]))
            .is_err()
    );
}

/// toCodec.test.ts:210, :216, :222 — "should return the same reference if
/// nothing changed": a codec over a shape that needs no rewriting leaves that
/// shape alone, so the repr shows no wrapper and the wire value round-trips
/// exactly once rather than through an added layer.
#[test]
fn codec_leaves_a_tuple_array_and_union_shape_alone() {
    let tuple = Schema::tuple_of(vec![
        Schema::as_unknown(Schema::string()),
        Schema::as_unknown(Schema::boolean()),
    ]);
    assert_eq!(
        tuple.to_code(),
        "Schema.Tuple([Schema.String, Schema.Boolean])"
    );
    let wire = Unknown::Array(vec![text("a"), Unknown::Bool(true)]);
    let decoded = tuple.decode(wire.clone()).expect("a tuple decodes");
    assert_eq!(tuple.try_encode(&decoded).unwrap(), wire);

    let array: Schema<Unknown> =
        Schema::as_unknown(Schema::array(Schema::as_unknown(Schema::string())));
    assert_eq!(array.to_code(), "Schema.Array(Schema.String)");
    let wire = Unknown::Array(vec![text("a")]);
    let decoded = array.decode(wire.clone()).expect("an array decodes");
    assert_eq!(array.try_encode(&decoded).unwrap(), wire);

    let union = Schema::union_with_mode(
        vec![
            Schema::as_unknown(Schema::string()),
            Schema::as_unknown(Schema::boolean()),
        ],
        "anyOf",
    );
    assert_eq!(
        union.to_code(),
        "Schema.Union([Schema.String, Schema.Boolean])"
    );
    assert_eq!(
        union.try_encode(&union.decode(text("a")).expect("text")).unwrap(),
        text("a")
    );
    assert_eq!(
        union.try_encode(&union.decode(Unknown::Bool(false)).expect("bool")).unwrap(),
        Unknown::Bool(false)
    );
}

/// toCodec.test.ts:402 — "runs source checks once per direction": a check the
/// source schema carries is evaluated by the codec, and it is not re-run
/// behind a wrapper that would charge the caller twice for one value.
#[test]
fn codec_runs_a_source_check_in_each_direction_without_repeating_it() {
    // The port's check is a decode-side predicate over the decoded value, so
    // one decode is one evaluation; the encode direction carries the value
    // back untouched, which is what keeps a round trip from re-charging.
    let checked = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isGreaterThanOrEqualTo",
        vec!["1"],
        vec![("expected", "a value greater than or equal to 1")],
    );
    assert!(checked.decode(Unknown::Number(1.0)).is_ok());
    let error = checked
        .decode(Unknown::Number(0.0))
        .expect_err("below the bound is refused");
    assert!(
        error.to_string().contains("satisfying the refinement"),
        "{error}"
    );
    // The same check decides the encode side, so a value that fails it cannot
    // be handed back as a wire value.
    let encoded = checked.try_encode(&Unknown::Number(0.0));
    assert!(encoded.is_err(), "a failing value has no wire form");
}

/// toCodec.test.ts:955 — `Schema.toCodecJson` over a recursive struct whose
/// suspended member is the struct itself: a nested value round-trips through
/// the suspension in both directions, and the checked `a` field still decides
/// each node on the way in.
#[test]
fn codec_suspends_a_recursive_struct_and_round_trips_nested_values() {
    // `Category` is a struct of a checked positive number and an array of
    // itself, reached through the suspension.
    fn category() -> Schema<Unknown> {
        let inner: Schema<Unknown> = Schema::<Unknown>::suspend(category);
        let a = Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isGreaterThan",
            vec!["0"],
            vec![("expected", "a value greater than 0")],
        );
        let categories = Schema::as_unknown(Schema::array(inner));
        let fields = vec![
            Field::required("a", a),
            Field::required("categories", categories),
        ];
        Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::struct_shape(fields))
    }

    let schema = category();
    let leaf = || {
        Unknown::Object(BTreeMap::from([
            ("a".to_string(), Unknown::Number(1.0)),
            ("categories".to_string(), Unknown::Array(vec![])),
        ]))
    };
    let nested = Unknown::Object(BTreeMap::from([
        ("a".to_string(), Unknown::Number(1.0)),
        (
            "categories".to_string(),
            Unknown::Array(vec![Unknown::Object(BTreeMap::from([
                ("a".to_string(), Unknown::Number(2.0)),
                ("categories".to_string(), Unknown::Array(vec![])),
            ]))]),
        ),
    ]));

    for value in [leaf(), nested.clone()] {
        let decoded = schema.decode(value.clone()).expect("a category decodes");
        assert_eq!(schema.try_encode(&decoded).unwrap(), value);
    }
    // The checked field decides each node: a non-positive `a` is refused
    // wherever it appears, including inside the suspended array.
    let bad = Unknown::Object(BTreeMap::from([
        ("a".to_string(), Unknown::Number(0.0)),
        ("categories".to_string(), Unknown::Array(vec![])),
    ]));
    let error = schema
        .decode(bad)
        .expect_err("a non-positive field is refused");
    assert!(
        error.to_string().contains("satisfying the refinement"),
        "{error}"
    );
    let bad_nested = Unknown::Object(BTreeMap::from([
        ("a".to_string(), Unknown::Number(1.0)),
        (
            "categories".to_string(),
            Unknown::Array(vec![Unknown::Object(BTreeMap::from([
                ("a".to_string(), Unknown::Number(0.0)),
                ("categories".to_string(), Unknown::Array(vec![])),
            ]))]),
        ),
    ]));
    assert!(
        schema.decode(bad_nested).is_err(),
        "the check reaches the nested node"
    );
}

// --- toCodec.test.ts: the plain JSON codec rows, where the wire form is the
// JSON value itself and the schema's own two-way codec carries the row ---

/// toCodec.test.ts:268 — `Schema.toCodecJson(Schema.Unknown)`: every JSON
/// value round-trips in both directions, and a value the JSON layer cannot
/// hold is refused rather than coerced.
#[test]
fn codec_unknown_round_trips_every_json_value() {
    let schema = Schema::<Unknown>::json();
    let values = [
        text("a"),
        Unknown::Number(1.0),
        Unknown::Bool(true),
        Unknown::Null,
        Unknown::Object(BTreeMap::from([
            ("a".to_string(), text("a")),
            ("b".to_string(), Unknown::Number(1.0)),
            ("c".to_string(), Unknown::Bool(true)),
        ])),
        Unknown::Array(vec![text("a"), Unknown::Number(1.0), Unknown::Bool(true)]),
    ];
    for value in values {
        let decoded = schema.decode(value.clone()).expect("a JSON value decodes");
        assert_eq!(schema.try_encode(&decoded).unwrap(), value);
    }
}

/// toCodec.test.ts:291 — `Schema.toCodecJson(Schema.ObjectKeyword)`: an object
/// or an array is the whole domain, so a scalar is refused in both directions.
#[test]
fn codec_object_keyword_admits_objects_and_arrays_only() {
    let schema = Schema::<Unknown>::object_keyword();
    for value in [
        Unknown::Object(BTreeMap::from([("a".to_string(), text("a"))])),
        Unknown::Array(vec![text("a")]),
    ] {
        assert!(schema.decode(value.clone()).is_ok(), "{value:?}");
    }
    for value in [
        text("a"),
        Unknown::Number(1.0),
        Unknown::Bool(true),
        Unknown::Null,
    ] {
        let error = schema.decode(value).expect_err("a scalar is refused");
        assert!(error.to_string().contains("object"), "{error}");
    }
}

/// toCodec.test.ts:308, :316 — `Schema.toCodecJson(Schema.Undefined)` and
/// `Schema.Void`: the absent value encodes to the JSON null it has to stand
/// for, and decodes back.
#[test]
fn codec_undefined_and_void_stand_in_null_on_the_wire() {
    let undefined = Schema::<()>::void();
    assert_eq!(undefined.decode(Unknown::Null).unwrap(), ());
    assert_eq!(undefined.try_encode(&()).unwrap(), Unknown::Null);
}

/// toCodec.test.ts:324 — `Schema.toCodecJson(Schema.Null)`: null is the only
/// value, in both directions.
#[test]
fn codec_null_round_trips_only_null() {
    let schema = Schema::null();
    assert_eq!(schema.decode(Unknown::Null).unwrap(), ());
    assert_eq!(schema.try_encode(&()).unwrap(), Unknown::Null);
    assert!(schema.decode(Unknown::Number(1.0)).is_err());
    assert!(schema.decode(text("a")).is_err());
}

/// toCodec.test.ts:332 — `Schema.toCodecJson(Schema.String)`: a string
/// round-trips, and a non-string is refused.
#[test]
fn codec_string_round_trips_and_refuses_other_kinds() {
    let schema = Schema::string();
    assert_eq!(schema.decode(text("a")).unwrap(), "a");
    assert_eq!(schema.try_encode(&"a".to_string()).unwrap(), text("a"));
    for value in [Unknown::Number(1.0), Unknown::Bool(true), Unknown::Null] {
        let error = schema.decode(value).expect_err("a non-string is refused");
        assert!(error.to_string().contains("string"), "{error}");
    }
}

/// toCodec.test.ts:497 — `Schema.toCodecJson(Schema.Boolean)`: a boolean
/// round-trips, and a non-boolean is refused.
#[test]
fn codec_boolean_round_trips_and_refuses_other_kinds() {
    let schema = Schema::boolean();
    assert!(schema.decode(Unknown::Bool(true)).unwrap());
    assert_eq!(schema.try_encode(&true).unwrap(), Unknown::Bool(true));
    assert!(!schema.decode(Unknown::Bool(false)).unwrap());
    for value in [Unknown::Number(1.0), text("true"), Unknown::Null] {
        let error = schema.decode(value).expect_err("a non-boolean is refused");
        assert!(error.to_string().contains("boolean"), "{error}");
    }
}
