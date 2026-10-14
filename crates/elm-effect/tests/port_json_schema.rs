//! Behavior port for `JsonSchema` (`JsonSchema.test.ts`): the dialect
//! conversion surface from `src/JsonSchema.ts`.
//!
//! Modules load via `#[path]` includes, so no facade edit is needed while
//! `lib.rs` is peer-held.
//!
//! Two residuals are stated per row rather than papered over:
//!
//! - Rows whose assertion runs `ajv.validateSchema` / `ajv.compile` prove the
//!   emitted document is *valid* and *equivalent* to the source dialect. This
//!   port has no JSON Schema validator, so those rows witness the document
//!   shape only and say so in the test that carries them; the validation leg
//!   stays unported.
//! - Rows that assert a JavaScript value type with no `Json` variant (symbols,
//!   functions, bigints) cannot be witnessed at all and stay unported.

#[path = "../src/json.rs"]
mod json;
#[path = "../src/json_pointer.rs"]
mod json_pointer;
#[path = "../src/json_schema.rs"]
mod json_schema;

use std::collections::BTreeMap;

use json::Json;

fn obj(pairs: &[(&str, Json)]) -> Json {
    Json::Obj(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    )
}

fn arr(items: &[Json]) -> Json {
    Json::Arr(items.to_vec())
}

fn text(value: &str) -> Json {
    Json::Str(value.to_string())
}

fn num(value: f64) -> Json {
    Json::Num(value)
}

fn flag(value: bool) -> Json {
    Json::Bool(value)
}

fn defs(pairs: &[(&str, Json)]) -> BTreeMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

/// The pin's `deepStrictEqual(document, { dialect, schema, definitions })`.
fn document_json(document: &json_schema::Document) -> Json {
    obj(&[
        ("dialect", text(document.dialect.as_str())),
        ("schema", document.schema.clone()),
        ("definitions", Json::Obj(document.definitions.clone())),
    ])
}

