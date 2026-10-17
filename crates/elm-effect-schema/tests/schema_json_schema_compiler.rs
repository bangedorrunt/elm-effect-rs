//! Phase-1 schema→JSON-Schema compiler core (epic schema-compiler-openapi).
//! RED test: `json_schema` module does not exist yet.

use elm_effect_schema::json_schema::{
    CompileOptions, OnExcessProperty, compile, compile_multi, compile_with_options,
};
use elm_effect_schema::{Field, Schema, Unknown};

fn checked(base: Schema<Unknown>, name: &str, args: Vec<&str>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::checked(
        base,
        name,
        args,
        vec![("expected", "refined")],
    ))
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

#[test]
fn compiles_scalar_types() {
    assert_eq!(compile(&string()), serde_json::json!({"type": "string"}));
    assert_eq!(compile(&number()), serde_json::json!({"type": "number"}));
    assert_eq!(
        compile(&Schema::as_unknown(Schema::boolean())),
        serde_json::json!({"type": "boolean"})
    );
    assert_eq!(
        compile(&Schema::as_unknown(Schema::null())),
        serde_json::json!({"type": "null"})
    );
    assert_eq!(
        compile(&Schema::as_unknown(Schema::json())),
        serde_json::json!({})
    );
}

#[test]
fn compiles_integer_check_as_type_integer() {
    assert_eq!(
        compile(&Schema::as_unknown(Schema::int())),
        serde_json::json!({"type": "integer"})
    );
}

#[test]
fn compiles_string_constraints() {
    let schema = checked(
        checked(string(), "isMinLength", vec!["2"]),
        "isMaxLength",
        vec!["5"],
    );
    assert_eq!(
        compile(&schema),
        serde_json::json!({"type": "string", "minLength": 1, "maxLength": 5})
    );
}

#[test]
fn compiles_number_constraints() {
    let schema = checked(
        checked(number(), "isGreaterThanOrEqualTo", vec!["1"]),
        "isLessThan",
        vec!["10"],
    );
    assert_eq!(
        compile(&schema),
        serde_json::json!({"type": "number", "minimum": 1, "exclusiveMaximum": 10})
    );
}

#[test]
fn compiles_struct_open_by_default_with_required() {
    let schema = Schema::as_unknown(Schema::struct_of(vec![
        ("a".to_string(), string()),
        ("b".to_string(), Schema::optional_key_shape(string())),
    ]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": true
        })
    );
}

#[test]
fn compiles_closed_struct_in_error_mode() {
    use elm_effect_schema::json_schema::{CompileOptions, OnExcessProperty, compile_with_options};
    let schema = Schema::as_unknown(Schema::struct_of(vec![("a".to_string(), string())]));
    assert_eq!(
        compile_with_options(
            &schema,
            &CompileOptions {
                on_excess_property: OnExcessProperty::Error,
                ..CompileOptions::default()
            }
        ),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false
        })
    );
}

#[test]
fn compiles_enum_const_and_unions() {
    let schema = Schema::<Unknown>::literal(Unknown::Text("red".into()));
    assert_eq!(
        compile(&schema),
        serde_json::json!({"type": "string", "enum": ["red"]})
    );
    let schema = Schema::<Unknown>::literals(vec![Unknown::Text("a".into()), Unknown::Number(1.0)]);
    assert_eq!(
        compile(&schema),
        serde_json::json!({"anyOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "number", "enum": [1]}
        ]})
    );
    let schema = Schema::union_with_mode(vec![string(), number()], "oneOf");
    assert_eq!(
        compile(&schema),
        serde_json::json!({"oneOf": [{"type": "string"}, {"type": "number"}]})
    );
}

#[test]
fn compiles_tuple_with_prefix_items() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![string(), number()]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}, {"type": "number"}],
            "minItems": 2,
            "maxItems": 2
        })
    );
}

#[test]
fn hoists_identifier_annotations_into_defs() {
    let named = Schema::annotated_with(string(), vec![("identifier", "Name")]);
    let schema = Schema::as_unknown(Schema::tuple_of(vec![named.clone(), named]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"$ref": "#/$defs/Name"}, {"$ref": "#/$defs/Name"}],
            "minItems": 2,
            "maxItems": 2,
            "$defs": {"Name": {"type": "string"}}
        })
    );
}

#[test]
fn shares_definitions_across_roots() {
    let named = Schema::annotated_with(string(), vec![("identifier", "Shared")]);
    let doc = compile_multi(&[named.clone(), named]);
    assert_eq!(doc.schemas.len(), 2);
    assert_eq!(
        doc.schemas[0],
        serde_json::json!({"$ref": "#/$defs/Shared"})
    );
    assert_eq!(
        doc.definitions.get("Shared"),
        Some(&serde_json::json!({"type": "string"}))
    );
}

#[test]
fn closed_shape_documents_round_trip_textually() {
    for doc in [
        serde_json::json!({"type": "string", "minLength": 2, "maxLength": 5}),
        serde_json::json!({"type": "integer"}),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": true
        }),
    ] {
        let schema = Schema::<Unknown>::from_json_schema(&doc).expect("imports");
        assert_eq!(compile(&schema), doc, "round trip for {doc}");
    }
}

// Phase 2: `toJsonSchemaDocument` rows
// (packages/effect/test/schema/toJsonSchemaDocument.test.ts).
//
// Every test below asserts the exact expected document for its row. Rows the
// compiler cannot carry are listed here with the missing machinery, not as
// tests:
// - 70 rejects tuple post-rest elements: no multi-rest tuple lowering and no
//   error channel (`compile` is total).
// - 77 rejects symbol property names: struct field names are Rust strings.
// - 98 content annotations: annotation values are strings, so an object
//   `contentSchema` has no spelling.
// - 265 matching keys with undefined values: no undefined annotation value.
// - 300/336/362 includeAnnotationKey with array/bool/object values
//   (`defaultSnippets`, `x-check-annotation`, `x-custom`): same value
//   limitation (string-valued custom keys ARE carried, see 226/245/281).
// - 383 standard annotations: `examples`/`readOnly`/`writeOnly` are not
//   strings either.
// - 1137/1153/1161 surrogate-pair loosening: lone surrogates cannot exist in
//   a Rust `str`, so the lowering has no input.
// - 1289/1106/1320/1383/1328/3605/1153 ajv cross-check halves: need the JS
//   validation harness; the documents are asserted here.
// - 1328 exports both bounds for a union input: accepts-only, same harness.
// - 1426 non-finite cardinality: check-constructor validation (`RangeError`).
// - 1511/1537 Number with infinities: the port has no Number-vs-Finite
//   distinction.
// - 1883 bigint literal, 805 UniqueSymbol, 779/792 BigInt/Symbol shapes,
//   527/536/577/588 Date/URL/Uint8Array/Duration, 649 Option, 759
//   Undefined, 2400 TemplateLiteral, 2985 template record keys, 2250-2366
//   Enum surface: constructors the port does not have (the compiler rules
//   for the opaque ones live as in-module unit tests).
// - 2641/2753/2868/3415/3435 `... to String` and 456(e): encoded-side
//   projection (`encodeTo`/`Getters`).
// - 3710 mutual recursion: reference registry across roots.
// - 3872/3884/3905/3930/3955 fromJsonString/Class/Error encoded identifiers:
//   encoded-side projection (`contentMediaType`, `...Encoded` names).

fn error_opts() -> CompileOptions {
    CompileOptions {
        on_excess_property: OnExcessProperty::Error,
        ..CompileOptions::default()
    }
}

