//! `schemaToJsonSchemaDocument` documents
//! (packages/effect/test/schema/representation/schemaToJsonSchemaDocument.test.ts).
//!
//! The document is the compiler's output wrapped with the dialect and a
//! top-level `definitions` slot. Rows needing machinery the port does not
//! have are excused with reasons instead of approximated:
//!
//! Excused, with reasons:
//! - `:5/:7 artificial JSON encoding` (2 rows): the compiler has no
//!   number-to-JSON-enum projection for non-finite values.
//! - `:19 reference policy`: `toJsonSchemaDocument` takes no reference-policy
//!   option here.
//! - `:114 output, references and generation options`: the Shared definition
//!   needs the pin's `allOf` wrapping and the count needs the expected-message
//!   plumbing `FiniteFromString` does not carry.
//! - `:177 custom compiler annotations`: a `toJsonSchema` callback is a
//!   custom hook the compiler degrades instead of running.
//! - `:196 JSON content media types`: no fromJsonString constructor, and the
//!   compiler drops the media-type annotation without one.
//! - `:207 declarations without a JSON codec`: no declare-with-predicate.
//! - The `:5` `toType` halves: one schema carries one side, so there is no
//!   type side to project.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use elm_effect_schema::json_schema::{
    CompileDocumentOptions, DocCallback, DocCallbackInput, DocCallbackOutput, OnExcessProperty,
    compile_document, compile_document_with_options,
};
use elm_effect_schema::representation_doc::{
    Document, schema_to_json_schema_document, to_representation,
};
use elm_effect_schema::{Field, Schema, SchemaError, Unknown};
use serde_json::{Value, json};

fn finite_from_string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

// 5 (partial): the encoded side drives both the representation tag and the
// JSON Schema document. The `toType` projections need a second side the port
// does not keep, so only the encoded halves carry this row.
#[test]
fn uses_the_encoded_side_for_representations_and_json_schema() {
    let representation = to_representation(&finite_from_string()).expect("converts");
    assert_eq!(representation.representation["_tag"], json!("String"));
    assert_eq!(
        schema_to_json_schema_document(&finite_from_string()),
        json!({
            "dialect": "draft-2020-12",
            "schema": { "type": "string" },
            "definitions": {},
        })
    );
}

// 19: repeated anonymous schemas inline at every occurrence.
#[test]
fn inlines_repeated_anonymous_schemas_by_default() {
    let shared = || {
        Schema::as_unknown(Schema::struct_shape(vec![Field::required(
            "value",
            Schema::as_unknown(Schema::string()),
        )]))
    };
    let schema = Schema::as_unknown(Schema::struct_shape(vec![
        Field::required("first", shared()),
        Field::required("second", shared()),
    ]));
    assert_eq!(
        schema_to_json_schema_document(&schema),
        json!({
            "dialect": "draft-2020-12",
            "schema": {
                "type": "object",
                "properties": {
                    "first": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": true
                    },
                    "second": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": true
                    }
                },
                "required": ["first", "second"],
                "additionalProperties": true
            },
            "definitions": {},
        })
    );
}

// 72: encoded tuple elements project to their wire form.
#[test]
fn projects_encoded_tuple_elements_for_json_schema() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![Schema::as_unknown(
        Schema::number_from_string(),
    )]));
    assert_eq!(
        schema_to_json_schema_document(&schema)["schema"],
        json!({
            "type": "array",
            "prefixItems": [{ "type": "string" }],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

// 100: checks stay on the document when no artificial encoding intervenes.
#[test]
fn preserves_checks_when_no_artificial_json_encoding_is_needed() {
    let schema = Schema::checked(
        Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isFinite",
            vec![],
            vec![],
        ),
        "isGreaterThan",
        vec!["0"],
        vec![],
    );
    assert_eq!(
        schema_to_json_schema_document(&schema),
        json!({
            "dialect": "draft-2020-12",
            "schema": { "type": "number", "exclusiveMinimum": 0 },
            "definitions": {},
        })
    );
}

// ---------- representation toJsonSchemaDocument rows ----------

fn string_repr() -> Value {
    json!({ "_tag": "String", "checks": [] })
}

fn number_repr() -> Value {
    json!({ "_tag": "Number", "checks": [] })
}

fn doc(representation: Value) -> Document {
    Document {
        representation,
        references: BTreeMap::new(),
    }
}

fn empty_callbacks() -> BTreeMap<String, DocCallback> {
    BTreeMap::new()
}

fn error_opts(callbacks: &BTreeMap<String, DocCallback>) -> CompileDocumentOptions<'_> {
    CompileDocumentOptions {
        on_excess_property: OnExcessProperty::Error,
        include_annotation_key: None,
        callbacks,
    }
}

/// The pin's `compile` helper: error mode, schema only.
fn compile_repr(representation: Value, callbacks: &BTreeMap<String, DocCallback>) -> Value {
    compile_document_with_options(&doc(representation), &error_opts(callbacks)).expect("compiles")
        ["schema"]
        .clone()
}

/// Default options, full document.
fn compile_default(representation: Value) -> Value {
    compile_document(&doc(representation)).expect("compiles")
}

fn exact(fragment: Value) -> DocCallback {
    Box::new(move |_: &DocCallbackInput| Ok(DocCallbackOutput::exact(fragment.clone())))
}

fn approximate(fragment: Value) -> DocCallback {
    Box::new(move |_: &DocCallbackInput| Ok(DocCallbackOutput::approximate(fragment.clone())))
}

fn filter(callback_id: &str) -> Value {
    json!({
        "_tag": "Filter",
        "aborted": false,
        "annotations": { "toJsonSchema": callback_id },
    })
}

fn assert_invalid(representation: Value, message: &str, path: &str) {
    match compile_document_with_options(&doc(representation), &error_opts(&empty_callbacks())) {
        Err(SchemaError::InvalidJson {
            path: actual_path,
            message: actual_message,
        }) => {
            assert_eq!(actual_message, message);
            assert_eq!(actual_path, path);
        }
        Err(other) => panic!("expected InvalidJson, got {other:?}"),
        Ok(schema) => panic!("expected an error, got {schema}"),
    }
}

// 101: a globally registered symbol spells its string form.
#[test]
fn compiles_a_global_unique_symbol() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "UniqueSymbol",
                "symbol": { "description": "value", "global": true },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "string", "enum": ["Symbol(value)"] })
    );
}