fn failure_message(result: Result<json_schema::Document, json_schema::ConversionError>) -> String {
    result.expect_err("the conversion is refused").0
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:43 meta-schema URIs
// ---------------------------------------------------------------------------

#[test]
fn meta_schema_uris_are_exported() {
    assert_eq!(
        json_schema::META_SCHEMA_URI_DRAFT_04,
        "http://json-schema.org/draft-04/schema#"
    );
    assert_eq!(
        json_schema::META_SCHEMA_URI_DRAFT_07,
        "http://json-schema.org/draft-07/schema#"
    );
    assert_eq!(
        json_schema::META_SCHEMA_URI_DRAFT_2020_12,
        "https://json-schema.org/draft/2020-12/schema"
    );
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:53-98 sanitizeOpenApiComponentsSchemasKey
// ---------------------------------------------------------------------------

#[test]
fn sanitize_returns_underscore_for_empty_input() {
    assert_eq!(
        json_schema::sanitize_open_api_components_schemas_key(""),
        "_"
    );
}

#[test]
fn sanitize_returns_input_when_already_valid() {
    for value in [
        "Simple",
        "with-dash",
        "with_underscore",
        "with.dot",
        "A1.B2-_",
    ] {
        assert_eq!(
            json_schema::sanitize_open_api_components_schemas_key(value),
            value
        );
    }
}

#[test]
fn sanitize_replaces_invalid_characters_with_underscore() {
    for value in [
        "a b", "a/b", "a:b", "a@b", "a#b", "a?b", "a+b", "a*b", "a,b", "a;b", "a|b", "a=b",
    ] {
        assert_eq!(
            json_schema::sanitize_open_api_components_schemas_key(value),
            "a_b",
            "{value} should sanitize to a_b"
        );
    }
}

#[test]
fn sanitize_preserves_length_when_only_replacing() {
    for value in ["a b", "..", "a--b"] {
        assert_eq!(
            json_schema::sanitize_open_api_components_schemas_key(value)
                .chars()
                .count(),
            value.chars().count()
        );
    }
}

#[test]
fn sanitize_replaces_non_ascii_characters() {
    assert_eq!(
        json_schema::sanitize_open_api_components_schemas_key("café"),
        "caf_"
    );
    assert_eq!(
        json_schema::sanitize_open_api_components_schemas_key("你好"),
        "__"
    );
    assert_eq!(
        json_schema::sanitize_open_api_components_schemas_key("🤖"),
        "_"
    );
    assert_eq!(
        json_schema::sanitize_open_api_components_schemas_key("a🤖b"),
        "a_b"
    );
}

#[test]
fn sanitize_is_idempotent() {
    for value in ["", "Simple", "a b", "a/b", "a..b", "café", "🤖", "A1.B2-_"] {
        let once = json_schema::sanitize_open_api_components_schemas_key(value);
        let twice = json_schema::sanitize_open_api_components_schemas_key(&once);
        assert_eq!(once, twice);
    }
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:118-475 fromSchemaDraft07
// ---------------------------------------------------------------------------

#[test]
fn draft07_rewrites_the_meta_schema_uri_without_an_empty_fragment() {
    let document = json_schema::from_schema_draft07(&obj(&[(
        "$schema",
        text("http://json-schema.org/draft-07/schema"),
    )]))
    .expect("draft-07 is a supported input");

    assert_eq!(
        document.schema,
        obj(&[("$schema", text(json_schema::META_SCHEMA_URI_DRAFT_2020_12))])
    );
}

/// The pin also validates the canonical document with `ajv`; this port has no
/// validator, so the shape is the witness and the validation leg stays
/// unported.
#[test]
fn draft07_preserves_semantics_and_opaque_custom_keywords() {
    let custom = obj(&[("$ref", text("#/definitions/Literal"))]);
    let required_value = obj(&[("required", arr(&[text("value")]))]);
    let not_required = obj(&[("not", required_value.clone())]);
    let kind_const = obj(&[("kind", obj(&[("const", text("full"))]))]);
    let kind_properties = obj(&[("properties", kind_const.clone())]);
    let label_required = obj(&[("required", arr(&[text("label")]))]);
    let dependencies = obj(&[
        ("enabled", arr(&[text("value")])),
        ("kind", label_required.clone()),
    ]);

    let input = obj(&[
        ("if", kind_properties.clone()),
        ("then", required_value.clone()),
        ("else", not_required.clone()),
        ("contains", obj(&[("type", text("number"))])),
        ("dependencies", dependencies),
        ("x-custom", custom.clone()),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("if", kind_properties),
                    ("then", required_value),
                    ("else", not_required),
                    ("contains", obj(&[("type", text("number"))])),
                    (
                        "dependentRequired",
                        obj(&[("enabled", arr(&[text("value")]))])
                    ),
                    ("dependentSchemas", obj(&[("kind", label_required)])),
                    ("x-custom", custom),
                ]),
            ),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn draft07_converts_plain_name_identifiers_to_anchors() {
    let document = json_schema::from_schema_draft07(&obj(&[
        ("$id", text("https://example.com/node#entry")),
        ("type", text("string")),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("$id", text("https://example.com/node")),
            ("$anchor", text("entry")),
            ("type", text("string")),
        ])
    );
}

#[test]
fn canonical_anchors_become_legacy_plain_name_identifiers() {
    let document = json_schema::Document {
        dialect: json_schema::Dialect::Draft2020_12,
        schema: obj(&[("$anchor", text("entry"))]),
        definitions: BTreeMap::new(),
    };

    let draft07 = json_schema::to_document_draft07(&document).expect("draft-07 lowers");
    assert_eq!(draft07.schema, obj(&[("$id", text("#entry"))]));

    let draft04 = json_schema::to_document_draft04(&document).expect("draft-04 lowers");
    assert_eq!(draft04.schema, obj(&[("id", text("#entry"))]));
}

#[test]
fn an_anchor_beside_a_schema_identifier_is_refused() {
    for id in ["https://example.com/node", "https://example.com/node#"] {
        let document = json_schema::Document {
            dialect: json_schema::Dialect::Draft2020_12,
            schema: obj(&[("$id", text(id)), ("$anchor", text("entry"))]),
            definitions: BTreeMap::new(),
        };

        assert!(
            failure_message(json_schema::to_document_draft07(&document))
                .starts_with("Cannot convert JSON Schema keyword \"$anchor\" to Draft-07")
        );
        assert!(
            failure_message(json_schema::to_document_draft04(&document))
                .starts_with("Cannot convert JSON Schema keyword \"$anchor\" to Draft-04")
        );
    }
}

#[test]
fn an_anchor_that_is_not_a_legacy_identifier_is_refused() {
    let document = json_schema::Document {
        dialect: json_schema::Dialect::Draft2020_12,
        schema: obj(&[("$anchor", text("_node"))]),
        definitions: BTreeMap::new(),
    };

    assert!(
        failure_message(json_schema::to_document_draft07(&document))
            .starts_with("Cannot convert JSON Schema keyword \"$anchor\" to Draft-07")
    );
    assert!(
        failure_message(json_schema::to_document_draft04(&document))
            .starts_with("Cannot convert JSON Schema keyword \"$anchor\" to Draft-04")
    );
}

#[test]
fn a_draft07_identifier_fragment_that_is_not_an_anchor_is_refused() {
    let message = failure_message(json_schema::from_schema_draft07(&obj(&[(
        "$id",
        text("https://example.com/node#not/a/plain-name"),
    )])));

    assert!(message.starts_with("Cannot convert JSON Schema keyword \"$id\" to Draft 2020-12"));
}

#[test]
fn draft07_keywords_that_would_activate_in_the_canonical_dialect_are_refused() {
    for schema in [
        obj(&[(
            "dependentRequired",
            obj(&[("value", arr(&[text("other")]))]),
        )]),
        obj(&[(
            "$vocabulary",
            obj(&[("https://example.com/vocabulary", flag(true))]),
        )]),
        obj(&[("contentSchema", obj(&[]))]),
        obj(&[("deprecated", flag(true))]),
    ] {
        let message = failure_message(json_schema::from_schema_draft07(&schema));
        assert!(
            message.starts_with("Cannot convert JSON Schema keyword")
                && message.ends_with(
                    "to Draft 2020-12: it is not active in Draft-07 but would become active in the target"
                ),
            "unexpected message: {message}"
        );
    }
}

#[test]
fn draft07_preserves_not() {
    let document =
        json_schema::from_schema_draft07(&obj(&[("not", obj(&[("type", text("string"))]))]))
            .expect("draft-07 lifts");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            ("schema", obj(&[("not", obj(&[("type", text("string"))]))])),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn a_schema_without_definitions_normalizes_to_the_canonical_shape() {
    let document = json_schema::from_schema_draft07(&obj(&[("type", text("string"))]))
        .expect("draft-07 lifts");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            ("schema", obj(&[("type", text("string"))])),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

/// The pin additionally compiles the canonical document with `ajv` and checks
/// that three values validate as `[true, false, false]`. Without a validator
/// this witnesses the document, which is where the rewrite shows.
#[test]
fn draft07_root_definitions_are_extracted_and_refs_rewritten() {
    let input = obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                ("a", obj(&[("$ref", text("#/definitions/A"))])),
                ("b", obj(&[("$ref", text("#/definitions/B"))])),
            ]),
        ),
        (
            "definitions",
            obj(&[
                (
                    "A",
                    obj(&[("type", text("string")), ("$ref", text("#/definitions/B"))]),
                ),
                ("B", obj(&[("type", text("number"))])),
            ]),
        ),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[
                            ("a", obj(&[("$ref", text("#/$defs/A"))])),
                            ("b", obj(&[("$ref", text("#/$defs/B"))])),
                        ]),
                    ),
                ]),
            ),
            (
                "definitions",
                obj(&[
                    ("A", obj(&[("$ref", text("#/$defs/B"))])),
                    ("B", obj(&[("type", text("number"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn an_id_beside_a_draft07_ref_is_ignored_when_tracking_resources() {
    let document = json_schema::from_schema_draft07(&obj(&[(
        "definitions",
        obj(&[
            ("Target", obj(&[("type", text("string"))])),
            (
                "Reference",
                obj(&[
                    ("$id", text("nested.json")),
                    ("$ref", text("#/definitions/Target")),
                ]),
            ),
        ]),
    )]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.definitions.get("Reference"),
        Some(&obj(&[("$ref", text("#/$defs/Target"))]))
    );
}

#[test]
fn draft07_tuple_items_become_prefix_items() {
    let document = json_schema::from_schema_draft07(&obj(&[
        ("type", text("array")),
        (
            "items",
            arr(&[
                obj(&[("type", text("string"))]),
                obj(&[("type", text("number"))]),
            ]),
        ),
        ("additionalItems", obj(&[("type", text("boolean"))])),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("type", text("array")),
                    (
                        "prefixItems",
                        arr(&[
                            obj(&[("type", text("string"))]),
                            obj(&[("type", text("number"))]),
                        ]),
                    ),
                    ("items", obj(&[("type", text("boolean"))])),
                ]),
            ),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn refs_into_converted_tuple_items_and_schema_dependencies_are_relocated() {
    let document = json_schema::from_schema_draft07(&obj(&[(
        "properties",
        obj(&[
            (
                "container",
                obj(&[
                    ("items", arr(&[obj(&[("type", text("string"))])])),
                    (
                        "dependencies",
                        obj(&[("value", obj(&[("type", text("number"))]))]),
                    ),
                ]),
            ),
            (
                "tupleItem",
                obj(&[("$ref", text("#/properties/container/items/0"))]),
            ),
            (
                "dependency",
                obj(&[("$ref", text("#/properties/container/dependencies/value"))]),
            ),
        ]),
    )]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.schema,
        obj(&[(
            "properties",
            obj(&[
                (
                    "container",
                    obj(&[
                        ("prefixItems", arr(&[obj(&[("type", text("string"))])])),
                        (
                            "dependentSchemas",
                            obj(&[("value", obj(&[("type", text("number"))]))]),
                        ),
                    ]),
                ),
                (
                    "tupleItem",
                    obj(&[("$ref", text("#/properties/container/prefixItems/0"))]),
                ),
                (
                    "dependency",
                    obj(&[(
                        "$ref",
                        text("#/properties/container/dependentSchemas/value")
                    )]),
                ),
            ]),
        )])
    );
}

#[test]
fn a_single_items_schema_stays_items() {
    let document = json_schema::from_schema_draft07(&obj(&[
        ("type", text("array")),
        ("items", obj(&[("type", text("string"))])),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("array")),
            ("items", obj(&[("type", text("string"))])),
        ])
    );
}

#[test]
fn draft07_preserves_annotations() {
    let input = obj(&[
        ("type", text("string")),
        ("title", text("My String")),
        ("description", text("A string value")),
        ("default", text("default")),
        ("examples", arr(&[text("example1"), text("example2")])),
        ("format", text("email")),
        ("readOnly", flag(true)),
        ("writeOnly", flag(true)),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(document.schema, input);
}

#[test]
fn draft07_preserves_string_constraints() {
    let input = obj(&[
        ("type", text("string")),
        ("pattern", text("^[a-z]+$")),
        ("minLength", num(1.0)),
        ("maxLength", num(100.0)),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(document.schema, input);
}

#[test]
fn draft2020_12_returns_the_canonical_document_shape() {
    let document = json_schema::from_schema_draft2020_12(&obj(&[("type", text("string"))]));

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            ("schema", obj(&[("type", text("string"))])),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn draft2020_12_extracts_definitions_and_leaves_refs_alone() {
    let document = json_schema::from_schema_draft2020_12(&obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                ("a", obj(&[("$ref", text("#/$defs/A"))])),
                ("b", obj(&[("$ref", text("#/$defs/B"))])),
            ]),
        ),
        (
            "$defs",
            obj(&[
                (
                    "A",
                    obj(&[("type", text("string")), ("$ref", text("#/$defs/B"))]),
                ),
                ("B", obj(&[("type", text("number"))])),
            ]),
        ),
    ]));

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[
                            ("a", obj(&[("$ref", text("#/$defs/A"))])),
                            ("b", obj(&[("$ref", text("#/$defs/B"))])),
                        ]),
                    ),
                ]),
            ),
            (
                "definitions",
                obj(&[
                    (
                        "A",
                        obj(&[("type", text("string")), ("$ref", text("#/$defs/B")),]),
                    ),
                    ("B", obj(&[("type", text("number"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn draft2020_12_preserves_nested_definitions_and_local_refs() {
    let nested = obj(&[
        (
            "definitions",
            obj(&[("NestedType", obj(&[("type", text("number"))]))]),
        ),
        ("$ref", text("#/properties/nested/definitions/NestedType")),
    ]);
    let document = json_schema::from_schema_draft2020_12(&obj(&[
        ("type", text("object")),
        ("properties", obj(&[("nested", nested.clone())])),
    ]));

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            ("properties", obj(&[("nested", nested)])),
        ])
    );
    assert!(document.definitions.is_empty());
}

#[test]
fn draft2020_12_keeps_non_standard_properties() {
    let document = json_schema::from_schema_draft2020_12(&obj(&[
        ("type", text("string")),
        ("x-custom", text("value")),
    ]));

    assert_eq!(
        document.schema,
        obj(&[("type", text("string")), ("x-custom", text("value"))])
    );
    assert!(document.definitions.is_empty());
}

#[test]
fn draft07_preserves_number_constraints() {
    let input = obj(&[
        ("type", text("number")),
        ("minimum", num(0.0)),
        ("maximum", num(100.0)),
        ("exclusiveMinimum", num(0.0)),
        ("exclusiveMaximum", num(100.0)),
        ("multipleOf", num(2.0)),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            ("schema", input),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn draft07_preserves_array_constraints() {
    let input = obj(&[
        ("type", text("array")),
        ("items", obj(&[("type", text("string"))])),
        ("minItems", num(1.0)),
        ("maxItems", num(10.0)),
        ("uniqueItems", flag(true)),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(document.schema, input);
}

#[test]
fn draft07_preserves_object_constraints() {
    let properties = obj(&[
        ("name", obj(&[("type", text("string"))])),
        ("age", obj(&[("type", text("number"))])),
    ]);
    let pattern_properties = obj(&[("^S_", obj(&[("type", text("string"))]))]);
    let input = obj(&[
        ("type", text("object")),
        ("properties", properties.clone()),
        ("required", arr(&[text("name")])),
        ("patternProperties", pattern_properties.clone()),
        ("additionalProperties", obj(&[("type", text("boolean"))])),
        ("propertyNames", obj(&[("pattern", text("^[A-Z]"))])),
        ("minProperties", num(1.0)),
        ("maxProperties", num(10.0)),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(document.schema, input);
}

/// The `anyOf` and `oneOf` members are schema positions, so the tuple spelling
/// inside them is converted too.
#[test]
fn draft07_preserves_enum_const_and_the_combinators() {
    let input = obj(&[
        ("enum", arr(&[text("a"), text("b"), text("c")])),
        ("const", text("constant")),
        (
            "allOf",
            arr(&[
                obj(&[
                    ("type", text("array")),
                    ("items", obj(&[("type", text("string"))])),
                ]),
                obj(&[("minItems", num(1.0))]),
            ]),
        ),
        (
            "anyOf",
            arr(&[
                obj(&[
                    ("type", text("array")),
                    ("items", arr(&[obj(&[("type", text("string"))])])),
                ]),
                obj(&[("type", text("number"))]),
            ]),
        ),
        (
            "oneOf",
            arr(&[
                obj(&[
                    ("type", text("array")),
                    ("items", arr(&[obj(&[("type", text("string"))])])),
                    ("additionalItems", obj(&[("type", text("number"))])),
                ]),
                obj(&[("type", text("boolean"))]),
            ]),
        ),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(
        document.schema,
        obj(&[
            ("enum", arr(&[text("a"), text("b"), text("c")])),
            ("const", text("constant")),
            (
                "allOf",
                arr(&[
                    obj(&[
                        ("type", text("array")),
                        ("items", obj(&[("type", text("string"))])),
                    ]),
                    obj(&[("minItems", num(1.0))]),
                ]),
            ),
            (
                "anyOf",
                arr(&[
                    obj(&[
                        ("type", text("array")),
                        ("prefixItems", arr(&[obj(&[("type", text("string"))])])),
                    ]),
                    obj(&[("type", text("number"))]),
                ]),
            ),
            (
                "oneOf",
                arr(&[
                    obj(&[
                        ("type", text("array")),
                        ("prefixItems", arr(&[obj(&[("type", text("string"))])])),
                        ("items", obj(&[("type", text("number"))])),
                    ]),
                    obj(&[("type", text("boolean"))]),
                ]),
            ),
        ])
    );
}

#[test]
fn draft07_nested_definitions_move_and_local_refs_follow() {
    let nested = obj(&[
        (
            "definitions",
            obj(&[("NestedType", obj(&[("type", text("number"))]))]),
        ),
        ("$ref", text("#/properties/nested/definitions/NestedType")),
    ]);
    let document = json_schema::from_schema_draft07(&obj(&[
        ("type", text("object")),
        ("properties", obj(&[("nested", nested)])),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[(
                    "nested",
                    obj(&[
                        (
                            "$defs",
                            obj(&[("NestedType", obj(&[("type", text("number"))]))]),
                        ),
                        ("$ref", text("#/properties/nested/$defs/NestedType")),
                    ]),
                )]),
            ),
        ])
    );
    assert!(document.definitions.is_empty());
}

/// A nested `$id` opens its own resource, so a fragment inside it resolves
/// against that resource rather than the document root.
#[test]
fn fragment_refs_relocate_relative_to_a_nested_resource() {
    let document = json_schema::from_schema_draft07(&obj(&[(
        "properties",
        obj(&[(
            "nested",
            obj(&[
                ("$id", text("nested.json")),
                (
                    "definitions",
                    obj(&[("Value", obj(&[("type", text("string"))]))]),
                ),
                (
                    "allOf",
                    arr(&[obj(&[("$ref", text("#/definitions/Value"))])]),
                ),
            ]),
        )]),
    )]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.schema,
        obj(&[(
            "properties",
            obj(&[(
                "nested",
                obj(&[
                    ("$id", text("nested.json")),
                    ("$defs", obj(&[("Value", obj(&[("type", text("string"))]))]),),
                    ("allOf", arr(&[obj(&[("$ref", text("#/$defs/Value"))])])),
                ]),
            )]),
        )])
    );
}

/// A reference that names another resource by URI keeps its URI part and has
/// only the fragment rewritten; `schemas/../nested.json` also proves the dot
/// segments are folded before the resource is compared.
#[test]
fn uri_refs_relocate_to_a_nested_resource() {
    let document = json_schema::from_schema_draft07(&obj(&[
        ("$id", text("https://example.com/root.json")),
        (
            "properties",
            obj(&[(
                "value",
                obj(&[("$ref", text("nested.json#/definitions/Value"))]),
            )]),
        ),
        (
            "definitions",
            obj(&[(
                "Nested",
                obj(&[
                    ("$id", text("schemas/../nested.json")),
                    (
                        "definitions",
                        obj(&[("Value", obj(&[("type", text("string"))]))]),
                    ),
                ]),
            )]),
        ),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("$id", text("https://example.com/root.json")),
                    (
                        "properties",
                        obj(&[("value", obj(&[("$ref", text("nested.json#/$defs/Value"))]),)]),
                    ),
                ]),
            ),
            (
                "definitions",
                obj(&[(
                    "Nested",
                    obj(&[
                        ("$id", text("schemas/../nested.json")),
                        ("$defs", obj(&[("Value", obj(&[("type", text("string"))]))]),),
                    ]),
                )]),
            ),
        ])
    );
}

/// Percent-encoding is decoded before the pointer is split, then re-applied
/// token by token: `%56alue` and `a%20b` both have to find their definition.
/// Two refs are malformed and must be left exactly as they arrived.
#[test]
fn json_pointer_uri_fragments_relocate() {
    let definitions = obj(&[
        ("a b", obj(&[("type", text("string"))])),
        ("Value", obj(&[("type", text("number"))])),
        ("a%b", obj(&[("type", text("boolean"))])),
        ("a/b", obj(&[("type", text("object"))])),
        ("café", obj(&[("type", text("array"))])),
    ]);
    let properties = obj(&[
        ("value", obj(&[("$ref", text("#/definitions/a%20b"))])),
        (
            "equivalent",
            obj(&[("$ref", text("#/definitions/%56alue"))]),
        ),
        (
            "encodedSeparators",
            obj(&[("$ref", text("#%2Fdefinitions%2FValue"))]),
        ),
        ("percent", obj(&[("$ref", text("#/definitions/a%25b"))])),
        ("escaped", obj(&[("$ref", text("#/definitions/a~1b"))])),
        ("utf8", obj(&[("$ref", text("#/definitions/caf%C3%A9"))])),
        ("invalid", obj(&[("$ref", text("#/%"))])),
        ("invalidUtf8", obj(&[("$ref", text("#/%FF"))])),
    ]);

    let document = json_schema::from_schema_draft07(&obj(&[
        ("properties", properties),
        ("definitions", definitions.clone()),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[(
                    "properties",
                    obj(&[
                        ("value", obj(&[("$ref", text("#/$defs/a%20b"))])),
                        ("equivalent", obj(&[("$ref", text("#/$defs/Value"))])),
                        ("encodedSeparators", obj(&[("$ref", text("#/$defs/Value"))]),),
                        ("percent", obj(&[("$ref", text("#/$defs/a%25b"))])),
                        ("escaped", obj(&[("$ref", text("#/$defs/a~1b"))])),
                        ("utf8", obj(&[("$ref", text("#/$defs/caf%C3%A9"))])),
                        ("invalid", obj(&[("$ref", text("#/%"))])),
                        ("invalidUtf8", obj(&[("$ref", text("#/%FF"))])),
                    ]),
                )]),
            ),
            ("definitions", definitions),
        ])
    );
}

#[test]
fn draft07_preserves_custom_properties() {
    let input = obj(&[("type", text("string")), ("x-custom", text("value"))]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(document.schema, input);
}

/// Every keyword here holds a value of the wrong shape. A malformed value is
/// carried through unchanged rather than dropped or repaired: the pin refuses
/// to guess what an invalid document meant.
#[test]
fn draft07_preserves_malformed_values_for_recognized_keywords() {
    let invalid = arr(&[num(1.0), obj(&[("not", flag(false))])]);
    let input = obj(&[
        ("$ref", num(1.0)),
        ("definitions", obj(&[("Invalid", invalid.clone())])),
        (
            "properties",
            obj(&[(
                "nested",
                obj(&[
                    ("definitions", text("invalid")),
                    ("not", arr(&[flag(false), obj(&[("type", text("string"))])])),
                ]),
            )]),
        ),
        ("patternProperties", text("invalid")),
        ("allOf", text("invalid")),
    ]);

    let document = json_schema::from_schema_draft07(&input).expect("draft-07 lifts");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("$ref", num(1.0)),
                    (
                        "properties",
                        obj(&[(
                            "nested",
                            obj(&[
                                ("$defs", text("invalid")),
                                ("not", arr(&[flag(false), obj(&[("type", text("string"))])])),
                            ]),
                        )]),
                    ),
                    ("patternProperties", text("invalid")),
                    ("allOf", text("invalid")),
                ]),
            ),
            ("definitions", obj(&[("Invalid", invalid)])),
        ])
    );
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:903-1283 fromSchemaOpenApi3_1 / fromSchemaOpenApi3_0
// ---------------------------------------------------------------------------

#[test]
fn openapi31_normalizes_the_base_dialect_uri() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[(
        "$schema",
        text("https://spec.openapis.org/oas/3.1/dialect/base"),
    )]))
    .expect("openapi 3.1 lifts");

    assert_eq!(
        document.schema,
        obj(&[("$schema", text(json_schema::META_SCHEMA_URI_DRAFT_2020_12))])
    );
}

/// A non-string `$ref` and an array where a schema map belongs are carried
/// through: the walk only rewrites a `$ref` it can read as text, and a
/// malformed map has no keys to descend into.
#[test]
fn openapi31_preserves_non_string_refs_and_malformed_schema_maps() {
    let input = obj(&[
        ("$ref", Json::Null),
        ("enum", arr(&[Json::Null])),
        (
            "properties",
            arr(&[obj(&[("$ref", text("#/components/schemas/Literal"))])]),
        ),
    ]);

    let document = json_schema::from_schema_openapi3_1(&input).expect("openapi 3.1 lifts");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            ("schema", input),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn openapi31_rewrites_component_refs_to_defs() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                ("a", obj(&[("$ref", text("#/components/schemas/A"))])),
                ("b", obj(&[("$ref", text("#/components/schemas/B"))])),
                (
                    "dynamic",
                    obj(&[("$dynamicRef", text("#/components/schemas/Dynamic"))]),
                ),
            ]),
        ),
    ]))
    .expect("openapi 3.1 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[
                    ("a", obj(&[("$ref", text("#/$defs/A"))])),
                    ("b", obj(&[("$ref", text("#/$defs/B"))])),
                    ("dynamic", obj(&[("$dynamicRef", text("#/$defs/Dynamic"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn openapi31_rewrites_percent_encoded_component_refs() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[(
        "$ref",
        text("#%2Fcomponents%2Fschemas%2FA%20B"),
    )]))
    .expect("openapi 3.1 lifts");

    assert_eq!(document.schema, obj(&[("$ref", text("#/$defs/A%20B"))]));
    assert!(document.definitions.is_empty());
}

#[test]
fn openapi31_normalizes_singular_examples_throughout() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[
        ("example", text("root")),
        (
            "properties",
            obj(&[("value", obj(&[("example", text("nested"))]))]),
        ),
    ]))
    .expect("openapi 3.1 lifts");

    let nested_value = obj(&[("examples", arr(&[text("nested")]))]);
    assert_eq!(
        document.schema,
        obj(&[
            ("examples", arr(&[text("root")])),
            ("properties", obj(&[("value", nested_value)])),
        ])
    );
}

#[test]
fn openapi31_merges_singular_and_array_examples() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[
        ("example", text("singular")),
        ("examples", arr(&[text("first"), text("second")])),
    ]))
    .expect("openapi 3.1 lifts");

    assert_eq!(
        document.schema,
        obj(&[(
            "examples",
            arr(&[text("singular"), text("first"), text("second")]),
        )])
    );
}

