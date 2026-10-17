//! `toRepresentation` exact documents
//! (packages/effect/test/schema/representation/toRepresentation.test.ts).
//!
//! Each test pins the exact document the converter emits for one row. The
//! converter only sees `SchemaKind`, so anything the kind cannot see is
//! excused with a reason instead of guessed:
//!
//! Excused, with reasons:
//! - `:37 global unique symbol`: no UniqueSymbol constructor or domain.
//! - `:45 enum`: no name-preserving Enum constructor (`Literals` drops names).
//! - `:59 template literal`: parts are folded into an opaque kind marker.
//! - `:172/:183/:201/:222/:243 encoded and type projections` (5 rows): the
//!   port stores one side per schema, so there is no encoded side to project.
//! - `:255 omits brands`: no brand machinery.
//! - `:265/:292 declarations` (2 rows): custom code callbacks and
//!   declare-with-predicate are outside the port.
//! - `:302 custom filter callbacks`: live callbacks have no document spelling.
//! - `:367 filter groups`: no group constructor in the public API.
//! - `:388 isPropertyNames dependencies`: the key schema travels in the
//!   closure alone, so the converter cannot re-emit it.
//! - `:424 shared filter dependencies`: needs the `referencePolicy` option.
//! - `:477 fromJsonString annotations`: no fromJsonString constructor.
//! - `:555 inline policy`, `:571 synthetic reference under a policy`: need the
//!   `referencePolicy` option.
//! - `:615 non-recursive suspend`: a thunk is a closure the kind does not
//!   carry, so only direct recursion under an identifier converts.
//! - `:759 Class identifier`: needs type-side projection plus Class.
//! - `:775 anonymous recursion`: an unidentified suspend has no name to point at.
//!
//! Substitutions, disclosed: `:97` and `:140` build `Symbol`/`BigInt` shapes
//! the port cannot construct, so the same document shapes are carried with
//! `String`/`Number` in their place; the original rows stay excused.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use elm_effect_schema::filters::{FilterOutcome, FilterSpec};
use elm_effect_schema::representation_doc::{to_representation, Document};
use elm_effect_schema::{Field, Schema, Unknown};
use serde_json::{json, Value};

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn number() -> Schema<Unknown> {
    Schema::as_unknown(Schema::number())
}

fn boolean() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

fn document(schema: &Schema<Unknown>) -> Value {
    let Document {
        representation,
        references,
    } = to_representation(schema).expect("converts");
    json!({ "representation": representation, "references": references })
}

fn identified(schema: Schema<Unknown>, id: &str) -> Schema<Unknown> {
    Schema::annotated_with(schema, vec![("identifier".to_string(), id.to_string())])
}

// 22 (partial): the keyword loop. Undefined, BigInt and Symbol have no
// constructors in this port, so the nine expressible keywords carry the row.
#[test]
fn converts_keywords() {
    for (tag, schema) in [
        ("Null", Schema::as_unknown(Schema::null())),
        ("Void", Schema::as_unknown(Schema::void())),
        ("Never", Schema::as_unknown(Schema::never())),
        ("Unknown", Schema::as_unknown(Schema::unknown())),
        ("Any", Schema::as_unknown(Schema::any())),
        ("String", string()),
        ("Number", number()),
        ("Boolean", boolean()),
        (
            "ObjectKeyword",
            Schema::as_unknown(Schema::object_keyword()),
        ),
    ] {
        assert_eq!(
            document(&schema),
            json!({ "representation": { "_tag": tag, "checks": [] }, "references": {} }),
            "keyword {tag}",
        );
    }
}

// 30: a literal converts to its plain value.
#[test]
fn converts_literal() {
    assert_eq!(
        document(&Schema::as_unknown(Schema::literal(Unknown::Text(
            "value".to_string()
        )))),
        json!({
            "representation": { "_tag": "Literal", "literal": "value", "checks": [] },
            "references": {},
        })
    );
}