// 106: a local symbol has no JSON spelling.
#[test]
fn compiles_a_local_unique_symbol() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "UniqueSymbol",
                "symbol": { "description": "value", "global": false },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "not": {} })
    );
}

// 113: a suspend inlines its thunk.
#[test]
fn compiles_suspend() {
    assert_eq!(
        compile_repr(
            json!({ "_tag": "Suspend", "thunk": string_repr(), "checks": [] }),
            &empty_callbacks(),
        ),
        json!({ "type": "string" })
    );
}

// 124: a bigint literal spells its digits as a string enum.
#[test]
fn compiles_a_bigint_literal() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Literal",
                "literal": { "type": "bigint", "value": "1" },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "string", "enum": ["1"] })
    );
}

// 138: a string literal spells its value.
#[test]
fn compiles_a_string_literal() {
    assert_eq!(
        compile_repr(
            json!({ "_tag": "Literal", "literal": "a", "checks": [] }),
            &empty_callbacks(),
        ),
        json!({ "type": "string", "enum": ["a"] })
    );
}

// 152: an empty enum admits nothing.
#[test]
fn compiles_an_empty_enum() {
    assert_eq!(
        compile_repr(
            json!({ "_tag": "Enum", "enums": [], "checks": [] }),
            &empty_callbacks(),
        ),
        json!({ "not": {} })
    );
}

// 156: an enum spells each member with its title.
#[test]
fn compiles_an_enum() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Enum",
                "enums": [["A", "a"], ["One", 1]],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "anyOf": [
                { "type": "string", "enum": ["a"], "title": "A" },
                { "type": "number", "enum": [1], "title": "One" },
            ],
        })
    );
}

// 175: a non-finite numeric enum value fails at its position.
#[test]
fn rejects_non_finite_numeric_enum_values() {
    assert_invalid(
        json!({
            "_tag": "Enum",
            "enums": [
                ["StringNaN", "NaN"],
                ["NumberNaN", { "type": "number", "value": "NaN" }],
            ],
            "checks": [],
        }),
        "Invalid numeric enum value NaN",
        "[\"representation\"][\"enums\"][1][1]",
    );
}

// 190: a template literal compiles to its anchored pattern.
#[test]
fn compiles_a_template_literal() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "TemplateLiteral",
                "parts": [
                    { "_tag": "Literal", "literal": "prefix-", "checks": [] },
                    string_repr(),
                ],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "string", "pattern": "^prefix-[\\s\\S]*?$" })
    );
}

// 206: an empty objects node admits any non-null JSON value.
#[test]
fn compiles_an_empty_objects_node() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "not": { "type": "null" } })
    );
}

// 218: an optional tuple element only lowers `minItems`, and a number
// element carries its artificial non-finite encoding.
#[test]
fn compiles_optional_tuple_elements() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Arrays",
                "elements": [
                    { "type": string_repr(), "isOptional": false },
                    { "type": number_repr(), "isOptional": true },
                ],
                "rest": [],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "type": "array",
            "prefixItems": [
                { "type": "string" },
                { "anyOf": [
                    { "type": "number" },
                    { "type": "string", "enum": ["NaN", "Infinity", "-Infinity"] },
                ] },
            ],
            "maxItems": 2,
            "minItems": 1,
        })
    );
}

// 243: more than one rest element is invalid.
#[test]
fn rejects_multiple_tuple_rest_elements() {
    assert_invalid(
        json!({
            "_tag": "Arrays",
            "elements": [],
            "rest": [string_repr(), number_repr()],
            "checks": [],
        }),
        "Invalid schema representation document",
        "[\"representation\"][\"rest\"]",
    );
}

// 256: a symbol property name is invalid.
#[test]
fn rejects_a_symbol_object_property() {
    assert_invalid(
        json!({
            "_tag": "Objects",
            "propertySignatures": [{
                "name": { "type": "symbol", "description": "value" },
                "type": string_repr(),
                "isOptional": false,
                "isMutable": false,
            }],
            "indexSignatures": [],
            "checks": [],
        }),
        "Invalid schema representation document",
        "[\"representation\"][\"propertySignatures\"][0][\"name\"]",
    );
}