/// `const`, `enum`, `default` and `examples` hold values, not schemas, so a
/// `$ref` inside one is data and is left alone.
#[test]
fn openapi31_rewrites_refs_only_in_schema_positions() {
    let literal = obj(&[("$ref", text("#/components/schemas/Literal"))]);
    let input = obj(&[
        (
            "properties",
            obj(&[(
                "value",
                obj(&[("$ref", text("#/components/schemas/Value"))]),
            )]),
        ),
        ("const", literal.clone()),
        ("enum", arr(&[literal.clone()])),
        ("default", literal.clone()),
        ("examples", arr(&[literal.clone()])),
    ]);

    let document = json_schema::from_schema_openapi3_1(&input).expect("openapi 3.1 lifts");
    assert_eq!(
        document.schema,
        obj(&[
            (
                "properties",
                obj(&[("value", obj(&[("$ref", text("#/$defs/Value"))]))]),
            ),
            ("const", literal.clone()),
            ("enum", arr(&[literal.clone()])),
            ("default", literal.clone()),
            ("examples", arr(&[literal.clone()])),
        ])
    );
}

/// Every 2020-12 schema position is walked, and a root `$defs` still becomes
/// the document's definitions map.
#[test]
fn openapi31_rewrites_refs_throughout_draft2020_12_subschemas() {
    let reference = obj(&[("$ref", text("#/components/schemas/Value"))]);
    let rewritten = obj(&[("$ref", text("#/$defs/Value"))]);
    let input = obj(&[
        ("$defs", obj(&[("Alias", reference.clone())])),
        ("properties", obj(&[("value", reference.clone())])),
        ("patternProperties", obj(&[("pattern", reference.clone())])),
        (
            "dependentSchemas",
            obj(&[("dependency", reference.clone())]),
        ),
        ("allOf", arr(&[reference.clone()])),
        ("anyOf", arr(&[reference.clone()])),
        ("oneOf", arr(&[reference.clone()])),
        ("prefixItems", arr(&[reference.clone()])),
        ("additionalProperties", reference.clone()),
        ("unevaluatedProperties", reference.clone()),
        ("propertyNames", reference.clone()),
        ("items", reference.clone()),
        ("contains", reference.clone()),
        ("unevaluatedItems", reference.clone()),
        ("not", reference.clone()),
        ("if", reference.clone()),
        ("then", reference.clone()),
        ("else", reference.clone()),
        ("contentSchema", reference.clone()),
    ]);

    let document = json_schema::from_schema_openapi3_1(&input).expect("openapi 3.1 lifts");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("properties", obj(&[("value", rewritten.clone())])),
                    ("patternProperties", obj(&[("pattern", rewritten.clone())])),
                    (
                        "dependentSchemas",
                        obj(&[("dependency", rewritten.clone())])
                    ),
                    ("allOf", arr(&[rewritten.clone()])),
                    ("anyOf", arr(&[rewritten.clone()])),
                    ("oneOf", arr(&[rewritten.clone()])),
                    ("prefixItems", arr(&[rewritten.clone()])),
                    ("additionalProperties", rewritten.clone()),
                    ("unevaluatedProperties", rewritten.clone()),
                    ("propertyNames", rewritten.clone()),
                    ("items", rewritten.clone()),
                    ("contains", rewritten.clone()),
                    ("unevaluatedItems", rewritten.clone()),
                    ("not", rewritten.clone()),
                    ("if", rewritten.clone()),
                    ("then", rewritten.clone()),
                    ("else", rewritten.clone()),
                    ("contentSchema", rewritten),
                ]),
            ),
            (
                "definitions",
                obj(&[("Alias", obj(&[("$ref", text("#/$defs/Value"))]))])
            ),
        ])
    );
}