// 77: tuple elements keep their optionality, the rest follows.
#[test]
fn converts_tuple_elements_and_rest() {
    let schema = Schema::as_unknown(Schema::tuple_with_rest(
        vec![string(), Schema::optional_key_shape(number())],
        boolean(),
    ));
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "Arrays",
                "elements": [
                    { "type": { "_tag": "String", "checks": [] }, "isOptional": false },
                    { "type": { "_tag": "Number", "checks": [] }, "isOptional": true }
                ],
                "rest": [{ "_tag": "Boolean", "checks": [] }],
                "checks": []
            },
            "references": {},
        })
    );
}

// 97 (substituted): object properties and index signatures. The pin keys the
// catch-all by Symbol/BigInt, which have no constructors here.
#[test]
fn converts_object_properties_and_index_signatures() {
    let schema = Schema::as_unknown(Schema::struct_with_rest(
        vec![
            ("required".to_string(), string()),
            ("optional".to_string(), Schema::optional_key_shape(number())),
        ],
        Schema::as_unknown(Schema::record2(string(), number())),
    ));
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "Objects",
                "propertySignatures": [
                    {
                        "name": "required",
                        "type": { "_tag": "String", "checks": [] },
                        "isOptional": false,
                        "isMutable": false
                    },
                    {
                        "name": "optional",
                        "type": { "_tag": "Number", "checks": [] },
                        "isOptional": true,
                        "isMutable": false
                    }
                ],
                "indexSignatures": [{
                    "parameter": { "_tag": "String", "checks": [] },
                    "type": { "_tag": "Number", "checks": [] }
                }],
                "checks": []
            },
            "references": {},
        })
    );
}

// 140 (substituted): union members keep their order. BigInt is swapped for
// Number, which the port can construct.
#[test]
fn converts_union_members_in_order() {
    let schema = Schema::union_with_mode(vec![string(), number()], "anyOf");
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "Union",
                "types": [
                    { "_tag": "String", "checks": [] },
                    { "_tag": "Number", "checks": [] }
                ],
                "checks": []
            },
            "references": {},
        })
    );
}

// 154: supplied Union options are kept, absent options are omitted.
#[test]
fn preserves_union_options_and_omits_absent_options() {
    let schema = Schema::union_with_mode(vec![string(), number()], "oneOf");
    let with_options = document(&schema);
    assert_eq!(
        with_options["representation"]["options"],
        json!({ "mode": "oneOf" })
    );
    let schema = Schema::union_with_mode(vec![string(), number()], "anyOf");
    let without_options = document(&schema);
    assert_eq!(without_options["representation"]["_tag"], json!("Union"));
    assert!(without_options["representation"].get("options").is_none());
}

// 339: a filter without persistence metadata converts bare.
#[test]
fn converts_filters_without_persistence_metadata() {
    let spec = FilterSpec::new(|_| FilterOutcome::from_bool(true))
        .annotated(vec![("expected".to_string(), "custom".to_string())]);
    let schema = Schema::filtered(string(), vec![spec], false);
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "String",
                "checks": [{
                    "_tag": "Filter",
                    "annotations": { "expected": "custom" },
                    "aborted": false
                }]
            },
            "references": {},
        })
    );
}

// 355: a filter without annotations converts with only its abort flag.
#[test]
fn converts_a_filter_without_annotations() {
    let spec = FilterSpec::new(|_| FilterOutcome::from_bool(true));
    let schema = Schema::filtered(string(), vec![spec], false);
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "String",
                "checks": [{ "_tag": "Filter", "aborted": false }]
            },
            "references": {},
        })
    );
}

// 447: a check on an array converts with its built-in identity.
#[test]
fn converts_checks_on_arrays() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::array(string())),
        "isMinLength",
        vec!["1"],
        vec![],
    );
    let converted = document(&schema);
    assert_eq!(converted["representation"]["_tag"], json!("Arrays"));
    assert_eq!(
        converted["representation"]["checks"][0]["representation"],
        json!({ "id": "effect/schema/isMinLength", "payload": { "minLength": 1 } })
    );
}

