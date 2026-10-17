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
//!   stays unported. A row whose only assertion is such an outcome, and whose
//!   document uses no keyword the draft-04 reader below cannot read, is
//!   witnessed through that reader instead of being excused.
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

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:156 fromSchemaDraft07 validation semantics
// ---------------------------------------------------------------------------

/// The pin's `makeCanonicalSchema`: the lowered document as a standalone schema,
/// which is the shape it hands the 2020-12 validator.
fn canonical_schema(document: &json_schema::Document) -> Json {
    let Json::Obj(mut map) = document.schema.clone() else {
        panic!("a document root is an object");
    };
    map.insert(
        "$schema".to_string(),
        text(json_schema::META_SCHEMA_URI_DRAFT_2020_12),
    );
    if !document.definitions.is_empty() {
        map.insert("$defs".to_string(), Json::Obj(document.definitions.clone()));
    }
    Json::Obj(map)
}

/// The pin proves this row by compiling the Draft-07 source and the lowered
/// canonical schema and asserting two validators agree on six instances. This
/// port has no validator, so both sides go through [`accepts`]: the source side
/// is checked against the verdicts Draft-07's own rules give those six, which is
/// what stops the canonical side from being one reader agreeing with itself.
#[test]
fn draft07_preserves_validation_semantics_in_the_canonical_dialect() {
    let source = obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                (
                    "kind",
                    obj(&[("enum", arr(&[text("full"), text("compact")]))]),
                ),
                ("value", obj(&[("type", text("string"))])),
                ("extra", flag(true)),
                (
                    "values",
                    obj(&[
                        ("type", text("array")),
                        ("contains", obj(&[("type", text("number"))])),
                    ]),
                ),
            ]),
        ),
        ("required", arr(&[text("kind"), text("values")])),
        ("dependencies", obj(&[("value", arr(&[text("extra")]))])),
        (
            "if",
            obj(&[(
                "properties",
                obj(&[("kind", obj(&[("const", text("full"))]))]),
            )]),
        ),
        ("then", obj(&[("required", arr(&[text("value")]))])),
        (
            "else",
            obj(&[("not", obj(&[("required", arr(&[text("value")]))]))]),
        ),
    ]);

    let document = json_schema::from_schema_draft07(&source).expect("draft-07 lifts");
    let canonical = canonical_schema(&document);

    // The one keyword the canonical dialect spells differently is lowered, and
    // the shared ones are left in place for the reader below to find.
    assert!(!mentions(&canonical, "dependencies"), "{:?}", canonical);
    assert!(mentions(&canonical, "dependentRequired"), "{:?}", canonical);
    assert!(mentions(&canonical, "if"), "{:?}", canonical);
    assert!(mentions(&canonical, "contains"), "{:?}", canonical);

    let cases = [
        // The dependencies step is satisfied, and `then` needs its value.
        (
            obj(&[
                ("kind", text("full")),
                ("value", text("ok")),
                ("extra", flag(true)),
                ("values", arr(&[num(1.0)])),
            ]),
            true,
        ),
        // `then` holds: full mode without a value is refused.
        (
            obj(&[("kind", text("full")), ("values", arr(&[num(1.0)]))]),
            false,
        ),
        // The same instance with no `extra` fails the dependency.
        (
            obj(&[
                ("kind", text("full")),
                ("value", text("missing dependency")),
                ("values", arr(&[num(1.0)])),
            ]),
            false,
        ),
        // `else` holds: no value in compact mode.
        (
            obj(&[("kind", text("compact")), ("values", arr(&[num(1.0)]))]),
            true,
        ),
        (
            obj(&[
                ("kind", text("compact")),
                ("value", text("forbidden")),
                ("extra", flag(true)),
                ("values", arr(&[num(1.0)])),
            ]),
            false,
        ),
        // Nothing in the array is a number, so `contains` refuses.
        (
            obj(&[
                ("kind", text("full")),
                ("value", text("ok")),
                ("extra", flag(true)),
                ("values", arr(&[text("no")])),
            ]),
            false,
        ),
    ];

    for (value, expected) in cases {
        assert_eq!(
            accepts(&document, &source, &value),
            expected,
            "draft-07 source on {:?}",
            value
        );
        assert_eq!(
            accepts(&document, &canonical, &value),
            expected,
            "canonical form on {:?}",
            value
        );
    }
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:1884-2417 toDocumentDraft04
// ---------------------------------------------------------------------------

/// Draft-04 has no two-valued schema: a `false` in a schema position becomes
/// the schema that rejects everything.
fn not_any() -> Json {
    obj(&[("not", obj(&[]))])
}