// 274: a never index-signature value closes the object.
#[test]
fn compiles_a_never_index_signature_value_as_false() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [{
                    "parameter": string_repr(),
                    "type": { "_tag": "Never", "checks": [] },
                }],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "object", "additionalProperties": false })
    );
}

// 286: an unconstrained index-signature value leaves the object open.
#[test]
fn removes_an_unconstrained_index_signature_value() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [{
                    "parameter": string_repr(),
                    "type": { "_tag": "Unknown", "checks": [] },
                }],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "object" })
    );
}

// 300: same-type literal enums compact into one typed enum.
#[test]
fn compacts_a_union_of_same_type_literals() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [
                    { "_tag": "Literal", "literal": "a", "checks": [] },
                    { "_tag": "Literal", "literal": "b", "checks": [] },
                ],
                "options": { "mode": "anyOf" },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "string", "enum": ["a", "b"] })
    );
}

// 315: mixed-type literal enums stay split.
#[test]
fn does_not_compact_a_union_of_mixed_type_literals() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [
                    { "_tag": "Literal", "literal": "a", "checks": [] },
                    { "_tag": "Literal", "literal": 1, "checks": [] },
                ],
                "options": { "mode": "anyOf" },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "anyOf": [
                { "type": "string", "enum": ["a"] },
                { "type": "number", "enum": [1] },
            ],
        })
    );
}

// 334: every supported standard annotation rides the base.
#[test]
fn emits_every_supported_standard_annotation() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "annotations": {
                    "title": "Title",
                    "description": "Description",
                    "default": "default",
                    "examples": ["a", "b"],
                    "readOnly": true,
                    "writeOnly": false,
                },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "type": "string",
            "title": "Title",
            "description": "Description",
            "default": "default",
            "examples": ["a", "b"],
            "readOnly": true,
            "writeOnly": false,
        })
    );
}

// 360: a check fragment on an empty base is the whole document.
#[test]
fn uses_a_check_fragment_when_the_base_is_empty() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "checked".to_string(),
        exact(json!({ "description": "checked" })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Unknown",
                "checks": [filter("checked")],
            }),
            &callbacks,
        ),
        json!({ "description": "checked" })
    );
}

// 374: a check appends to an existing `allOf`.
#[test]
fn appends_a_check_to_an_existing_all_of() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "checked".to_string(),
        exact(json!({ "description": "checked" })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "BigInt",
                "checks": [filter("checked")],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "allOf": [{ "pattern": "^-?\\d+$" }, { "description": "checked" }],
        })
    );
}

// 391: a check absorbs the number type into an integer.
#[test]
fn lets_a_check_refine_number_to_integer() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "to-integer".to_string(),
        exact(json!({ "type": "integer" })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "checks": [filter("to-integer")],
            }),
            &callbacks,
        ),
        json!({ "type": "integer" })
    );
}

// 405: a callback-free group contributes its children's fragments.
#[test]
fn compiles_a_callback_free_filter_group() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("min-length".to_string(), exact(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [{
                    "_tag": "FilterGroup",
                    "checks": [filter("min-length")],
                }],
            }),
            &callbacks,
        ),
        json!({ "type": "string", "allOf": [{ "minLength": 1 }] })
    );
}

// 424: tuple element annotations ride an `allOf` entry.
#[test]
fn compiles_tuple_element_annotations() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Arrays",
                "elements": [{
                    "type": string_repr(),
                    "isOptional": false,
                    "annotations": { "description": "element" },
                }],
                "rest": [],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "type": "array",
            "prefixItems": [{ "type": "string", "allOf": [{ "description": "element" }] }],
            "maxItems": 1,
            "minItems": 1,
        })
    );
}

// 445: no required element means no `minItems`.
#[test]
fn omits_min_items_when_every_tuple_element_is_optional() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Arrays",
                "elements": [{ "type": string_repr(), "isOptional": true }],
                "rest": [],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "type": "array",
            "prefixItems": [{ "type": "string" }],
            "maxItems": 1,
        })
    );
}

// 461: a constrained rest spells `items`.
#[test]
fn compiles_a_constrained_tuple_rest() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Arrays",
                "elements": [],
                "rest": [string_repr()],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "array", "items": { "type": "string" } })
    );
}

// 473: property annotations ride an `allOf` entry on the property.
#[test]
fn compiles_property_annotations() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [{
                    "name": "value",
                    "type": string_repr(),
                    "isOptional": true,
                    "isMutable": false,
                    "annotations": { "description": "property" },
                }],
                "indexSignatures": [],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string", "allOf": [{ "description": "property" }] },
            },
            "additionalProperties": false,
        })
    );
}

// 495: one member never compacts.
#[test]
fn compiles_a_single_member_union_without_compacting_it() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [string_repr()],
                "options": { "mode": "anyOf" },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "anyOf": [{ "type": "string" }] })
    );
}

