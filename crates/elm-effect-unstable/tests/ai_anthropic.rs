//! `packages/effect/test/ai/AnthropicStructuredOutput.test.ts` witnesses.
//!
//! The pin's `toCodecAnthropic` is two layers: a codec transform (encoded-side
//! lowering of tuples to numeric-key objects, records to entry arrays,
//! optionals to required-nullable) and a JSON Schema rewrite to the Anthropic
//! subset. This crate ports the rewrite layer as
//! [`rewrite_anthropic_schema`](elm_effect_unstable::ai::rewrite_anthropic_schema)
//! plus the rows whose provider-side shape is already strict. Rows needing the
//! codec transform or schema-compiler derivation are not carried:
//! - tuple/record/optional lowering and every encoding/decoding roundtrip
//!   (TS:26, TS:323, TS:362, TS:395, TS:439, TS:511, TS:538, TS:600) need
//!   `InternalStructuredOutput.toCodec`, which has no counterpart here;
//! - non-string property names (TS:45) need property-name validation;
//! - Suspend resolution and recursive-reference rejection (TS:53, TS:57) need
//!   reference indirection plus `hasReferenceCycle`;
//! - Class `$defs` derivation (TS:474) and `Tool.EmptyParams` lowering
//!   (TS:500) need declaration/record-never lowering (the provider halves of
//!   both ARE carried below).
//!
//! Anthropic divergences from the OpenAI subset, covered here: `uri` is a
//! supported format, `allOf` is kept (never merged), `const` is kept, and no
//! empty `properties: {}` is added beside `additionalProperties: false`.

use elm_effect_unstable::ai::rewrite_anthropic_schema;
use serde_json::json;

#[test]
fn null_stays_null() {
    // AnthropicStructuredOutput.test.ts:17 (Undefined), :70 (Null).
    for doc in [json!({"type": "null"})] {
        assert_eq!(rewrite_anthropic_schema(&doc), doc);
    }
}

#[test]
fn keeps_scalar_shapes() {
    // AnthropicStructuredOutput.test.ts:77 (String + description), :162
    // (Finite + description), :202 (Int + description).
    for doc in [
        json!({"type": "string"}),
        json!({"type": "string", "description": "description"}),
        json!({"type": "number"}),
        json!({"type": "number", "description": "description"}),
        json!({"type": "integer"}),
        json!({"type": "integer", "description": "description"}),
    ] {
        assert_eq!(rewrite_anthropic_schema(&doc), doc);
    }
}

#[test]
fn bigint_literal_stays_a_string_enum() {
    // AnthropicStructuredOutput.test.ts:21.
    let doc = json!({"type": "string", "enum": ["1"]});
    assert_eq!(rewrite_anthropic_schema(&doc), doc);
}

#[test]
fn keeps_supported_string_formats_including_uri() {
    // AnthropicStructuredOutput.test.ts:87 — `date-time` survives. `uri` is
    // Anthropic-only (the OpenAI subset drops it).
    for format in ["date-time", "uri"] {
        let doc = json!({"type": "string", "format": format});
        assert_eq!(rewrite_anthropic_schema(&doc), doc);
    }
}

#[test]
fn drops_unsupported_format_into_description() {
    // AnthropicStructuredOutput.test.ts:94 (String), :179 (Finite), :219 (Int).
    assert_eq!(
        rewrite_anthropic_schema(&json!({"type": "string", "format": "int32"})),
        json!({"type": "string", "description": "a value with a format of int32"})
    );
    assert_eq!(
        rewrite_anthropic_schema(&json!({"type": "number", "format": "int32"})),
        json!({"type": "number", "description": "a value with a format of int32"})
    );
    assert_eq!(
        rewrite_anthropic_schema(&json!({"type": "integer", "format": "int32"})),
        json!({"type": "integer", "description": "a value with a format of int32"})
    );
}

