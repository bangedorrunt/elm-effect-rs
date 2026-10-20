//! Slice 4: options/eagerness semantics.
//! Binds Schema.test.ts:131 (`does not interpret custom parseOptions
//! metadata`), :905 (`{ errors: "all" }`), :5882/:5899/:5917 (the three `tag`
//! optionality rows) and :5945 (`tagDefaultOmit`). The other two tests here are
//! port-local: their pin rows (:59 eagerness, :735 `__proto__`) assert
//! JavaScript surfaces this port does not have, and the ledger carries the
//! reason rather than a witness.

use elm_effect_schema::transform::{Errors, ParseOptions};
use elm_effect_schema::{ExcessProperty, Field, Schema, SchemaError, Unknown};
use std::collections::BTreeMap;

/// One parse-option pair, spelled once so the witnesses read as decisions.
fn options(errors: Errors, on_excess_property: ExcessProperty) -> ParseOptions {
    ParseOptions {
        errors,
        on_excess_property,
    }
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// `Schema.Number.check(Schema.isGreaterThan(0), Schema.isInt())` — the base the
/// metadata row annotates. Rebuilt per call so both spellings stay independent.
fn bounded_int() -> Schema<Unknown> {
    Schema::checked(
        Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isGreaterThan",
            vec!["0"],
            vec![],
        ),
        "isInt",
        vec![],
        vec![],
    )
}

/// Schema.test.ts:131 `does not interpret custom parseOptions metadata`.
/// The annotation rides the codec: the annotated schema rejects and accepts
/// exactly what the bare one does, so `errors: "all"` changes nothing here. The
/// pin's expected text names the failing check; this port's check errors are
/// generic `TypeMismatch`es, so the claim covers which value fails.
#[test]
fn custom_metadata_is_not_interpreted() {
    let plain = bounded_int();
    let annotated = Schema::annotated_with(
        bounded_int(),
        vec![("parseOptions", r#"{"errors": "all"}"#)],
    );

    let rejected = Unknown::Number(-1.2);
    assert!(plain.decode(rejected.clone()).is_err());
    assert!(annotated.decode(rejected).is_err());

    let accepted = Unknown::Number(1.0);
    assert_eq!(annotated.decode(accepted.clone()), plain.decode(accepted));
    // A non-integer inside the bound still fails, so the annotation did not
    // relax the second check either.
    assert!(annotated.decode(Unknown::Number(1.5)).is_err());
}

/// Schema.test.ts:905 `{ errors: "all" }`. A struct with two required fields,
/// asked to take `{}`, reports both misses on all three axes: construction,
/// decoding and encoding. The fail-fast entry points name only the first, so
/// the count is what proves the walk kept going.
#[test]
fn all_errors_reports_every_missing_field() {
    let schema = Schema::struct_shape(vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(Schema::number())),
    ]);
    let empty = obj(vec![]);
    let no_value = BTreeMap::new();

    let made = schema.construct_collect(empty.clone()).unwrap_err();
    let decoded = schema.decode_collect(empty.clone()).unwrap_err();
    let encoded = schema.encode_collect(&no_value).unwrap_err();

    for (axis, issues) in [("make", &made), ("decode", &decoded), ("encode", &encoded)] {
        let messages: Vec<String> = issues.iter().map(ToString::to_string).collect();
        assert_eq!(messages.len(), 2, "{axis}: {messages:?}");
        assert!(messages[0].contains("at $.a"), "{axis}: {messages:?}");
        assert!(messages[1].contains("at $.b"), "{axis}: {messages:?}");
        assert!(
            messages.iter().all(|message| message.contains("missing")),
            "{axis}: {messages:?}"
        );
    }

    // Fail-fast stops at the first field on every axis; that is the difference
    // this row pins.
    assert!(
        schema
            .construct(empty.clone())
            .unwrap_err()
            .to_string()
            .contains("at $.a")
    );
    assert!(
        schema
            .decode(empty)
            .unwrap_err()
            .to_string()
            .contains("at $.a")
    );

    // A value with nothing to report encodes to exactly the fail-fast result,
    // so the collecting walk adds no second spelling.
    let good: BTreeMap<String, Unknown> = [
        ("a".to_string(), Unknown::Text("x".to_string())),
        ("b".to_string(), Unknown::Number(1.0)),
    ]
    .into_iter()
    .collect();
    assert!(schema.encode_collect(&good).is_ok());
    assert_eq!(
        schema.encode_collect(&good).ok(),
        schema.try_encode(&good).ok()
    );
}