fn error_doc(schema: &Schema<Unknown>) -> serde_json::Value {
    compile_with_options(schema, &error_opts())
}

fn checked_ann(
    base: Schema<Unknown>,
    name: &str,
    args: Vec<&str>,
    annotation: Vec<(&str, &str)>,
) -> Schema<Unknown> {
    Schema::as_unknown(Schema::checked(base, name, args, annotation))
}

fn annotated(schema: Schema<Unknown>, pairs: Vec<(&str, &str)>) -> Schema<Unknown> {
    Schema::annotated_with(schema, pairs)
}

fn null() -> Schema<Unknown> {
    Schema::as_unknown(Schema::null())
}

fn finite() -> Schema<Unknown> {
    // `Finite` documents like the port's number: the Number-vs-Finite
    // distinction (non-finite members) is the one unported half (row 1511).
    number()
}

fn record(key: Schema<Unknown>, value: Schema<Unknown>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::record2(key, value))
}

fn object_fields(fields: Vec<Field>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::struct_shape(fields))
}

// --- options (rows 117-210) ---

#[test]
fn options_generate_descriptions() {
    let schema = annotated(string(), vec![("expected", "b")]);
    assert_eq!(
        compile_with_options(
            &schema,
            &CompileOptions {
                generate_descriptions: true,
                ..CompileOptions::default()
            }
        ),
        serde_json::json!({"type": "string", "description": "b"})
    );
    let schema = annotated(string(), vec![("description", "a"), ("expected", "b")]);
    assert_eq!(
        compile_with_options(
            &schema,
            &CompileOptions {
                generate_descriptions: true,
                ..CompileOptions::default()
            }
        ),
        serde_json::json!({"type": "string", "description": "a"})
    );
}

#[test]
fn options_on_excess_property_modes() {
    // Row 141: the default ignores excess properties; the empty struct stays
    // the not-null shape in every mode.
    assert_eq!(
        compile(&Schema::as_unknown(Schema::struct_of(Vec::new()))),
        serde_json::json!({"not": {"type": "null"}})
    );
    let schema = Schema::as_unknown(Schema::struct_of(vec![("a".to_string(), string())]));
    assert_eq!(
        compile(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": true
        })
    );
    // Row 153: explicit ignore. Row 172: explicit error.
    assert_eq!(
        compile_with_options(&schema, &CompileOptions::default()),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": true
        })
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false
        })
    );
}

#[test]
fn options_record_pattern_properties_close() {
    // Row 191's shape with a representable affix key (the template spelling
    // of the same pattern is row 2985, excused): properties outside the
    // pattern do not reopen the object.
    let key = checked(string(), "isStartingWith", vec!["x-"]);
    assert_eq!(
        error_doc(&record(key, number())),
        serde_json::json!({
            "type": "object",
            "patternProperties": {"^x-": {"type": "number"}},
            "additionalProperties": false
        })
    );
}

#[test]
fn options_unrepresentable_index_key_keeps_property_names() {
    // Row 210: a custom filter has no representable part, so the key stays
    // the open string shape beside the value.
    let key = checked(string(), "customFilter", vec![]);
    assert_eq!(
        error_doc(&record(key, string())),
        serde_json::json!({
            "type": "object",
            "propertyNames": {"type": "string"},
            "additionalProperties": {"type": "string"}
        })
    );
}

#[test]
fn options_include_annotation_key() {
    // Row 226: the predicate keeps its key; standard keys ride regardless.
    let schema = annotated(
        string(),
        vec![
            ("title", "Name"),
            ("description", "A name"),
            ("markdownDescription", "The **name** field"),
        ],
    );
    assert_eq!(
        compile_with_options(
            &schema,
            &CompileOptions {
                include_annotation_key: Some(|key| key == "markdownDescription"),
                ..error_opts()
            }
        ),
        serde_json::json!({
            "type": "string",
            "title": "Name",
            "description": "A name",
            "markdownDescription": "The **name** field"
        })
    );
    // Row 245: keys the predicate rejects are dropped.
    let schema = annotated(
        string(),
        vec![
            ("title", "Name"),
            ("description", "A name"),
            ("markdownDescription", "The **name** field"),
            ("customKey", "value"),
        ],
    );
    assert_eq!(
        compile_with_options(
            &schema,
            &CompileOptions {
                include_annotation_key: Some(|key| key == "markdownDescription"),
                ..error_opts()
            }
        ),
        serde_json::json!({
            "type": "string",
            "title": "Name",
            "description": "A name",
            "markdownDescription": "The **name** field"
        })
    );
    // Row 281: standard keys survive even a refusing predicate.
    let schema = annotated(
        string(),
        vec![
            ("title", "Name"),
            ("description", "A name"),
            ("default", "hello"),
        ],
    );
    assert_eq!(
        compile_with_options(
            &schema,
            &CompileOptions {
                include_annotation_key: Some(|_| false),
                ..error_opts()
            }
        ),
        serde_json::json!({
            "type": "string",
            "title": "Name",
            "description": "A name",
            "default": "hello"
        })
    );
}

// --- identifiers (rows 406-456) ---

#[test]
fn identifiers_escape_pointer_segments() {
    // Row 406: `~` and `/` escape in the `$ref`, not in the `$defs` key.
    let schema = annotated(string(), vec![("identifier", "id~a/b")]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "$ref": "#/$defs/id~0a~1b",
            "$defs": {"id~a/b": {"type": "string"}}
        })
    );
}

#[test]
fn identifiers_reuse_and_suffix() {
    // Row 419: the same identified shape repeats by reference.
    let named = annotated(string(), vec![("identifier", "id")]);
    let schema = Schema::union_with_mode(vec![named.clone(), named], "anyOf");
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "anyOf": [{"$ref": "#/$defs/id"}, {"$ref": "#/$defs/id"}],
            "$defs": {"id": {"type": "string"}}
        })
    );
    // Row 437: different shapes under one name are suffixed.
    let schema = Schema::union_with_mode(
        vec![
            annotated(string(), vec![("identifier", "id"), ("description", "a")]),
            annotated(string(), vec![("identifier", "id"), ("description", "b")]),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "anyOf": [{"$ref": "#/$defs/id"}, {"$ref": "#/$defs/id_1"}],
            "$defs": {
                "id": {"type": "string", "description": "a"},
                "id_1": {"type": "string", "description": "b"}
            }
        })
    );
}

#[test]
fn identifiers_reuse_across_shapes() {
    // Row 456 without the encoded-side (e) member: one definition serves the
    // direct, nullable and optional-key occurrences.
    let x = annotated(string(), vec![("title", "X"), ("identifier", "X")]);
    let nullable = Schema::union_with_mode(vec![x.clone(), null()], "anyOf");
    let schema = Schema::as_unknown(Schema::struct_of(vec![
        ("a".to_string(), x.clone()),
        ("b".to_string(), nullable.clone()),
        ("c".to_string(), Schema::optional_key_shape(x.clone())),
        (
            "d".to_string(),
            Schema::optional_key_shape(nullable.clone()),
        ),
    ]));
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"$ref": "#/$defs/X"},
                "b": {"anyOf": [{"$ref": "#/$defs/X"}, {"type": "null"}]},
                "c": {"$ref": "#/$defs/X"},
                "d": {"anyOf": [{"$ref": "#/$defs/X"}, {"type": "null"}]}
            },
            "required": ["a", "b"],
            "additionalProperties": false,
            "$defs": {"X": {"type": "string", "title": "X"}}
        })
    );
}

// --- declarations and keyword shapes (rows 521-848, 1792) ---