/// A `$id` on the root makes the whole document one schema resource, and a
/// resource keeps its own component names.
#[test]
fn openapi31_does_not_rewrite_refs_inside_a_root_resource() {
    let input = obj(&[
        ("$id", text("inline.json")),
        (
            "allOf",
            arr(&[
                obj(&[("$ref", text("#/components/schemas/Value"))]),
                obj(&[("$dynamicRef", text("#/components/schemas/Value"))]),
            ]),
        ),
    ]);

    let document = json_schema::from_schema_openapi3_1(&input).expect("openapi 3.1 lifts");
    assert_eq!(document.schema, input);
}

#[test]
fn openapi31_extracts_root_defs_after_rewriting_component_refs() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[("a", obj(&[("$ref", text("#/components/schemas/A"))]))]),
        ),
        (
            "$defs",
            obj(&[("MyType", obj(&[("type", text("string"))]))]),
        ),
    ]))
    .expect("openapi 3.1 lifts");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-2020-12")),
            (
                "schema",
                obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[("a", obj(&[("$ref", text("#/$defs/A"))]))]),
                    ),
                ]),
            ),
            (
                "definitions",
                obj(&[("MyType", obj(&[("type", text("string"))]))]),
            ),
        ])
    );
}

/// `definitions` is not a 2020-12 keyword, so a nested one is opaque: neither
/// its content nor the pointer into it is touched.
#[test]
fn openapi31_preserves_nested_definitions_and_local_refs() {
    let nested = obj(&[
        (
            "definitions",
            obj(&[("NestedType", obj(&[("type", text("number"))]))]),
        ),
        ("$ref", text("#/properties/nested/definitions/NestedType")),
    ]);
    let document = json_schema::from_schema_openapi3_1(&obj(&[
        ("type", text("object")),
        ("properties", obj(&[("nested", nested.clone())])),
    ]))
    .expect("openapi 3.1 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            ("properties", obj(&[("nested", nested)])),
        ])
    );
    assert!(document.definitions.is_empty());
}