/// Schema.test.ts:144 `applies errors to nested schemas without annotation
/// overrides`. The inner struct is annotated `errors: "first"`; a collecting
/// decode entering through the outer struct still reports both inner misses,
/// because this port treats `parseOptions` as data and never lets it travel.
#[test]
fn a_nested_annotation_does_not_narrow_collection() {
    let inner = Schema::annotated_with(
        Schema::struct_shape(vec![
            Field::required("c", Schema::as_unknown(Schema::string())),
            Field::required("d", Schema::as_unknown(Schema::string())),
        ]),
        vec![("parseOptions", r#"{"errors": "first"}"#)],
    );
    let outer = Schema::struct_shape(vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(inner)),
    ]);

    let issues = outer
        .decode_collect(obj(vec![("a", text("a")), ("b", obj(vec![]))]))
        .unwrap_err();
    let messages: Vec<String> = issues.iter().map(ToString::to_string).collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("at $.b.c"), "{messages:?}");
    assert!(messages[1].contains("at $.b.d"), "{messages:?}");

    // The outer field that did arrive is not reported, so the walk ran the
    // child instead of guessing from the parent's shape.
    let partly = outer
        .decode_collect(obj(vec![("a", Unknown::Number(1.0)), ("b", obj(vec![]))]))
        .unwrap_err();
    let partly: Vec<String> = partly.iter().map(ToString::to_string).collect();
    assert_eq!(partly.len(), 3, "{partly:?}");
    assert!(partly[0].contains("at $.a"), "{partly:?}");
}

/// Schema.test.ts:187 `decoders can receive options when they are created`.
/// A decoder bound to `onExcessProperty: "error"` refuses an undeclared key,
/// one call may override that for itself, and the next call without an
/// override is back on the bound option.
#[test]
fn a_decoder_keeps_the_options_it_was_created_with() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let strict = options(Errors::First, ExcessProperty::Reject);
    let lenient = options(Errors::First, ExcessProperty::Ignore);
    let decode = schema.decoder(strict);

    let input = || obj(vec![("a", text("a")), ("b", text("b"))]);
    assert!(
        decode.decode(input()).is_err(),
        "the bound option refuses b"
    );
    let accepted = decode.decode_with(input(), &lenient).unwrap();
    assert_eq!(
        accepted.get("b"),
        None,
        "the excess key is stripped, not kept"
    );
    assert_eq!(accepted.get("a"), Some(&text("a")));
    assert!(
        decode.decode(input()).is_err(),
        "the override belongs to that call, not to the decoder"
    );
}

/// Schema.test.ts:202 `encoders can receive options when they are created`,
/// the encoding twin of the row above.
#[test]
fn an_encoder_keeps_the_options_it_was_created_with() {
    let schema = Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::string()),
    )]);
    let strict = options(Errors::First, ExcessProperty::Reject);
    let lenient = options(Errors::First, ExcessProperty::Ignore);
    let encode = schema.encoder(strict);
    let value: BTreeMap<String, Unknown> =
        [("a".to_string(), text("a")), ("b".to_string(), text("b"))]
            .into_iter()
            .collect();

    assert!(encode.encode(&value).is_err(), "the bound option refuses b");
    let encoded = encode.encode_with(&value, &lenient).unwrap();
    assert!(matches!(&encoded, Unknown::Object(m) if m.get("b").is_none()));
    assert!(matches!(&encoded, Unknown::Object(m) if m.get("a") == Some(&text("a"))));
    assert!(
        encode.encode(&value).is_err(),
        "the override belongs to that call, not to the encoder"
    );
}

/// Schema.test.ts:1038 `should fail on unexpected indexes`. Tuple overflow is
/// a refusal at each extra position; the default reports the first and
/// `errors: "all"` reports every one.
#[test]
fn a_tuple_refuses_positions_past_its_end() {
    let schema = Schema::tuple_of(vec![Schema::as_unknown(Schema::string())]);

    let first = schema
        .decode(Unknown::Array(vec![text("a"), text("b")]))
        .unwrap_err()
        .to_string();
    assert!(first.contains("at $[1]"), "{first}");
    assert!(first.contains("no excess property"), "{first}");

    let error = schema
        .decode_with(
            Unknown::Array(vec![text("a"), text("b"), text("c")]),
            &options(Errors::All, ExcessProperty::Ignore),
        )
        .unwrap_err();
    let messages: Vec<String> = match error {
        SchemaError::Composite { issues } => issues.iter().map(ToString::to_string).collect(),
        other => panic!("expected every issue, got {other}"),
    };
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("at $[1]"), "{messages:?}");
    assert!(messages[1].contains("at $[2]"), "{messages:?}");
}