/// A reader for the rows whose only assertion in the pin is an `ajv` outcome.
/// Those rows exist to prove a conversion keeps meaning, and this port has no
/// validator, so a document is read back through the keywords it actually uses.
/// For the draft-04 rows a reader this small is enough because `contains` and
/// `if`/`then`/`else` arrive already rewritten to `anyOf`/`allOf`/`not`/`items`;
/// the draft-07 lift at :156 keeps them, so they are read here too.
///
/// ponytail: no `oneOf`, no `patternProperties`, no `minContains`/`maxContains`,
/// no `$ref` outside `/definitions/`; add each when a ported row needs it.
fn accepts(document: &json_schema::Document, schema: &Json, instance: &Json) -> bool {
    let map = match schema {
        Json::Obj(map) => map,
        Json::Bool(true) => return true,
        Json::Bool(false) => return false,
        _ => return true,
    };

    if let Some(Json::Str(target)) = map.get("$ref") {
        let name = target
            .strip_prefix("#/definitions/")
            .expect("only definitions refs reach this reader");
        let resolved = document
            .definitions
            .get(name)
            .expect("the ref names a definition of this document");
        if !accepts(document, resolved, instance) {
            return false;
        }
    }

    if let Some(expected) = map.get("type") {
        let matches_type = |name: &str| match (name, instance) {
            ("object", Json::Obj(_)) | ("array", Json::Arr(_)) | ("string", Json::Str(_)) => true,
            ("number", Json::Num(_)) => true,
            ("integer", Json::Num(value)) => value.fract() == 0.0,
            ("boolean", Json::Bool(_)) | ("null", Json::Null) => true,
            _ => false,
        };
        let satisfied = match expected {
            Json::Str(name) => matches_type(name),
            Json::Arr(names) => names.iter().any(|candidate| match candidate {
                Json::Str(name) => matches_type(name),
                _ => false,
            }),
            _ => true,
        };
        if !satisfied {
            return false;
        }
    }

    if let (Some(Json::Obj(fields)), Json::Obj(values)) = (map.get("properties"), instance) {
        for (key, subschema) in fields {
            if let Some(value) = values.get(key) {
                if !accepts(document, subschema, value) {
                    return false;
                }
            }
        }
    }

    if let (Json::Obj(values), Some(Json::Arr(required))) = (instance, map.get("required")) {
        for name in required {
            if let Json::Str(name) = name {
                if !values.contains_key(name) {
                    return false;
                }
            }
        }
    }

    if let Some(Json::Arr(allowed)) = map.get("enum") {
        if !allowed.contains(instance) {
            return false;
        }
    }

    if let Some(expected) = map.get("const") {
        if instance != expected {
            return false;
        }
    }

    // Draft-07 `dependencies` carries both meanings in one keyword: a list of
    // names that must then be present, or a schema the instance must still
    // satisfy. The canonical dialect splits them, so both spellings are read.
    if let (Json::Obj(values), Some(Json::Obj(dependencies))) = (instance, map.get("dependencies"))
    {
        for (key, rule) in dependencies {
            if values.contains_key(key) && !dependency_holds(document, rule, values, instance) {
                return false;
            }
        }
    }

    if let (Json::Obj(values), Some(Json::Obj(required_by))) =
        (instance, map.get("dependentRequired"))
    {
        for (key, names) in required_by {
            if values.contains_key(key) && !dependency_holds(document, names, values, instance) {
                return false;
            }
        }
    }

    if let (Json::Obj(values), Some(Json::Obj(schemas))) = (instance, map.get("dependentSchemas")) {
        for (key, subschema) in schemas {
            if values.contains_key(key) && !accepts(document, subschema, instance) {
                return false;
            }
        }
    }

    if let (Json::Num(value), Some(Json::Num(minimum))) = (instance, map.get("minimum")) {
        let exclusive = matches!(map.get("exclusiveMinimum"), Some(Json::Bool(true)));
        if value < minimum || (exclusive && value == minimum) {
            return false;
        }
    }
    if let (Json::Num(value), Some(Json::Num(maximum))) = (instance, map.get("maximum")) {
        let exclusive = matches!(map.get("exclusiveMaximum"), Some(Json::Bool(true)));
        if value > maximum || (exclusive && value == maximum) {
            return false;
        }
    }
    if let (Json::Num(value), Some(Json::Num(minimum))) = (instance, map.get("exclusiveMinimum")) {
        if value <= minimum {
            return false;
        }
    }
    if let (Json::Num(value), Some(Json::Num(maximum))) = (instance, map.get("exclusiveMaximum")) {
        if value >= maximum {
            return false;
        }
    }
    if let (Json::Str(value), Some(Json::Num(minimum))) = (instance, map.get("minLength")) {
        if (value.chars().count() as f64) < *minimum {
            return false;
        }
    }

    if let (Json::Arr(items), Some(subschema)) = (instance, map.get("items")) {
        match subschema {
            Json::Arr(tuple) => {
                for (index, entry) in items.iter().enumerate() {
                    match tuple.get(index) {
                        Some(subschema) => {
                            if !accepts(document, subschema, entry) {
                                return false;
                            }
                        }
                        None => {
                            if let Some(rest) = map.get("additionalItems") {
                                if !accepts(document, rest, entry) {
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                for entry in items {
                    if !accepts(document, subschema, entry) {
                        return false;
                    }
                }
            }
        }
    }

    if let (Json::Arr(items), Some(subschema)) = (instance, map.get("contains")) {
        if !items.iter().any(|item| accepts(document, subschema, item)) {
            return false;
        }
    }

    if let Some(condition) = map.get("if") {
        let branch = if accepts(document, condition, instance) {
            map.get("then")
        } else {
            map.get("else")
        };
        if let Some(branch) = branch {
            if !accepts(document, branch, instance) {
                return false;
            }
        }
    }

    for keyword in ["allOf", "anyOf"] {
        if let Some(Json::Arr(branches)) = map.get(keyword) {
            let satisfied = if keyword == "allOf" {
                branches
                    .iter()
                    .all(|branch| accepts(document, branch, instance))
            } else {
                branches
                    .iter()
                    .any(|branch| accepts(document, branch, instance))
            };
            if !satisfied {
                return false;
            }
        }
    }

    if let Some(negated) = map.get("not") {
        if accepts(document, negated, instance) {
            return false;
        }
    }

    true
}

/// A dependency rule is either a list of property names that must then be
/// present, or a schema the whole instance must then still satisfy.
fn dependency_holds(
    document: &json_schema::Document,
    rule: &Json,
    values: &BTreeMap<String, Json>,
    instance: &Json,
) -> bool {
    match rule {
        Json::Arr(names) => names.iter().all(|name| match name {
            Json::Str(name) => values.contains_key(name),
            _ => true,
        }),
        _ => accepts(document, rule, instance),
    }
}

/// Does any position of this document still spell `keyword`?
fn mentions(schema: &Json, keyword: &str) -> bool {
    match schema {
        Json::Obj(map) => {
            map.contains_key(keyword) || map.values().any(|value| mentions(value, keyword))
        }
        Json::Arr(items) => items.iter().any(|item| mentions(item, keyword)),
        _ => false,
    }
}

#[test]
fn draft04_rewrites_the_canonical_meta_schema_uri_with_an_empty_fragment() {
    let schema = obj(&[(
        "$schema",
        text(&format!("{}#", json_schema::META_SCHEMA_URI_DRAFT_2020_12)),
    )]);

    let document = json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-04");

    assert_eq!(
        document.schema,
        obj(&[("$schema", text(json_schema::META_SCHEMA_URI_DRAFT_04))])
    );
}

#[test]
fn draft04_omits_the_canonical_meta_schema_uri_from_embedded_resources() {
    let document = canonical_document(
        obj(&[]),
        defs(&[(
            "Embedded",
            obj(&[
                ("$id", text("embedded.json")),
                ("$schema", text(json_schema::META_SCHEMA_URI_DRAFT_2020_12)),
                ("type", text("string")),
            ]),
        )]),
    );

    let lowered = json_schema::to_document_draft04(&document).expect("2020-12 lowers");

    // The canonical URI becomes the legacy `id`, because draft-04 spells a
    // resource identifier that way and only the root may declare a dialect.
    assert_eq!(
        lowered.definitions.get("Embedded"),
        Some(&obj(&[
            ("id", text("embedded.json")),
            ("type", text("string")),
        ]))
    );
}

#[test]
fn draft04_rejects_custom_dialects_in_embedded_resources() {
    let document = canonical_document(
        obj(&[]),
        defs(&[(
            "Embedded",
            obj(&[
                ("$id", text("embedded.json")),
                ("$schema", text("https://example.com/dialect")),
                ("type", text("string")),
            ]),
        )]),
    );

    let message = failure_message(json_schema::to_document_draft04(&document));

    assert!(
        message.starts_with("Cannot convert JSON Schema keyword \"$schema\" to Draft-04"),
        "{message}"
    );
}

#[test]
fn draft04_lowers_dependencies_and_keeps_opaque_custom_keywords() {
    let schema = obj(&[
        (
            "dependentRequired",
            obj(&[("enabled", arr(&[text("value")]))]),
        ),
        (
            "dependentSchemas",
            obj(&[("mode", obj(&[("required", arr(&[text("value")]))]))]),
        ),
        ("x-custom", obj(&[("value", flag(true))])),
    ]);

    let document = json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-04");

    // A key that only requires others stays a list; one that carries a schema
    // stays a schema. Custom keywords ride through untouched.
    assert_eq!(
        document.schema,
        obj(&[
            (
                "dependencies",
                obj(&[
                    ("enabled", arr(&[text("value")])),
                    ("mode", obj(&[("required", arr(&[text("value")]))])),
                ]),
            ),
            ("x-custom", obj(&[("value", flag(true))])),
        ])
    );
}

#[test]
fn draft04_rewrites_defs_refs_to_definitions_refs() {
    let document = canonical_document(
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[("value", obj(&[("$ref", text("#/$defs/Value"))]))]),
            ),
        ]),
        defs(&[("Value", obj(&[("type", text("string"))]))]),
    );

    let lowered = json_schema::to_document_draft04(&document).expect("2020-12 lowers");

    assert_eq!(
        document_json(&lowered),
        obj(&[
            ("dialect", text("draft-04")),
            (
                "schema",
                obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[("value", obj(&[("$ref", text("#/definitions/Value"))]))]),
                    ),
                ]),
            ),
            (
                "definitions",
                obj(&[("Value", obj(&[("type", text("string"))]))]),
            ),
        ])
    );
}

