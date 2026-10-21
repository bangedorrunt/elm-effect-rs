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
use elm_effect_schema::class::{class_from, tagged_error};
use elm_effect_schema::filters::{FilterIssue, FilterOutcome, FilterSpec};
use elm_effect_schema::transform::{Errors, ParseOptions, Transformation};
use elm_effect_schema::{ExcessProperty, Field, Schema, SchemaError, Unknown};

fn number_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

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


/// `Schema.test.ts:2319` — a link exposes the schema it reads and the schema it
/// produces, so `FiniteFromString` answers `String` and `Finite`.
#[test]
fn a_link_exposes_its_source_and_its_target() {
    let schema = Schema::<f64>::finite_from_string();

    let from = schema
        .link_from()
        .expect("FiniteFromString keeps its source");
    assert_eq!(from.to_code(), "Schema.String");

    let to = schema.link_to().expect("FiniteFromString keeps its target");
    assert!(to.decode(Unknown::Number(1.5)).is_ok());
    assert!(
        to.decode(Unknown::Number(f64::NAN)).is_err(),
        "the target is Finite, so a non-finite number is not its value"
    );
}

/// `Schema.test.ts:6195` — `transformEffect` runs the caller's step and reports
/// that step's own issue, on both axes.
#[test]
fn a_transform_effect_step_reports_its_own_issue() {
    let schema = Schema::<String>::decode_to_effect(
        Schema::string(),
        Transformation::effect(
            |value: String| {
                if value == "a" {
                    Err(SchemaError::FilterFailed {
                        path: "$".to_string(),
                        message: "input should not be \"a\"".to_string(),
                    })
                } else {
                    Ok(value)
                }
            },
            |value: String| {
                if value == "b" {
                    Err(SchemaError::FilterFailed {
                        path: "$".to_string(),
                        message: "input should not be \"b\"".to_string(),
                    })
                } else {
                    Ok(value)
                }
            },
        ),
    );

    assert_eq!(schema.decode(text("b")).unwrap(), "b");
    let refused = schema.decode(text("a")).unwrap_err().to_string();
    assert!(refused.contains("input should not be \"a\""), "{refused}");

    assert_eq!(schema.try_encode(&"a".to_string()).unwrap(), text("a"));
    let refused = schema.try_encode(&"b".to_string()).unwrap_err().to_string();
    assert!(refused.contains("input should not be \"b\""), "{refused}");

    // The step came from an Effect, which is what a sync adapter reads.
    assert!(schema.requires_effect());
}

/// `Schema.test.ts:8323` — a `checkEffect` refuses with the issue it produced,
/// and leaves a passing value alone.
#[test]
fn a_check_effect_reports_its_own_issue() {
    let schema = Schema::checked_effect(
        string(),
        filter("input should not be empty string", |value| {
            !matches!(value, Unknown::Text(text) if text.is_empty())
        }),
    );

    assert_eq!(schema.decode(text("a")).unwrap(), text("a"));
    let refused = decode_error(&schema, text(""));
    assert!(
        refused.contains("input should not be empty string"),
        "{refused}"
    );
    assert!(schema.requires_effect());
}

/// `Schema.test.ts:727` — an error class encodes its discriminant and declared
/// fields only, so the internals a runtime error carries (a message, a stack)
/// are not wire properties even under the strictest excess policy.
#[test]
fn an_error_class_encodes_without_its_runtime_internals() {
    let schema = tagged_error("NotFound", "NotFound", vec![("id", number_schema())]);
    let value = schema
        .make(obj(vec![("id", Unknown::Number(1.0))]))
        .expect("the constructor fills the declared field");

    let wire = ParseOptions {
        errors: Errors::First,
        on_excess_property: ExcessProperty::Reject,
    }
    .scoped(|| schema.try_encode(&value));

    assert_eq!(
        wire.unwrap(),
        obj(vec![
            ("_tag", text("NotFound")),
            ("id", Unknown::Number(1.0))
        ]),
        "only the discriminant and the declared fields travel"
    );
}

/// `Schema.test.ts:3802` — `disableChecks` skips the source's checks and still
/// applies the constructor default, because the default belongs to the field.
#[test]
fn a_constructor_with_checks_disabled_still_fills_defaults() {
    let defaulted = Schema::with_constructor_default(string(), || Ok(text("default")));
    let source = Schema::filtered(
        Schema::as_unknown(Schema::struct_shape(vec![Field::required(
            "a", defaulted,
        )])),
        vec![filter("this source refuses every value", |_| false)],
        false,
    );
    let class = class_from("A", source);

    assert!(
        class.make(obj(vec![])).is_err(),
        "with checks enabled the source refuses"
    );

    let instance = class
        .make_unchecked(obj(vec![]))
        .expect("checks are disabled for this construction");
    assert_eq!(instance.get("a"), Some(&text("default")));
}