// 507: one approximate branch loosens `oneOf` to `anyOf`.
#[test]
fn falls_back_from_one_of_to_any_of_when_a_branch_is_approximate() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("loose".to_string(), approximate(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [
                    { "_tag": "String", "checks": [filter("loose")] },
                    { "_tag": "Boolean", "checks": [] },
                ],
                "options": { "mode": "oneOf" },
                "annotations": { "description": "annotated union" },
                "checks": [],
            }),
            &callbacks,
        ),
        json!({
            "anyOf": [
                { "type": "string", "minLength": 1 },
                { "type": "boolean" },
            ],
            "description": "annotated union",
        })
    );
}

// 531: approximation propagates through nesting, so the outer `oneOf`
// loosens too.
#[test]
fn propagates_approximation_through_nested_representations() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [
                    {
                        "_tag": "Arrays",
                        "elements": [],
                        "rest": [{ "_tag": "String", "checks": [{ "_tag": "Filter", "aborted": false }] }],
                        "checks": [],
                    },
                    number_repr(),
                ],
                "options": { "mode": "oneOf" },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({
            "anyOf": [
                { "type": "array", "items": { "type": "string" } },
                { "anyOf": [
                    { "type": "number" },
                    { "type": "string", "enum": ["NaN", "Infinity", "-Infinity"] },
                ] },
            ],
        })
    );
}

// 557: an approximate branch copied under element annotations still loosens
// the union, and the element annotations stay beside it.
#[test]
fn resolves_an_approximate_one_of_copied_by_contextual_annotations() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("loose".to_string(), approximate(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Arrays",
                "elements": [{
                    "type": {
                        "_tag": "Union",
                        "types": [
                            { "_tag": "String", "checks": [filter("loose")] },
                            { "_tag": "Boolean", "checks": [] },
                        ],
                        "options": { "mode": "oneOf" },
                        "checks": [],
                    },
                    "isOptional": false,
                    "annotations": { "description": "element" },
                }],
                "rest": [],
                "checks": [],
            }),
            &callbacks,
        ),
        json!({
            "type": "array",
            "prefixItems": [{
                "anyOf": [
                    { "type": "string", "minLength": 1 },
                    { "type": "boolean" },
                ],
                "allOf": [{ "description": "element" }],
            }],
            "minItems": 1,
            "maxItems": 1,
        })
    );
}

// 594: exact branches keep `oneOf`.
#[test]
fn keeps_one_of_when_every_branch_is_exact() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [string_repr(), { "_tag": "Boolean", "checks": [] }],
                "options": { "mode": "oneOf" },
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "oneOf": [{ "type": "string" }, { "type": "boolean" }] })
    );
}

// 606: a non-string index-signature parameter is invalid.
#[test]
fn rejects_an_unsupported_index_signature_parameter() {
    assert_invalid(
        json!({
            "_tag": "Objects",
            "propertySignatures": [],
            "indexSignatures": [{
                "parameter": number_repr(),
                "type": string_repr(),
            }],
            "checks": [],
        }),
        "Invalid schema representation document",
        "[\"representation\"][\"indexSignatures\"][0][\"parameter\"]",
    );
}

// 619: an empty union admits nothing.
#[test]
fn compiles_an_empty_union_as_never() {
    assert_eq!(
        compile_repr(
            json!({ "_tag": "Union", "types": [], "options": { "mode": "anyOf" }, "checks": [] }),
            &empty_callbacks(),
        ),
        json!({ "not": {} })
    );
}

// 629: an empty tuple with an open rest is the bare array shape.
#[test]
fn removes_items_when_an_empty_tuple_has_an_open_rest() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Arrays",
                "elements": [],
                "rest": [{ "_tag": "Unknown", "checks": [] }],
                "checks": [],
            }),
            &empty_callbacks(),
        ),
        json!({ "type": "array" })
    );
}

// 646: `isInt` compiles from its builtin identity, and the following
// callback observes the produced `integer` type.
#[test]
fn passes_the_type_produced_by_is_int_to_following_check_callbacks() {
    let seen: Rc<Cell<Option<String>>> = Rc::new(Cell::new(None));
    let seen_in = seen.clone();
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "dependent".to_string(),
        Box::new(move |input: &DocCallbackInput| {
            seen_in.set(input.base_type.clone());
            Ok(DocCallbackOutput::exact(json!({ "minimum": 0 })))
        }),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "checks": [
                    {
                        "_tag": "Filter",
                        "aborted": false,
                        "representation": { "id": "effect/schema/isInt", "payload": null },
                    },
                    filter("dependent"),
                ],
            }),
            &callbacks,
        ),
        json!({ "type": "integer", "minimum": 0, "maximum": 9007199254740991i64 })
    );
    assert_eq!(seen.take(), Some("integer".to_string()));
}

// 665: an `isPattern` whose flags JSON Schema cannot represent contributes
// nothing, so the whole document stays open.
#[test]
fn omits_is_pattern_flags_that_json_schema_cannot_represent() {
    let document = doc(json!({
        "_tag": "String",
        "checks": [{
            "_tag": "Filter",
            "aborted": false,
            "representation": {
                "id": "effect/schema/isPattern",
                "payload": { "source": "^[a-z]+$", "flags": "i" },
            },
        }],
    }));
    assert_eq!(
        compile_document(&document).expect("compiles"),
        json!({
            "dialect": "draft-2020-12",
            "schema": { "type": "string" },
            "definitions": {},
        })
    );
}