#[test]
fn openapi31_keeps_non_standard_properties() {
    let document = json_schema::from_schema_openapi3_1(&obj(&[
        ("type", text("string")),
        ("x-custom", text("value")),
    ]))
    .expect("openapi 3.1 lifts");

    assert_eq!(
        document.schema,
        obj(&[("type", text("string")), ("x-custom", text("value"))])
    );
}

#[test]
fn openapi30_rewrites_component_refs_to_defs() {
    let document = json_schema::from_schema_openapi3_0(&obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                ("a", obj(&[("$ref", text("#/components/schemas/A"))])),
                ("b", obj(&[("$ref", text("#/components/schemas/B"))])),
            ]),
        ),
    ]))
    .expect("openapi 3.0 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[
                    ("a", obj(&[("$ref", text("#/$defs/A"))])),
                    ("b", obj(&[("$ref", text("#/$defs/B"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn openapi30_rewrites_percent_encoded_component_refs() {
    let document = json_schema::from_schema_openapi3_0(&obj(&[(
        "$ref",
        text("#%2Fcomponents%2Fschemas%2FA%20B"),
    )]))
    .expect("openapi 3.0 lifts");

    assert_eq!(document.schema, obj(&[("$ref", text("#/$defs/A%20B"))]));
}

/// OpenAPI 3.0 has no `definitions`, but an input may carry one as an
/// extension, and an extension is not a keyword the conversion may move.
#[test]
fn openapi30_preserves_definitions_as_an_opaque_extension() {
    let definitions = obj(&[("Value", obj(&[("type", text("string"))]))]);
    let document =
        json_schema::from_schema_openapi3_0(&obj(&[("definitions", definitions.clone())]))
            .expect("openapi 3.0 lifts");

    assert_eq!(document.schema, obj(&[("definitions", definitions)]));
    assert!(document.definitions.is_empty());
}

#[test]
fn openapi30_normalizes_a_singular_example() {
    let document = json_schema::from_schema_openapi3_0(&obj(&[
        ("type", text("string")),
        ("example", text("a")),
    ]))
    .expect("openapi 3.0 lifts");

    assert_eq!(
        document.schema,
        obj(&[("type", text("string")), ("examples", arr(&[text("a")]))])
    );
}

#[test]
fn openapi30_preserves_the_deprecated_annotation() {
    let input = obj(&[("type", text("string")), ("deprecated", flag(true))]);

    let document = json_schema::from_schema_openapi3_0(&input).expect("openapi 3.0 lifts");
    assert_eq!(document.schema, input);
}

#[test]
fn openapi30_refuses_examples() {
    let message = failure_message(json_schema::from_schema_openapi3_0(&obj(&[(
        "examples",
        arr(&[text("value")]),
    )])));

    assert!(
        message.starts_with("Cannot convert JSON Schema keyword \"examples\" to Draft 2020-12")
    );
}

#[test]
fn openapi30_refuses_draft2020_12_keywords() {
    let message = failure_message(json_schema::from_schema_openapi3_0(&obj(&[(
        "unevaluatedProperties",
        flag(false),
    )])));

    assert!(message.starts_with(
        "Cannot convert JSON Schema keyword \"unevaluatedProperties\" to Draft 2020-12"
    ));
}

/// `default`, `example`, `discriminator` and an `x-` extension hold values, so
/// the `$ref` inside them is data; `example` still becomes an `examples` list.
#[test]
fn openapi30_does_not_traverse_opaque_values() {
    let literal = obj(&[
        ("nullable", flag(true)),
        ("$ref", text("#/components/schemas/Literal")),
    ]);
    let discriminator = obj(&[(
        "mapping",
        obj(&[("value", text("#/components/schemas/Value"))]),
    )]);
    let input = obj(&[
        ("type", text("object")),
        ("default", literal.clone()),
        ("example", literal.clone()),
        ("discriminator", discriminator.clone()),
        ("x-custom", literal.clone()),
    ]);

    let document = json_schema::from_schema_openapi3_0(&input).expect("openapi 3.0 lifts");
    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            ("default", literal.clone()),
            ("examples", arr(&[literal.clone()])),
            ("discriminator", discriminator),
            ("x-custom", literal),
        ])
    );
}

/// `nullable` only widens a type; it never widens `enum`, never survives as a
/// keyword, and on an `allOf` member it is normalized on its own, because that
/// member is converted independently of its parent.
#[test]
fn openapi30_nullable_cases() {
    let cases: Vec<(Json, Json)> = vec![
        (obj(&[("nullable", flag(true))]), obj(&[])),
        (
            obj(&[("type", text("string")), ("nullable", flag(true))]),
            obj(&[("type", arr(&[text("string"), text("null")]))]),
        ),
        (
            obj(&[
                ("type", text("string")),
                ("enum", arr(&[text("a"), text("b")])),
                ("nullable", flag(true)),
            ]),
            obj(&[
                ("type", arr(&[text("string"), text("null")])),
                ("enum", arr(&[text("a"), text("b")])),
            ]),
        ),
        (
            obj(&[
                ("type", text("string")),
                ("enum", arr(&[text("a"), text("b"), Json::Null])),
                ("nullable", flag(true)),
            ]),
            obj(&[
                ("type", arr(&[text("string"), text("null")])),
                ("enum", arr(&[text("a"), text("b"), Json::Null])),
            ]),
        ),
        (
            obj(&[
                ("type", text("string")),
                ("enum", arr(&[Json::Null])),
                ("nullable", flag(true)),
            ]),
            obj(&[
                ("type", arr(&[text("string"), text("null")])),
                ("enum", arr(&[Json::Null])),
            ]),
        ),
        (
            obj(&[("nullable", flag(true)), ("minimum", num(0.0))]),
            obj(&[("minimum", num(0.0))]),
        ),
        (
            obj(&[("type", text("string")), ("nullable", flag(false))]),
            obj(&[("type", text("string"))]),
        ),
        (
            obj(&[
                ("type", text("string")),
                ("allOf", arr(&[obj(&[("nullable", flag(true))])])),
            ]),
            obj(&[("type", text("string")), ("allOf", arr(&[obj(&[])]))]),
        ),
        (
            obj(&[
                ("type", text("string")),
                ("nullable", flag(true)),
                ("allOf", arr(&[obj(&[("nullable", flag(true))])])),
            ]),
            obj(&[
                ("type", arr(&[text("string"), text("null")])),
                ("allOf", arr(&[obj(&[])])),
            ]),
        ),
    ];

    for (input, expected) in cases {
        let document = json_schema::from_schema_openapi3_0(&input).expect("openapi 3.0 lifts");
        assert_eq!(document.schema, expected, "input: {input:?}");
    }
}

/// The `exclusivity` describe beside `nullable`; not a ledger row of its own,
/// carried because `convertOpenApiExclusiveBound` has no other witness.
#[test]
fn openapi30_exclusive_bound_cases() {
    let cases: Vec<(Json, Json)> = vec![
        (
            obj(&[
                ("type", text("number")),
                ("minimum", num(10.0)),
                ("exclusiveMinimum", flag(true)),
            ]),
            obj(&[("type", text("number")), ("exclusiveMinimum", num(10.0))]),
        ),
        (
            obj(&[
                ("type", text("number")),
                ("maximum", num(100.0)),
                ("exclusiveMaximum", flag(true)),
            ]),
            obj(&[("type", text("number")), ("exclusiveMaximum", num(100.0))]),
        ),
        (
            obj(&[
                ("type", text("number")),
                ("minimum", num(10.0)),
                ("exclusiveMinimum", flag(false)),
            ]),
            obj(&[("type", text("number")), ("minimum", num(10.0))]),
        ),
        (
            obj(&[
                ("type", text("number")),
                ("maximum", num(100.0)),
                ("exclusiveMaximum", flag(false)),
            ]),
            obj(&[("type", text("number")), ("maximum", num(100.0))]),
        ),
        (
            obj(&[("type", text("number")), ("exclusiveMinimum", flag(true))]),
            obj(&[("type", text("number"))]),
        ),
        (
            obj(&[("type", text("number")), ("exclusiveMaximum", flag(true))]),
            obj(&[("type", text("number"))]),
        ),
    ];

    for (input, expected) in cases {
        let document = json_schema::from_schema_openapi3_0(&input).expect("openapi 3.0 lifts");
        assert_eq!(document.schema, expected, "input: {input:?}");
    }
}

#[test]
fn additional_items_is_ignored_when_items_is_not_a_tuple() {
    let document = json_schema::from_schema_draft07(&obj(&[
        ("type", text("array")),
        ("items", obj(&[("type", text("string"))])),
        ("additionalItems", flag(false)),
    ]))
    .expect("draft-07 lifts");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("array")),
            ("items", obj(&[("type", text("string"))])),
        ])
    );
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:1380-1856 toDocumentDraft07
// ---------------------------------------------------------------------------