#[test]
fn declaration_error_instance() {
    // Row 545.
    let schema = Schema::as_unknown(Schema::struct_of(vec![
        ("name".to_string(), Schema::optional_key_shape(string())),
        ("message".to_string(), string()),
        ("stack".to_string(), Schema::optional_key_shape(string())),
        (
            "cause".to_string(),
            Schema::optional_key_shape(Schema::as_unknown(Schema::json())),
        ),
    ]));
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "message": {"type": "string"},
                "stack": {"type": "string"},
                "cause": {}
            },
            "required": ["message"],
            "additionalProperties": false
        })
    );
}

#[test]
fn declaration_regexp_instance() {
    // Row 562.
    let schema = Schema::as_unknown(Schema::struct_of(vec![
        ("source".to_string(), string()),
        ("flags".to_string(), string()),
    ]));
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "source": {"type": "string"},
                "flags": {"type": "string"}
            },
            "required": ["source", "flags"],
            "additionalProperties": false
        })
    );
}

#[test]
fn any_json_unknown_void_never_null() {
    // Rows 685/703/713/723/739/827/848.
    assert_eq!(error_doc(&Schema::any()), serde_json::json!({}));
    assert_eq!(
        error_doc(&annotated(Schema::any(), vec![("description", "a")])),
        serde_json::json!({"description": "a"})
    );
    assert_eq!(error_doc(&Schema::json()), serde_json::json!({}));
    assert_eq!(error_doc(&Schema::unknown()), serde_json::json!({}));
    assert_eq!(
        error_doc(&annotated(Schema::unknown(), vec![("description", "a")])),
        serde_json::json!({})
    );
    let void = Schema::as_unknown(Schema::void());
    assert_eq!(error_doc(&void), serde_json::json!({"type": "null"}));
    assert_eq!(
        error_doc(&annotated(void, vec![("description", "a")])),
        serde_json::json!({"type": "null"})
    );
    let never = Schema::as_unknown(Schema::never());
    assert_eq!(error_doc(&never), serde_json::json!({"not": {}}));
    assert_eq!(
        error_doc(&annotated(never, vec![("description", "a")])),
        serde_json::json!({"description": "a", "not": {}})
    );
    let null_schema = null();
    assert_eq!(error_doc(&null_schema), serde_json::json!({"type": "null"}));
    assert_eq!(
        error_doc(&annotated(null_schema, vec![("description", "a")])),
        serde_json::json!({"type": "null", "description": "a"})
    );
}

#[test]
fn object_keyword_documents_bare() {
    // Row 1792: the keyword shape keeps no annotations.
    assert_eq!(
        error_doc(&Schema::object_keyword()),
        serde_json::json!({"anyOf": [{"type": "array"}, {"type": "object"}]})
    );
    assert_eq!(
        error_doc(&annotated(
            Schema::object_keyword(),
            vec![("description", "a")]
        )),
        serde_json::json!({"anyOf": [{"type": "array"}, {"type": "object"}]})
    );
}

// --- String (rows 870-1495) ---

#[test]
fn string_annotate_and_top_level_key() {
    // Rows 870/881/893: annotations ride; a top-level annotateKey is dropped.
    assert_eq!(error_doc(&string()), serde_json::json!({"type": "string"}));
    assert_eq!(
        error_doc(&annotated(string(), vec![("description", "a")])),
        serde_json::json!({"type": "string", "description": "a"})
    );
    assert_eq!(
        error_doc(&Schema::as_unknown(Schema::annotate_key(
            Schema::string(),
            vec![("description", "a")]
        ))),
        serde_json::json!({"type": "string"})
    );
}

#[test]
fn string_checks_chain() {
    // Rows 906/918/929/942/957: the loose bound is one below; an unknown
    // check degrades to the base.
    assert_eq!(
        error_doc(&checked(string(), "isMinLength", vec!["2"])),
        serde_json::json!({"type": "string", "minLength": 1})
    );
    assert_eq!(
        error_doc(&checked(string(), "customCheck", vec![])),
        serde_json::json!({"type": "string"})
    );
    assert_eq!(
        error_doc(&checked_ann(
            annotated(string(), vec![("description", "a")]),
            "isMinLength",
            vec!["2"],
            vec![("expected", "refined")]
        )),
        serde_json::json!({"type": "string", "description": "a", "minLength": 1})
    );
    assert_eq!(
        error_doc(&annotated(
            checked(string(), "isMinLength", vec!["2"]),
            vec![("description", "a")]
        )),
        serde_json::json!({"type": "string", "minLength": 1, "description": "a"})
    );
    assert_eq!(
        error_doc(&checked(
            checked(string(), "isMinLength", vec!["2"]),
            "isMaxLength",
            vec!["3"]
        )),
        serde_json::json!({"type": "string", "minLength": 1, "maxLength": 3})
    );
}