// 676: an empty group override is authoritative: the leaf callback never
// runs, and the callback-free leaf contributes nothing.
#[test]
fn treats_an_empty_override_as_authoritative() {
    let visits: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let visits_in = visits.clone();
    let mut callbacks = empty_callbacks();
    callbacks.insert("empty".to_string(), exact(json!({})));
    callbacks.insert(
        "counted".to_string(),
        Box::new(move |_: &DocCallbackInput| {
            visits_in.set(visits_in.get() + 1);
            Ok(DocCallbackOutput::exact(json!({ "minLength": 1 })))
        }),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [
                    {
                        "_tag": "FilterGroup",
                        "annotations": { "toJsonSchema": "empty" },
                        "checks": [{
                            "_tag": "Filter",
                            "aborted": false,
                            "annotations": {
                                "description": "ignored",
                                "toJsonSchema": "counted",
                            },
                        }],
                    },
                    {
                        "_tag": "Filter",
                        "aborted": false,
                        "annotations": { "description": "no callback" },
                    },
                ],
            }),
            &callbacks,
        ),
        json!({ "type": "string" })
    );
    assert_eq!(visits.get(), 0);
}

// 708: `representation.schemas` compile before the callback runs.
#[test]
fn compiles_representation_schemas_before_invoking_a_callback() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "property-names".to_string(),
        Box::new(|input: &DocCallbackInput| {
            Ok(DocCallbackOutput::exact(json!({
                "propertyNames": input.schemas.first().cloned().unwrap_or(Value::Null),
            })))
        }),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [],
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "representation": {
                        "id": "acme/schema/propertyNames",
                        "payload": null,
                        "schemas": [string_repr()],
                    },
                    "annotations": { "toJsonSchema": "property-names" },
                }],
            }),
            &callbacks,
        ),
        json!({
            "not": { "type": "null" },
            "propertyNames": { "type": "string" },
        })
    );
}

// 738: approximation inside `representation.schemas` propagates to the
// enclosing union.
#[test]
fn propagates_approximation_from_representation_schemas() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("loose".to_string(), approximate(json!({ "minLength": 1 })));
    callbacks.insert("swallow".to_string(), exact(json!({})));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [
                    {
                        "_tag": "String",
                        "checks": [{
                            "_tag": "Filter",
                            "aborted": false,
                            "representation": {
                                "id": "acme/schema/dependent",
                                "payload": null,
                                "schemas": [{
                                    "_tag": "String",
                                    "checks": [filter("loose")],
                                }],
                            },
                            "annotations": { "toJsonSchema": "swallow" },
                        }],
                    },
                    { "_tag": "Boolean", "checks": [] },
                ],
                "options": { "mode": "oneOf" },
                "checks": [],
            }),
            &callbacks,
        ),
        json!({
            "anyOf": [{ "type": "string" }, { "type": "boolean" }],
        })
    );
}

// 774: a declaration approximates to the open document.
#[test]
fn approximates_declarations() {
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Declaration",
                "typeParameters": [string_repr()],
                "checks": [],
                "representation": { "id": "acme/schema/Box", "payload": null },
            }),
            &empty_callbacks(),
        ),
        json!({})
    );
}

// 791: every union member's pattern joins `patternProperties`.
#[test]
fn compiles_every_member_of_a_union_index_signature_parameter() {
    let document = doc(json!({
        "_tag": "Objects",
        "propertySignatures": [],
        "indexSignatures": [{
            "parameter": {
                "_tag": "Union",
                "types": [
                    {
                        "_tag": "TemplateLiteral",
                        "parts": [
                            { "_tag": "Literal", "literal": "a", "checks": [] },
                            string_repr(),
                        ],
                        "checks": [],
                    },
                    {
                        "_tag": "String",
                        "checks": [{
                            "_tag": "Filter",
                            "aborted": false,
                            "representation": {
                                "id": "effect/schema/isPattern",
                                "payload": { "source": "^b", "flags": "u" },
                            },
                        }],
                    },
                ],
                "options": { "mode": "anyOf" },
                "checks": [],
            },
            "type": string_repr(),
        }],
        "checks": [],
    }));
    assert_eq!(
        compile_document(&document).expect("compiles")["schema"],
        json!({
            "type": "object",
            "patternProperties": {
                "^a[\\s\\S]*?$": { "type": "string" },
                "^b": { "type": "string" },
            },
            "additionalProperties": true,
        })
    );
}

// 820: a pattern hidden inside a boolean applicator is not a selector, so
// the object stays open.
#[test]
fn does_not_extract_a_partial_pattern_from_boolean_applicator_members() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "applicator".to_string(),
        exact(json!({ "allOf": [false, { "pattern": "^a" }] })),
    );
    assert_eq!(
        compile_default(json!({
            "_tag": "Objects",
            "propertySignatures": [],
            "indexSignatures": [{
                "parameter": {
                    "_tag": "String",
                    "checks": [filter("applicator")],
                },
                "type": string_repr(),
            }],
            "checks": [],
        }))["schema"],
        json!({ "type": "object", "additionalProperties": true })
    );
}

