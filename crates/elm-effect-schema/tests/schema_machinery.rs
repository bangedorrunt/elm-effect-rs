//! `Schema.test.ts` rows that needed machinery rather than another carried
//! assertion: `mutable`, `annotateEncoded`, and the dispatcher's treatment of a
//! recovering union member.
//!
//!     cargo test -p elm-effect-schema --test schema_machinery
//!
//! Each test names the pin line it answers. The machinery notes live beside the
//! constructors in `src/lib.rs`; what is asserted here is the behaviour the
//! rows name, not the shape of the machinery.

use elm_effect_schema::checks::is_min_length;
use elm_effect_schema::filters::{FilterIssue, FilterOutcome, FilterSpec};
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

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn array_of_string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::array(Schema::string()))
}

/// The port validates a typed value on its way out through the Rust type, so an
/// Unknown-level schema's encode is identity. A REFINEMENT is the one schema
/// whose encode can still refuse, which is what an encoding-side fallback needs.
fn wire_string() -> Schema<Unknown> {
    Schema::filtered(
        string(),
        vec![filter("Expected string", |value| {
            matches!(value, Unknown::Text(_))
        })],
        false,
    )
}

/// The same refined spelling for one literal, so its encode refuses another.
fn wire_literal(value: &str) -> Schema<Unknown> {
    let expected = text(value);
    Schema::filtered(
        literal(value),
        vec![filter("Expected a literal", move |value| *value == expected)],
        false,
    )
}

fn literal(value: &str) -> Schema<Unknown> {
    Schema::as_unknown(Schema::literal(text(value)))
}

/// A struct built from declared fields, which is what keeps a field's optional
/// markers and its decoding default.
fn struct_of(fields: Vec<(&str, Schema<Unknown>)>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_shape(
        fields
            .into_iter()
            .map(|(name, schema)| Field::required(name, schema))
            .collect(),
    ))
}

/// A filter carrying the pin's own message, for the checks `checks.rs` does not
/// name.
fn filter(message: &str, holds: impl Fn(&Unknown) -> bool + Send + Sync + 'static) -> FilterSpec {
    let message = message.to_string();
    FilterSpec::new(move |value| {
        if holds(value) {
            FilterOutcome::Pass
        } else {
            FilterOutcome::Fail(FilterIssue::new(message.clone()))
        }
    })
}

/// A struct at the `Unknown` level whose ENCODE still runs its field codecs.
/// `struct_of` goes through `as_unknown`, whose encode is identity because its
/// value already is the wire form; a union of structs needs the struct's own
/// encode, which is where a field's encoded-side fallback lives.
fn unknown_struct(fields: Vec<(&str, Schema<Unknown>)>) -> Schema<Unknown> {
    Schema::convert(
        Schema::struct_shape(
            fields
                .into_iter()
                .map(|(name, schema)| Field::required(name, schema))
                .collect(),
        ),
        |fields| Ok(Unknown::Object(fields)),
        |value| match value {
            Unknown::Object(fields) => fields.clone(),
            _ => Default::default(),
        },
    )
}

fn decode_error(schema: &Schema<Unknown>, input: Unknown) -> String {
    schema
        .decode(input)
        .expect_err("the row expects a refusal")
        .to_string()
}

fn encode_error(schema: &Schema<Unknown>, value: &Unknown) -> String {
    schema
        .try_encode(value)
        .expect_err("the row expects a refusal")
        .to_string()
}

/// `Schema.test.ts:1160` — `mutable` carries a decoding default that follows it,
/// so a missing key and a null both fill, and a present array travels.
#[test]
fn a_decoding_default_after_mutable_still_fills() {
    let default = Unknown::Array(vec![text("default")]);
    let field = Schema::with_decoding_default_type(
        Schema::mutable(array_of_string()),
        {
            let default = default.clone();
            move || Ok(default.clone())
        },
    );
    let schema = struct_of(vec![("value", field)]);

    assert_eq!(
        schema.decode(obj(vec![])).unwrap(),
        obj(vec![("value", default.clone())])
    );
    assert_eq!(
        schema.decode(obj(vec![("value", Unknown::Null)])).unwrap(),
        obj(vec![("value", default)]),
        "the port's null is the absence marker this spelling fills"
    );

    let present = obj(vec![(
        "value",
        Unknown::Array(vec![text("a")]),
    )]);
    assert_eq!(schema.decode(present.clone()).unwrap(), present);
    assert_eq!(schema.try_encode(&present).unwrap(), present);
}

