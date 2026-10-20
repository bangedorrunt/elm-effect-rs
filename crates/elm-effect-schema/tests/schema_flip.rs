//! `Schema.test.ts` "flip" rows: the swapped codec, the check that lands on
//! the encoded side, and the options a derivation keeps.
//!
//!     cargo test -p elm-effect-schema --test schema_flip
//!
//! The pin splits checks into a from-side list and an `encodingChecks` list,
//! and a `flip` moves one into the other. This port's filter chain runs on
//! both axes of whatever schema carries it, so `flip & check & flip` needs no
//! such split: the check rides the codec and decides every direction. What the
//! witnesses assert is the behaviour the pin's split produces — the check
//! refusing on both the decoding and the encoding path — plus the two things
//! the split is visible through here, `flip_source` and the repr.
//!
//! `:2662` (the `isMinLength(3)` check on a transform's *output*) is not
//! carried, and the reason is `as_unknown`: re-typing a typed transform
//! normalizes its decode back to the wire form (`"3"` in, `"3"` out), so a
//! flip of it produces no text to measure. The check-through-transform
//! mechanism (`schema_checked_transform.rs`) gives a check the typed value, but
//! the value the flip would hand it no longer exists at this level.

use elm_effect_schema::filters::{FilterIssue, FilterOutcome, FilterSpec};
use elm_effect_schema::{Field, Schema, Unknown, flip_source};
use std::collections::BTreeMap;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

/// One filter with the pin's own message.
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

/// `Schema.Struct({ a: String })`, the shape the Struct row flips.
fn struct_a() -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_shape(vec![Field::required("a", string())]))
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// The `flip & check & flip` shape every one of these rows uses: a check
/// applied after a flip, then flipped back.
fn flip_check_flip(schema: Schema<Unknown>, spec: FilterSpec) -> Schema<Unknown> {
    Schema::flip(Schema::filtered(Schema::flip(schema), vec![spec], false))
}

/// Schema.test.ts:2654 — "should expose the schema". A flip hands back the
/// schema it flipped, and an annotation over the flip keeps it reachable.
#[test]
fn a_flip_exposes_the_schema_it_flipped() {
    let schema = struct_a();
    let flipped = Schema::flip(schema.clone());

    let source = flip_source(&flipped).expect("a flip exposes the schema it flipped");
    let wire = obj(vec![("a", text("x"))]);
    assert_eq!(
        source.decode(wire.clone()).unwrap(),
        schema.decode(wire.clone()).unwrap()
    );

    let annotated = Schema::annotated_with(flipped, vec![("title", "flipped")]);
    let source = flip_source(&annotated).expect("an annotation keeps the schema it flipped");
    assert_eq!(
        source.decode(wire.clone()).unwrap(),
        schema.decode(wire.clone()).unwrap()
    );

    // Flipping twice is the schema it started from, on both axes.
    let twice = Schema::flip(Schema::flip(schema.clone()));
    assert_eq!(twice.decode(wire.clone()), schema.decode(wire.clone()));
    assert_eq!(
        twice.try_encode(&obj(vec![("a", text("y"))])),
        schema.try_encode(&obj(vec![("a", text("y"))]))
    );
}

/// Schema.test.ts:2684 — "Struct & flip & check & flip should apply the check
/// to the encoded side".
#[test]
fn a_struct_check_after_a_flip_decides_both_directions() {
    let schema = flip_check_flip(
        struct_a(),
        filter("a length > 1", |value| match value {
            Unknown::Object(map) => {
                matches!(map.get("a"), Some(Unknown::Text(text)) if text.len() > 1)
            }
            _ => false,
        }),
    );

    assert!(schema.decode(obj(vec![("a", text("aa"))])).is_ok());
    assert_eq!(
        schema
            .decode(obj(vec![("a", text("a"))]))
            .unwrap_err()
            .to_string(),
        "a length > 1 at $"
    );
    assert_eq!(
        schema
            .try_encode(&obj(vec![("a", text("a"))]))
            .unwrap_err()
            .to_string(),
        "a length > 1 at $"
    );
    assert!(schema.try_encode(&obj(vec![("a", text("aa"))])).is_ok());
}

/// `Schema.Struct({ a: String, b: String })`, the shape `:164` collects over.
fn struct_ab() -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_shape(vec![
        Field::required("a", string()),
        Field::required("b", string()),
    ]))
}