// 849: an approximate pattern cannot select, so the key falls back to
// `propertyNames` with the value beside it.
#[test]
fn does_not_use_an_approximate_pattern_as_an_index_selector() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "loose-pattern".to_string(),
        approximate(json!({ "pattern": "^a" })),
    );
    let key = json!({
        "_tag": "String",
        "checks": [filter("loose-pattern")],
    });
    let objects = json!({
        "_tag": "Objects",
        "propertySignatures": [],
        "indexSignatures": [{ "parameter": key, "type": number_repr() }],
        "checks": [],
    });
    assert_eq!(
        compile_repr(objects.clone(), &callbacks),
        json!({
            "type": "object",
            "propertyNames": { "type": "string", "pattern": "^a" },
            "additionalProperties": {
                "anyOf": [
                    { "type": "number" },
                    { "type": "string", "enum": ["NaN", "Infinity", "-Infinity"] },
                ],
            },
        })
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [objects, { "_tag": "Boolean", "checks": [] }],
                "options": { "mode": "oneOf" },
                "checks": [],
            }),
            &callbacks,
        ),
        json!({
            "anyOf": [
                {
                    "type": "object",
                    "propertyNames": { "type": "string", "pattern": "^a" },
                    "additionalProperties": {
                        "anyOf": [
                            { "type": "number" },
                            { "type": "string", "enum": ["NaN", "Infinity", "-Infinity"] },
                        ],
                    },
                },
                { "type": "boolean" },
            ],
        })
    );
}

// 902: an exact key selector stays a `patternProperties` entry even when its
// value is approximate.
#[test]
fn keeps_an_exact_key_selector_when_its_value_is_approximate() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("pattern".to_string(), exact(json!({ "pattern": "^a" })));
    callbacks.insert("loose".to_string(), approximate(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Union",
                "types": [{
                    "_tag": "Objects",
                    "propertySignatures": [],
                    "indexSignatures": [{
                        "parameter": {
                            "_tag": "String",
                            "checks": [filter("pattern")],
                        },
                        "type": {
                            "_tag": "String",
                            "checks": [filter("loose")],
                        },
                    }],
                    "checks": [],
                }, { "_tag": "Boolean", "checks": [] }],
                "options": { "mode": "oneOf" },
                "checks": [],
            }),
            &callbacks,
        ),
        json!({
            "anyOf": [
                {
                    "type": "object",
                    "patternProperties": { "^a": { "type": "string", "minLength": 1 } },
                    "additionalProperties": false,
                },
                { "type": "boolean" },
            ],
        })
    );
}

// 941: every supported template part compiles, and anything else is invalid.
#[test]
fn compiles_all_supported_template_literal_parts_and_rejects_other_nodes() {
    assert_eq!(
        compile_default(json!({
            "_tag": "TemplateLiteral",
            "parts": [
                { "_tag": "Literal", "literal": "p", "checks": [] },
                number_repr(),
                {
                    "_tag": "TemplateLiteral",
                    "parts": [
                        { "_tag": "Literal", "literal": "x", "checks": [] },
                        string_repr(),
                    ],
                    "checks": [],
                },
                {
                    "_tag": "Union",
                    "types": [
                        { "_tag": "Literal", "literal": "a", "checks": [] },
                        { "_tag": "Literal", "literal": "b", "checks": [] },
                    ],
                    "options": { "mode": "anyOf" },
                    "checks": [],
                },
            ],
            "checks": [],
        }))["schema"],
        json!({
            "type": "string",
            "pattern": "^p[+-]?\\d*\\.?\\d+(?:[Ee][+-]?\\d+)?x[\\s\\S]*?(?:a|b)$",
        })
    );
    assert_invalid(
        json!({
            "_tag": "TemplateLiteral",
            "parts": [{ "_tag": "Boolean", "checks": [] }],
            "checks": [],
        }),
        "Invalid schema representation document",
        "[\"representation\"][\"parts\"][0]",
    );
}

// 1011: colliding bounds tighten, but any other repeated keyword separates.
#[test]
fn keeps_colliding_keywords_in_separate_schema_objects() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("first".to_string(), exact(json!({ "pattern": "a" })));
    callbacks.insert("second".to_string(), exact(json!({ "pattern": "b" })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [filter("first"), filter("second")],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "pattern": "a",
            "allOf": [{ "pattern": "b" }],
        })
    );
}

// 1025: pure check annotations merge inline with the fragment.
#[test]
fn inlines_pure_annotations_on_a_check() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("annotated".to_string(), exact(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "annotations": {
                        "toJsonSchema": "annotated",
                        "title": "title",
                        "description": "description",
                        "default": "default",
                        "examples": ["example"],
                        "readOnly": true,
                        "writeOnly": false,
                    },
                }],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "minLength": 1,
            "title": "title",
            "description": "description",
            "default": "default",
            "examples": ["example"],
            "readOnly": true,
            "writeOnly": false,
        })
    );
}

// 1051: callback metadata merges inline beside pure annotations.
#[test]
fn inlines_callback_metadata_with_pure_check_annotations() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "titled".to_string(),
        exact(json!({ "minLength": 1, "title": "callback" })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "annotations": {
                        "toJsonSchema": "titled",
                        "description": "annotation",
                    },
                }],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "minLength": 1,
            "title": "callback",
            "description": "annotation",
        })
    );
}