#[test]
fn draft04_preserves_supported_keywords_and_newer_annotations_as_extensions() {
    let schema = obj(&[
        ("type", text("object")),
        ("required", arr(&[text("value")])),
        ("enum", arr(&[text("a"), text("b")])),
        ("title", text("title")),
        ("description", text("description")),
        ("default", text("a")),
        ("format", text("custom")),
        ("pattern", text("^a$")),
        ("minimum", num(0.0)),
        ("maximum", num(10.0)),
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
        ("allOf", arr(&[obj(&[("type", text("object"))])])),
        ("anyOf", arr(&[obj(&[("type", text("string"))])])),
        ("oneOf", arr(&[obj(&[("type", text("number"))])])),
        ("items", obj(&[("type", text("string"))])),
        ("examples", arr(&[text("a")])),
        ("readOnly", flag(true)),
        ("writeOnly", flag(true)),
    ]);

    let document = json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-04");

    // Everything the target dialect knows survives verbatim; only the `false`
    // in a schema position is respelled.
    assert_eq!(
        document.schema,
        obj(&[
            ("type", text("object")),
            ("required", arr(&[text("value")])),
            ("enum", arr(&[text("a"), text("b")])),
            ("title", text("title")),
            ("description", text("description")),
            ("default", text("a")),
            ("format", text("custom")),
            ("pattern", text("^a$")),
            ("minimum", num(0.0)),
            ("maximum", num(10.0)),
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
            ("additionalProperties", not_any()),
            ("allOf", arr(&[obj(&[("type", text("object"))])])),
            ("anyOf", arr(&[obj(&[("type", text("string"))])])),
            ("oneOf", arr(&[obj(&[("type", text("number"))])])),
            ("items", obj(&[("type", text("string"))])),
            ("examples", arr(&[text("a")])),
            ("readOnly", flag(true)),
            ("writeOnly", flag(true)),
        ])
    );
}

#[test]
fn draft04_rejects_validation_keywords_it_cannot_represent() {
    let refused = [
        obj(&[("propertyNames", obj(&[("minLength", num(1.0))]))]),
        obj(&[
            ("contains", obj(&[("type", text("string"))])),
            ("minContains", num(0.0)),
        ]),
        obj(&[
            ("contains", obj(&[("type", text("string"))])),
            ("minContains", num(2.0)),
        ]),
        obj(&[
            ("if", obj(&[("$id", text("condition"))])),
            ("then", obj(&[])),
            ("else", obj(&[])),
        ]),
        obj(&[("unevaluatedProperties", flag(false))]),
        obj(&[("$dynamicRef", text("#node"))]),
        obj(&[(
            "$vocabulary",
            obj(&[("https://example.com/vocabulary", flag(true))]),
        )]),
        obj(&[("id", text("legacy-id"))]),
    ];

    for schema in refused {
        let document = canonical_document(schema.clone(), BTreeMap::new());
        let message = failure_message(json_schema::to_document_draft04(&document));
        assert!(
            message.starts_with("Cannot convert JSON Schema keyword \"")
                && message.contains("to Draft-04"),
            "{schema:?} => {message}"
        );
    }
}

#[test]
fn draft04_converts_inactive_conditional_schema_bodies() {
    // A conditional body is a schema position even when its keyword is inert,
    // so its `$defs` moves and the pointer into it follows.
    for key in ["if", "then", "else"] {
        let schema = obj(&[
            (
                key,
                obj(&[("$defs", obj(&[("Value", obj(&[("const", text("value"))]))]))]),
            ),
            (
                "allOf",
                arr(&[obj(&[("$ref", text(&format!("#/{key}/$defs/Value")))])]),
            ),
        ]);

        let document =
            json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
                .expect("2020-12 lowers to draft-04");

        assert_eq!(
            document.schema,
            obj(&[
                (
                    key,
                    obj(&[(
                        "definitions",
                        obj(&[("Value", obj(&[("enum", arr(&[text("value")]))]))]),
                    )]),
                ),
                (
                    "allOf",
                    arr(&[obj(&[(
                        "$ref",
                        text(&format!("#/{key}/definitions/Value")),
                    )])]),
                ),
            ]),
            "{key}"
        );
    }
}