/// Schema.test.ts:5075 `applies excess-property options after fixed and index
/// signatures`. A struct with rest, one fixed field and an index signature
/// that covers `a`: the uncovered key `b` is dropped by default and named under
/// `onExcessProperty: "error"`, on decode and encode alike.
#[test]
fn excess_options_apply_after_fixed_and_index_signatures() {
    // A literal key schema is this port's spelling for the row's `^a` pattern:
    // it covers exactly the keys it names.
    let rest = Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::record2(
        Schema::literal(Unknown::Text("a".to_string())),
        Schema::as_unknown(Schema::number()),
    ));
    let schema = Schema::<Unknown>::struct_with_rest(
        vec![("fixed".to_string(), Schema::as_unknown(Schema::number()))],
        rest,
    );

    let input = || {
        obj(vec![
            ("fixed", number(0.0)),
            ("a", number(1.0)),
            ("b", number(2.0)),
        ])
    };
    let expected = obj(vec![("fixed", number(0.0)), ("a", number(1.0))]);

    // Decoding drops the key no signature covers.
    assert_eq!(schema.decode(input()).unwrap(), expected);
    let refused = schema
        .decode_with(input(), &options(Errors::First, ExcessProperty::Reject))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at $.b"), "{refused}");

    // Encoding makes the same decision on the way out.
    assert_eq!(schema.try_encode(&input()).unwrap(), expected);
    let refused = schema
        .encode_with(&input(), &options(Errors::First, ExcessProperty::Reject))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at $.b"), "{refused}");
}

/// Port-local (pin row Schema.test.ts:735 is not-applicable): with no prototype
/// chain, `__proto__` is an ordinary key that decodes as its own field.
#[test]
fn proto_key_is_ordinary_data() {
    let schema = Schema::struct_shape(vec![Field::required(
        "__proto__",
        Schema::as_unknown(Schema::string()),
    )]);
    let out: BTreeMap<String, Unknown> = schema
        .decode(obj(vec![(
            "__proto__",
            Unknown::Text("polluted".to_string()),
        )]))
        .unwrap();
    assert_eq!(
        out.get("__proto__"),
        Some(&Unknown::Text("polluted".to_string()))
    );
    assert!(
        schema
            .decode(obj(vec![("other", Unknown::Text("a".to_string()))]))
            .is_err()
    );
}

/// Schema.test.ts:5945 `tagDefaultOmit`.
#[test]
fn tag_fills_default_and_omits_on_encode() {
    let schema = Schema::struct_shape(vec![
        Field::tag("_tag", Unknown::Text("a".to_string())),
        Field::required("a", Schema::as_unknown(Schema::string())),
    ]);
    // make fills the missing tag.
    let made = schema
        .make(
            [("a".to_string(), Unknown::Text("1".to_string()))]
                .into_iter()
                .collect(),
        )
        .unwrap();
    assert_eq!(made.get("_tag"), Some(&Unknown::Text("a".to_string())));
    // decoding fills it too.
    let decoded = schema
        .decode(obj(vec![("a", Unknown::Text("1".to_string()))]))
        .unwrap();
    assert_eq!(decoded.get("_tag"), Some(&Unknown::Text("a".to_string())));
    // A tag other than the default fails decoding.
    assert!(
        schema
            .decode(obj(vec![
                ("_tag", Unknown::Text("c".to_string())),
                ("a", Unknown::Text("1".to_string()))
            ]))
            .is_err()
    );
    // encoding omits it, and leaves the payload field.
    let encoded = schema.try_encode(&decoded).unwrap();
    assert!(matches!(encoded, Unknown::Object(ref m) if !m.contains_key("_tag")));
    assert!(matches!(encoded, Unknown::Object(ref m) if m.contains_key("a")));
    let encoded_made = schema.try_encode(&made).unwrap();
    assert!(matches!(encoded_made, Unknown::Object(ref m) if !m.contains_key("_tag")));
}

/// The payload every `tag` row carries: a number whose wire form is text, so a
/// witness can tell the decoded value from the encoded one.
fn a_field() -> Field {
    Field::required("a", Schema::as_unknown(Schema::finite_from_string()))
}