#[test]
fn drops_string_length_bounds_but_keeps_description() {
    // AnthropicStructuredOutput.test.ts:101, :108.
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "string",
            "description": "a value with a length of at least 1",
            "minLength": 1
        })),
        json!({"type": "string", "description": "a value with a length of at least 1"})
    );
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "string",
            "description": "a value with a length of at least 1 and a value with a length of at most 2",
            "minLength": 1,
            "maxLength": 2
        })),
        json!({"type": "string", "description": "a value with a length of at least 1 and a value with a length of at most 2"})
    );
}

#[test]
fn keeps_a_compatible_pattern() {
    // AnthropicStructuredOutput.test.ts:115, :134, :142.
    let doc = json!({
        "type": "string",
        "description": "a string starting with \"a\"",
        "pattern": "^a"
    });
    assert_eq!(rewrite_anthropic_schema(&doc), doc);
}

#[test]
fn keeps_all_of_instead_of_merging_it() {
    // AnthropicStructuredOutput.test.ts:123 — the OpenAI rewrite would merge
    // the two patterns; Anthropic keeps `pattern` plus `allOf`.
    let doc = json!({
        "type": "string",
        "description": "a string starting with \"a\" and a string ending with \"b\"",
        "pattern": "^a",
        "allOf": [{"pattern": "b$"}]
    });
    assert_eq!(rewrite_anthropic_schema(&doc), doc);
}

#[test]
fn hoists_member_descriptions_and_drops_type_mismatched_patterns() {
    // `hoistAllOfDescriptions`: member descriptions join the parent,
    // `pattern` members vanish when the parent is not a string, and members
    // left empty disappear with the bounds the subset drops.
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "number",
            "description": "a value",
            "allOf": [
                {"pattern": "^a", "description": "a value greater than 1", "minimum": 1}
            ]
        })),
        json!({"type": "number", "description": "a value and a value greater than 1"})
    );
}

#[test]
fn hoists_member_format_and_title_when_absent() {
    // `hoistAllOfDescriptions`: a member `format`/`title` moves up only when
    // the parent has none; the remaining member stays in `allOf`.
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "string",
            "description": "a value",
            "allOf": [{"description": "more", "format": "date-time", "pattern": "^b"}]
        })),
        json!({
            "type": "string",
            "description": "a value and more",
            "format": "date-time",
            "allOf": [{"pattern": "^b"}]
        })
    );
}

#[test]
fn drops_numeric_bounds_but_keeps_description() {
    // AnthropicStructuredOutput.test.ts:186, :193 (Finite), :226, :233 (Int).
    // Anthropic has no minimum/maximum keywords at all, so bounds drop where
    // the OpenAI rewrite would merge them.
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "number",
            "description": "a value greater than 1",
            "minimum": 1
        })),
        json!({"type": "number", "description": "a value greater than 1"})
    );
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "number",
            "description": "a value greater than 1 and a value less than 2",
            "minimum": 1,
            "maximum": 2
        })),
        json!({"type": "number", "description": "a value greater than 1 and a value less than 2"})
    );
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "integer",
            "description": "a value greater than 1 and a value less than 2",
            "minimum": 1,
            "maximum": 2
        })),
        json!({"type": "integer", "description": "a value greater than 1 and a value less than 2"})
    );
}

#[test]
fn drops_array_bounds_but_keeps_description() {
    // AnthropicStructuredOutput.test.ts:298, :306.
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "a value with a length of at least 1 and a value with a length of at most 2",
            "minItems": 1,
            "maxItems": 2
        })),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "a value with a length of at least 1 and a value with a length of at most 2"
        })
    );
}