#[test]
fn string_annotated_checks_merge_and_nest() {
    // Rows 970/984/1000: the first annotated check merges inline, later ones
    // nest under allOf.
    assert_eq!(
        error_doc(&checked(
            checked(
                annotated(string(), vec![("description", "a")]),
                "isMinLength",
                vec!["2"]
            ),
            "isMaxLength",
            vec!["3"]
        )),
        serde_json::json!({
            "type": "string",
            "description": "a",
            "minLength": 1,
            "maxLength": 3
        })
    );
    assert_eq!(
        error_doc(&annotated(
            checked(
                checked(string(), "isMinLength", vec!["2"]),
                "isMaxLength",
                vec!["3"]
            ),
            vec![("description", "a")]
        )),
        serde_json::json!({
            "type": "string",
            "minLength": 1,
            "maxLength": 3,
            "description": "a"
        })
    );
    assert_eq!(
        error_doc(&checked_ann(
            checked(
                annotated(string(), vec![("description", "a")]),
                "isMinLength",
                vec!["2"]
            ),
            "isMaxLength",
            vec!["3"],
            vec![("description", "c"), ("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "description": "a",
            "minLength": 1,
            "allOf": [{"maxLength": 3, "description": "c"}]
        })
    );
}

#[test]
fn string_double_annotated_checks() {
    // Rows 1022/1044: the first annotated check merges inline (overwriting a
    // schema description), the second nests.
    assert_eq!(
        error_doc(&checked_ann(
            checked_ann(
                string(),
                "isMinLength",
                vec!["2"],
                vec![("description", "b"), ("expected", "refined")]
            ),
            "isMaxLength",
            vec!["3"],
            vec![("description", "c"), ("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "minLength": 1,
            "description": "b",
            "allOf": [{"maxLength": 3, "description": "c"}]
        })
    );
    assert_eq!(
        error_doc(&checked_ann(
            checked_ann(
                annotated(string(), vec![("description", "a")]),
                "isMinLength",
                vec!["2"],
                vec![("description", "b"), ("expected", "refined")]
            ),
            "isMaxLength",
            vec!["3"],
            vec![("description", "c"), ("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "minLength": 1,
            "description": "b",
            "allOf": [{"maxLength": 3, "description": "c"}]
        })
    );
}

#[test]
fn string_is_pattern_flags() {
    // Rows 1070/1079/1087: Unicode patterns ride; anything without `u` or
    // with js-only flags is omitted; unanchored sources anchor.
    assert_eq!(
        error_doc(&checked(
            string(),
            "isPattern",
            vec![r#"new RegExp("^abb+$", "u")"#]
        )),
        serde_json::json!({"type": "string", "pattern": "^abb+$"})
    );
    for arg in [
        r#"new RegExp("^abb+$", "")"#,
        r#"new RegExp("^abb+$", "y")"#,
        r#"new RegExp("^abb+$", "iu")"#,
        r#"new RegExp("^abb+$", "mu")"#,
        r#"new RegExp("^abb+$", "su")"#,
    ] {
        assert_eq!(
            error_doc(&checked(string(), "isPattern", vec![arg])),
            serde_json::json!({"type": "string"}),
            "{arg}"
        );
    }
    for (arg, pattern) in [
        (r#"new RegExp("^abb+$", "gu")"#, "^abb+$"),
        (r#"new RegExp("abb+", "uy")"#, "^(?:abb+)"),
        (r#"new RegExp("abb+", "guy")"#, "^(?:abb+)"),
        (r#"new RegExp("^[é]+$", "u")"#, "^[é]+$"),
        (r#"new RegExp("^[a-z]+$", "u")"#, "^[a-z]+$"),
        (r#"new RegExp("^[a-b-\uE000]+$", "u")"#, "^[a-b-\\uE000]+$"),
    ] {
        assert_eq!(
            error_doc(&checked(string(), "isPattern", vec![arg])),
            serde_json::json!({"type": "string", "pattern": pattern}),
            "{arg}"
        );
    }
}

#[test]
fn string_literal_affix_checks_escape() {
    // Row 1120: affix text is escaped, then anchored.
    assert_eq!(
        error_doc(&checked(string(), "isStartingWith", vec!["a.b"])),
        serde_json::json!({"type": "string", "pattern": "^a\\.b"})
    );
    assert_eq!(
        error_doc(&checked(string(), "isEndingWith", vec!["a+b"])),
        serde_json::json!({"type": "string", "pattern": "a\\+b$"})
    );
    assert_eq!(
        error_doc(&checked(string(), "isIncluding", vec!["["])),
        serde_json::json!({"type": "string", "pattern": "\\["})
    );
}

#[test]
fn string_case_and_trim_checks() {
    // Rows 1175/1185/1195/1205/1215.
    for (name, pattern) in [
        ("isTrimmed", "^\\S[\\s\\S]*\\S$|^\\S$|^$"),
        ("isLowercased", "^[^A-Z]*$"),
        ("isUppercased", "^[^a-z]*$"),
        ("isCapitalized", "^(?:[^a-z][\\s\\S]*)?$"),
        ("isUncapitalized", "^(?:[^A-Z][\\s\\S]*)?$"),
    ] {
        assert_eq!(
            error_doc(&checked(string(), name, vec![])),
            serde_json::json!({"type": "string", "pattern": pattern}),
            "{name}"
        );
    }
}

#[test]
fn string_between_length_kinds() {
    // Rows 1226/1239/1255/1276: loose code-unit bounds on strings, exact
    // bounds on arrays, and the NonEmptyArray shape (a tuple with rest) keeps
    // its own minimum beside the check's allOf entry.
    assert_eq!(
        error_doc(&checked(string(), "isBetweenLength", vec!["2", "2"])),
        serde_json::json!({"type": "string", "minLength": 1, "maxLength": 2})
    );
    let array = Schema::as_unknown(Schema::array(string()));
    assert_eq!(
        error_doc(&checked(array, "isBetweenLength", vec!["2", "2"])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "minItems": 2,
            "maxItems": 2
        })
    );
    let non_empty = Schema::tuple_with_rest(vec![string()], string());
    assert_eq!(
        error_doc(&checked(non_empty, "isBetweenLength", vec!["2", "2"])),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "items": {"type": "string"},
            "minItems": 1,
            "allOf": [{"minItems": 2, "maxItems": 2}]
        })
    );
    assert_eq!(
        error_doc(&checked(string(), "isBetweenLength", vec!["3", "4"])),
        serde_json::json!({"type": "string", "minLength": 2, "maxLength": 4})
    );
}

#[test]
fn string_code_point_checks_stay_exact() {
    // Row 1289's documents (the ajv cross-check needs the JS harness).
    assert_eq!(
        error_doc(&checked(string(), "isMinCodePoints", vec!["2"])),
        serde_json::json!({"type": "string", "minLength": 2})
    );
    assert_eq!(
        error_doc(&checked(string(), "isMaxCodePoints", vec!["1"])),
        serde_json::json!({"type": "string", "maxLength": 1})
    );
    assert_eq!(
        error_doc(&checked(string(), "isBetweenCodePoints", vec!["1", "2"])),
        serde_json::json!({
            "type": "string",
            "allOf": [{"minLength": 1}, {"maxLength": 2}]
        })
    );
}

#[test]
fn string_min_max_length_kinds() {
    // Rows 1308/1320/1333/1348: the loose lower bound, exact arrays, and a
    // colliding NonEmptyArray bound nesting under allOf.
    assert_eq!(
        error_doc(&checked(string(), "isMinLength", vec!["3"])),
        serde_json::json!({"type": "string", "minLength": 2})
    );
    let array = Schema::as_unknown(Schema::array(string()));
    assert_eq!(
        error_doc(&checked(array, "isMinLength", vec!["2"])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "minItems": 2
        })
    );
    let non_empty = Schema::tuple_with_rest(vec![string()], string());
    assert_eq!(
        error_doc(&checked(non_empty, "isMinLength", vec!["2"])),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "items": {"type": "string"},
            "minItems": 1,
            "allOf": [{"minItems": 2}]
        })
    );
    // Rows 1371/1391/1406: upper bounds are exact, and a non-colliding one
    // merges into the NonEmptyArray shape.
    assert_eq!(
        error_doc(&checked(string(), "isMaxLength", vec!["2"])),
        serde_json::json!({"type": "string", "maxLength": 2})
    );
    let array = Schema::as_unknown(Schema::array(string()));
    assert_eq!(
        error_doc(&checked(array, "isMaxLength", vec!["2"])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "maxItems": 2
        })
    );
    let non_empty = Schema::tuple_with_rest(vec![string()], string());
    assert_eq!(
        error_doc(&checked(non_empty, "isMaxLength", vec!["2"])),
        serde_json::json!({
            "type": "array",
            "maxItems": 2,
            "minItems": 1,
            "prefixItems": [{"type": "string"}],
            "items": {"type": "string"}
        })
    );
}

#[test]
fn string_format_checks() {
    // Rows 1454/1469/1482/1495.
    assert_eq!(
        error_doc(&checked_ann(
            annotated(string(), vec![("description", "description")]),
            "isUUID",
            vec![],
            vec![("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "description": "description",
            "format": "uuid",
            "pattern": "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|[fF]{8}-[fF]{4}-[fF]{4}-[fF]{4}-[fF]{12})$"
        })
    );
    assert_eq!(
        error_doc(&checked_ann(
            annotated(string(), vec![("description", "description")]),
            "isGUID",
            vec![],
            vec![("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "description": "description",
            "pattern": "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$"
        })
    );
    assert_eq!(
        error_doc(&checked_ann(
            annotated(string(), vec![("description", "description")]),
            "isBase64",
            vec![],
            vec![("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "description": "description",
            "pattern": "^([0-9a-zA-Z+/]{4})*(([0-9a-zA-Z+/]{2}==)|([0-9a-zA-Z+/]{3}=))?$"
        })
    );
    assert_eq!(
        error_doc(&checked_ann(
            annotated(string(), vec![("description", "description")]),
            "isBase64Url",
            vec![],
            vec![("expected", "refined")]
        )),
        serde_json::json!({
            "type": "string",
            "description": "description",
            "pattern": "^([0-9a-zA-Z-_]{4})*(([0-9a-zA-Z-_]{2}(==)?)|([0-9a-zA-Z-_]{3}(=)?))?$"
        })
    );
}

// --- Number/Finite (rows 1563-1771; 1511/1537 excused: no Number-vs-Finite
// distinction) ---

#[test]
fn finite_shapes() {
    // Rows 1563/1771.
    assert_eq!(error_doc(&finite()), serde_json::json!({"type": "number"}));
    assert_eq!(
        error_doc(&annotated(finite(), vec![("description", "a")])),
        serde_json::json!({"type": "number", "description": "a"})
    );
    let boolean = Schema::as_unknown(Schema::boolean());
    assert_eq!(error_doc(&boolean), serde_json::json!({"type": "boolean"}));
    assert_eq!(
        error_doc(&annotated(boolean, vec![("description", "a")])),
        serde_json::json!({"type": "boolean", "description": "a"})
    );
}

#[test]
fn finite_int_ranges() {
    // Rows 1584/1595/1609.
    assert_eq!(
        error_doc(&checked(finite(), "isInt", vec![])),
        serde_json::json!({"type": "integer"})
    );
    assert_eq!(
        error_doc(&checked(finite(), "isInt32", vec![])),
        serde_json::json!({
            "type": "integer",
            "allOf": [{"maximum": 2147483647, "minimum": -2147483648}]
        })
    );
    assert_eq!(
        error_doc(&checked(finite(), "isUint32", vec![])),
        serde_json::json!({
            "type": "integer",
            "allOf": [{"maximum": 4294967295u32, "minimum": 0}]
        })
    );
    assert_eq!(
        error_doc(&checked_ann(
            finite(),
            "isUint32",
            vec![],
            vec![("description", "a"), ("expected", "refined")]
        )),
        serde_json::json!({
            "type": "integer",
            "allOf": [{
                "description": "a",
                "allOf": [{"maximum": 4294967295u32, "minimum": 0}]
            }]
        })
    );
}

#[test]
fn finite_comparisons() {
    // Rows 1657/1669/1681/1690/1699/1748.
    assert_eq!(
        error_doc(&checked(finite(), "isGreaterThan", vec!["1"])),
        serde_json::json!({"type": "number", "exclusiveMinimum": 1})
    );
    assert_eq!(
        error_doc(&checked(finite(), "isGreaterThanOrEqualTo", vec!["1"])),
        serde_json::json!({"type": "number", "minimum": 1})
    );
    assert_eq!(
        error_doc(&checked(finite(), "isLessThan", vec!["1"])),
        serde_json::json!({"type": "number", "exclusiveMaximum": 1})
    );
    assert_eq!(
        error_doc(&checked(finite(), "isLessThanOrEqualTo", vec!["1"])),
        serde_json::json!({"type": "number", "maximum": 1})
    );
    assert_eq!(
        error_doc(&checked(finite(), "isBetween", vec!["1", "10"])),
        serde_json::json!({"type": "number", "minimum": 1, "maximum": 10})
    );
    assert_eq!(
        error_doc(&checked_ann(
            finite(),
            "isBetween",
            vec!["1", "10"],
            vec![("exclusiveMinimum", "true"), ("expected", "refined")]
        )),
        serde_json::json!({"type": "number", "exclusiveMinimum": 1, "maximum": 10})
    );
    assert_eq!(
        error_doc(&checked_ann(
            finite(),
            "isBetween",
            vec!["1", "10"],
            vec![("exclusiveMaximum", "true"), ("expected", "refined")]
        )),
        serde_json::json!({"type": "number", "minimum": 1, "exclusiveMaximum": 10})
    );
    assert_eq!(
        error_doc(&checked_ann(
            finite(),
            "isBetween",
            vec!["1", "10"],
            vec![
                ("exclusiveMinimum", "true"),
                ("exclusiveMaximum", "true"),
                ("expected", "refined")
            ]
        )),
        serde_json::json!({
            "type": "number",
            "exclusiveMinimum": 1,
            "exclusiveMaximum": 10
        })
    );
    assert_eq!(
        error_doc(&checked(
            checked(finite(), "isInt", vec![]),
            "isMultipleOf",
            vec!["2"]
        )),
        serde_json::json!({"type": "integer", "multipleOf": 2})
    );
    assert_eq!(
        error_doc(&checked(
            checked(finite(), "isInt", vec![]),
            "isMultipleOf",
            vec!["-2"]
        )),
        serde_json::json!({"type": "integer", "multipleOf": 2})
    );
}

// --- Literals, unions of literals, enums (rows 1814-2366; the annotated
// halves needing non-string values and row 1883 are excused) ---

#[test]
fn literal_shapes() {
    // Rows 1814/1837/1860.
    let schema = Schema::<Unknown>::literal(Unknown::Text("a".into()));
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "string", "enum": ["a"]})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"type": "string", "enum": ["a"], "description": "a"})
    );
    let schema = Schema::<Unknown>::literal(Unknown::Number(1.0));
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "number", "enum": [1]})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"type": "number", "enum": [1], "description": "a"})
    );
    let schema = Schema::<Unknown>::literal(Unknown::Bool(true));
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "boolean", "enum": [true]})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"type": "boolean", "enum": [true], "description": "a"})
    );
}