/// The canonical document a lowering starts from.
fn canonical_document(schema: Json, definitions: BTreeMap<String, Json>) -> json_schema::Document {
    json_schema::Document {
        dialect: json_schema::Dialect::Draft2020_12,
        schema,
        definitions,
    }
}

#[test]
fn draft07_target_rewrites_the_canonical_meta_schema_uri() {
    let schema = obj(&[(
        "$schema",
        text(&format!("{}#", json_schema::META_SCHEMA_URI_DRAFT_2020_12)),
    )]);

    let document = json_schema::to_document_draft07(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-07");

    assert_eq!(
        document.schema,
        obj(&[("$schema", text(json_schema::META_SCHEMA_URI_DRAFT_07))])
    );
}

/// Only the document root may declare a dialect, so the canonical URI on an
/// embedded resource is dropped rather than carried into a dialect it does not
/// belong to.
#[test]
fn draft07_target_omits_the_canonical_uri_from_embedded_resources() {
    let embedded = obj(&[
        ("$id", text("embedded.json")),
        ("$schema", text(json_schema::META_SCHEMA_URI_DRAFT_2020_12)),
        ("type", text("string")),
    ]);

    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[]),
        defs(&[("Embedded", embedded)]),
    ))
    .expect("2020-12 lowers to draft-07");

    assert_eq!(
        document.definitions.get("Embedded"),
        Some(&obj(&[
            ("$id", text("embedded.json")),
            ("type", text("string"))
        ]))
    );
}

#[test]
fn draft07_target_rejects_custom_dialects_in_embedded_resources() {
    let embedded = obj(&[
        ("$id", text("embedded.json")),
        ("$schema", text("https://example.com/dialect")),
        ("type", text("string")),
    ]);

    let message = failure_message(json_schema::to_document_draft07(&canonical_document(
        obj(&[]),
        defs(&[("Embedded", embedded)]),
    )));

    assert!(message.starts_with("Cannot convert JSON Schema keyword \"$schema\" to Draft-07"));
}

/// The two dependency keywords become the one draft-07 keyword: a key with a
/// list keeps the list, a key with a schema keeps the schema, and a key with
/// both becomes an `allOf` of the schema and the requirement.
#[test]
fn draft07_target_lowers_dependencies_and_keeps_opaque_custom_keywords() {
    let custom = obj(&[("$ref", text("#/$defs/Literal"))]);
    let const_full = obj(&[("const", text("full"))]);
    let kind_const = obj(&[("kind", const_full)]);
    let if_schema = obj(&[("properties", kind_const.clone())]);
    let required_value = obj(&[("required", arr(&[text("value")]))]);
    let else_schema = obj(&[("not", required_value.clone())]);
    let min_properties = obj(&[("minProperties", num(1.0))]);
    let label_required = obj(&[("required", arr(&[text("label")]))]);

    let schema = obj(&[
        ("if", if_schema.clone()),
        ("then", required_value.clone()),
        ("else", else_schema.clone()),
        ("contains", obj(&[("type", text("number"))])),
        (
            "dependentRequired",
            obj(&[
                ("enabled", arr(&[text("value")])),
                ("kind", arr(&[text("label")])),
            ]),
        ),
        (
            "dependentSchemas",
            obj(&[
                ("kind", min_properties.clone()),
                ("mode", required_value.clone()),
            ]),
        ),
        ("x-custom", custom.clone()),
    ]);

    let document = json_schema::to_document_draft07(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-07");

    let kind_combined = obj(&[("allOf", arr(&[min_properties, label_required]))]);
    assert_eq!(
        document.schema,
        obj(&[
            ("if", if_schema),
            ("then", required_value.clone()),
            ("else", else_schema),
            ("contains", obj(&[("type", text("number"))])),
            (
                "dependencies",
                obj(&[
                    ("enabled", arr(&[text("value")])),
                    ("kind", kind_combined),
                    ("mode", required_value),
                ]),
            ),
            ("x-custom", custom),
        ])
    );
}

#[test]
fn draft07_target_moves_nested_defs_and_relocates_local_refs() {
    let nested = obj(&[
        ("$defs", obj(&[("Value", obj(&[("type", text("string"))]))])),
        ("$ref", text("#/properties/nested/$defs/Value")),
    ]);
    let schema = obj(&[("properties", obj(&[("nested", nested)]))]);

    let document = json_schema::to_document_draft07(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-07");

    assert_eq!(
        document.schema,
        obj(&[(
            "properties",
            obj(&[(
                "nested",
                obj(&[
                    (
                        "definitions",
                        obj(&[("Value", obj(&[("type", text("string"))]))]),
                    ),
                    (
                        "allOf",
                        arr(&[obj(&[(
                            "$ref",
                            text("#/properties/nested/definitions/Value"),
                        )])]),
                    ),
                ]),
            )]),
        )])
    );
}

/// One input, both lowerings. A reference that names another resource keeps
/// its URI and has only the fragment rewritten; a reference to a resource this
/// document does not hold is left alone, because there is nothing to relocate
/// it to.
#[test]
fn nested_resource_fragment_refs_relocate_when_emitting() {
    let input = canonical_document(
        obj(&[(
            "allOf",
            arr(&[
                obj(&[("$ref", text("nested.json#/$defs/Value"))]),
                obj(&[("$ref", text("external.json#/$defs/Value"))]),
            ]),
        )]),
        defs(&[(
            "Nested",
            obj(&[
                ("$id", text("nested.json")),
                ("$defs", obj(&[("Value", obj(&[("type", text("string"))]))])),
                ("allOf", arr(&[obj(&[("$ref", text("#/$defs/Value"))])])),
            ]),
        )]),
    );

    let draft07 = json_schema::to_document_draft07(&input).expect("lowers to draft-07");
    assert_eq!(
        document_json(&draft07),
        obj(&[
            ("dialect", text("draft-07")),
            (
                "schema",
                obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("nested.json#/definitions/Value"))]),
                        obj(&[("$ref", text("external.json#/$defs/Value"))]),
                    ]),
                )]),
            ),
            (
                "definitions",
                obj(&[(
                    "Nested",
                    obj(&[
                        ("$id", text("nested.json")),
                        (
                            "definitions",
                            obj(&[("Value", obj(&[("type", text("string"))]))]),
                        ),
                        (
                            "allOf",
                            arr(&[obj(&[("$ref", text("#/definitions/Value"))])]),
                        ),
                    ]),
                )]),
            ),
        ])
    );

    let draft04 = json_schema::to_document_draft04(&input).expect("lowers to draft-04");
    assert_eq!(
        document_json(&draft04),
        obj(&[
            ("dialect", text("draft-04")),
            (
                "schema",
                obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("nested.json#/definitions/Value"))]),
                        obj(&[("$ref", text("external.json#/$defs/Value"))]),
                    ]),
                )]),
            ),
            (
                "definitions",
                obj(&[(
                    "Nested",
                    obj(&[
                        ("id", text("nested.json")),
                        (
                            "definitions",
                            obj(&[("Value", obj(&[("type", text("string"))]))]),
                        ),
                        (
                            "allOf",
                            arr(&[obj(&[("$ref", text("#/definitions/Value"))])]),
                        ),
                    ]),
                )]),
            ),
        ])
    );
}