/// `Schema.test.ts:1175` — the same default placed BEFORE `mutable` is refused
/// at construction, because `mutable` cannot rewrite an encoding link.
#[test]
#[should_panic(expected = "mutable does not support encodings")]
fn mutable_refuses_a_schema_that_already_carries_an_encoding() {
    let defaulted = Schema::with_decoding_default_type(
        array_of_string(),
        || Ok(Unknown::Array(vec![text("default")])),
    );
    let _ = struct_of(vec![("value", Schema::mutable(defaulted))]);
}

/// `Schema.test.ts:1188` — `mutable` preserves the annotations, checks and
/// encoded-side checks already on the node, so both axes still decide.
#[test]
fn mutable_preserves_the_annotations_and_checks_on_both_axes() {
    let annotated = Schema::annotated(array_of_string(), Some("A"), None);
    let checked = Schema::filtered(annotated, vec![is_min_length(1)], false);
    // `flip & check & flip` lands the second check on the encoded side.
    let flipped = Schema::flip(checked);
    let encoding_checked = Schema::filtered(
        flipped,
        vec![filter("Expected a value with a length of at most 1", |value| {
            matches!(value, Unknown::Array(items) if items.len() <= 1)
        })],
        false,
    );
    let schema = Schema::optional_key_shape(Schema::flip(encoding_checked));

    let mutable = Schema::mutable(schema.clone());

    assert_eq!(
        mutable.resolve_annotations(),
        schema.resolve_annotations(),
        "the annotation rides the same node"
    );
    assert_eq!(
        decode_error(&mutable, Unknown::Array(vec![])),
        "Expected a value with a length of at least 1 at $",
        "the decoding-side check survives"
    );
    assert_eq!(
        encode_error(
            &mutable,
            &Unknown::Array(vec![text("a"), text("b")])
        ),
        "Expected a value with a length of at most 1 at $",
        "the encoded-side check survives"
    );
}

/// `Schema.test.ts:8777` — `annotateEncoded` over a non-transforming schema
/// annotates that schema's encoded side, which is the node itself.
#[test]
fn annotate_encoded_lands_on_a_plain_nodes_encoded_side() {
    let schema = Schema::annotate_encoded(string(), vec![("title", "encoded title")]);

    assert_eq!(
        schema.to_encoded().resolve_annotations(),
        Some(vec![(
            "title".to_string(),
            "encoded title".to_string()
        )])
    );
}

/// `Schema.test.ts:8784` — over a transforming schema the annotation lands on
/// the schema the transform reads, not on the transformed node.
#[test]
fn annotate_encoded_lands_on_a_transforms_source() {
    let transform = Schema::as_unknown(Schema::finite_from_string());
    let schema = Schema::annotate_encoded(transform, vec![("title", "encoded title")]);

    let encoded = schema.to_encoded();
    assert!(
        encoded.to_code().starts_with("Schema.String"),
        "the encoded side of FiniteFromString is String, got {}",
        encoded.to_code()
    );
    assert_eq!(
        encoded.resolve_annotations(),
        Some(vec![(
            "title".to_string(),
            "encoded title".to_string()
        )])
    );
}