#[test]
fn keeps_literals_and_the_number_fallback() {
    // AnthropicStructuredOutput.test.ts:243, :250, :257 (Literal),
    // :152 (Number with the Infinity/NaN fallback).
    for doc in [
        json!({"type": "string", "enum": ["a"]}),
        json!({"type": "number", "enum": [1]}),
        json!({"type": "boolean", "enum": [true]}),
        json!({
            "anyOf": [
                {"type": "number"},
                {"type": "string", "enum": ["Infinity", "-Infinity", "NaN"]}
            ]
        }),
    ] {
        assert_eq!(rewrite_anthropic_schema(&doc), doc);
    }
}

#[test]
fn keeps_unions_as_any_of() {
    // AnthropicStructuredOutput.test.ts:266 (oneOf), :275 (NonEmptyString/Int).
    for doc in [
        json!({"anyOf": [{"type": "string"}, {"type": "number"}]}),
        json!({
            "anyOf": [
                {"type": "string", "description": "a value with a length of at least 1"},
                {"type": "integer"}
            ]
        }),
    ] {
        assert_eq!(rewrite_anthropic_schema(&doc), doc);
    }
}

#[test]
fn keeps_arrays() {
    // AnthropicStructuredOutput.test.ts:286.
    for doc in [
        json!({"type": "array", "items": {"type": "string"}}),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "description"
        }),
    ] {
        assert_eq!(rewrite_anthropic_schema(&doc), doc);
    }
}

#[test]
fn keeps_false_items_for_empty_tuples() {
    // AnthropicStructuredOutput.test.ts:316 — `items: false` is data, not a
    // schema, so the walk leaves it alone.
    let doc = json!({"type": "array", "items": false});
    assert_eq!(rewrite_anthropic_schema(&doc), doc);
}

#[test]
fn keeps_tuple_objects_verbatim() {
    // AnthropicStructuredOutput.test.ts:323, :347 (plain), :362 (optional
    // element), :395 (rest), :26 (post-rest `__tail_0__`). The tuple-to-object
    // lowering itself needs the codec transform; the provider shapes are
    // already strict so the rewrite is a fixpoint.
    let plain = json!({
        "type": "object",
        "description": "Tuple encoded as an object with numeric string keys ('0', '1', ...). If present, '__rest__' contains remaining elements",
        "properties": {"0": {"type": "string"}, "1": {"type": "number"}},
        "required": ["0", "1"],
        "additionalProperties": false
    });
    assert_eq!(rewrite_anthropic_schema(&plain), plain);
    let with_rest_and_tail = json!({
        "type": "object",
        "properties": {
            "__rest__": {"type": "array", "items": {"type": "string"}},
            "__tail_0__": {"type": "string"}
        },
        "required": ["__rest__", "__tail_0__"],
        "additionalProperties": false,
        "description": "Tuple encoded as an object with numeric string keys ('0', '1', ...). If present, '__rest__' contains remaining elements. Post-rest elements use '__tail_0__', '__tail_1__', and so on"
    });
    assert_eq!(
        rewrite_anthropic_schema(&with_rest_and_tail),
        with_rest_and_tail
    );
    let optional_element = json!({
        "type": "object",
        "description": "Tuple encoded as an object with numeric string keys ('0', '1', ...). If present, '__rest__' contains remaining elements",
        "properties": {
            "0": {"type": "string"},
            "1": {"anyOf": [{"type": "number"}, {"type": "null"}]}
        },
        "required": ["0", "1"],
        "additionalProperties": false
    });
    assert_eq!(
        rewrite_anthropic_schema(&optional_element),
        optional_element
    );
}

#[test]
fn keeps_struct_shapes_including_nullable_optionals() {
    // AnthropicStructuredOutput.test.ts:428 (required), :439 (optional as
    // required-nullable). The optional-to-nullable lowering needs the codec
    // transform; the provider shape is already strict.
    let required = json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "required": ["a"],
        "additionalProperties": false
    });
    assert_eq!(rewrite_anthropic_schema(&required), required);
    let optional = json!({
        "type": "object",
        "properties": {
            "a": {"type": "string"},
            "b": {"anyOf": [{"type": "number"}, {"type": "null"}]}
        },
        "required": ["a", "b"],
        "additionalProperties": false
    });
    assert_eq!(rewrite_anthropic_schema(&optional), optional);
}