#[test]
fn draft04_lowers_conditionals_and_contains_while_preserving_semantics() {
    let schema = obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                (
                    "mode",
                    obj(&[("enum", arr(&[text("full"), text("compact")]))]),
                ),
                ("value", obj(&[("type", text("string"))])),
                (
                    "values",
                    obj(&[("contains", obj(&[("type", text("number"))]))]),
                ),
            ]),
        ),
        ("required", arr(&[text("mode"), text("values")])),
        (
            "if",
            obj(&[(
                "properties",
                obj(&[("mode", obj(&[("const", text("full"))]))]),
            )]),
        ),
        ("then", obj(&[("required", arr(&[text("value")]))])),
        (
            "else",
            obj(&[("not", obj(&[("required", arr(&[text("value")]))]))]),
        ),
    ]);

    let document = json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-04");

    assert!(!mentions(&document.schema, "if"), "{:?}", document.schema);
    assert!(
        !mentions(&document.schema, "contains"),
        "{:?}",
        document.schema
    );
    assert!(
        !mentions(&document.schema, "const"),
        "{:?}",
        document.schema
    );

    let full_with_value = obj(&[
        ("mode", text("full")),
        ("value", text("ok")),
        ("values", arr(&[num(1.0)])),
    ]);
    assert!(accepts(&document, &document.schema, &full_with_value));

    // `then` holds: the full mode needs a value.
    let full_without_value = obj(&[("mode", text("full")), ("values", arr(&[num(1.0)]))]);
    assert!(!accepts(&document, &document.schema, &full_without_value));

    let compact = obj(&[("mode", text("compact")), ("values", arr(&[num(1.0)]))]);
    assert!(accepts(&document, &document.schema, &compact));

    // `else` holds: the compact mode must not carry one.
    let compact_with_value = obj(&[
        ("mode", text("compact")),
        ("value", text("unexpected")),
        ("values", arr(&[num(1.0)])),
    ]);
    assert!(!accepts(&document, &document.schema, &compact_with_value));

    let no_contained_number = obj(&[
        ("mode", text("full")),
        ("value", text("ok")),
        ("values", arr(&[text("no")])),
    ]);
    assert!(!accepts(&document, &document.schema, &no_contained_number));

    // `contains` says nothing about a non-array, and the rewrite keeps that.
    let scalar_values = obj(&[
        ("mode", text("full")),
        ("value", text("ok")),
        ("values", num(1.0)),
    ]);
    assert!(accepts(&document, &document.schema, &scalar_values));
}

#[test]
fn draft04_omits_an_empty_required_array() {
    let schema = obj(&[("type", text("object")), ("required", arr(&[]))]);

    let document = json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-04");

    // Draft-04's meta-schema will not accept an empty `required`.
    assert_eq!(document.schema, obj(&[("type", text("object"))]));
}

#[test]
fn draft04_omits_empty_dependent_required_arrays() {
    let empty = canonical_document(
        obj(&[("dependentRequired", obj(&[("value", arr(&[]))]))]),
        BTreeMap::new(),
    );
    assert_eq!(
        json_schema::to_document_draft04(&empty)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[])
    );

    // With a schema dependency on the same key, the empty list drops and the
    // schema survives alone.
    let combined = canonical_document(
        obj(&[
            ("dependentRequired", obj(&[("value", arr(&[]))])),
            (
                "dependentSchemas",
                obj(&[("value", obj(&[("type", text("string"))]))]),
            ),
        ]),
        BTreeMap::new(),
    );
    assert_eq!(
        json_schema::to_document_draft04(&combined)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[(
            "dependencies",
            obj(&[("value", obj(&[("type", text("string"))]))]),
        )])
    );
}

#[test]
fn draft04_converts_const_to_enum() {
    let with_enum = canonical_document(
        obj(&[
            ("enum", arr(&[text("a"), text("b")])),
            ("const", text("b")),
            ("allOf", arr(&[obj(&[("type", text("string"))])])),
        ]),
        BTreeMap::new(),
    );
    // Beside an existing `enum` the constraint cannot be merged, so it keeps
    // its meaning as another `allOf` member.
    assert_eq!(
        json_schema::to_document_draft04(&with_enum)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[
            ("enum", arr(&[text("a"), text("b")])),
            (
                "allOf",
                arr(&[
                    obj(&[("type", text("string"))]),
                    obj(&[("enum", arr(&[text("b")]))]),
                ]),
            ),
        ])
    );

    let only_const = canonical_document(obj(&[("const", text("a"))]), BTreeMap::new());
    assert_eq!(
        json_schema::to_document_draft04(&only_const)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[("enum", arr(&[text("a")]))])
    );

    let enum_and_const = canonical_document(
        obj(&[("enum", arr(&[text("a"), text("b")])), ("const", text("b"))]),
        BTreeMap::new(),
    );
    assert_eq!(
        json_schema::to_document_draft04(&enum_and_const)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[
            ("enum", arr(&[text("a"), text("b")])),
            ("allOf", arr(&[obj(&[("enum", arr(&[text("b")]))])])),
        ])
    );
}

#[test]
fn draft04_preserves_refs_in_literal_values() {
    // A `$ref` inside a literal is data, not a reference, so it must not move
    // with the definitions it looks like it points at.
    let literal = obj(&[("$ref", text("#/$defs/Literal"))]);

    let only_const = canonical_document(obj(&[("const", literal.clone())]), BTreeMap::new());
    assert_eq!(
        json_schema::to_document_draft04(&only_const)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[("enum", arr(&[literal.clone()]))])
    );

    let enum_and_default = canonical_document(
        obj(&[
            ("enum", arr(&[literal.clone()])),
            ("default", literal.clone()),
        ]),
        BTreeMap::new(),
    );
    assert_eq!(
        json_schema::to_document_draft04(&enum_and_default)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[("enum", arr(&[literal.clone()])), ("default", literal),])
    );
}