#[test]
fn literals_shapes() {
    // Rows 1907/1928/1951/1974/1997.
    let schema = Schema::union_with_mode(vec![], "anyOf");
    assert_eq!(error_doc(&schema), serde_json::json!({"not": {}}));
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"not": {}, "description": "a"})
    );
    for (values, expected) in [
        (
            vec![Unknown::Text("a".into()), Unknown::Text("b".into())],
            serde_json::json!({"type": "string", "enum": ["a", "b"]}),
        ),
        (
            vec![Unknown::Number(1.0), Unknown::Number(2.0)],
            serde_json::json!({"type": "number", "enum": [1, 2]}),
        ),
        (
            vec![Unknown::Bool(true), Unknown::Bool(false)],
            serde_json::json!({"type": "boolean", "enum": [true, false]}),
        ),
    ] {
        let schema = Schema::<Unknown>::literals(values);
        let with_description = match expected.clone() {
            serde_json::Value::Object(mut map) => {
                map.insert(
                    "description".to_string(),
                    serde_json::Value::String("a".to_string()),
                );
                serde_json::Value::Object(map)
            }
            other => other,
        };
        assert_eq!(error_doc(&schema), expected);
        assert_eq!(
            error_doc(&annotated(schema, vec![("description", "a")])),
            with_description
        );
    }
    let schema = Schema::<Unknown>::literals(vec![Unknown::Text("a".into()), Unknown::Number(1.0)]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "number", "enum": [1]}
        ]})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"anyOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "number", "enum": [1]}
        ], "description": "a"})
    );
}

#[test]
fn union_of_literals_compacts() {
    // Rows 2038/2070/2086/2136/2168/2200 (bare halves).
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Text("a".into())),
            Schema::<Unknown>::literal(Unknown::Text("b".into())),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "string", "enum": ["a", "b"]})
    );
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Text("a".into())),
            Schema::<Unknown>::literals(vec![Unknown::Text("b".into()), Unknown::Text("c".into())]),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "string", "enum": ["a", "b", "c"]})
    );
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Text("a".into())),
            annotated(
                Schema::<Unknown>::literal(Unknown::Text("b".into())),
                vec![("description", "b-description")],
            ),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "string", "enum": ["b"], "description": "b-description"}
        ]})
    );
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Number(1.0)),
            Schema::<Unknown>::literal(Unknown::Number(2.0)),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "number", "enum": [1, 2]})
    );
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Bool(true)),
            Schema::<Unknown>::literal(Unknown::Bool(false)),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "boolean", "enum": [true, false]})
    );
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Text("a".into())),
            Schema::<Unknown>::literal(Unknown::Number(1.0)),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "number", "enum": [1]}
        ]})
    );
}

