//! `packages/effect/test/ai/OpenAiStructuredOutput.test.ts` witnesses (57 rows).
//!
//! Carried (40): root-shape validation through
//! [`validate_openai_root`](elm_effect_unstable::ai::validate_openai_root) and
//! provider-shape coverage through
//! [`rewrite_openai_schema`](elm_effect_unstable::ai::rewrite_openai_schema).
//! Each test cites its TS line; rewrite inputs mirror the pin compiler's
//! intermediate document for that row.
//!
//! Excused (17), all needing schema-compiler derivation this crate does not
//! have: TS:63 post-rest elements (tuple-with-rest to object lowering plus
//! entry codec), TS:84 symbol property keys (serde_json objects are
//! string-keyed by construction, so the pin's compiler error is
//! unrepresentable), TS:94 TS:139 recursive schemas (`$defs`/`$ref` emission
//! and preservation; the rewrite drops `$defs`), TS:493 TS:517 TS:532 TS:565
//! tuple-to-object lowering plus entry codecs, TS:598 TS:609 struct closing
//! (`additionalProperties: false`) plus optional-null codecs, TS:644 Class
//! `$defs` derivation, TS:670 TS:683 TS:716 TS:760 TS:780 TS:807
//! Record-to-entry-array lowering plus size-bound transport.

use elm_effect_schema::{Schema, Unknown};
use elm_effect_unstable::ai::{rewrite_openai_schema, validate_openai_root};
use serde_json::json;

const ROOT_MESSAGE: &str =
    "OpenAiStructuredOutput: Root JSON Schema must have type \"object\" and must not use \"anyOf\"";

#[test]
fn rejects_primitive_roots() {
    // OpenAiStructuredOutput.test.ts:29 — String derives `{type: string}`.
    assert_eq!(
        validate_openai_root(&json!({"type": "string"})),
        Err(ROOT_MESSAGE.to_owned())
    );
}

#[test]
fn rejects_array_roots() {
    // OpenAiStructuredOutput.test.ts:33 — Array(String) is not an object root.
    assert_eq!(
        validate_openai_root(&json!({"type": "array", "items": {"type": "string"}})),
        Err(ROOT_MESSAGE.to_owned())
    );
}

#[test]
fn rejects_anyof_roots() {
    // OpenAiStructuredOutput.test.ts:37 — a union root uses `anyOf`.
    assert_eq!(
        validate_openai_root(&json!({
            "anyOf": [
                {"type": "object", "properties": {"a": {"type": "string"}}},
                {"type": "object", "properties": {"b": {"type": "string"}}}
            ]
        })),
        Err(ROOT_MESSAGE.to_owned())
    );
}

#[test]
fn rejects_unconstrained_roots() {
    // OpenAiStructuredOutput.test.ts:47 — Any/Unknown derive `{}`, Never
    // derives `{not: {}}`; none is an object root. A strict root passes.
    for doc in [json!({}), json!({"not": {}})] {
        assert_eq!(validate_openai_root(&doc), Err(ROOT_MESSAGE.to_owned()));
    }
    assert_eq!(
        validate_openai_root(&json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false
        })),
        Ok(())
    );
}