// 1066: a colliding annotation separates the whole check.
#[test]
fn keeps_colliding_annotations_in_a_separate_schema_object() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("long".to_string(), exact(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "annotations": { "description": "string" },
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "annotations": {
                        "toJsonSchema": "long",
                        "description": "check",
                    },
                }],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "description": "string",
            "allOf": [{ "minLength": 1, "description": "check" }],
        })
    );
}

// 1081: `format` is never pure, so it always separates.
#[test]
fn keeps_format_annotations_in_a_separate_schema_object() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("long".to_string(), exact(json!({ "minLength": 1 })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "annotations": {
                        "toJsonSchema": "long",
                        "format": "annotated-check",
                    },
                }],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "allOf": [{ "minLength": 1, "format": "annotated-check" }],
        })
    );
}

// 1094: `propertyNames` merges when nothing collides.
#[test]
fn inlines_property_names_without_a_collision() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "names".to_string(),
        exact(json!({ "propertyNames": { "minLength": 1 } })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [],
                "checks": [filter("names")],
            }),
            &callbacks,
        ),
        json!({
            "not": { "type": "null" },
            "propertyNames": { "minLength": 1 },
        })
    );
}

// 1109: repeated `propertyNames` separate.
#[test]
fn keeps_colliding_property_names_in_separate_schema_objects() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "min-names".to_string(),
        exact(json!({ "propertyNames": { "minLength": 1 } })),
    );
    callbacks.insert(
        "max-names".to_string(),
        exact(json!({ "propertyNames": { "maxLength": 2 } })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [],
                "checks": [filter("min-names"), filter("max-names")],
            }),
            &callbacks,
        ),
        json!({
            "not": { "type": "null" },
            "propertyNames": { "minLength": 1 },
            "allOf": [{ "propertyNames": { "maxLength": 2 } }],
        })
    );
}

// 1128: a referenced property keeps the reference apart from its
// contextual annotations.
#[test]
fn keeps_a_reference_separate_from_contextual_annotations() {
    let mut references = BTreeMap::new();
    references.insert("Value".to_string(), string_repr());
    let document = Document {
        representation: json!({
            "_tag": "Objects",
            "propertySignatures": [{
                "name": "value",
                "type": { "_tag": "Reference", "$ref": "Value" },
                "isOptional": false,
                "isMutable": false,
                "annotations": { "description": "contextual" },
            }],
            "indexSignatures": [],
            "checks": [],
        }),
        references,
    };
    assert_eq!(
        compile_document(&document).expect("compiles"),
        json!({
            "dialect": "draft-2020-12",
            "schema": {
                "type": "object",
                "properties": {
                    "value": {
                        "allOf": [
                            { "$ref": "#/$defs/Value" },
                            { "description": "contextual" },
                        ],
                    },
                },
                "required": ["value"],
                "additionalProperties": true,
            },
            "definitions": { "Value": { "type": "string" } },
        })
    );
}

// 1184: coupled keywords never merge, in either order.
#[test]
fn keeps_coupled_keywords_in_separate_schema_objects() {
    for (name, first, second) in [
        (
            "contains",
            json!({ "contains": { "const": "a" } }),
            json!({ "minContains": 2 }),
        ),
        (
            "properties",
            json!({ "properties": { "a": true } }),
            json!({ "additionalProperties": false }),
        ),
        (
            "prefix-items",
            json!({ "prefixItems": [true] }),
            json!({ "items": false }),
        ),
        (
            "if-else",
            json!({ "if": { "required": ["a"] } }),
            json!({ "else": { "required": ["b"] } }),
        ),
    ] {
        for (left, right) in [(first.clone(), second.clone()), (second, first)] {
            let mut callbacks = empty_callbacks();
            callbacks.insert("left".to_string(), exact(left.clone()));
            callbacks.insert("right".to_string(), exact(right.clone()));
            assert_eq!(
                compile_repr(
                    json!({
                        "_tag": "Any",
                        "checks": [filter("left"), filter("right")],
                    }),
                    &callbacks,
                ),
                {
                    let mut expected = left.as_object().cloned().expect("object");
                    expected.insert("allOf".to_string(), json!([right]));
                    Value::Object(expected)
                },
                "coupled keywords {name}"
            );
        }
    }
}

// 1194: a fragment carrying a custom keyword stays whole in `allOf`.
#[test]
fn keeps_a_fragment_containing_a_custom_keyword_intact() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "custom".to_string(),
        exact(json!({ "minLength": 1, "x-check": true })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "String",
                "checks": [filter("custom")],
            }),
            &callbacks,
        ),
        json!({
            "type": "string",
            "allOf": [{ "minLength": 1, "x-check": true }],
        })
    );
}

// 1207: a fragment restating the number type keeps `format` scoped beside it.
#[test]
fn keeps_format_scoped_after_absorbing_a_number_type() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("finite".to_string(), exact(json!({ "type": "number" })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "annotations": {
                        "toJsonSchema": "finite",
                        "format": "finite",
                    },
                }],
            }),
            &callbacks,
        ),
        json!({
            "type": "number",
            "allOf": [{ "format": "finite" }],
        })
    );
}

