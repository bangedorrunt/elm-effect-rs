//! `Schema.test.ts` "Tagged unions" rows: `toTaggedUnion` over structs, several
//! tags, duplicate and absent discriminants, `__proto__`, a union of classes,
//! and the `Schema.TaggedUnion` builder.
//!
//! One gap is stated rather than papered over: the pin's `guards` reflects the
//! *decoded* type (`SchemaParser.is(toType(member))`), and every decoded value
//! in this port is the same struct type, so reflection has nothing to tell
//! apart. `TaggedUnion::accepts` asks the wire question instead and is
//! asserted here; where a member's field transforms its wire form the two
//! answers differ by design, and that difference is named in the module note.
//!
//!     cargo test -p elm-effect-schema --test schema_tagged_union

use std::collections::BTreeMap;

use elm_effect_schema::class::tagged_class;
use elm_effect_schema::{Field, Schema, TaggedUnionError, Unknown, tagged_union, to_tagged_union};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn boolean() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

/// A required field holding one exact value, the pin's `Schema.Literal`.
fn literal(name: &str, value: Unknown) -> Field {
    Field::required(name, Schema::literal(value))
}

fn member(fields: Vec<Field>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::<BTreeMap<String, Unknown>>::struct_shape(fields))
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// The pin's message for a refused union.
fn refusal(union: Schema<Unknown>, tag: &str) -> TaggedUnionError {
    match to_tagged_union(union, tag) {
        Ok(_) => panic!("expected the union to be refused"),
        Err(error) => error,
    }
}

/// `Schema.test.ts:9433` — "should augment a union of structs".
///
/// The pin's second member discriminates on a unique symbol, which has no
/// `Unknown` in this port; the witness uses a text discriminant there and
/// exercises the number-versus-spelling key on the nested member instead.
#[test]
fn to_tagged_union_indexes_every_member_by_its_literal() {
    let a = member(vec![
        literal("_tag", text("A")),
        Field::required("a", string()),
    ]);
    let b = member(vec![
        literal("_tag", text("B")),
        Field::required("b", finite()),
    ]);
    let one = member(vec![
        literal("_tag", Unknown::Number(1.0)),
        Field::required("c", boolean()),
    ]);
    let d = member(vec![
        literal("_tag", text("D")),
        Field::required("d", string()),
    ]);
    let nested = Schema::union_with_mode(vec![one.clone(), d.clone()], "anyOf");
    let union = Schema::union_with_mode(vec![a.clone(), b.clone(), nested], "anyOf");

    let tagged = to_tagged_union(union, "_tag").expect("indexes");

    assert_eq!(
        tagged.discriminants,
        vec![text("A"), text("B"), Unknown::Number(1.0), text("D")],
        "the walk descends into the nested union and keeps its order"
    );

    // Cases point at the member they name. A code rendering is how two schemas
    // are compared here: `Schema` holds closures and has no equality of its
    // own, and the codes differ between the four members.
    assert_eq!(tagged.case_for(&text("A")).unwrap().to_code(), a.to_code());
    assert_eq!(tagged.case_for(&text("B")).unwrap().to_code(), b.to_code());
    assert_eq!(tagged.case_for(&text("D")).unwrap().to_code(), d.to_code());
    // The nested member's own cases are reached from the outer index, and a
    // number discriminant is reached by its printed spelling too.
    assert_eq!(
        tagged.case_for(&Unknown::Number(1.0)).unwrap().to_code(),
        one.to_code()
    );
    assert_eq!(
        tagged.case_for(&text("1")).unwrap().to_code(),
        one.to_code()
    );
    assert!(tagged.case_for(&text("nope")).is_none());

    // isAnyOf
    let is_a_or_1 = vec![text("A"), Unknown::Number(1.0)];
    assert!(tagged.is_any_of(&is_a_or_1, &obj(vec![("_tag", text("A"))])));
    assert!(tagged.is_any_of(&is_a_or_1, &obj(vec![("_tag", Unknown::Number(1.0))])));
    assert!(!tagged.is_any_of(&is_a_or_1, &obj(vec![("_tag", text("D"))])));
    assert!(!tagged.is_any_of(&is_a_or_1, &obj(vec![("_tag", text("B"))])));

    // match / matchOrElse: the handler registered for the value's tag wins, and
    // a case that is present with no handler falls to the fallback.
    let cases = vec![
        (text("A"), Some("A")),
        (text("B"), Some("B")),
        (Unknown::Number(1.0), Some("C")),
        (text("D"), Some("D")),
    ];
    let a_value = obj(vec![("_tag", text("A")), ("a", text("a"))]);
    let one_value = obj(vec![
        ("_tag", Unknown::Number(1.0)),
        ("c", Unknown::Bool(true)),
    ]);
    assert_eq!(tagged.dispatch(&a_value, &cases, None), Some("A"));
    assert_eq!(tagged.dispatch(&one_value, &cases, None), Some("C"));

    let only_a = vec![(text("A"), Some("A"))];
    assert_eq!(
        tagged.dispatch(&one_value, &only_a, Some(&"fallback")),
        Some("fallback")
    );
    let absent_handler = vec![(text("A"), None::<&str>)];
    assert_eq!(
        tagged.dispatch(&a_value, &absent_handler, Some(&"fallback")),
        Some("fallback")
    );
}

