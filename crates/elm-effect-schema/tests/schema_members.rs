//! Slice 8: union and exact-value derivations that rewrite the member list.
//!
//! The pin's `Union.mapMembers` (and its `Literals` twin) takes a member
//! transformer, and borrows the same helpers `Tuple.mapElements` uses. These
//! tests assert the derived *member list*, which is what the pin's rows
//! compare, so they read the members back through `Schema::members`.
//! Binds Schema.test.ts rows 9061-9094.

use elm_effect_schema::{Schema, Unknown, element_append, element_evolve, element_map};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
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

fn bool_schema() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

/// `Tuple.map(Schema.Array)` for a member list.
fn to_array(member: Schema<Unknown>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::array(member))
}

fn member_codes(schema: &Schema<Unknown>) -> Vec<String> {
    schema
        .members()
        .expect("a derived union is still a union shape")
        .iter()
        .map(|member| member.to_code())
        .collect()
}

fn codes(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|code| code.to_string()).collect()
}

fn three_member_union() -> Schema<Unknown> {
    Schema::union_with_mode(vec![str_schema(), number_schema(), bool_schema()], "anyOf")
}

/// `Schema.Struct({ "_tag": <the member's own literal>, "<key>": <rest> })`, the
/// shape the pin's `Literals.mapMembers` row builds. Reading the tag off the
/// member's `literal_value` is what proves the derivation handed the member in
/// its position rather than some rebuilt copy.
fn tagged_struct(member: Schema<Unknown>, key: &str, rest: Schema<Unknown>) -> Schema<Unknown> {
    let tag = member
        .literal_value()
        .cloned()
        .expect("a literals member is a Literal");
    let fields: Vec<(String, Schema<Unknown>)> = vec![
        ("_tag".to_string(), Schema::literal(tag)),
        (key.to_string(), rest),
    ];
    Schema::as_unknown(Schema::struct_of(fields))
}

fn tag_as_a(member: Schema<Unknown>) -> Schema<Unknown> {
    tagged_struct(member, "a", str_schema())
}

fn tag_as_b(member: Schema<Unknown>) -> Schema<Unknown> {
    tagged_struct(member, "b", number_schema())
}

fn tag_as_c(member: Schema<Unknown>) -> Schema<Unknown> {
    tagged_struct(member, "c", bool_schema())
}

// --------------------------------------------------------- Union.mapMembers ---

/// Schema.test.ts:9061 `appendElement`: the appended member joins the list.
#[test]
fn union_map_members_appends_a_member() {
    let schema = Schema::union_with_mode(vec![str_schema(), number_schema()], "anyOf")
        .map_members(element_append(bool_schema()));

    assert_eq!(
        member_codes(&schema),
        codes(&["Schema.String", "Schema.Number", "Schema.Boolean"])
    );
}

/// The rebuild is what wires the new member into the decoder: a boolean is a
/// value the original pair refused.
#[test]
fn union_map_members_widens_the_decoder() {
    let schema = Schema::union_with_mode(vec![str_schema(), number_schema()], "anyOf")
        .map_members(element_append(bool_schema()));

    assert_eq!(
        schema.decode(Unknown::Bool(true)).unwrap(),
        Unknown::Bool(true)
    );
    assert!(schema.decode(text("still a member")).is_ok());
    assert!(
        Schema::union_with_mode(vec![str_schema(), number_schema()], "anyOf")
            .decode(Unknown::Bool(true))
            .is_err()
    );
}

/// Schema.test.ts:9067 `evolve`: two of three members rewrapped, the middle one
/// left alone by its `undefined` entry.
#[test]
fn union_map_members_evolves_two_members() {
    let schema = three_member_union().map_members(element_evolve(vec![
        Some(to_array),
        None,
        Some(to_array),
    ]));

    assert_eq!(
        member_codes(&schema),
        codes(&[
            "Schema.Array(Schema.String)",
            "Schema.Number",
            "Schema.Array(Schema.Boolean)"
        ])
    );
}

/// Schema.test.ts:9083 `Array`: every member through one function.
#[test]
fn union_map_members_maps_every_member() {
    let schema = Schema::union_with_mode(vec![str_schema(), number_schema()], "anyOf")
        .map_members(element_map(to_array));

    assert_eq!(
        member_codes(&schema),
        codes(&["Schema.Array(Schema.String)", "Schema.Array(Schema.Number)"])
    );
    assert!(schema.decode(Unknown::Array(vec![text("a")])).is_ok());
    assert!(
        schema.decode(text("a")).is_err(),
        "the bare string is no longer a member"
    );
}

/// Schema.test.ts:9049's half that does not need `flip`: the union's mode is a
/// property of the union, so a derivation preserves it. Both `mapMembers` rows
/// above use the default mode, where losing `oneOf` would be invisible.
#[test]
fn union_map_members_keeps_the_one_of_mode() {
    let schema = Schema::union_with_mode(vec![str_schema(), number_schema()], "oneOf")
        .map_members(element_append(bool_schema()));

    assert_eq!(
        schema.to_code(),
        "Schema.Union([Schema.String, Schema.Number, Schema.Boolean], { mode: \"oneOf\" })"
    );
}

// ------------------------------------------------------ Literals.mapMembers ---

/// Schema.test.ts:9094 `evolve`: an exact-value shape's members are one
/// `Literal` per value, and the derivation returns a plain union of whatever
/// the transforms produce. The three tags come back `"a"`, `"b"`, `"c"`, in
/// position.
#[test]
fn literals_map_members_becomes_a_union_of_the_transformed_members() {
    let schema =
        Schema::literals(vec![text("a"), text("b"), text("c")]).map_members(element_evolve(vec![
            Some(tag_as_a),
            Some(tag_as_b),
            Some(tag_as_c),
        ]));

    assert_eq!(
        member_codes(&schema),
        codes(&[
            r#"Schema.Struct({ "_tag": Schema.Literal("a"), "a": Schema.String })"#,
            r#"Schema.Struct({ "_tag": Schema.Literal("b"), "b": Schema.Number })"#,
            r#"Schema.Struct({ "_tag": Schema.Literal("c"), "c": Schema.Boolean })"#,
        ])
    );
}

/// The member list of an exact-value shape is derived, not stored: one `Literal`
/// per value, which is what the derivation above consumes.
#[test]
fn literals_expose_one_literal_member_per_value() {
    let members = Schema::literals(vec![text("a"), text("b")])
        .members()
        .expect("an exact-value shape has members");

    assert_eq!(
        members
            .iter()
            .map(|member| member.to_code())
            .collect::<Vec<_>>(),
        codes(&["Schema.Literal(\"a\")", "Schema.Literal(\"b\")"])
    );
    assert_eq!(
        members[0].literal_value().map(|value| value.clone()),
        Some(text("a"))
    );
}

// ----------------------------------------------------------------- guards ---

/// `mapMembers` is a union derivation, so a shape with no member list refuses
/// rather than inventing one.
#[test]
#[should_panic(expected = "mapMembers needs a union")]
fn map_members_refuses_a_shape_without_members() {
    let _ = str_schema().map_members(element_append(bool_schema()));
}