#[test]
fn draft04_converts_numeric_exclusive_bounds_to_boolean_exclusivity() {
    let cases = [
        (obj(&[("minimum", num(1.0))]), obj(&[("minimum", num(1.0))])),
        (
            obj(&[("exclusiveMinimum", num(1.0))]),
            obj(&[("minimum", num(1.0)), ("exclusiveMinimum", flag(true))]),
        ),
        (
            obj(&[("minimum", num(2.0)), ("exclusiveMinimum", num(1.0))]),
            obj(&[("minimum", num(2.0))]),
        ),
        (
            obj(&[("minimum", num(1.0)), ("exclusiveMinimum", num(1.0))]),
            obj(&[("minimum", num(1.0)), ("exclusiveMinimum", flag(true))]),
        ),
        (
            obj(&[("minimum", num(1.0)), ("exclusiveMinimum", num(2.0))]),
            obj(&[("minimum", num(2.0)), ("exclusiveMinimum", flag(true))]),
        ),
        (obj(&[("maximum", num(2.0))]), obj(&[("maximum", num(2.0))])),
        (
            obj(&[("exclusiveMaximum", num(2.0))]),
            obj(&[("maximum", num(2.0)), ("exclusiveMaximum", flag(true))]),
        ),
        (
            obj(&[("maximum", num(1.0)), ("exclusiveMaximum", num(2.0))]),
            obj(&[("maximum", num(1.0))]),
        ),
        (
            obj(&[("maximum", num(2.0)), ("exclusiveMaximum", num(2.0))]),
            obj(&[("maximum", num(2.0)), ("exclusiveMaximum", flag(true))]),
        ),
        (
            obj(&[("maximum", num(2.0)), ("exclusiveMaximum", num(1.0))]),
            obj(&[("maximum", num(1.0)), ("exclusiveMaximum", flag(true))]),
        ),
    ];

    for (schema, expected) in cases {
        let document = canonical_document(schema.clone(), BTreeMap::new());
        assert_eq!(
            json_schema::to_document_draft04(&document)
                .expect("2020-12 lowers to draft-04")
                .schema,
            expected,
            "{schema:?}"
        );
    }
}

#[test]
fn draft04_converts_boolean_schemas_only_in_schema_positions() {
    let document = canonical_document(
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[
                    ("allowed", flag(true)),
                    ("denied", flag(false)),
                    ("nested", obj(&[("not", flag(false))])),
                ]),
            ),
            ("additionalProperties", flag(false)),
            ("allOf", arr(&[flag(true)])),
            ("anyOf", arr(&[flag(false)])),
        ]),
        BTreeMap::new(),
    );

    assert_eq!(
        json_schema::to_document_draft04(&document)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[
                    ("allowed", obj(&[])),
                    ("denied", not_any()),
                    ("nested", obj(&[("not", not_any())])),
                ]),
            ),
            ("additionalProperties", not_any()),
            ("allOf", arr(&[obj(&[])])),
            ("anyOf", arr(&[not_any()])),
        ])
    );
}

#[test]
fn draft04_converts_tuple_members_and_the_trailing_boolean_schema() {
    let tuple = canonical_document(
        obj(&[
            ("type", text("array")),
            ("prefixItems", arr(&[flag(true), flag(false)])),
            ("items", flag(false)),
        ]),
        BTreeMap::new(),
    );
    assert_eq!(
        json_schema::to_document_draft04(&tuple)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[
            ("type", text("array")),
            ("items", arr(&[obj(&[]), not_any()])),
            ("additionalItems", not_any()),
        ])
    );

    let single = canonical_document(
        obj(&[("type", text("array")), ("items", flag(false))]),
        BTreeMap::new(),
    );
    assert_eq!(
        json_schema::to_document_draft04(&single)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[("type", text("array")), ("items", not_any())])
    );
}

#[test]
fn draft04_converts_schemas_in_additional_properties_and_additional_items() {
    let document = canonical_document(
        obj(&[
            ("type", text("object")),
            ("additionalProperties", obj(&[("const", text("value"))])),
            (
                "properties",
                obj(&[(
                    "tuple",
                    obj(&[
                        ("type", text("array")),
                        ("prefixItems", arr(&[obj(&[("type", text("string"))])])),
                        ("items", obj(&[("const", text("rest"))])),
                    ]),
                )]),
            ),
        ]),
        BTreeMap::new(),
    );

    assert_eq!(
        json_schema::to_document_draft04(&document)
            .expect("2020-12 lowers to draft-04")
            .schema,
        obj(&[
            ("type", text("object")),
            (
                "additionalProperties",
                obj(&[("enum", arr(&[text("value")]))]),
            ),
            (
                "properties",
                obj(&[(
                    "tuple",
                    obj(&[
                        ("type", text("array")),
                        ("items", arr(&[obj(&[("type", text("string"))])])),
                        ("additionalItems", obj(&[("enum", arr(&[text("rest")]))])),
                    ]),
                )]),
            ),
        ])
    );
}

#[test]
fn draft04_preserves_malformed_values_for_recognized_keywords() {
    let malformed = obj(&[
        ("properties", text("invalid")),
        ("not", arr(&[num(1.0)])),
        ("allOf", text("invalid")),
    ]);

    let document =
        json_schema::to_document_draft04(&canonical_document(malformed.clone(), BTreeMap::new()))
            .expect("2020-12 lowers to draft-04");

    // A value the keyword does not accept is another document's problem; the
    // lowering carries it rather than inventing a schema.
    assert_eq!(document.schema, malformed);
}

#[test]
fn draft04_preserves_all_of_not_and_null() {
    let schema = obj(&[(
        "anyOf",
        arr(&[
            obj(&[("type", text("null"))]),
            obj(&[(
                "allOf",
                arr(&[obj(&[("not", obj(&[("type", text("string"))]))])]),
            )]),
        ]),
    )]);

    let document =
        json_schema::to_document_draft04(&canonical_document(schema.clone(), BTreeMap::new()))
            .expect("2020-12 lowers to draft-04");

    assert_eq!(document.schema, schema);
}

#[test]
fn draft04_preserves_newer_annotations_as_extensions() {
    let schema = obj(&[
        ("type", text("object")),
        ("examples", arr(&[obj(&[("value", num(1.0))])])),
    ]);

    let document =
        json_schema::to_document_draft04(&canonical_document(schema.clone(), BTreeMap::new()))
            .expect("2020-12 lowers to draft-04");

    assert_eq!(document.schema, schema);
}