/// `Schema.test.ts:9530` — "should support multiple tags": the index follows
/// the tag the caller names, not whichever literal comes first.
#[test]
fn to_tagged_union_follows_the_requested_tag() {
    let a = member(vec![
        Field::tag("_tag", text("A")),
        Field::tag("type", text("TypeA")),
        Field::required("a", string()),
    ]);
    let b = member(vec![
        Field::tag("_tag", text("B")),
        Field::tag("type", text("TypeB")),
        Field::required("b", finite()),
    ]);
    let tagged = to_tagged_union(
        Schema::union_with_mode(vec![a.clone(), b.clone()], "anyOf"),
        "type",
    )
    .expect("indexes");

    assert_eq!(tagged.discriminants, vec![text("TypeA"), text("TypeB")]);
    assert_eq!(
        tagged.case_for(&text("TypeA")).unwrap().to_code(),
        a.to_code()
    );
    assert_eq!(
        tagged.case_for(&text("TypeB")).unwrap().to_code(),
        b.to_code()
    );
    assert!(
        tagged.case_for(&text("A")).is_none(),
        "the other tag key is not an index"
    );
}

/// `Schema.test.ts:9541` — "should throw on duplicate discriminants", including
/// a repeat that only shows up across a nested union.
#[test]
fn to_tagged_union_refuses_a_duplicate_discriminant() {
    let repeated = Schema::union_with_mode(
        vec![
            member(vec![
                literal("event", text("A")),
                Field::required("a", string()),
            ]),
            Schema::union_with_mode(
                vec![
                    member(vec![
                        literal("event", text("B")),
                        Field::required("b", string()),
                    ]),
                    member(vec![
                        literal("event", text("A")),
                        Field::required("c", string()),
                    ]),
                ],
                "anyOf",
            ),
        ],
        "anyOf",
    );
    assert_eq!(
        refusal(repeated, "event").to_string(),
        "Duplicate discriminant: A"
    );

    // A number and its printed spelling are the same object key.
    let by_spelling = Schema::union_with_mode(
        vec![
            member(vec![literal("event", Unknown::Number(1.0))]),
            member(vec![literal("event", text("1"))]),
        ],
        "anyOf",
    );
    assert_eq!(
        refusal(by_spelling, "event").to_string(),
        "Duplicate discriminant: 1"
    );
}