// 461: a check on an object converts with its built-in identity.
#[test]
fn converts_checks_on_objects() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::record2(string(), number())),
        "isMinProperties",
        vec!["1"],
        vec![],
    );
    let converted = document(&schema);
    assert_eq!(converted["representation"]["_tag"], json!("Objects"));
    assert_eq!(
        converted["representation"]["checks"][0]["representation"],
        json!({ "id": "effect/schema/isMinProperties", "payload": { "minProperties": 1 } })
    );
}

// 494: tuple element annotations convert onto the position.
#[test]
fn converts_tuple_element_annotations() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![Schema::annotate_key(
        string(),
        vec![("description".to_string(), "element".to_string())],
    )]));
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "Arrays",
                "elements": [{
                    "type": { "_tag": "String", "checks": [] },
                    "isOptional": false,
                    "annotations": { "description": "element" }
                }],
                "rest": [],
                "checks": []
            },
            "references": {},
        })
    );
}

// 513: property annotations convert onto the property.
#[test]
fn converts_property_annotations() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::required(
        "value",
        Schema::annotate_key(
            string(),
            vec![("description".to_string(), "property".to_string())],
        ),
    )]));
    assert_eq!(
        document(&schema),
        json!({
            "representation": {
                "_tag": "Objects",
                "propertySignatures": [{
                    "name": "value",
                    "type": { "_tag": "String", "checks": [] },
                    "isOptional": false,
                    "isMutable": false,
                    "annotations": { "description": "property" }
                }],
                "indexSignatures": [],
                "checks": []
            },
            "references": {},
        })
    );
}

// 538: only identifier-annotated schemas are extracted by default.
#[test]
fn extracts_only_candidates_with_identifiers_by_default() {
    let anonymous = Schema::as_unknown(Schema::struct_shape(vec![Field::required(
        "value",
        string(),
    )]));
    let identified_schema = identified(
        Schema::as_unknown(Schema::struct_shape(vec![Field::required(
            "value",
            string(),
        )])),
        "Identified",
    );
    let schema = Schema::as_unknown(Schema::tuple_of(vec![anonymous, identified_schema]));
    let converted = document(&schema);
    let references = converted["references"]
        .as_object()
        .expect("references")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(references, vec!["Identified".to_string()]);
    assert_eq!(converted["representation"]["_tag"], json!("Arrays"));
    let tags = converted["representation"]["elements"]
        .as_array()
        .expect("elements")
        .iter()
        .map(|element| element["type"]["_tag"].clone())
        .collect::<Vec<_>>();
    assert_eq!(tags, vec![json!("Objects"), json!("Reference")]);
}

// 588: a named schema converts to a reference plus its definition.
#[test]
fn extracts_a_named_schema_into_references() {
    let schema = Schema::annotated_with(
        string(),
        vec![
            ("identifier".to_string(), "Value".to_string()),
            ("description".to_string(), "value".to_string()),
        ],
    );
    assert_eq!(
        document(&schema),
        json!({
            "representation": { "_tag": "Reference", "$ref": "Value" },
            "references": {
                "Value": {
                    "_tag": "String",
                    "annotations": { "identifier": "Value", "description": "value" },
                    "checks": []
                }
            },
        })
    );
}

// 603: a definition literally named `__proto__` converts like any other key.
#[test]
fn supports_proto_as_an_identifier() {
    let schema = identified(string(), "__proto__");
    let converted = document(&schema);
    assert_eq!(
        converted["representation"],
        json!({ "_tag": "Reference", "$ref": "__proto__" })
    );
    let references = converted["references"].as_object().expect("references");
    assert_eq!(
        references.keys().cloned().collect::<Vec<_>>(),
        vec!["__proto__".to_string()]
    );
    assert_eq!(references["__proto__"]["_tag"], json!("String"));
}

// A recursive node: a struct with an optional suspended back edge, held in a
// slot the suspend thunk reads once the struct exists.
fn recursive_node(id: &str) -> Schema<Unknown> {
    let slot: Arc<OnceLock<Schema<Unknown>>> = Arc::new(OnceLock::new());
    let read = slot.clone();
    let suspended = Schema::<Unknown>::suspend(move || read.get().cloned().expect("node is set"));
    let node = Schema::annotated_with(
        Schema::as_unknown(Schema::struct_shape(vec![Field::optional_key(
            "next", suspended,
        )])),
        vec![("identifier".to_string(), id.to_string())],
    );
    slot.set(node.clone()).ok();
    node
}