/// `Schema.test.ts:4504` — a recovering member is a candidate for every runtime
/// type, so it still gets the turn its fallback exists for.
#[test]
fn a_recovering_member_is_dispatched_for_every_runtime_type() {
    let decoding_fallback = Schema::catch_decoding(string(), |_| Some(text("fallback")));
    let decoding = Schema::union_with_mode(
        vec![decoding_fallback.clone(), Schema::as_unknown(Schema::null())],
        "anyOf",
    );
    assert_eq!(
        decoding.decode(Unknown::Null).unwrap(),
        text("fallback"),
        "the String member recovers where the Null member would have won"
    );

    // A transform's recovering member recovers into its own decoded type.
    let transformed = Schema::catch_decoding(
        Schema::as_unknown(Schema::number_from_string()),
        |_| Some(Unknown::Number(0.0)),
    );
    let transformed_union = Schema::union_with_mode(
        vec![transformed, Schema::as_unknown(Schema::null())],
        "anyOf",
    );
    assert_eq!(
        transformed_union.decode(Unknown::Null).unwrap(),
        Unknown::Number(0.0)
    );

    // The recovery nests: a union member carrying a recovering member.
    let nested = Schema::union_with_mode(
        vec![decoding_fallback.clone(), string()],
        "anyOf",
    );
    let nested_union = Schema::union_with_mode(
        vec![nested, Schema::as_unknown(Schema::null())],
        "anyOf",
    );
    assert_eq!(
        nested_union.decode(Unknown::Null).unwrap(),
        text("fallback")
    );

    // A record keyed by a recovering member stays object-only: an index
    // signature's key schema takes no part in union selection.
    let record = Schema::as_unknown(Schema::record(decoding_fallback.clone()));
    let record_union = Schema::union_with_mode(
        vec![record, Schema::as_unknown(Schema::null())],
        "anyOf",
    );
    assert_eq!(record_union.decode(Unknown::Null).unwrap(), Unknown::Null);

    // A literal member recovers too, which is the case a literal-only index
    // would have narrowed away.
    let literal_fallback = Schema::catch_decoding(literal("a"), |_| Some(text("a")));
    let literals = Schema::union_with_mode(vec![literal_fallback, literal("b")], "anyOf");
    assert_eq!(literals.decode(text("b")).unwrap(), text("a"));

    // `oneOf` still demands a single winner, so the recovering member and the
    // member whose type it recovers from are a conflict.
    let one_of = Schema::union_with_mode(
        vec![
            Schema::catch_decoding(string(), |_| Some(text("fallback"))),
            Schema::as_unknown(Schema::null()),
        ],
        "oneOf",
    );
    assert_eq!(
        decode_error(&one_of, Unknown::Null),
        "expected exactly one member to match at $, got null"
    );

    // The encoding axis is the same rule, which is why a catchEncoding member
    // is dispatched for every type as well.
    let encoding_fallback = Schema::catch_encoding(wire_string(), |_| Some(text("fallback")));
    let encoding = Schema::union_with_mode(
        vec![encoding_fallback, Schema::as_unknown(Schema::null())],
        "anyOf",
    );
    assert_eq!(
        encoding.try_encode(&Unknown::Null).unwrap(),
        text("fallback")
    );

    let encoding_one_of = Schema::union_with_mode(
        vec![
            Schema::catch_encoding(wire_string(), |_| Some(text("fallback"))),
            Schema::as_unknown(Schema::null()),
        ],
        "oneOf",
    );
    assert_eq!(
        encode_error(&encoding_one_of, &Unknown::Null),
        "expected exactly one member to match at $, got null"
    );
}

/// `Schema.test.ts:4551` — recovering members also survive SENTINEL dispatch: a
/// member whose tag field recovers demands no tag, so it is not narrowed away
/// by the input's different tag.
#[test]
fn a_recovering_member_survives_sentinel_dispatch() {
    let decoding_tag = Schema::catch_decoding(literal("a"), |_| Some(text("a")));
    let decoding_first = struct_of(vec![("kind", decoding_tag)]);
    let second = struct_of(vec![("kind", literal("b"))]);
    let input = obj(vec![("kind", text("b"))]);

    let decoding = Schema::union_with_mode(vec![decoding_first.clone(), second.clone()], "anyOf");
    assert_eq!(
        decoding.decode(input.clone()).unwrap(),
        obj(vec![("kind", text("a"))])
    );

    // A recovering ROOT member is the same statement one level up.
    let root = Schema::catch_decoding(
        struct_of(vec![("kind", literal("a"))]),
        |_| Some(obj(vec![("kind", text("a"))])),
    );
    let root_union = Schema::union_with_mode(vec![root, second.clone()], "anyOf");
    assert_eq!(
        root_union.decode(input.clone()).unwrap(),
        obj(vec![("kind", text("a"))])
    );

    let one_of = Schema::union_with_mode(vec![decoding_first, second.clone()], "oneOf");
    assert_eq!(
        decode_error(&one_of, input.clone()),
        "expected exactly one member to match at $, got object"
    );

    let encoding_tag = Schema::catch_encoding(wire_literal("a"), |_| Some(text("a")));
    let encoding_first = unknown_struct(vec![("kind", encoding_tag)]);
    let encoding_second = unknown_struct(vec![("kind", literal("b"))]);
    let encoding = Schema::union_with_mode(
        vec![encoding_first.clone(), encoding_second.clone()],
        "anyOf",
    );
    assert_eq!(
        encoding.try_encode(&input).unwrap(),
        obj(vec![("kind", text("a"))])
    );

    let encoding_one_of = Schema::union_with_mode(vec![encoding_first, encoding_second], "oneOf");
    assert_eq!(
        encode_error(&encoding_one_of, &input),
        "expected exactly one member to match at $, got object"
    );
}