/// `Schema.test.ts:9563` — "should collect no discriminants from an empty
/// union".
#[test]
fn to_tagged_union_of_an_empty_union_has_no_cases() {
    let tagged = to_tagged_union(Schema::union_with_mode(Vec::new(), "anyOf"), "event")
        .expect("an empty union indexes");

    assert!(tagged.discriminants.is_empty());
    assert!(tagged.cases.is_empty());
    assert!(tagged.case_for(&text("event")).is_none());
}

/// `Schema.test.ts:9569` — "should support `__proto__` as a discriminant". The
/// index is a list rather than an object literal, so the name is ordinary.
#[test]
fn to_tagged_union_admits_the_proto_key() {
    let only = member(vec![
        literal("event", text("__proto__")),
        Field::required("value", string()),
    ]);
    let tagged = to_tagged_union(
        Schema::union_with_mode(vec![only.clone()], "anyOf"),
        "event",
    )
    .expect("indexes");

    assert_eq!(
        tagged.case_for(&text("__proto__")).unwrap().to_code(),
        only.to_code()
    );
    assert!(tagged.accepts(
        &text("__proto__"),
        &obj(vec![("event", text("__proto__")), ("value", text("a"))])
    ));
}

/// `Schema.test.ts:9579` — "should augment a union of classes": a class keeps
/// its discriminant in the class metadata, and the index reads it from there.
#[test]
fn to_tagged_union_indexes_a_union_of_classes() {
    let a = tagged_class("A", vec![("a", string())]);
    let b = tagged_class("B", vec![("b", finite())]);
    let tagged = to_tagged_union(
        Schema::union_with_mode(
            vec![Schema::as_unknown(a.clone()), Schema::as_unknown(b.clone())],
            "anyOf",
        ),
        "_tag",
    )
    .expect("indexes");

    assert_eq!(tagged.discriminants, vec![text("A"), text("B")]);
    // Each case is the class it names: the other class refuses the tag.
    let a_value = obj(vec![("_tag", text("A")), ("a", text("a"))]);
    assert!(
        tagged
            .case_for(&text("A"))
            .unwrap()
            .decode(a_value.clone())
            .is_ok()
    );
    assert!(
        tagged
            .case_for(&text("B"))
            .unwrap()
            .decode(a_value)
            .is_err()
    );
    // The class still decodes through the indexed union.
    assert!(
        tagged
            .schema
            .decode(obj(vec![("_tag", text("A")), ("a", text("x"))]))
            .is_ok(),
        "the union behind the index is untouched"
    );
}

/// `Schema.test.ts:9598` — "should create a tagged union": each case key
/// becomes the member's `_tag` literal.
#[test]
fn tagged_union_builds_its_members_from_the_case_list() {
    let schema = tagged_union(vec![
        ("A", vec![Field::required("a", string())]),
        ("C", vec![Field::required("c", boolean())]),
        ("B", vec![Field::required("b", finite())]),
    ])
    .expect("builds");

    assert_eq!(
        schema.discriminants,
        vec![text("A"), text("C"), text("B")],
        "the case list keeps the order it was given"
    );

    for (name, field, code) in [
        ("A", "a", string().to_code()),
        ("B", "b", finite().to_code()),
        ("C", "c", boolean().to_code()),
    ] {
        let case = schema.case_for(&text(name)).expect("a case");
        let fields = case.fields().expect("a struct shape");
        let tag = fields
            .iter()
            .find(|field| field.name() == "_tag")
            .expect("every case carries `_tag`");
        assert_eq!(tag.inner().literal_value(), Some(&text(name)));
        let declared = fields
            .iter()
            .find(|declared| declared.name() == field)
            .expect("the declared field is there");
        assert_eq!(declared.inner().to_code(), code);
    }

    let is_a_or_b = vec![text("A"), text("B")];
    assert!(schema.is_any_of(&is_a_or_b, &obj(vec![("_tag", text("A"))])));
    assert!(schema.is_any_of(&is_a_or_b, &obj(vec![("_tag", text("B"))])));
    assert!(!schema.is_any_of(&is_a_or_b, &obj(vec![("_tag", text("C"))])));
}
