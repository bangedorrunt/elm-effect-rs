//! Slice 9: the `oneOf` union mode refuses a value two members accept.
//!
//! The pin's `Union(members, { mode: "oneOf" })` is a decode rule, not a repr
//! detail: a value that matches more than one member is an error, including the
//! degenerate case of the same member listed twice. These are the rows that
//! distinguish the mode from the default `anyOf`, which keeps the first match.
//! Binds Schema.test.ts rows 4404-4472.

use elm_effect_schema::{Schema, Unknown};

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

fn literal_of(value: &str) -> Schema<Unknown> {
    Schema::literal(text(value))
}

fn str_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

/// A `Struct` member, the spelling the pin's `oneOf` rows use. Its decode drops
/// undeclared keys, which is what lets one object match two members.
fn struct_member(fields: Vec<(&str, Schema<Unknown>)>) -> Schema<Unknown> {
    let fields: Vec<(String, Schema<Unknown>)> = fields
        .into_iter()
        .map(|(name, schema)| (name.to_string(), schema))
        .collect();
    Schema::as_unknown(Schema::struct_of(fields))
}

fn member_error(schema: &Schema<Unknown>, value: Unknown) -> String {
    schema
        .decode(value)
        .expect_err("an ambiguous value has no member to resolve to")
        .to_string()
}

/// Schema.test.ts:4404 `mode: "oneOf"`: each member alone decodes, and the
/// object both members accept does not.
#[test]
fn one_of_refuses_an_object_two_members_accept() {
    let schema = Schema::union_with_mode(
        vec![
            struct_member(vec![("a", str_schema())]),
            struct_member(vec![("b", number_schema())]),
        ],
        "oneOf",
    );

    assert_eq!(
        schema.decode(obj(vec![("a", text("a"))])).unwrap(),
        obj(vec![("a", text("a"))])
    );
    assert_eq!(
        schema
            .decode(obj(vec![("b", Unknown::Number(1.0))]))
            .unwrap(),
        obj(vec![("b", Unknown::Number(1.0))])
    );
    assert_eq!(
        member_error(
            &schema,
            obj(vec![("a", text("a")), ("b", Unknown::Number(1.0))])
        ),
        "expected exactly one member to match at $, got object"
    );
}

/// The contrast that makes the mode the cause: the same members under the
/// default mode resolve the ambiguous object to whoever comes first, and the
/// first member's decode is what comes back (`onExcessProperty: "ignore"`
/// drops the key it never declared).
#[test]
fn any_of_still_resolves_an_ambiguous_object_to_the_first_member() {
    let schema = Schema::union_with_mode(
        vec![
            struct_member(vec![("a", str_schema())]),
            struct_member(vec![("b", number_schema())]),
        ],
        "anyOf",
    );

    let ambiguous = obj(vec![("a", text("a")), ("b", Unknown::Number(1.0))]);
    assert_eq!(
        schema.decode(ambiguous).unwrap(),
        obj(vec![("a", text("a"))])
    );
}

/// Schema.test.ts:4420 `mode: "oneOf" with different sentinel keys`: the two
/// members discriminate on different keys, so an object carrying both
/// discriminants matches both.
#[test]
fn one_of_refuses_an_object_carrying_both_sentinels() {
    let schema = Schema::union_with_mode(
        vec![
            struct_member(vec![("kind", literal_of("a")), ("value", str_schema())]),
            struct_member(vec![
                ("status", literal_of("ready")),
                ("value", str_schema()),
            ]),
        ],
        "oneOf",
    );

    let both = obj(vec![
        ("kind", text("a")),
        ("status", text("ready")),
        ("value", text("value")),
    ]);
    assert!(schema.decode(both).is_err());
    assert!(
        schema
            .decode(obj(vec![("kind", text("a")), ("value", text("value"))]))
            .is_ok()
    );
}

/// Schema.test.ts:4434 `mode: "oneOf" with nested and contradicted sentinels`.
/// The nested union is a single member of the outer one, so a block matching
/// exactly one nested member is not ambiguous, while a block matching none is
/// still a failure. The pin asserts its own type message here; the port's error
/// names the path instead.
#[test]
fn one_of_inside_a_struct_counts_the_nested_member_once() {
    let nested = Schema::union_with_mode(
        vec![
            struct_member(vec![
                ("kind", literal_of("a")),
                ("variant", literal_of("x")),
            ]),
            struct_member(vec![
                ("kind", literal_of("a")),
                ("variant", literal_of("y")),
            ]),
        ],
        "anyOf",
    );
    let block = Schema::union_with_mode(
        vec![nested, struct_member(vec![("kind", literal_of("b"))])],
        "oneOf",
    );
    let schema = struct_member(vec![("block", block)]);

    assert_eq!(
        schema
            .decode(obj(vec![(
                "block",
                obj(vec![("kind", text("a")), ("variant", text("x"))])
            )]))
            .unwrap(),
        obj(vec![(
            "block",
            obj(vec![("kind", text("a")), ("variant", text("x"))])
        )])
    );
    // No nested member accepts this block, so the outer member does not match
    // and the union has nothing to hand back.
    let contradicted = obj(vec![(
        "block",
        obj(vec![("kind", text("a")), ("variant", text("z"))]),
    )]);
    assert!(schema.decode(contradicted).is_err());
}

/// Schema.test.ts:4460 `mode: "oneOf" counts repeated member occurrences`: the
/// same member twice is still two matches, which is why the count cannot stop
/// at the first success.
#[test]
fn one_of_counts_the_same_member_twice() {
    let member = struct_member(vec![("kind", literal_of("a"))]);
    let schema = Schema::union_with_mode(vec![member.clone(), member], "oneOf");

    assert_eq!(
        member_error(&schema, obj(vec![("kind", text("a"))])),
        "expected exactly one member to match at $, got object"
    );
}

/// Schema.test.ts:4472 `mode: "oneOf" counts repeated literal occurrences`, the
/// same rule on the exact-value path.
#[test]
fn one_of_counts_the_same_literal_twice() {
    let schema = Schema::union_with_mode(vec![literal_of("a"), literal_of("a")], "oneOf");

    assert_eq!(
        member_error(&schema, text("a")),
        "expected exactly one member to match at $, got string"
    );
    assert!(schema.decode(text("b")).is_err(), "no member accepts it");
}

/// The one-of count is per decode, so a refusal does not make the schema
/// single-use.
#[test]
fn one_of_refuses_the_same_ambiguous_value_every_time() {
    let schema = Schema::union_with_mode(vec![literal_of("a"), literal_of("a")], "oneOf");

    assert!(schema.decode(text("a")).is_err());
    assert!(schema.decode(text("a")).is_err());
}