#[test]
fn keeps_empty_params_without_adding_properties() {
    // AnthropicStructuredOutput.test.ts:500 — unlike the OpenAI rewrite, no
    // empty `properties: {}` is added beside `additionalProperties: false`.
    let doc = json!({"type": "object", "additionalProperties": false});
    let rewritten = rewrite_anthropic_schema(&doc);
    assert_eq!(rewritten, doc);
    assert!(
        !rewritten
            .as_object()
            .is_some_and(|map| map.contains_key("properties")),
        "must not add properties: {rewritten}"
    );
}

#[test]
fn keeps_record_entry_arrays() {
    // AnthropicStructuredOutput.test.ts:511, :580 (entry array),
    // :600/:625 (`isMinProperties` description, bounds dropped),
    // :538 (fixed literal plus index signature). The object-to-entry-array
    // lowering needs the codec transform; the provider shapes below are
    // already strict.
    let entry = json!({
        "type": "object",
        "description": "Tuple encoded as an object with numeric string keys ('0', '1', ...). If present, '__rest__' contains remaining elements",
        "properties": {"0": {"type": "string"}, "1": {"type": "number"}},
        "required": ["0", "1"],
        "additionalProperties": false
    });
    let record = json!({
        "type": "array",
        "description": "Object encoded as array of [key, value] pairs. Apply object constraints to the decoded object",
        "items": entry
    });
    assert_eq!(rewrite_anthropic_schema(&record), record);
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "array",
            "description": "Object encoded as array of [key, value] pairs. Apply object constraints to the decoded object; a value with at least 2 entries",
            "minProperties": 2,
            "items": entry
        })),
        json!({
            "type": "array",
            "description": "Object encoded as array of [key, value] pairs. Apply object constraints to the decoded object; a value with at least 2 entries",
            "items": entry
        })
    );
    let fixed = json!({
        "type": "array",
        "description": "Object encoded as array of [key, value] pairs. Apply object constraints to the decoded object",
        "items": {
            "type": "object",
            "description": "Tuple encoded as an object with numeric string keys ('0', '1', ...). If present, '__rest__' contains remaining elements",
            "properties": {
                "0": {"anyOf": [
                    {"type": "string", "enum": ["fixed"]},
                    {"type": "string"}
                ]},
                "1": {"type": "string"}
            },
            "required": ["0", "1"],
            "additionalProperties": false
        }
    });
    assert_eq!(rewrite_anthropic_schema(&fixed), fixed);
}

#[test]
fn keeps_definitions_by_rewriting_their_values() {
    // AnthropicStructuredOutput.test.ts:474 (Class) — the Class-to-`$defs`
    // lowering needs declaration machinery; the provider half keeps `$defs`
    // and rewrites each definition value.
    let doc = json!({
        "type": "object",
        "properties": {"name": {"type": "string"}},
        "required": ["name"],
        "additionalProperties": false,
        "$defs": {
            "PersonEncoded": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
                "additionalProperties": false
            }
        }
    });
    assert_eq!(rewrite_anthropic_schema(&doc), doc);
    assert_eq!(
        rewrite_anthropic_schema(&json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
            "additionalProperties": false,
            "$defs": {
                "PersonEncoded": {
                    "type": "object",
                    "properties": {"name": {"type": "string", "format": "int32"}},
                    "required": ["name"],
                    "additionalProperties": false
                }
            }
        })),
        json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
            "additionalProperties": false,
            "$defs": {
                "PersonEncoded": {
                    "type": "object",
                    "properties": {"name": {
                        "type": "string",
                        "description": "a value with a format of int32"
                    }},
                    "required": ["name"],
                    "additionalProperties": false
                }
            }
        })
    );
}