#[test]
fn draft04_preserves_constraints_next_to_refs() {
    let document = canonical_document(
        obj(&[("$ref", text("#/$defs/S")), ("minLength", num(3.0))]),
        defs(&[("S", obj(&[("type", text("string"))]))]),
    );

    let lowered = json_schema::to_document_draft04(&document).expect("2020-12 lowers");

    // Draft-04 ignores sibling keywords beside `$ref`, so the reference has to
    // move into an `allOf` member for the constraint to keep its meaning.
    assert_eq!(
        document_json(&lowered),
        obj(&[
            ("dialect", text("draft-04")),
            (
                "schema",
                obj(&[
                    ("allOf", arr(&[obj(&[("$ref", text("#/definitions/S"))])])),
                    ("minLength", num(3.0)),
                ]),
            ),
            (
                "definitions",
                obj(&[("S", obj(&[("type", text("string"))]))]),
            ),
        ])
    );

    let lowered_schema = lowered.schema.clone();
    assert!(accepts(&lowered, &lowered_schema, &text("abc")));
    assert!(!accepts(&lowered, &lowered_schema, &text("a")));
}

#[test]
fn draft04_preserves_validation_semantics_for_converted_constraints() {
    let schema = obj(&[
        ("type", text("object")),
        (
            "properties",
            obj(&[
                ("mode", obj(&[("const", text("on"))])),
                (
                    "value",
                    obj(&[
                        ("type", text("number")),
                        ("exclusiveMinimum", num(0.0)),
                        ("exclusiveMaximum", num(2.0)),
                    ]),
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

    let document = json_schema::to_document_draft04(&canonical_document(schema, BTreeMap::new()))
        .expect("2020-12 lowers to draft-04");

    assert!(
        !mentions(&document.schema, "const"),
        "{:?}",
        document.schema
    );

    let valid = obj(&[
        ("mode", text("on")),
        ("value", num(1.0)),
        ("tuple", arr(&[text("a")])),
    ]);
    assert!(accepts(&document, &document.schema, &valid));

    let wrong_mode = obj(&[
        ("mode", text("off")),
        ("value", num(1.0)),
        ("tuple", arr(&[text("a")])),
    ]);
    assert!(!accepts(&document, &document.schema, &wrong_mode));

    // The exclusive bound survives as a boolean beside the inclusive one.
    let zero_value = obj(&[
        ("mode", text("on")),
        ("value", num(0.0)),
        ("tuple", arr(&[text("a")])),
    ]);
    assert!(!accepts(&document, &document.schema, &zero_value));

    let extra_tuple_member = obj(&[
        ("mode", text("on")),
        ("value", num(1.0)),
        ("tuple", arr(&[text("a"), text("b")])),
    ]);
    assert!(!accepts(&document, &document.schema, &extra_tuple_member));
}

// ---------------------------------------------------------------------------
// JsonSchema.test.ts:2445-2842 toMultiDocumentOpenApi3_1
// ---------------------------------------------------------------------------

fn canonical_multi_document(
    schemas: Vec<Json>,
    definitions: BTreeMap<String, Json>,
) -> json_schema::MultiDocument {
    json_schema::MultiDocument {
        dialect: json_schema::Dialect::Draft2020_12,
        schemas,
        definitions,
    }
}

/// The pin's `deepStrictEqual(result, { dialect, schemas, definitions })`.
fn multi_document_json(document: &json_schema::MultiDocument) -> Json {
    obj(&[
        ("dialect", text(document.dialect.as_str())),
        ("schemas", Json::Arr(document.schemas.clone())),
        ("definitions", Json::Obj(document.definitions.clone())),
    ])
}

fn multi_failure(
    result: Result<json_schema::MultiDocument, json_schema::ConversionError>,
) -> String {
    result.expect_err("the conversion is refused").0
}

#[test]
fn multi_rewrites_defs_refs_to_component_refs() {
    let document = canonical_multi_document(
        vec![obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[
                    ("a", obj(&[("$ref", text("#/$defs/A"))])),
                    ("b", obj(&[("$ref", text("#/$defs/B"))])),
                ]),
            ),
        ])],
        defs(&[
            (
                "A",
                obj(&[("type", text("string")), ("$ref", text("#/$defs/B"))]),
            ),
            ("B", obj(&[("type", text("number"))])),
        ]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    // The shared pool becomes `components/schemas`, and a reference inside the
    // pool is rewritten too.
    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[
                            ("a", obj(&[("$ref", text("#/components/schemas/A"))])),
                            ("b", obj(&[("$ref", text("#/components/schemas/B"))])),
                        ]),
                    ),
                ])]),
            ),
            (
                "definitions",
                obj(&[
                    (
                        "A",
                        obj(&[
                            ("type", text("string")),
                            ("$ref", text("#/components/schemas/B")),
                        ]),
                    ),
                    ("B", obj(&[("type", text("number"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn multi_rejects_opaque_keywords_open_api3_1_would_activate() {
    let refused = [
        (vec![obj(&[("example", text("value"))])], BTreeMap::new()),
        (
            vec![obj(&[(
                "properties",
                obj(&[("value", obj(&[("discriminator", obj(&[]))]))]),
            )])],
            BTreeMap::new(),
        ),
        (
            vec![obj(&[])],
            defs(&[("Value", obj(&[("xml", obj(&[]))]))]),
        ),
        (vec![obj(&[("externalDocs", obj(&[]))])], BTreeMap::new()),
    ];

    for (schemas, definitions) in refused {
        let document = canonical_multi_document(schemas, definitions);
        let message = multi_failure(json_schema::to_multi_document_open_api3_1(&document));
        assert!(
            message.starts_with("Cannot convert JSON Schema keyword \"")
                && message.contains("to OpenAPI 3.1"),
            "{message}"
        );
    }
}

#[test]
fn multi_does_not_inspect_opaque_values_for_keyword_collisions() {
    // The same four keywords, but as *data* under `const`, where OpenAPI 3.1
    // activates nothing.
    let literal = obj(&[
        ("example", text("value")),
        ("discriminator", obj(&[])),
        ("xml", obj(&[])),
        ("externalDocs", obj(&[])),
    ]);
    let document =
        canonical_multi_document(vec![obj(&[("const", literal.clone())])], BTreeMap::new());

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    assert_eq!(converted.schemas, vec![obj(&[("const", literal)])]);
}

#[test]
fn multi_rewrites_refs_only_in_schema_positions() {
    let literal = obj(&[("$ref", text("#/$defs/Literal"))]);
    let document = canonical_multi_document(
        vec![obj(&[
            (
                "properties",
                obj(&[("value", obj(&[("$ref", text("#/$defs/Value"))]))]),
            ),
            ("const", literal.clone()),
            ("enum", arr(&[literal.clone()])),
            ("default", literal.clone()),
            ("examples", arr(&[literal.clone()])),
        ])],
        defs(&[("Value", obj(&[("type", text("string"))]))]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[
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
                    ("examples", arr(&[literal])),
                ])]),
            ),
            (
                "definitions",
                obj(&[("Value", obj(&[("type", text("string"))]))]),
            ),
        ])
    );
}

#[test]
fn multi_sanitizes_component_keys_and_rewritten_refs_together() {
    let document = canonical_multi_document(
        vec![obj(&[
            ("type", text("object")),
            (
                "properties",
                obj(&[("A.B", obj(&[("$ref", text("#/$defs/A$B"))]))]),
            ),
        ])],
        defs(&[
            ("A$B", obj(&[("$ref", text("#/$defs/B$C"))])),
            ("B$C", obj(&[("type", text("string"))])),
        ]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    // The property name is not a component key, so it keeps its dot; the
    // definition keys and every reference to them move together.
    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[
                    ("type", text("object")),
                    (
                        "properties",
                        obj(&[("A.B", obj(&[("$ref", text("#/components/schemas/A_B"))]),)]),
                    ),
                ])]),
            ),
            (
                "definitions",
                obj(&[
                    ("A_B", obj(&[("$ref", text("#/components/schemas/B_C"))]),),
                    ("B_C", obj(&[("type", text("string"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn multi_unescapes_a_definition_key_before_sanitizing_its_ref() {
    // `~1` is an escaped `/` in a JSON Pointer, so the reference names the key
    // `A/B`, which sanitizes to `A_B`.
    let document = canonical_multi_document(
        vec![obj(&[("$ref", text("#/$defs/A~1B"))])],
        defs(&[("A/B", obj(&[("type", text("string"))]))]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[("$ref", text("#/components/schemas/A_B"))])]),
            ),
            (
                "definitions",
                obj(&[("A_B", obj(&[("type", text("string"))]))]),
            ),
        ])
    );
}

#[test]
fn multi_decodes_a_definition_key_before_sanitizing_its_ref() {
    let document = canonical_multi_document(
        vec![obj(&[(
            "allOf",
            arr(&[
                obj(&[("$ref", text("#/$defs/A%20B"))]),
                obj(&[("$dynamicRef", text("#/$defs/A%20B"))]),
            ]),
        )])],
        defs(&[("A B", obj(&[("type", text("string"))]))]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    // Percent-decoding happens before sanitizing, so the space becomes an
    // underscore and `$dynamicRef` is rewritten beside `$ref`.
    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("#/components/schemas/A_B"))]),
                        obj(&[("$dynamicRef", text("#/components/schemas/A_B"))]),
                    ]),
                )])]),
            ),
            (
                "definitions",
                obj(&[("A_B", obj(&[("type", text("string"))]))]),
            ),
        ])
    );
}