#[test]
fn keeps_null_and_undefined_shapes() {
    // OpenAiStructuredOutput.test.ts:54 (Undefined lowers to null) and :201
    // (Null): both provider shapes are already strict.
    for doc in [json!({"type": "null"})] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn keeps_bigint_literal_shape() {
    // OpenAiStructuredOutput.test.ts:58 — `Literal(1n)` lowers to a string
    // enum; `enum` is in the strict subset so the rewrite keeps it.
    let doc = json!({"type": "string", "enum": ["1"]});
    assert_eq!(rewrite_openai_schema(&doc), doc);
}

#[test]
fn keeps_string_shapes() {
    // OpenAiStructuredOutput.test.ts:208 (String, with and without a
    // description), :218 (supported `date-time` format), :246 (startsWith keeps
    // its pattern), :262 (pattern keeps its pattern): already strict.
    for doc in [
        json!({"type": "string"}),
        json!({"type": "string", "description": "description"}),
        json!({"type": "string", "format": "date-time"}),
        json!({
            "type": "string",
            "description": "a string starting with \"a\"",
            "pattern": "^a"
        }),
        json!({
            "type": "string",
            "description": "a string matching the RegExp ^a",
            "pattern": "^a"
        }),
    ] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn moves_unsupported_string_format_to_description() {
    // OpenAiStructuredOutput.test.ts:225 — `int32` is outside the subset.
    assert_eq!(
        rewrite_openai_schema(&json!({"type": "string", "format": "int32"})),
        json!({"type": "string", "description": "a value with a format of int32"})
    );
}

#[test]
fn drops_string_length_bounds_but_keeps_description() {
    // OpenAiStructuredOutput.test.ts:232 (isMinLength) and :239 (isMinLength
    // plus isMaxLength): length keywords are outside the subset.
    assert_eq!(
        rewrite_openai_schema(&json!({
            "type": "string",
            "description": "a value with a length of at least 1",
            "minLength": 1
        })),
        json!({"type": "string", "description": "a value with a length of at least 1"})
    );
    assert_eq!(
        rewrite_openai_schema(&json!({
            "type": "string",
            "description": "a value with a length of at least 1 and a value with a length of at most 2",
            "minLength": 1,
            "maxLength": 2
        })),
        json!({
            "type": "string",
            "description": "a value with a length of at least 1 and a value with a length of at most 2"
        })
    );
}

#[test]
fn merges_combined_string_patterns() {
    // OpenAiStructuredOutput.test.ts:254 — startsWith plus endsWith arrive as
    // `allOf` members and merge into one lookahead pattern.
    assert_eq!(
        rewrite_openai_schema(&json!({
            "type": "string",
            "allOf": [
                {"description": "a string starting with \"a\"", "pattern": "^a"},
                {"description": "a string ending with \"b\"", "pattern": "b$"}
            ]
        })),
        json!({
            "type": "string",
            "description": "a string starting with \"a\" and a string ending with \"b\"",
            "pattern": "^(?=[\\s\\S]*?(?:^a))(?=[\\s\\S]*?(?:b$))"
        })
    );
}

#[test]
fn keeps_number_union_shape() {
    // OpenAiStructuredOutput.test.ts:272 — Number is a union with its
    // non-finite string names; both branches are already strict.
    let doc = json!({
        "anyOf": [
            {"type": "number"},
            {"type": "string", "enum": ["Infinity", "-Infinity", "NaN"]}
        ]
    });
    assert_eq!(rewrite_openai_schema(&doc), doc);
}

#[test]
fn keeps_finite_shapes() {
    // OpenAiStructuredOutput.test.ts:282 (Finite, with and without a
    // description), :306 (isGreaterThan), :314 (isGreaterThanOrEqualTo), :322
    // (isLessThan), :330 (isLessThanOrEqualTo), :338 (isBetween), :347
    // (isMultipleOf), :355 (isGreaterThan plus isLessThan): numeric bound
    // keywords are in the strict subset.
    for doc in [
        json!({"type": "number"}),
        json!({"type": "number", "description": "description"}),
        json!({
            "type": "number",
            "description": "a value greater than 1",
            "exclusiveMinimum": 1
        }),
        json!({
            "type": "number",
            "description": "a value greater than or equal to 1",
            "minimum": 1
        }),
        json!({
            "type": "number",
            "description": "a value less than 2",
            "exclusiveMaximum": 2
        }),
        json!({
            "type": "number",
            "description": "a value less than or equal to 2",
            "maximum": 2
        }),
        json!({
            "type": "number",
            "description": "a value between 1 and 2",
            "minimum": 1,
            "maximum": 2
        }),
        json!({
            "type": "number",
            "description": "a value that is a multiple of 2",
            "multipleOf": 2
        }),
        json!({
            "type": "number",
            "description": "a value greater than 1 and a value less than 2",
            "exclusiveMinimum": 1,
            "exclusiveMaximum": 2
        }),
    ] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn moves_unsupported_number_format_to_description() {
    // OpenAiStructuredOutput.test.ts:292 (`duration` on a number) and :299
    // (`int32` on a number): `format` only survives on strings.
    for format in ["duration", "int32"] {
        assert_eq!(
            rewrite_openai_schema(&json!({"type": "number", "format": format})),
            json!({"type": "number", "description": format!("a value with a format of {format}")})
        );
    }
}

#[test]
fn keeps_int_shapes() {
    // OpenAiStructuredOutput.test.ts:366 (Int, with and without a
    // description), :390 (isGreaterThan), :398 (isGreaterThan plus isLessThan).
    for doc in [
        json!({"type": "integer"}),
        json!({"type": "integer", "description": "description"}),
        json!({
            "type": "integer",
            "description": "a value greater than 1",
            "exclusiveMinimum": 1
        }),
        json!({
            "type": "integer",
            "description": "a value greater than 1 and a value less than 2",
            "exclusiveMinimum": 1,
            "exclusiveMaximum": 2
        }),
    ] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn moves_unsupported_int_format_to_description() {
    // OpenAiStructuredOutput.test.ts:376 (`duration` on an integer) and :383
    // (`int32` on an integer).
    for format in ["duration", "int32"] {
        assert_eq!(
            rewrite_openai_schema(&json!({"type": "integer", "format": format})),
            json!({"type": "integer", "description": format!("a value with a format of {format}")})
        );
    }
}

#[test]
fn keeps_literal_shapes() {
    // OpenAiStructuredOutput.test.ts:410 (string), :417 (number), :424
    // (boolean): `enum` is in the strict subset.
    for doc in [
        json!({"type": "string", "enum": ["a"]}),
        json!({"type": "number", "enum": [1]}),
        json!({"type": "boolean", "enum": [true]}),
    ] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn keeps_union_shapes() {
    // OpenAiStructuredOutput.test.ts:433 (oneOf) and :442
    // (Union(NonEmptyString, Int), whose first branch carries the length
    // description): `anyOf` branches rewrite bottom-up and survive.
    for doc in [
        json!({"anyOf": [{"type": "string"}, {"type": "number"}]}),
        json!({
            "anyOf": [
                {"type": "string", "description": "a value with a length of at least 1"},
                {"type": "integer"}
            ]
        }),
        json!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [
                        {"type": "string", "description": "a value with a length of at least 1"},
                        {"type": "integer"}
                    ]
                }
            },
            "required": ["value"],
            "additionalProperties": false
        }),
    ] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn keeps_array_shapes() {
    // OpenAiStructuredOutput.test.ts:453 (Array(String), with and without a
    // description), :465 (plus isMinLength), :474 (plus isMinLength and
    // isMaxLength): array bound keywords are in the strict subset.
    for doc in [
        json!({"type": "array", "items": {"type": "string"}}),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "description"
        }),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "a value with a length of at least 1",
            "minItems": 1
        }),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "a value with a length of at least 1 and a value with a length of at most 2",
            "minItems": 1,
            "maxItems": 2
        }),
    ] {
        assert_eq!(rewrite_openai_schema(&doc), doc);
    }
}

#[test]
fn keeps_empty_tuple_shape() {
    // OpenAiStructuredOutput.test.ts:486 — `Tuple([])` derives an array closed
    // with `items: false`; the rewrite keeps non-object `items` verbatim.
    let doc = json!({"type": "array", "items": false});
    assert_eq!(rewrite_openai_schema(&doc), doc);
}

#[test]
fn checked_codecs_still_enforce_provider_bounds() {
    // Spot-checks for :232 (isMinLength) and :306 (isGreaterThan): the checked
    // codec refuses what the provider-facing description describes.
    let at_least_two = Schema::checked(
        Schema::json(),
        "isMinLength",
        vec!["2"],
        vec![("description", "a value with a length of at least 2")],
    );
    assert!(at_least_two.decode(Unknown::Text("a".to_owned())).is_err());
    assert!(at_least_two.decode(Unknown::Text("ab".to_owned())).is_ok());
    let greater_than_one = Schema::checked(
        Schema::json(),
        "isGreaterThan",
        vec!["1"],
        vec![("description", "a value greater than 1")],
    );
    assert!(greater_than_one.decode(Unknown::Number(1.0)).is_err());
    assert!(greater_than_one.decode(Unknown::Number(2.0)).is_ok());
}
