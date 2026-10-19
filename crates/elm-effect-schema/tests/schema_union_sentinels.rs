//! Union dispatch narrows on sentinels before it decodes: a struct or tuple
//! member whose literal tag does not match the input is not tried at all.
//! Derives from Schema.test.ts:4793, :4807, :4821 and :4845.
//!
//!     cargo test -p elm-effect-schema --test schema_union_sentinels
//!
//! The observable rule is what the ledger row names: a union that sentinel
//! selection leaves with NO candidate reports the union (naming every member),
//! and one it leaves with exactly ONE candidate reports that member's own
//! failure instead.

use elm_effect_schema::{Field, Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn literal(value: &str) -> Schema<Unknown> {
    Schema::as_unknown(Schema::literal(text(value)))
}

fn struct_of(fields: Vec<(&str, Schema<Unknown>)>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_of(
        fields
            .into_iter()
            .map(|(name, schema)| (name.to_string(), schema))
            .collect(),
    ))
}

fn error(schema: &Schema<Unknown>, value: Unknown) -> String {
    schema
        .decode(value)
        .expect_err("this value matches no member")
        .to_string()
}

/// The field-list spelling of a struct (`struct_shape`) is discriminated too:
/// it records itself as a plain object, so the tag comes from its field list.
/// Both spellings of `Schema.Struct` have to narrow, or a union built from one
/// would silently claim every object.
#[test]
fn a_field_list_struct_narrows_by_its_tag() {
    let member = |tag: &str, payload: &str| {
        Schema::as_unknown(Schema::struct_shape(vec![
            Field::required("_tag", literal(tag)),
            Field::required(payload, string()),
        ]))
    };
    let schema = Schema::union_with_mode(vec![member("a", "a"), member("b", "b")], "anyOf");

    // The tag picks the member, so the missing payload is that member's
    // failure; without the field list the union could only report itself.
    assert_eq!(
        error(&schema, obj(vec![("_tag", text("a"))])),
        "expected required field at $.a, got missing"
    );
    assert!(schema.decode(obj(vec![("_tag", text("c"))])).is_err());
}

/// Schema.test.ts:4793 "string | struct": the struct's `_tag` sentinel excludes
/// it from `{}`, and the string member excludes itself by runtime type, so no
/// candidate is left and the refusal has to name both members.
#[test]
fn a_string_or_struct_union_names_both_members() {
    let schema = Schema::union_with_mode(
        vec![
            string(),
            struct_of(vec![("_tag", literal("a")), ("a", string())]),
        ],
        "anyOf",
    );

    assert_eq!(
        error(&schema, obj(Vec::new())),
        "expected string | { readonly \"_tag\": \"a\", readonly \"a\": string } at $, got object"
    );
    // A value the tag does select decodes through the member it selected.
    assert_eq!(
        schema
            .decode(obj(vec![("_tag", text("a")), ("a", text("payload"))]))
            .unwrap(),
        obj(vec![("_tag", text("a")), ("a", text("payload"))])
    );
    assert_eq!(schema.decode(text("plain")).unwrap(), text("plain"));
}

/// Schema.test.ts:4807 "string | tuple": the same rule with a positional
/// sentinel, which the tuple keeps at index 0.
#[test]
fn a_string_or_tuple_union_names_both_members() {
    let schema = Schema::union_with_mode(
        vec![
            string(),
            Schema::as_unknown(Schema::tuple_of(vec![literal("a"), string()])),
        ],
        "anyOf",
    );

    assert_eq!(
        error(&schema, Unknown::Array(Vec::new())),
        "expected string | readonly [\"a\", string] at $, got array"
    );
    assert_eq!(
        schema
            .decode(Unknown::Array(vec![text("a"), text("payload")]))
            .unwrap(),
        Unknown::Array(vec![text("a"), text("payload")])
    );
}

/// Schema.test.ts:4821 "tagged struct union": the tag picks the member, so a
/// `{_tag: "a"}` missing its payload is the member's failure and not the
/// union's; only an unknown tag leaves nothing selected.
#[test]
fn a_tagged_struct_union_reports_the_selected_member() {
    let schema = Schema::union_with_mode(
        vec![
            struct_of(vec![("_tag", literal("a")), ("a", string())]),
            struct_of(vec![("_tag", literal("b")), ("b", number())]),
        ],
        "anyOf",
    );

    assert_eq!(
        error(&schema, obj(vec![("_tag", text("a"))])),
        "expected a declared field at $.a, got absent"
    );
    assert_eq!(
        error(&schema, obj(vec![("_tag", text("b"))])),
        "expected a declared field at $.b, got absent"
    );
    assert_eq!(
        error(&schema, obj(vec![("_tag", text("c"))])),
        "expected { readonly \"_tag\": \"a\", readonly \"a\": string } | { readonly \"_tag\": \"b\", readonly \"b\": number } at $, got object"
    );
    // Each tag still resolves to its own member on a complete value.
    assert_eq!(
        schema
            .decode(obj(vec![("_tag", text("b")), ("b", Unknown::Number(2.0))]))
            .unwrap(),
        obj(vec![("_tag", text("b")), ("b", Unknown::Number(2.0))])
    );
}

/// Schema.test.ts:4845 "tagged tuple union": the same rule with the tag at
/// index 0, where the missing payload is the next position rather than a key.
#[test]
fn a_tagged_tuple_union_reports_the_selected_member() {
    let schema = Schema::union_with_mode(
        vec![
            Schema::as_unknown(Schema::tuple_of(vec![literal("a"), string()])),
            Schema::as_unknown(Schema::tuple_of(vec![literal("b"), number()])),
        ],
        "anyOf",
    );

    assert_eq!(
        error(&schema, Unknown::Array(vec![text("a")])),
        "expected required field at $[1], got missing"
    );
    assert_eq!(
        error(&schema, Unknown::Array(vec![text("b")])),
        "expected required field at $[1], got missing"
    );
    assert_eq!(
        error(&schema, Unknown::Array(vec![text("c")])),
        "expected readonly [\"a\", string] | readonly [\"b\", number] at $, got array"
    );
    assert_eq!(
        schema
            .decode(Unknown::Array(vec![text("a"), text("payload")]))
            .unwrap(),
        Unknown::Array(vec![text("a"), text("payload")])
    );
}