/// The `$defs` an embedded resource declares belong to that resource, so a
/// pointer that reaches them must not be sent to the shared pool.
fn embedded_resource_document() -> json_schema::MultiDocument {
    canonical_multi_document(
        vec![obj(&[(
            "allOf",
            arr(&[
                obj(&[("$dynamicRef", text("#/$defs/Embedded"))]),
                obj(&[
                    ("$id", text("inline.json")),
                    ("$defs", obj(&[("Inner", obj(&[("type", text("string"))]))])),
                    ("$dynamicRef", text("#/$defs/Inner")),
                ]),
            ]),
        )])],
        defs(&[(
            "Embedded",
            obj(&[
                ("$id", text("embedded.json")),
                ("$defs", obj(&[("Inner", obj(&[("type", text("number"))]))])),
                (
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("#/$defs/Inner"))]),
                        obj(&[("$dynamicRef", text("#/$defs/Inner"))]),
                    ]),
                ),
            ]),
        )]),
    )
}

#[test]
fn multi_does_not_rewrite_local_refs_inside_embedded_schema_resources() {
    let converted = json_schema::to_multi_document_open_api3_1(&embedded_resource_document())
        .expect("2020-12 lowers to openapi 3.1");

    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$dynamicRef", text("#/components/schemas/Embedded"))]),
                        obj(&[
                            ("$id", text("inline.json")),
                            ("$defs", obj(&[("Inner", obj(&[("type", text("string"))]))]),),
                            ("$dynamicRef", text("#/$defs/Inner")),
                        ]),
                    ]),
                )])]),
            ),
            (
                "definitions",
                obj(&[(
                    "Embedded",
                    obj(&[
                        ("$id", text("embedded.json")),
                        ("$defs", obj(&[("Inner", obj(&[("type", text("number"))]))]),),
                        (
                            "allOf",
                            arr(&[
                                obj(&[("$ref", text("#/$defs/Inner"))]),
                                obj(&[("$dynamicRef", text("#/$defs/Inner"))]),
                            ]),
                        ),
                    ]),
                )]),
            ),
        ])
    );
}

#[test]
fn multi_does_not_rewrite_local_refs_inside_root_schema_resources() {
    let schema = obj(&[
        ("$id", text("inline.json")),
        ("$defs", obj(&[("Inner", obj(&[("type", text("string"))]))])),
        (
            "allOf",
            arr(&[
                obj(&[("$ref", text("#/$defs/Inner"))]),
                obj(&[("$dynamicRef", text("#/$defs/Inner"))]),
            ]),
        ),
    ]);

    let converted = json_schema::to_multi_document_open_api3_1(&canonical_multi_document(
        vec![schema.clone()],
        BTreeMap::new(),
    ))
    .expect("2020-12 lowers to openapi 3.1");

    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            ("schemas", arr(&[schema])),
            ("definitions", obj(&[])),
        ])
    );
}

#[test]
fn multi_rejects_shared_definition_refs_from_root_schema_resources() {
    for keyword in ["$ref", "$dynamicRef"] {
        let schema = obj(&[
            ("$id", text("inline.json")),
            (keyword, text("#/$defs/Value")),
        ]);
        let document = canonical_multi_document(
            vec![schema],
            defs(&[("Value", obj(&[("type", text("string"))]))]),
        );

        let message = multi_failure(json_schema::to_multi_document_open_api3_1(&document));

        assert!(
            message.starts_with(&format!(
                "Cannot convert JSON Schema keyword \"{keyword}\" to OpenAPI 3.1"
            )),
            "{message}"
        );
    }
}