/// `contentSchema` is a schema position in both targets, so its `$defs` moves
/// and the pointer into it follows; draft-04 additionally has to spell `const`
/// as a one-value `enum`.
#[test]
fn content_schema_bodies_and_references_convert() {
    let const_value = obj(&[("const", text("value"))]);
    let value_definition = obj(&[("Value", const_value)]);
    let defs_body = obj(&[("$defs", value_definition)]);

    let input = canonical_document(
        obj(&[
            ("contentSchema", defs_body),
            (
                "allOf",
                arr(&[obj(&[("$ref", text("#/contentSchema/$defs/Value"))])]),
            ),
        ]),
        BTreeMap::new(),
    );

    let draft07 = json_schema::to_document_draft07(&input).expect("lowers to draft-07");
    assert_eq!(
        draft07.schema,
        obj(&[
            (
                "contentSchema",
                obj(&[(
                    "definitions",
                    obj(&[("Value", obj(&[("const", text("value"))]))]),
                )]),
            ),
            (
                "allOf",
                arr(&[obj(&[("$ref", text("#/contentSchema/definitions/Value"),)])]),
            ),
        ])
    );

    let draft04 = json_schema::to_document_draft04(&input).expect("lowers to draft-04");
    assert_eq!(
        draft04.schema,
        obj(&[
            (
                "contentSchema",
                obj(&[(
                    "definitions",
                    obj(&[("Value", obj(&[("enum", arr(&[text("value")]))]),)]),
                )]),
            ),
            (
                "allOf",
                arr(&[obj(&[("$ref", text("#/contentSchema/definitions/Value"),)])]),
            ),
        ])
    );
}

#[test]
fn draft07_target_rejects_canonical_constraints_it_cannot_represent() {
    let contains_two = obj(&[
        ("contains", obj(&[("type", text("string"))])),
        ("minContains", num(2.0)),
    ]);
    let contains_max = obj(&[
        ("contains", obj(&[("type", text("string"))])),
        ("maxContains", num(3.0)),
    ]);
    let cases = vec![
        contains_two,
        contains_max,
        obj(&[("unevaluatedProperties", flag(false))]),
        obj(&[("unevaluatedItems", flag(false))]),
        obj(&[("$dynamicRef", text("#node"))]),
        obj(&[(
            "$vocabulary",
            obj(&[("https://example.com/vocabulary", flag(true))]),
        )]),
        obj(&[("dependencies", obj(&[("value", arr(&[text("other")]))]))]),
    ];

    for schema in cases {
        let message = failure_message(json_schema::to_document_draft07(&canonical_document(
            schema,
            BTreeMap::new(),
        )));
        assert!(
            message.starts_with("Cannot convert JSON Schema keyword")
                && message.contains("to Draft-07"),
            "unexpected message: {message}"
        );
    }
}

/// The pin builds the input with `Schema.toJsonSchemaDocument(Schema.Never)`,
/// which this port does not have; the canonical document that step produces is
/// written out literally, and the lowering is what is witnessed.
#[test]
fn draft07_target_preserves_schema_never() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[("not", obj(&[]))]),
        BTreeMap::new(),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-07")),
            ("schema", obj(&[("not", obj(&[]))])),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn draft07_target_omits_an_empty_required_array() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[("type", text("object")), ("required", arr(&[]))]),
        BTreeMap::new(),
    ))
    .expect("lowers to draft-07");

    assert_eq!(document.schema, obj(&[("type", text("object"))]));
}

/// Every keyword both dialects share survives the lowering unchanged.
#[test]
fn draft07_target_preserves_annotations_and_validation_keywords() {
    let schema = obj(&[
        ("type", text("object")),
        ("required", arr(&[text("value")])),
        ("enum", arr(&[text("a"), text("b")])),
        ("const", text("a")),
        ("title", text("title")),
        ("description", text("description")),
        ("default", text("a")),
        ("examples", arr(&[text("a")])),
        ("format", text("custom")),
        ("readOnly", flag(true)),
        ("writeOnly", flag(true)),
        ("pattern", text("^a$")),
        ("minimum", num(0.0)),
        ("maximum", num(10.0)),
        ("exclusiveMinimum", num(0.0)),
        ("exclusiveMaximum", num(10.0)),
        ("minLength", num(1.0)),
        ("maxLength", num(10.0)),
        ("minItems", num(1.0)),
        ("maxItems", num(10.0)),
        ("minProperties", num(1.0)),
        ("maxProperties", num(10.0)),
        ("multipleOf", num(2.0)),
        ("uniqueItems", flag(true)),
        (
            "properties",
            obj(&[("value", obj(&[("type", text("string"))]))]),
        ),
        (
            "patternProperties",
            obj(&[("^x-", obj(&[("type", text("string"))]))]),
        ),
        ("not", obj(&[("type", text("null"))])),
        ("additionalProperties", flag(false)),
        ("propertyNames", obj(&[("minLength", num(1.0))])),
        ("allOf", arr(&[obj(&[("type", text("object"))])])),
        ("anyOf", arr(&[obj(&[("type", text("string"))])])),
        ("oneOf", arr(&[obj(&[("type", text("number"))])])),
        ("items", obj(&[("type", text("string"))])),
    ]);

    let document =
        json_schema::to_document_draft07(&canonical_document(schema.clone(), BTreeMap::new()))
            .expect("lowers to draft-07");
    assert_eq!(document.schema, schema);
}

/// The pin also compiles this document with `ajv` and checks four values
/// against it. Without a validator, the document is the witness.
#[test]
fn draft07_target_lowers_validation_shapes() {
    let schema = obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                ("mode", obj(&[("const", text("on"))])),
                (
                    "value",
                    obj(&[("type", text("number")), ("exclusiveMinimum", num(0.0))]),
                ),
                (
                    "tuple",
                    obj(&[
                        ("type", text("array")),
                        ("prefixItems", arr(&[obj(&[("type", text("string"))])])),
                        ("items", flag(false)),
                    ]),
                ),
            ]),
        ),
        (
            "required",
            arr(&[text("mode"), text("value"), text("tuple")]),
        ),
        ("additionalProperties", flag(false)),
    ]);

    let document = json_schema::to_document_draft07(&canonical_document(schema, BTreeMap::new()))
        .expect("lowers to draft-07");

    let properties = obj(&[
        ("mode", obj(&[("const", text("on"))])),
        (
            "value",
            obj(&[("type", text("number")), ("exclusiveMinimum", num(0.0))]),
        ),
        (
            "tuple",
            obj(&[
                ("type", text("array")),
                ("items", arr(&[obj(&[("type", text("string"))])])),
                ("additionalItems", flag(false)),
            ]),
        ),
    ]);
    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            ("properties", properties),
            (
                "required",
                arr(&[text("mode"), text("value"), text("tuple")])
            ),
            ("additionalProperties", flag(false)),
        ])
    );
}

#[test]
fn draft07_target_rewrites_defs_refs_to_definitions_refs() {
    let input = canonical_document(
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[
                    ("a", obj(&[("$ref", text("#/$defs/A"))])),
                    ("b", obj(&[("$ref", text("#/$defs/B"))])),
                ]),
            ),
        ]),
        defs(&[
            (
                "A",
                obj(&[("type", text("string")), ("$ref", text("#/$defs/B"))]),
            ),
            ("B", obj(&[("type", text("number"))])),
        ]),
    );

    let document = json_schema::to_document_draft07(&input).expect("lowers to draft-07");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-07")),
            (
                "schema",
                obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[
                            ("a", obj(&[("$ref", text("#/definitions/A"))])),
                            ("b", obj(&[("$ref", text("#/definitions/B"))])),
                        ]),
                    ),
                ]),
            ),
            (
                "definitions",
                obj(&[
                    (
                        "A",
                        obj(&[
                            ("type", text("string")),
                            ("allOf", arr(&[obj(&[("$ref", text("#/definitions/B"))])]),),
                        ]),
                    ),
                    ("B", obj(&[("type", text("number"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn draft07_target_rewrites_refs_only_in_schema_positions() {
    let literal = obj(&[("$ref", text("#/$defs/Literal"))]);
    let input = canonical_document(
        obj(&[
            (
                "properties",
                obj(&[("value", obj(&[("$ref", text("#/$defs/Value"))]))]),
            ),
            ("const", literal.clone()),
            ("enum", arr(&[literal.clone()])),
            ("default", literal.clone()),
            ("examples", arr(&[literal.clone()])),
        ]),
        defs(&[("Value", obj(&[("type", text("string"))]))]),
    );

    let document = json_schema::to_document_draft07(&input).expect("lowers to draft-07");
    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-07")),
            (
                "schema",
                obj(&[
                    (
                        "properties",
                        obj(&[("value", obj(&[("$ref", text("#/definitions/Value"))]))]),
                    ),
                    ("const", literal.clone()),
                    ("enum", arr(&[literal.clone()])),
                    ("default", literal.clone()),
                    ("examples", arr(&[literal.clone()])),
                ]),
            ),
            (
                "definitions",
                obj(&[("Value", obj(&[("type", text("string"))]))]),
            ),
        ])
    );
}

/// A `$ref` cannot sit beside other keywords in draft-07, so it joins the
/// existing `allOf` last, keeping the constraints around it and the locations
/// of members already there.
#[test]
fn draft07_target_preserves_constraints_next_to_refs_and_existing_all_of() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[
            ("$ref", text("#/$defs/S")),
            ("minLength", num(3.0)),
            ("allOf", arr(&[obj(&[("maxLength", num(5.0))])])),
        ]),
        defs(&[("S", obj(&[("type", text("string"))]))]),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document.schema,
        obj(&[
            ("minLength", num(3.0)),
            (
                "allOf",
                arr(&[
                    obj(&[("maxLength", num(5.0))]),
                    obj(&[("$ref", text("#/definitions/S"))]),
                ]),
            ),
        ])
    );
}

#[test]
fn draft07_target_keeps_all_of_locations_stable_when_wrapping_a_ref() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[
            ("$ref", text("#/$defs/Base")),
            ("allOf", arr(&[obj(&[("type", text("string"))])])),
            (
                "properties",
                obj(&[("copy", obj(&[("$ref", text("#/allOf/0"))]))]),
            ),
        ]),
        defs(&[("Base", obj(&[("type", text("object"))]))]),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document.schema,
        obj(&[
            (
                "allOf",
                arr(&[
                    obj(&[("type", text("string"))]),
                    obj(&[("$ref", text("#/definitions/Base"))]),
                ]),
            ),
            (
                "properties",
                obj(&[("copy", obj(&[("$ref", text("#/allOf/0"))]))]),
            ),
        ])
    );
}