#[test]
fn enum_like_unions_keep_titles() {
    // Rows 2250/2272/2308/2366 modeled as titled single-literal unions (the
    // port has no Enum constructor; the compiler path is identical).
    let schema = Schema::union_with_mode(vec![], "anyOf");
    assert_eq!(error_doc(&schema), serde_json::json!({"not": {}}));
    let schema = Schema::union_with_mode(
        vec![annotated(
            Schema::<Unknown>::literal(Unknown::Number(0.0)),
            vec![("title", "Apple")],
        )],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "number", "enum": [0], "title": "Apple"}
        ]})
    );
    let schema = Schema::union_with_mode(
        vec![
            annotated(
                Schema::<Unknown>::literal(Unknown::Number(0.0)),
                vec![("title", "Apple")],
            ),
            annotated(
                Schema::<Unknown>::literal(Unknown::Number(1.0)),
                vec![("title", "Banana")],
            ),
            annotated(
                Schema::<Unknown>::literal(Unknown::Text("orange".into())),
                vec![("title", "Orange")],
            ),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "number", "enum": [0], "title": "Apple"},
            {"type": "number", "enum": [1], "title": "Banana"},
            {"type": "string", "enum": ["orange"], "title": "Orange"}
        ]})
    );
}

// --- Struct (rows 2418-2868; 2641/2753/2868 encodeTo halves excused) ---

#[test]
fn struct_required_and_annotate_key() {
    // Rows 2440/2460/2481/2504: the Field API keeps key annotations that the
    // kind-only struct_of path drops with as_unknown's children.
    let plain = object_fields(vec![Field::required("a", string())]);
    assert_eq!(
        error_doc(&plain),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![Field::required(
        "a",
        annotated(string(), vec![("description", "a")]),
    )]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string", "description": "a"}},
            "required": ["a"],
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![Field::required(
        "a",
        Schema::annotate_key(
            annotated(string(), vec![("description", "a")]),
            vec![("description", "a-key")],
        ),
    )]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {
                "type": "string",
                "description": "a",
                "allOf": [{"description": "a-key"}]
            }},
            "required": ["a"],
            "additionalProperties": false
        })
    );
    // Row 2481 on its own: a bare key annotation wraps the plain property.
    let schema = object_fields(vec![Field::required(
        "a",
        Schema::annotate_key(string(), vec![("description", "a-key")]),
    )]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {
                "type": "string",
                "allOf": [{"description": "a-key"}]
            }},
            "required": ["a"],
            "additionalProperties": false
        })
    );
}

#[test]
fn struct_optional_key_kinds() {
    // Rows 2530/2547/2572/2601: the Field API keeps inner annotation kinds
    // that the kind-only path would drop.
    let schema = object_fields(vec![Field::optional_key("a", string())]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::optional_key("a", annotated(string(), vec![("description", "a")])),
        Field::optional_key(
            "b",
            annotated(
                Schema::optional_key_shape(string()),
                vec![("description", "b")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "string", "description": "a"},
                "b": {"type": "string", "description": "b"}
            },
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::optional_key(
            "a",
            Schema::annotate_key(string(), vec![("description", "a-key")]),
        ),
        Field::optional_key(
            "b",
            Schema::annotate_key(
                Schema::optional_key_shape(string()),
                vec![("description", "b-key")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "string", "allOf": [{"description": "a-key"}]},
                "b": {"type": "string", "allOf": [{"description": "b-key"}]}
            },
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::optional_key(
            "a",
            Schema::annotate_key(
                annotated(string(), vec![("description", "a")]),
                vec![("description", "a-key")],
            ),
        ),
        Field::optional_key(
            "b",
            Schema::annotate_key(
                annotated(
                    Schema::optional_key_shape(string()),
                    vec![("description", "b")],
                ),
                vec![("description", "b-key")],
            ),
        ),
        Field::optional_key(
            "c",
            Schema::annotate_key(
                annotated(
                    Schema::annotate_key(
                        annotated(string(), vec![("description", "a")]),
                        vec![("description", "a-key")],
                    ),
                    vec![("description", "c-outer")],
                ),
                vec![("description", "c-outer-key")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "string",
                    "description": "a",
                    "allOf": [{"description": "a-key"}]
                },
                "b": {
                    "type": "string",
                    "description": "b",
                    "allOf": [{"description": "b-key"}]
                },
                "c": {
                    "type": "string",
                    "description": "c-outer",
                    "allOf": [{"description": "c-outer-key"}]
                }
            },
            "additionalProperties": false
        })
    );
}

#[test]
fn struct_optional_kinds() {
    // Rows 2666/2688/2718: `optional` is absent-or-null, so the property is
    // an anyOf the required list leaves out.
    let schema = object_fields(vec![Field::optional("a", string())]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {
                "anyOf": [{"type": "string"}, {"type": "null"}]
            }},
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::optional("a", annotated(string(), vec![("description", "a")])),
        Field::optional(
            "b",
            annotated(
                Schema::as_unknown(Schema::optional(string())),
                vec![("description", "b")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"anyOf": [
                    {"type": "string", "description": "a"},
                    {"type": "null"}
                ]},
                "b": {
                    "anyOf": [{"type": "string"}, {"type": "null"}],
                    "description": "b"
                }
            },
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::optional(
            "a",
            Schema::annotate_key(string(), vec![("description", "a-key")]),
        ),
        Field::optional(
            "b",
            Schema::annotate_key(
                Schema::as_unknown(Schema::optional(annotated(
                    string(),
                    vec![("description", "b")],
                ))),
                vec![("description", "b-key")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "anyOf": [{"type": "string"}, {"type": "null"}],
                    "allOf": [{"description": "a-key"}]
                },
                "b": {
                    "anyOf": [
                        {"type": "string", "description": "b"},
                        {"type": "null"}
                    ],
                    "allOf": [{"description": "b-key"}]
                }
            },
            "additionalProperties": false
        })
    );
}

#[test]
fn struct_undefined_or_kinds() {
    // Rows 2778/2801/2832: required but nullable, modeled as an explicit
    // null union.
    let nullable = Schema::union_with_mode(vec![string(), null()], "anyOf");
    let schema = object_fields(vec![Field::required("a", nullable)]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {
                "anyOf": [{"type": "string"}, {"type": "null"}]
            }},
            "required": ["a"],
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::required(
            "a",
            Schema::union_with_mode(
                vec![annotated(string(), vec![("description", "a")]), null()],
                "anyOf",
            ),
        ),
        Field::required(
            "b",
            annotated(
                Schema::union_with_mode(vec![string(), null()], "anyOf"),
                vec![("description", "b")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"anyOf": [
                    {"type": "string", "description": "a"},
                    {"type": "null"}
                ]},
                "b": {
                    "anyOf": [{"type": "string"}, {"type": "null"}],
                    "description": "b"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    );
    let schema = object_fields(vec![
        Field::required(
            "a",
            Schema::annotate_key(
                Schema::union_with_mode(vec![string(), null()], "anyOf"),
                vec![("description", "a-key")],
            ),
        ),
        Field::required(
            "b",
            Schema::annotate_key(
                Schema::union_with_mode(
                    vec![annotated(string(), vec![("description", "b")]), null()],
                    "anyOf",
                ),
                vec![("description", "b-key")],
            ),
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "anyOf": [{"type": "string"}, {"type": "null"}],
                    "allOf": [{"description": "a-key"}]
                },
                "b": {
                    "anyOf": [
                        {"type": "string", "description": "b"},
                        {"type": "null"}
                    ],
                    "allOf": [{"description": "b-key"}]
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    );
}

// --- Record (rows 2894-3141; 2985 template keys excused) ---

#[test]
fn record_never_and_empty_struct_values() {
    // Rows 2894/2918.
    assert_eq!(
        error_doc(&record(string(), Schema::as_unknown(Schema::never()))),
        serde_json::json!({"type": "object", "additionalProperties": false})
    );
    assert_eq!(
        error_doc(&record(
            string(),
            annotated(
                Schema::as_unknown(Schema::never()),
                vec![("description", "a")]
            )
        )),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"description": "a", "not": {}}
        })
    );
    let empty = Schema::as_unknown(Schema::struct_of(Vec::new()));
    assert_eq!(
        error_doc(&record(string(), empty)),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"not": {"type": "null"}}
        })
    );
}