/// Schema.test.ts:164 — "applies errors with encoding checks". A check added
/// through a flip is the *encoding* side's check in the pin, so the struct
/// under it still reports one issue per absent key and the encode path is the
/// one the check decides.
///
/// This used to report a phantom `$.flipped`: the flip stored the schema it
/// wrapped in `children`, which the object path reads as declared fields.
#[test]
fn a_check_behind_a_flip_still_collects_every_missing_key() {
    let schema = flip_check_flip(
        struct_ab(),
        filter("a value with at most 1 entry", |value| match value {
            Unknown::Object(map) => map.len() <= 1,
            _ => true,
        }),
    );

    // `errors: "all"` names both absent keys, at the fields they belong to.
    let issues = schema
        .decode_collect(obj(vec![]))
        .expect_err("an empty object has no required key");
    let reported: Vec<String> = issues.iter().map(|issue| issue.to_string()).collect();
    assert_eq!(issues.len(), 2, "{reported:?}");
    assert!(reported[0].contains("$.a"), "{reported:?}");
    assert!(reported[1].contains("$.b"), "{reported:?}");

    // The check decides the encoding path, so two keys have no wire form.
    let two = obj(vec![("a", text("x")), ("b", text("y"))]);
    assert_eq!(
        schema.try_encode(&two).unwrap_err().to_string(),
        "a value with at most 1 entry at $"
    );
    assert!(schema.try_encode(&obj(vec![("a", text("x"))])).is_ok());
}

/// Schema.test.ts:2710 — "Tuple & flip & check & flip should apply the check to
/// the encoded side".
#[test]
fn a_tuple_check_after_a_flip_decides_both_directions() {
    let schema = flip_check_flip(
        Schema::tuple_of(vec![string()]),
        filter("head length > 1", |value| match value {
            Unknown::Array(items) => {
                matches!(items.first(), Some(Unknown::Text(text)) if text.len() > 1)
            }
            _ => false,
        }),
    );

    assert!(schema.decode(Unknown::Array(vec![text("aa")])).is_ok());
    assert_eq!(
        schema
            .decode(Unknown::Array(vec![text("a")]))
            .unwrap_err()
            .to_string(),
        "head length > 1 at $"
    );
    assert_eq!(
        schema
            .try_encode(&Unknown::Array(vec![text("a")]))
            .unwrap_err()
            .to_string(),
        "head length > 1 at $"
    );
    assert!(schema.try_encode(&Unknown::Array(vec![text("aa")])).is_ok());
}

/// Schema.test.ts:2736 — "Union & flip & check & flip should apply the check to
/// the encoded side".
#[test]
fn a_union_check_after_a_flip_decides_both_directions() {
    let cases = Schema::union_with_mode(
        vec![Schema::literal(text("a")), Schema::literal(text("aa"))],
        "anyOf",
    );
    let schema = flip_check_flip(
        cases,
        filter(
            "\"aa\"",
            |value| matches!(value, Unknown::Text(text) if text == "aa"),
        ),
    );

    assert!(schema.decode(text("aa")).is_ok());
    assert_eq!(
        schema.decode(text("a")).unwrap_err().to_string(),
        "\"aa\" at $"
    );
    assert_eq!(
        schema.try_encode(&text("a")).unwrap_err().to_string(),
        "\"aa\" at $"
    );
    assert!(schema.try_encode(&text("aa")).is_ok());
}

/// Schema.test.ts:2762 — "Declaration & flip & check & flip should apply the
/// check to the encoded side". The pin's row declares a host predicate
/// (`typeof u === "string"`); the port's `declare` states the same rule over
/// the values it has.
#[test]
fn a_declaration_check_after_a_flip_decides_both_directions() {
    let declaration = Schema::declare(
        |value| matches!(value, Unknown::Text(_)),
        "string declaration",
    );
    let schema = flip_check_flip(
        declaration,
        filter(
            "a length > 1",
            |value| matches!(value, Unknown::Text(text) if text.len() > 1),
        ),
    );

    assert!(schema.decode(text("aa")).is_ok());
    assert_eq!(
        schema.decode(text("a")).unwrap_err().to_string(),
        "a length > 1 at $"
    );
    assert_eq!(
        schema.try_encode(&text("a")).unwrap_err().to_string(),
        "a length > 1 at $"
    );
    assert!(schema.try_encode(&text("aa")).is_ok());
}

/// Schema.test.ts:9049 — "preserves Union options through public
/// derivations". The port prints a union's mode in its repr, so that is where
/// the options are read back from.
#[test]
fn a_unions_options_survive_its_derivations() {
    let union = Schema::union_with_mode(
        vec![string(), Schema::as_unknown(Schema::number())],
        "oneOf",
    );
    assert!(union.to_code().contains("mode: \"oneOf\""));

    // A derivation that rebuilds the member list keeps them...
    let mapped = union.clone().map_members(|mut members| {
        members.push(Schema::as_unknown(Schema::boolean()));
        members
    });
    assert!(mapped.to_code().contains("mode: \"oneOf\""));
    assert_eq!(
        mapped.members().expect("a union keeps its members").len(),
        3
    );

    // ...and so does a flip, which is still a union afterwards.
    let flipped = Schema::flip(union);
    assert!(flipped.to_code().contains("mode: \"oneOf\""));
}