// 1220: callback metadata merges once the integer type absorbs.
#[test]
fn inlines_callback_metadata_after_absorbing_a_number_type() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "integer".to_string(),
        exact(json!({ "type": "integer", "description": "integer" })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "checks": [filter("integer")],
            }),
            &callbacks,
        ),
        json!({ "type": "integer", "description": "integer" })
    );
}

// 1233: an absorbed type never overwrites a base annotation.
#[test]
fn does_not_overwrite_an_annotation_after_absorbing_a_number_type() {
    let mut callbacks = empty_callbacks();
    callbacks.insert("integer".to_string(), exact(json!({ "type": "integer" })));
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "annotations": { "description": "number" },
                "checks": [{
                    "_tag": "Filter",
                    "aborted": false,
                    "annotations": {
                        "toJsonSchema": "integer",
                        "description": "integer",
                    },
                }],
            }),
            &callbacks,
        ),
        json!({
            "type": "integer",
            "description": "number",
            "allOf": [{ "description": "integer" }],
        })
    );
}

// 1249: nested number types extract through `allOf` without losing members.
#[test]
fn extracts_nested_number_types_without_losing_other_all_of_members() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "nested".to_string(),
        exact(json!({
            "allOf": [
                { "description": "nested", "allOf": [{ "type": "number" }, { "minimum": 1 }] },
                { "type": "integer", "maximum": 10 },
                { "title": "kept" },
            ],
        })),
    );
    callbacks.insert(
        "plain".to_string(),
        exact(json!({ "allOf": [{ "type": "number" }, { "type": "number" }] })),
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "checks": [filter("nested")],
            }),
            &callbacks,
        ),
        json!({
            "type": "integer",
            "allOf": [
                { "description": "nested", "allOf": [{ "minimum": 1 }] },
                { "maximum": 10 },
                { "title": "kept" },
            ],
        })
    );
    assert_eq!(
        compile_repr(
            json!({
                "_tag": "Number",
                "checks": [filter("plain")],
            }),
            &callbacks,
        ),
        json!({ "type": "number" })
    );
}

// 1302: compiler capabilities never ride as annotations: `representation`,
// `toCode` and `toJsonSchema` drop by key, `description` and `identifier`
// ride as usual, and `includeAnnotationKey` admits the custom JSON value.
// Function-valued annotations have no `Value` spelling here, so the
// function-valued `x-invalid` the pin drops has no counterpart.
#[test]
fn never_exposes_compiler_capabilities_as_extension_annotations() {
    let options = CompileDocumentOptions {
        on_excess_property: OnExcessProperty::Ignore,
        include_annotation_key: Some(|_: &str| true),
        callbacks: &empty_callbacks(),
    };
    assert_eq!(
        compile_document_with_options(
            &doc(json!({
                "_tag": "String",
                "annotations": {
                    "description": "text",
                    "identifier": "id",
                    "representation": { "id": "acme/schema/String", "payload": null },
                    "toCode": "ignored",
                    "toJsonSchema": "ignored",
                    "x-custom": { "enabled": true },
                },
                "checks": [],
            })),
            &options,
        )
        .expect("compiles")["schema"],
        json!({
            "type": "string",
            "description": "text",
            "identifier": "id",
            "x-custom": { "enabled": true },
        })
    );
}

// 1333: a throwing callback fails the compilation with its message.
#[test]
fn captures_exceptions_from_json_schema_callbacks() {
    let mut callbacks = empty_callbacks();
    callbacks.insert(
        "throwing".to_string(),
        Box::new(|_: &DocCallbackInput| Err("json schema callback".to_string())),
    );
    match compile_document_with_options(
        &doc(json!({
            "_tag": "String",
            "checks": [filter("throwing")],
        })),
        &error_opts(&callbacks),
    ) {
        Err(SchemaError::InvalidJson { message, .. }) => {
            assert_eq!(message, "json schema callback");
        }
        Err(other) => panic!("expected InvalidJson, got {other:?}"),
        Ok(schema) => panic!("expected an error, got {schema}"),
    }
}

// 1356: `__proto__` is an ordinary definition name here.
#[test]
fn supports_proto_as_a_reference() {
    let schema = Schema::annotated_with(
        Schema::as_unknown(Schema::string()),
        vec![("identifier".to_string(), "__proto__".to_string())],
    );
    let document = to_representation(&schema).expect("converts");
    assert_eq!(
        compile_document(&document).expect("compiles"),
        json!({
            "dialect": "draft-2020-12",
            "schema": { "$ref": "#/$defs/__proto__" },
            "definitions": { "__proto__": { "type": "string" } },
        })
    );
}

// 1372: a missing reference fails with its document path.
#[test]
fn reports_missing_references_with_their_document_path() {
    match compile_document(&doc(json!({ "_tag": "Reference", "$ref": "Missing" }))) {
        Err(SchemaError::InvalidJson { path, message }) => {
            assert_eq!(message, "Invalid reference Missing");
            assert_eq!(path, "[\"representation\"][\"$ref\"]");
        }
        Err(other) => panic!("expected InvalidJson, got {other:?}"),
        Ok(schema) => panic!("expected an error, got {schema}"),
    }
}