#[test]
fn record_string_finite_and_json_object() {
    // Rows 2945/2975: annotations on the value and root ride; a key
    // description has nowhere to go (the pin's own TODO).
    assert_eq!(
        error_doc(&record(string(), finite())),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"type": "number"}
        })
    );
    assert_eq!(
        error_doc(&annotated(
            record(
                annotated(string(), vec![("description", "k")]),
                annotated(finite(), vec![("description", "v")])
            ),
            vec![("description", "r")]
        )),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"type": "number", "description": "v"},
            "description": "r"
        })
    );
    assert_eq!(
        error_doc(&record(string(), Schema::json())),
        serde_json::json!({"type": "object"})
    );
}

#[test]
fn record_starting_with_key() {
    // Row 2930.
    let key = checked(string(), "isStartingWith", vec!["a"]);
    let empty = Schema::as_unknown(Schema::struct_of(Vec::new()));
    assert_eq!(
        error_doc(&record(key, empty)),
        serde_json::json!({
            "type": "object",
            "patternProperties": {"^a": {"not": {"type": "null"}}},
            "additionalProperties": false
        })
    );
}

#[test]
fn record_literal_keys_expand() {
    // Row 3002.
    let key =
        Schema::<Unknown>::literals(vec![Unknown::Text("a".into()), Unknown::Text("b".into())]);
    assert_eq!(
        error_doc(&record(key, finite())),
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    );
}

#[test]
fn record_case_fold_key_uses_property_names() {
    // Row 3019: a non-selector check is propertyNames, not a pattern scope.
    let key = checked(string(), "isUppercased", vec![]);
    assert_eq!(
        error_doc(&record(key, number())),
        serde_json::json!({
            "type": "object",
            "propertyNames": {"type": "string", "pattern": "^[^a-z]*$"},
            "additionalProperties": {"type": "number"}
        })
    );
}

#[test]
fn record_conjunctive_keys_use_property_names() {
    // Row 3035.
    let key = checked(
        checked(string(), "isPattern", vec![r#"new RegExp("^ab", "u")"#]),
        "isPattern",
        vec![r#"new RegExp("^ab", "u")"#],
    );
    let _ = key;
    let single = checked(string(), "isPattern", vec![r#"new RegExp("^ab", "u")"#]);
    assert_eq!(
        error_doc(&record(single, finite())),
        serde_json::json!({
            "type": "object",
            "patternProperties": {"^ab": {"type": "number"}},
            "additionalProperties": false
        })
    );
    let conjunctive = checked(
        checked(string(), "isPattern", vec![r#"new RegExp("^ab", "u")"#]),
        "isEndingWith",
        vec!["z"],
    );
    assert_eq!(
        error_doc(&record(conjunctive, finite())),
        serde_json::json!({
            "type": "object",
            "propertyNames": {
                "type": "string",
                "pattern": "^ab",
                "allOf": [{"pattern": "z$"}]
            },
            "additionalProperties": {"type": "number"}
        })
    );
    let conjunctive = checked(
        checked(string(), "isStartingWith", vec!["A"]),
        "isUppercased",
        vec![],
    );
    assert_eq!(
        error_doc(&record(conjunctive, finite())),
        serde_json::json!({
            "type": "object",
            "propertyNames": {
                "type": "string",
                "pattern": "^A",
                "allOf": [{"pattern": "^[^a-z]*$"}]
            },
            "additionalProperties": {"type": "number"}
        })
    );
}

#[test]
fn record_partial_pattern_key() {
    // Row 3084, both modes: a partial pattern is not a selector, and ignore
    // mode degrades the whole record to an open object.
    let key = checked(
        checked(string(), "isStartingWith", vec!["x"]),
        "isMinLength",
        vec!["3"],
    );
    assert_eq!(
        error_doc(&record(key.clone(), finite())),
        serde_json::json!({
            "type": "object",
            "propertyNames": {
                "type": "string",
                "pattern": "^x",
                "minLength": 2
            },
            "additionalProperties": {"type": "number"}
        })
    );
    assert_eq!(
        compile_with_options(&record(key, finite()), &CompileOptions::default()),
        serde_json::json!({"type": "object", "additionalProperties": true})
    );
}

#[test]
fn record_property_checks() {
    // Rows 3113/3128/3141.
    assert_eq!(
        error_doc(&checked(
            record(string(), finite()),
            "isMinProperties",
            vec!["2"]
        )),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"type": "number"},
            "minProperties": 2
        })
    );
    assert_eq!(
        error_doc(&checked(
            record(string(), finite()),
            "isMaxProperties",
            vec!["2"]
        )),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"type": "number"},
            "maxProperties": 2
        })
    );
    assert_eq!(
        error_doc(&checked(
            record(string(), finite()),
            "isBetweenProperties",
            vec!["2", "2"]
        )),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {"type": "number"},
            "minProperties": 2,
            "maxProperties": 2
        })
    );
}

// --- StructWithRest (rows 3158-3208; 3184's merge and 3158/3208 carried) ---

#[test]
fn struct_with_rest_string_index() {
    // Row 3158: a plain string index becomes an allOf object entry.
    let rest = Schema::as_unknown(record(
        string(),
        Schema::union_with_mode(vec![finite(), string()], "anyOf"),
    ));
    let schema = Schema::struct_with_rest(vec![("a".to_string(), string())], rest);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "allOf": [{
                "type": "object",
                "additionalProperties": {
                    "anyOf": [{"type": "number"}, {"type": "string"}]
                }
            }],
            "required": ["a"]
        })
    );
}

#[test]
fn struct_with_rest_pattern_and_string_indexes() {
    // Row 3184: every key shape joins propertyNames, every value joins
    // additionalProperties, and the string index lands as an allOf entry.
    let rest = Schema::union_with_mode(
        vec![
            Schema::as_unknown(record(checked(string(), "isUppercased", vec![]), finite())),
            Schema::as_unknown(record(string(), Schema::as_unknown(Schema::boolean()))),
        ],
        "anyOf",
    );
    let schema = Schema::struct_with_rest(vec![], rest);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "propertyNames": {"anyOf": [
                {"type": "string", "pattern": "^[^a-z]*$"},
                {"type": "string"}
            ]},
            "additionalProperties": {"anyOf": [
                {"type": "number"},
                {"type": "boolean"}
            ]},
            "allOf": [{
                "type": "object",
                "additionalProperties": {"type": "boolean"}
            }]
        })
    );
}