fn tag_of(value: &str) -> Schema<Unknown> {
    Schema::as_unknown(Schema::literal(Unknown::Text(value.to_string())))
}

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// Schema.test.ts:5882 `tag`, "decoding: required & encoding: required &
/// constructor: required". A plain literal tag is required on all three axes.
///
/// The payload is `FiniteFromString`, and this port's `Unknown` is the wire
/// form, so the value that comes back out is `"1"` at every level; what is
/// asserted here is that the field codec ran (a non-numeric string is refused)
/// and that the tag's axis is the pin's.
#[test]
fn a_required_tag_is_required_everywhere() {
    let schema = Schema::struct_shape(vec![Field::required("_tag", tag_of("a")), a_field()]);

    // Construction needs the tag: the default is the payload's, not the tag's.
    let made = schema
        .make(
            [
                ("_tag".to_string(), text("a")),
                ("a".to_string(), number(1.0)),
            ]
            .into_iter()
            .collect(),
        )
        .unwrap();
    assert_eq!(made.get("_tag"), Some(&text("a")));
    assert_eq!(made.get("a"), Some(&text("1")));
    assert!(
        schema
            .make([("a".to_string(), number(1.0))].into_iter().collect())
            .is_err(),
        "a required tag cannot be filled in by construction"
    );

    // Decoding is required too, and the payload's codec runs on the way in.
    let decoded = schema
        .decode(obj(vec![("_tag", text("a")), ("a", text("1"))]))
        .unwrap();
    assert_eq!(decoded.get("a"), Some(&text("1")));
    assert!(schema.decode(obj(vec![("a", text("1"))])).is_err());
    assert!(
        schema
            .decode(obj(vec![("_tag", text("a")), ("a", text("x"))]))
            .is_err(),
        "the payload field is decoded, not passed through"
    );

    // And so is encoding.
    let encoded = schema.try_encode(&decoded).unwrap();
    assert!(matches!(&encoded, Unknown::Object(m) if m.get("_tag") == Some(&text("a"))));
    assert!(matches!(&encoded, Unknown::Object(m) if m.get("a") == Some(&text("1"))));
}

/// Schema.test.ts:5899 `tag`, "decoding: required & encoding: required &
/// constructor: optional". Only construction may fill the tag, which is the
/// difference from `tagDefaultOmit`: decoding and encoding still demand it.
#[test]
fn a_constructor_defaulted_tag_stays_required_on_the_wire() {
    let tag =
        Schema::as_unknown(Schema::literal(text("a"))).with_constructor_default(|| Ok(text("a")));
    let schema = Schema::struct_shape(vec![Field::required("_tag", tag), a_field()]);

    let made = schema
        .make([("a".to_string(), number(1.0))].into_iter().collect())
        .unwrap();
    assert_eq!(made.get("_tag"), Some(&text("a")));
    assert_eq!(made.get("a"), Some(&text("1")));

    let decoded = schema
        .decode(obj(vec![("_tag", text("a")), ("a", text("1"))]))
        .unwrap();
    assert_eq!(decoded.get("a"), Some(&text("1")));
    assert!(
        schema.decode(obj(vec![("a", text("1"))])).is_err(),
        "the constructor default must not leak into decoding"
    );
    assert!(
        schema
            .decode(obj(vec![("_tag", text("a")), ("a", text("x"))]))
            .is_err(),
        "the payload field is decoded, not passed through"
    );

    let encoded = schema.try_encode(&decoded).unwrap();
    assert!(
        matches!(&encoded, Unknown::Object(m) if m.get("_tag") == Some(&text("a"))),
        "the constructor default must not leak into encoding"
    );
    assert!(matches!(&encoded, Unknown::Object(m) if m.get("a") == Some(&text("1"))));
}

/// Schema.test.ts:5917 `tag`, "decoding: default & encoding: omit &
/// constructor: optional". This is `Field::tag`: absent fills the default on
/// both the decode and the construct path, and the key does not travel out.
#[test]
fn a_defaulted_tag_is_filled_and_omitted() {
    let schema = Schema::struct_shape(vec![Field::tag("_tag", text("a")), a_field()]);

    let made = schema
        .make([("a".to_string(), number(1.0))].into_iter().collect())
        .unwrap();
    assert_eq!(made.get("_tag"), Some(&text("a")));

    let decoded = schema
        .decode(obj(vec![("_tag", text("a")), ("a", text("1"))]))
        .unwrap();
    assert_eq!(decoded.get("_tag"), Some(&text("a")));
    assert_eq!(decoded.get("a"), Some(&text("1")));
    assert!(
        schema
            .decode(obj(vec![("_tag", text("a")), ("a", text("x"))]))
            .is_err(),
        "the payload field is decoded, not passed through"
    );

    // Absent on the wire decodes to the default.
    let defaulted = schema.decode(obj(vec![("a", text("1"))])).unwrap();
    assert_eq!(defaulted.get("_tag"), Some(&text("a")));

    // Encoding omits the tag and keeps the payload's wire form.
    let encoded = schema.try_encode(&decoded).unwrap();
    assert!(matches!(&encoded, Unknown::Object(m) if !m.contains_key("_tag")));
    assert!(matches!(&encoded, Unknown::Object(m) if m.get("a") == Some(&text("1"))));
    let encoded_defaulted = schema.try_encode(&defaulted).unwrap();
    assert!(matches!(&encoded_defaulted, Unknown::Object(m) if !m.contains_key("_tag")));
}