#[test]
fn draft07_target_converts_prefix_items_to_an_items_tuple() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[
            ("type", text("array")),
            (
                "prefixItems",
                arr(&[
                    obj(&[("type", text("string"))]),
                    obj(&[("type", text("number"))]),
                ]),
            ),
            ("items", obj(&[("type", text("boolean"))])),
        ]),
        BTreeMap::new(),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-07")),
            (
                "schema",
                obj(&[
                    ("type", text("array")),
                    (
                        "items",
                        arr(&[
                            obj(&[("type", text("string"))]),
                            obj(&[("type", text("number"))]),
                        ]),
                    ),
                    ("additionalItems", obj(&[("type", text("boolean"))])),
                ]),
            ),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

#[test]
fn draft07_target_preserves_a_single_items_schema() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[
            ("type", text("array")),
            ("items", obj(&[("type", text("string"))])),
        ]),
        BTreeMap::new(),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("array")),
            ("items", obj(&[("type", text("string"))])),
        ])
    );
}

#[test]
fn draft07_target_preserves_custom_properties() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[("type", text("string")), ("x-custom", text("value"))]),
        BTreeMap::new(),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document.schema,
        obj(&[("type", text("string")), ("x-custom", text("value"))])
    );
}

/// `prefixItems` holding a string is malformed, and draft-07's `items` is the
/// slot it would have gone into, so the malformed value lands there rather
/// than being dropped.
#[test]
fn draft07_target_preserves_malformed_values() {
    let document = json_schema::to_document_draft07(&canonical_document(
        obj(&[
            ("$ref", num(1.0)),
            ("properties", text("invalid")),
            ("not", arr(&[flag(false), obj(&[("type", text("string"))])])),
            ("allOf", text("invalid")),
            ("prefixItems", text("invalid")),
        ]),
        BTreeMap::new(),
    ))
    .expect("lowers to draft-07");

    assert_eq!(
        document_json(&document),
        obj(&[
            ("dialect", text("draft-07")),
            (
                "schema",
                obj(&[
                    ("$ref", num(1.0)),
                    ("properties", text("invalid")),
                    ("not", arr(&[flag(false), obj(&[("type", text("string"))])])),
                    ("allOf", text("invalid")),
                    ("items", text("invalid")),
                ]),
            ),
            ("definitions", Json::Obj(BTreeMap::new())),
        ])
    );
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:2916-2952 input immutability + round trip
// ---------------------------------------------------------------------------

/// Each conversion takes its input by shared reference, so the input cannot
/// change; the assertion is that the value is byte-equal afterwards, which is
/// what catches an implementation that clones lazily or writes through.
#[test]
fn from_schema_conversions_do_not_mutate_their_input() {
    let draft07_input = obj(&[
        ("type", text("array")),
        ("items", arr(&[obj(&[("type", text("string"))])])),
        ("additionalItems", flag(false)),
        (
            "definitions",
            obj(&[("Value", obj(&[("type", text("string"))]))]),
        ),
    ]);
    let canonical_input = obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[("value", obj(&[("type", text("string"))]))]),
        ),
    ]);
    let openapi30_input = obj(&[
        ("type", text("array")),
        ("items", obj(&[("type", text("string"))])),
        ("nullable", flag(true)),
    ]);

    for input in [&draft07_input, &canonical_input, &openapi30_input] {
        let before = input.clone();
        let _ = json_schema::from_schema_draft07(input);
        assert_eq!(*input, before, "from_schema_draft07 wrote to its input");

        let before = input.clone();
        let _ = json_schema::from_schema_draft2020_12(input);
        assert_eq!(
            *input, before,
            "from_schema_draft2020_12 wrote to its input"
        );

        let before = input.clone();
        let _ = json_schema::from_schema_openapi3_1(input);
        assert_eq!(*input, before, "from_schema_openapi3_1 wrote to its input");

        let before = input.clone();
        let _ = json_schema::from_schema_openapi3_0(input);
        assert_eq!(*input, before, "from_schema_openapi3_0 wrote to its input");
    }
}

#[test]
fn lowering_does_not_mutate_its_input() {
    let document = canonical_document(
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[("value", obj(&[("type", text("string"))]))]),
            ),
        ]),
        defs(&[("Value", obj(&[("type", text("string"))]))]),
    );
    let before = document.clone();

    let _ = json_schema::to_document_draft07(&document);
    assert_eq!(document, before, "to_document_draft07 wrote to its input");

    let _ = json_schema::to_document_draft04(&document);
    assert_eq!(document, before, "to_document_draft04 wrote to its input");
}

#[test]
fn to_multi_document_open_api3_1_does_not_mutate_its_input() {
    let multi = json_schema::MultiDocument {
        dialect: json_schema::Dialect::Draft2020_12,
        schemas: vec![obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[("value", obj(&[("type", text("string"))]))]),
            ),
        ])],
        definitions: defs(&[("Value", obj(&[("type", text("string"))]))]),
    };
    let before = multi.clone();

    let _ = json_schema::to_multi_document_open_api3_1(&multi);
    assert_eq!(
        multi, before,
        "to_multi_document_open_api3_1 wrote to its input"
    );
}

/// A draft-07 document survives a lift and a lowering: the tuple spelling comes
/// back, the annotations come back, and the reference is re-pointed at
/// `definitions` on the way out.
#[test]
fn a_draft07_schema_round_trips_through_canonical_form() {
    let document = json_schema::from_schema_draft07(&obj(&[
        ("type", text("object")),
        ("readOnly", flag(true)),
        ("writeOnly", flag(true)),
        ("not", obj(&[("required", arr(&[text("forbidden")]))])),
        (
            "properties",
            obj(&[
                ("name", obj(&[("type", text("string"))])),
                (
                    "items",
                    obj(&[
                        ("type", text("array")),
                        (
                            "items",
                            arr(&[
                                obj(&[("type", text("string"))]),
                                obj(&[("type", text("number"))]),
                            ]),
                        ),
                        ("additionalItems", obj(&[("type", text("boolean"))])),
                    ]),
                ),
                ("ref", obj(&[("$ref", text("#/definitions/MyType"))])),
            ]),
        ),
        (
            "definitions",
            obj(&[("MyType", obj(&[("type", text("string"))]))]),
        ),
    ]))
    .expect("draft-07 lifts");

    let back = json_schema::to_document_draft07(&document).expect("2020-12 lowers");

    assert_eq!(
        back.schema,
        obj(&[
            ("type", text("object")),
            ("readOnly", flag(true)),
            ("writeOnly", flag(true)),
            ("not", obj(&[("required", arr(&[text("forbidden")]))])),
            (
                "properties",
                obj(&[
                    ("name", obj(&[("type", text("string"))])),
                    (
                        "items",
                        obj(&[
                            ("type", text("array")),
                            (
                                "items",
                                arr(&[
                                    obj(&[("type", text("string"))]),
                                    obj(&[("type", text("number"))]),
                                ]),
                            ),
                            ("additionalItems", obj(&[("type", text("boolean"))])),
                        ]),
                    ),
                    ("ref", obj(&[("$ref", text("#/definitions/MyType"))]),),
                ]),
            ),
        ])
    );
    assert_eq!(
        back.definitions,
        defs(&[("MyType", obj(&[("type", text("string"))]))])
    );
}