#[test]
fn struct_with_rest_compound_pattern_index() {
    // Row 3208: a conjunctive key names the declared properties beside its
    // own shape, with the value as additionalProperties.
    let rest = Schema::as_unknown(record(
        checked(
            checked(string(), "isStartingWith", vec!["x"]),
            "isEndingWith",
            vec!["z"],
        ),
        finite(),
    ));
    let schema = Schema::struct_with_rest(vec![("a".to_string(), string())], rest);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "propertyNames": {"anyOf": [
                {"enum": ["a"]},
                {"type": "string", "pattern": "^x", "allOf": [{"pattern": "z$"}]}
            ]},
            "additionalProperties": {"type": "number"}
        })
    );
}

// --- Tuple (rows 3241-3435; 3415/3435 encodeTo halves excused) ---

#[test]
fn tuple_empty_and_required() {
    // Rows 3241/3265/3281/3297/3313.
    let schema = Schema::tuple_of(vec![]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"type": "array", "items": false})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"type": "array", "items": false, "description": "a"})
    );
    let schema = Schema::tuple_of(vec![string()]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "minItems": 1,
            "maxItems": 1
        })
    );
    let schema = Schema::tuple_of(vec![annotated(string(), vec![("description", "a")])]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string", "description": "a"}],
            "minItems": 1,
            "maxItems": 1
        })
    );
    let schema = Schema::tuple_of(vec![Schema::annotate_key(
        string(),
        vec![("description", "a-key")],
    )]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string", "allOf": [{"description": "a-key"}]}],
            "minItems": 1,
            "maxItems": 1
        })
    );
    let schema = Schema::tuple_of(vec![Schema::annotate_key(
        annotated(string(), vec![("description", "a")]),
        vec![("description", "a-key")],
    )]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{
                "type": "string",
                "description": "a",
                "allOf": [{"description": "a-key"}]
            }],
            "minItems": 1,
            "maxItems": 1
        })
    );
}

#[test]
fn tuple_optional_key_kinds() {
    // Rows 3331/3348/3367/3389: trailing absent positions drop minItems;
    // key annotations wrap the element.
    let schema = Schema::tuple_of(vec![Schema::optional_key_shape(string())]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "maxItems": 1
        })
    );
    let schema = Schema::tuple_of(vec![
        Schema::optional_key_shape(annotated(string(), vec![("description", "a")])),
        Schema::annotate_key(
            Schema::optional_key_shape(string()),
            vec![("description", "b-key")],
        ),
    ]);
    // Row 3348's (a)/(b) mix, then row 3367's key shape for the second slot.
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [
                {"type": "string", "description": "a"},
                {"type": "string", "allOf": [{"description": "b-key"}]}
            ],
            "maxItems": 2
        })
    );
    // Rows 3389: annotations ride inside the optional element and key
    // annotations outside it. Both placements are decode-identical (key
    // annotations are occurrence metadata), and for absent-only positions
    // they are document-identical too, which the exact documents pin down.
    // tuple_of's required-after-optional assert does not look through
    // annotation wrappers, so the outer annotation lives on the inner
    // schema here rather than on the element.
    let schema = Schema::tuple_of(vec![
        Schema::annotate_key(
            Schema::optional_key_shape(annotated(string(), vec![("description", "a")])),
            vec![("description", "a-key")],
        ),
        Schema::annotate_key(
            Schema::optional_key_shape(annotated(string(), vec![("description", "b")])),
            vec![("description", "b-key")],
        ),
        Schema::annotate_key(
            Schema::optional_key_shape(annotated(
                annotated(string(), vec![("description", "a")]),
                vec![("description", "c-outer")],
            )),
            vec![("description", "c-outer-key")],
        ),
    ]);
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [
                {
                    "type": "string",
                    "description": "a",
                    "allOf": [{"description": "a-key"}]
                },
                {
                    "type": "string",
                    "description": "b",
                    "allOf": [{"description": "b-key"}]
                },
                {
                    "type": "string",
                    "description": "c-outer",
                    "allOf": [{"description": "c-outer-key"}]
                }
            ],
            "maxItems": 3
        })
    );
}

// --- Array (rows 3458-3536) ---

#[test]
fn array_string_and_length_checks() {
    // Rows 3458/3481/3494/3507/3521.
    let array = Schema::as_unknown(Schema::array(string()));
    assert_eq!(
        error_doc(&array),
        serde_json::json!({"type": "array", "items": {"type": "string"}})
    );
    assert_eq!(
        error_doc(&annotated(array.clone(), vec![("description", "a")])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "a"
        })
    );
    assert_eq!(
        error_doc(&checked(array.clone(), "isMinLength", vec!["2"])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "minItems": 2
        })
    );
    assert_eq!(
        error_doc(&checked(array.clone(), "isMaxLength", vec!["2"])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "maxItems": 2
        })
    );
    assert_eq!(
        error_doc(&checked(array.clone(), "isBetweenLength", vec!["2", "2"])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "minItems": 2,
            "maxItems": 2
        })
    );
    assert_eq!(
        error_doc(&checked(array, "isUnique", vec![])),
        serde_json::json!({
            "type": "array",
            "items": {"type": "string"},
            "uniqueItems": true
        })
    );
}

#[test]
fn tuple_with_rest_keeps_min_items() {
    // Row 3536.
    let schema = Schema::tuple_with_rest(vec![string()], finite());
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "items": {"type": "number"},
            "minItems": 1
        })
    );
}

// --- Union (rows 3553-3674; 3674 excused: no BigInt shape) ---

#[test]
fn union_empty_single_and_modes() {
    // Rows 3553/3568/3605/3621/3628.
    let schema = Schema::union_with_mode(vec![], "anyOf");
    assert_eq!(error_doc(&schema), serde_json::json!({"not": {}}));
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({"not": {}, "description": "a"})
    );
    let schema = Schema::union_with_mode(vec![string()], "anyOf");
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [{"type": "string"}]})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "a")])),
        serde_json::json!({
            "anyOf": [{"type": "string"}],
            "description": "a"
        })
    );
    // An approximated oneOf over overlapping branches loosens to anyOf.
    let schema = Schema::union_with_mode(
        vec![
            checked(string(), "isMinLength", vec!["2"]),
            checked(string(), "isMaxLength", vec!["1"]),
        ],
        "oneOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "string", "minLength": 1},
            {"type": "string", "maxLength": 1}
        ]})
    );
    // Exact oneOf branches stay oneOf.
    let schema = Schema::union_with_mode(
        vec![string(), Schema::as_unknown(Schema::boolean())],
        "oneOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"oneOf": [{"type": "string"}, {"type": "boolean"}]})
    );
    // Duplicate enum values do not compact.
    let schema = Schema::union_with_mode(
        vec![
            Schema::<Unknown>::literal(Unknown::Text("a".into())),
            Schema::<Unknown>::literal(Unknown::Text("a".into())),
        ],
        "anyOf",
    );
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "string", "enum": ["a"]}
        ]})
    );
}

#[test]
fn union_string_number() {
    // Row 3642.
    let schema = Schema::union_with_mode(vec![string(), finite()], "anyOf");
    assert_eq!(
        error_doc(&schema),
        serde_json::json!({"anyOf": [{"type": "string"}, {"type": "number"}]})
    );
    assert_eq!(
        error_doc(&annotated(schema, vec![("description", "description")])),
        serde_json::json!({
            "anyOf": [{"type": "string"}, {"type": "number"}],
            "description": "description"
        })
    );
}