// 626: a recursive schema converts to a reference with a suspended back edge.
#[test]
fn uses_an_outer_identifier_for_a_recursive_schema() {
    let converted = document(&recursive_node("Node"));
    assert_eq!(
        converted,
        json!({
            "representation": { "_tag": "Reference", "$ref": "Node" },
            "references": {
                "Node": {
                    "_tag": "Objects",
                    "annotations": { "identifier": "Node" },
                    "propertySignatures": [{
                        "name": "next",
                        "type": {
                            "_tag": "Suspend",
                            "thunk": { "_tag": "Reference", "$ref": "Node" },
                            "checks": []
                        },
                        "isOptional": true,
                        "isMutable": false
                    }],
                    "indexSignatures": [],
                    "checks": []
                }
            },
        })
    );
}

// 657: a second, different body under a taken name is suffixed and renamed.
#[test]
fn suffixes_duplicate_identifiers_on_recursive_schemas() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![
        recursive_node("Node"),
        recursive_node("Node"),
    ]));
    let converted = document(&schema);
    let tags = converted["representation"]["elements"]
        .as_array()
        .expect("elements")
        .iter()
        .map(|element| element["type"]["$ref"].clone())
        .collect::<Vec<_>>();
    assert_eq!(tags, vec![json!("Node"), json!("Node_1")]);
    let references = converted["references"].as_object().expect("references");
    assert_eq!(
        references.keys().cloned().collect::<Vec<_>>(),
        vec!["Node".to_string(), "Node_1".to_string()]
    );
    // The suffixed body carries the suffixed name, and its back edge follows.
    assert_eq!(
        references["Node_1"]["annotations"],
        json!({ "identifier": "Node_1" })
    );
    assert_eq!(
        references["Node_1"]["propertySignatures"][0]["type"]["thunk"],
        json!({ "_tag": "Reference", "$ref": "Node_1" })
    );
}

// 721: an identifier below a check stays inline instead of extracting.
#[test]
fn does_not_resolve_an_identifier_below_a_check() {
    let schema = Schema::checked(
        Schema::annotated_with(
            string(),
            vec![("identifier".to_string(), "Text".to_string())],
        ),
        "isMinLength",
        vec!["1"],
        vec![],
    );
    let converted = document(&schema);
    assert_eq!(converted["representation"]["_tag"], json!("String"));
    assert_eq!(converted["references"], json!({}));
    assert_eq!(
        converted["representation"]["annotations"],
        json!({ "identifier": "Text" })
    );
    assert_eq!(
        converted["representation"]["checks"][0]["representation"]["id"],
        json!("effect/schema/isMinLength")
    );
}

// 736: a fallback identifier extracts under its encoded name.
#[test]
fn uses_a_fallback_identifier_for_encoded_representations() {
    let schema = Schema::annotated_with(
        string(),
        vec![("~identifier".to_string(), "Person".to_string())],
    );
    assert_eq!(
        document(&schema),
        json!({
            "representation": { "_tag": "Reference", "$ref": "PersonEncoded" },
            "references": {
                "PersonEncoded": {
                    "_tag": "String",
                    "checks": [],
                    "annotations": { "~identifier": "Person" }
                }
            },
        })
    );
}

// 751: an explicit identifier wins over a fallback one.
#[test]
fn prefers_an_identifier_over_a_fallback_identifier() {
    let schema = Schema::annotated_with(
        string(),
        vec![
            ("identifier".to_string(), "EncodedPerson".to_string()),
            ("~identifier".to_string(), "Person".to_string()),
        ],
    );
    let converted = document(&schema);
    assert_eq!(
        converted["representation"],
        json!({ "_tag": "Reference", "$ref": "EncodedPerson" })
    );
    assert_eq!(
        converted["references"]
            .as_object()
            .expect("references")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["EncodedPerson".to_string()]
    );
}