#[test]
fn multi_suffixes_a_sanitized_key_that_collides_with_a_valid_key() {
    let document = canonical_multi_document(
        vec![obj(&[(
            "allOf",
            arr(&[
                obj(&[("$ref", text("#/$defs/A_B"))]),
                obj(&[("$ref", text("#/$defs/A~1B"))]),
            ]),
        )])],
        defs(&[
            ("A_B", obj(&[("type", text("string"))])),
            ("A/B", obj(&[("type", text("number"))])),
        ]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    // `A_B` is already taken by a key that never needed sanitizing, so the
    // sanitized `A/B` takes the first free suffix.
    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("#/components/schemas/A_B"))]),
                        obj(&[("$ref", text("#/components/schemas/A_B_1"))]),
                    ]),
                )])]),
            ),
            (
                "definitions",
                obj(&[
                    ("A_B", obj(&[("type", text("string"))])),
                    ("A_B_1", obj(&[("type", text("number"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn multi_allocates_suffixes_deterministically_and_skips_occupied_keys() {
    let incoming = arr(&[
        obj(&[("$ref", text("#/$defs/A_B"))]),
        obj(&[("$ref", text("#/$defs/A_B_1"))]),
        obj(&[("$ref", text("#/$defs/A~1B"))]),
        obj(&[("$ref", text("#/$defs/A?B"))]),
    ]);
    // `A_B` and `A_B_1` are keys that never needed sanitizing, so the two
    // invalid keys take the suffixes after them.
    let expected_refs = arr(&[
        obj(&[("$ref", text("#/components/schemas/A_B"))]),
        obj(&[("$ref", text("#/components/schemas/A_B_1"))]),
        obj(&[("$ref", text("#/components/schemas/A_B_2"))]),
        obj(&[("$ref", text("#/components/schemas/A_B_3"))]),
    ]);
    let expected = obj(&[
        ("dialect", text("openapi-3.1")),
        ("schemas", arr(&[obj(&[("allOf", expected_refs.clone())])])),
        (
            "definitions",
            obj(&[
                ("A_B", obj(&[("type", text("string"))])),
                ("A_B_1", obj(&[("type", text("boolean"))])),
                ("A_B_2", obj(&[("type", text("number"))])),
                ("A_B_3", obj(&[("type", text("null"))])),
            ]),
        ),
    ]);

    // The pin builds the same key set in two orders; the port's definitions
    // map is sorted, so both calls arrive identical and the assertion is that
    // the allocation depends on the key set alone.
    let orders = [
        defs(&[
            ("A?B", obj(&[("type", text("null"))])),
            ("A_B_1", obj(&[("type", text("boolean"))])),
            ("A/B", obj(&[("type", text("number"))])),
            ("A_B", obj(&[("type", text("string"))])),
        ]),
        defs(&[
            ("A_B", obj(&[("type", text("string"))])),
            ("A/B", obj(&[("type", text("number"))])),
            ("A_B_1", obj(&[("type", text("boolean"))])),
            ("A?B", obj(&[("type", text("null"))])),
        ]),
    ];

    for definitions in orders {
        let document =
            canonical_multi_document(vec![obj(&[("allOf", incoming.clone())])], definitions);
        let converted = json_schema::to_multi_document_open_api3_1(&document)
            .expect("2020-12 lowers to openapi 3.1");
        assert_eq!(multi_document_json(&converted), expected);
    }
}

#[test]
fn multi_reserves_sanitized_bases_before_allocating_suffixes() {
    let document = canonical_multi_document(
        vec![obj(&[(
            "allOf",
            arr(&[
                obj(&[("$ref", text("#/$defs/A~1B"))]),
                obj(&[("$ref", text("#/$defs/A?B"))]),
                obj(&[("$ref", text("#/$defs/A?B?1"))]),
            ]),
        )])],
        defs(&[
            ("A/B", obj(&[("type", text("number"))])),
            ("A?B", obj(&[("type", text("string"))])),
            ("A?B?1", obj(&[("type", text("boolean"))])),
        ]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    // Every key sanitizes to a base of `A_B`, so the pass that hands out
    // untouched bases runs before the one that appends suffixes; `A?B?1`
    // claims `A_B_1` on its own base, leaving `A?B` the `A_B_2` suffix.
    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("#/components/schemas/A_B"))]),
                        obj(&[("$ref", text("#/components/schemas/A_B_2"))]),
                        obj(&[("$ref", text("#/components/schemas/A_B_1"))]),
                    ]),
                )])]),
            ),
            (
                "definitions",
                obj(&[
                    ("A_B", obj(&[("type", text("number"))])),
                    ("A_B_2", obj(&[("type", text("string"))])),
                    ("A_B_1", obj(&[("type", text("boolean"))])),
                ]),
            ),
        ])
    );
}

#[test]
fn multi_rewrites_nested_definition_refs_without_changing_other_refs() {
    let document = canonical_multi_document(
        vec![obj(&[(
            "allOf",
            arr(&[
                obj(&[("$ref", text("#/$defs/A~1B/properties/value"))]),
                obj(&[("$ref", text("https://example.com/schema#/$defs/A~1B"))]),
                obj(&[("$ref", text("#/other/A~1B"))]),
            ]),
        )])],
        defs(&[(
            "A/B",
            obj(&[
                ("type", text("object")),
                (
                    "properties",
                    obj(&[("value", obj(&[("type", text("string"))]))]),
                ),
            ]),
        )]),
    );

    let converted = json_schema::to_multi_document_open_api3_1(&document)
        .expect("2020-12 lowers to openapi 3.1");

    // Only a pointer into the pool's own `$defs` moves; an absolute URI and a
    // pointer into another keyword stay exactly as written.
    assert_eq!(
        multi_document_json(&converted),
        obj(&[
            ("dialect", text("openapi-3.1")),
            (
                "schemas",
                arr(&[obj(&[(
                    "allOf",
                    arr(&[
                        obj(&[("$ref", text("#/components/schemas/A_B/properties/value"),)]),
                        obj(&[("$ref", text("https://example.com/schema#/$defs/A~1B"),)]),
                        obj(&[("$ref", text("#/other/A~1B"))]),
                    ]),
                )])]),
            ),
            (
                "definitions",
                obj(&[(
                    "A_B",
                    obj(&[
                        ("type", text("object")),
                        (
                            "properties",
                            obj(&[("value", obj(&[("type", text("string"))]))]),
                        ),
                    ]),
                )]),
            ),
        ])
    );
}
