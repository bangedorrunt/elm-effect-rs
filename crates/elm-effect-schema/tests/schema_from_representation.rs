//! Singular `fromRepresentation` documents
//! (packages/effect/test/schema/representation/fromRepresentation.test.ts).
//!
//! Carried: every row whose schemas exist in this port. `revive` mirrors the
//! pin's helper exactly (`fromRepresentation(fromJson(toJson(toRepresentation)))`),
//! and `assert_roundtrip` additionally pins the re-emitted document, so a row
//! passes only when conversion, validation, JSON form and revival all agree.
//!
//! Excused, with reasons:
//! - `:62 Undefined`, `:98 Symbol`, `:94 BigInt`, `:122 UniqueSymbol`: the
//!   `Unknown` domain has no undefined, symbol or bigint values.
//! - `:151/:157/:168 TemplateLiteral` (3 rows): template parts are folded into
//!   an opaque kind marker, so `to_representation` cannot rebuild them.
//! - `:409 Declaration`: reviving one needs the pin's JS predicate, a custom
//!   callback this port cannot carry.
//! - `:465 reviver exception identity`: Rust has no JS exception identity to
//!   preserve across the revive boundary.

use std::collections::BTreeMap;
use std::sync::Arc;

use elm_effect_schema::filters::{FilterOutcome, FilterSpec};
use elm_effect_schema::representation_doc::{
    annotate_with_representation, from_json, from_representation, is_greater_than_reviver,
    is_int_reviver, to_json, to_representation, Document, RevivedCheck, Reviver,
};
use elm_effect_schema::{Field, Schema, SchemaKind, Unknown};
use serde_json::{json, Value};

const FILTER_ID: &str = "acme/schema/minLength";

fn min_length_reviver() -> Reviver {
    Reviver::filter(
        FILTER_ID,
        Schema::as_unknown(Schema::struct_shape(vec![Field::required(
            "minimum",
            Schema::as_unknown(Schema::number()),
        )])),
        |input| {
            let minimum = match &input.payload {
                Unknown::Object(fields) => match fields.get("minimum") {
                    Some(Unknown::Number(minimum)) => *minimum,
                    _ => f64::NAN,
                },
                _ => f64::NAN,
            };
            RevivedCheck::Custom {
                predicate: Arc::new(
                    move |value| !matches!(value, Unknown::Text(text) if (text.chars().count() as f64) < minimum),
                ),
            }
        },
    )
}

fn revive(schema: Schema<Unknown>, revivers: Vec<Reviver>) -> Schema<Unknown> {
    let document = to_representation(&schema).expect("converts");
    let json = to_json(&document).expect("serializes");
    from_representation(&from_json(&json).expect("parses"), &revivers).expect("revives")
}

fn assert_roundtrip(schema: Schema<Unknown>, revivers: Vec<Reviver>) -> Schema<Unknown> {
    let expected = to_json(&to_representation(&schema).expect("converts")).expect("serializes");
    let revived = revive(schema, revivers);
    let actual = to_json(&to_representation(&revived).expect("re-converts")).expect("serializes");
    assert_eq!(actual, expected, "re-emitted document matches");
    revived
}

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    )
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn is_ok(schema: &Schema<Unknown>, value: Unknown) -> bool {
    schema.decode(value).is_ok()
}

// 58: Null revives and keeps rejecting its neighbors.
#[test]
fn revives_null() {
    let schema = assert_roundtrip(Schema::as_unknown(Schema::null()), vec![]);
    assert!(is_ok(&schema, Unknown::Null));
    assert!(!is_ok(&schema, Unknown::Bool(true)));
}

// 70: Void revives.
#[test]
fn revives_void() {
    let schema = assert_roundtrip(Schema::as_unknown(Schema::void()), vec![]);
    assert!(is_ok(&schema, Unknown::Null));
}

// 74: Never revives and rejects everything.
#[test]
fn revives_never() {
    let schema = assert_roundtrip(Schema::as_unknown(Schema::never()), vec![]);
    assert!(!is_ok(&schema, Unknown::Null));
    assert!(!is_ok(&schema, text("value")));
}

// 78/82: Unknown and Any revive as the open shapes.
#[test]
fn revives_unknown_and_any() {
    for schema in [
        assert_roundtrip(Schema::as_unknown(Schema::unknown()), vec![]),
        assert_roundtrip(Schema::as_unknown(Schema::any()), vec![]),
    ] {
        assert!(is_ok(&schema, text("value")));
        assert!(is_ok(&schema, Unknown::Number(1.0)));
    }
}

// 86/90/94-scalar: String, Number and Boolean revive with their decoders.
#[test]
fn revives_string_number_boolean() {
    let string = assert_roundtrip(Schema::as_unknown(Schema::string()), vec![]);
    assert!(is_ok(&string, text("value")));
    assert!(!is_ok(&string, Unknown::Number(1.0)));
    let number = assert_roundtrip(Schema::as_unknown(Schema::number()), vec![]);
    assert!(is_ok(&number, Unknown::Number(1.0)));
    assert!(!is_ok(&number, text("value")));
    let boolean = assert_roundtrip(Schema::as_unknown(Schema::boolean()), vec![]);
    assert!(is_ok(&boolean, Unknown::Bool(true)));
    assert!(!is_ok(&boolean, Unknown::Number(1.0)));
}

// 102: ObjectKeyword revives and still rejects scalars.
#[test]
fn revives_object_keyword() {
    let schema = assert_roundtrip(Schema::as_unknown(Schema::object_keyword()), vec![]);
    assert!(is_ok(&schema, obj(&[])));
    assert!(!is_ok(&schema, text("value")));
}

// 106: Literals revive without changing their type. The bigint pairs need a
// bigint domain, so the JSON-domain pairs carry the row.
#[test]
fn revives_literal_without_changing_its_type() {
    for (literal, different) in [
        (text("1"), Unknown::Bool(true)),
        (Unknown::Number(1.0), text("1")),
        (Unknown::Bool(true), text("true")),
    ] {
        let schema = assert_roundtrip(Schema::as_unknown(Schema::literal(literal.clone())), vec![]);
        assert!(is_ok(&schema, literal));
        assert!(!is_ok(&schema, different));
    }
}

// 129 (partial): an Enum revives to the literal union it admits. The member
// names are lost — the port has no name-preserving Enum constructor — so only
// the decode behavior carries this row, not the document round trip.
#[test]
fn revives_enum_values() {
    let document = Document {
        representation: json!({
            "_tag": "Enum",
            "enums": [["A", "a"], ["One", 1]],
            "checks": [],
        }),
        references: BTreeMap::new(),
    };
    let schema = from_representation(&document, &[]).expect("revives");
    assert!(is_ok(&schema, text("a")));
    assert!(is_ok(&schema, Unknown::Number(1.0)));
    assert!(!is_ok(&schema, text("other")));
}

// 136: a non-finite numeric Enum value is rejected. Raw NaN is unrepresentable
// in JSON, so the row arrives through the typed wrapper `to_json` would emit.
#[test]
fn rejects_non_finite_numeric_enum_values() {
    let document = Document {
        representation: json!({
            "_tag": "Enum",
            "enums": [["NumberNaN", { "type": "number", "value": "NaN" }]],
            "checks": [],
        }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        from_representation(&document, &[])
            .err()
            .expect("rejects")
            .to_string(),
        "A numeric enum value must be finite, got NaN\n  at [\"representation\"]"
    );
}

// 183: Tuple revives with its optional element.
#[test]
fn revives_tuple() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::tuple_of(vec![
            Schema::as_unknown(Schema::string()),
            Schema::optional_key_shape(Schema::as_unknown(Schema::number())),
        ])),
        vec![],
    );
    assert!(is_ok(
        &schema,
        Unknown::Array(vec![text("value"), Unknown::Number(1.0)])
    ));
}

// 187: Array revives.
#[test]
fn revives_array() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::array(Schema::as_unknown(Schema::string()))),
        vec![],
    );
    assert!(is_ok(&schema, Unknown::Array(vec![text("value")])));
    assert!(!is_ok(&schema, Unknown::Array(vec![Unknown::Number(1.0)])));
}

// 191: TupleWithRest revives.
#[test]
fn revives_tuple_with_rest() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::tuple_with_rest(
            vec![Schema::as_unknown(Schema::string())],
            vec![Schema::as_unknown(Schema::number())],
        )),
        vec![],
    );
    assert!(is_ok(
        &schema,
        Unknown::Array(vec![text("value"), Unknown::Number(1.0)])
    ));
}

// 197: Struct revives. Mutable fields have no constructor in this port, so the
// row's required/optional shape carries it.
#[test]
fn revives_struct() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::struct_shape(vec![
            Field::required("required", Schema::as_unknown(Schema::string())),
            Field::optional_key("optional", Schema::as_unknown(Schema::number())),
        ])),
        vec![],
    );
    assert!(is_ok(
        &schema,
        obj(&[
            ("required", text("value")),
            ("optional", Unknown::Number(1.0))
        ])
    ));
    assert!(is_ok(&schema, obj(&[("required", text("value"))])));
    assert!(!is_ok(&schema, obj(&[])));
}

// 205: Record revives.
#[test]
fn revives_record() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::record2(
            Schema::as_unknown(Schema::string()),
            Schema::as_unknown(Schema::number()),
        )),
        vec![],
    );
    assert!(is_ok(&schema, obj(&[("a", Unknown::Number(1.0))])));
}

// 209: a reference used as a Record key resolves to the concrete key schema.
#[test]
fn revives_a_reference_used_as_a_record_key() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::record2(
            Schema::annotated_with(
                Schema::as_unknown(Schema::string()),
                vec![("identifier".to_string(), "Key".to_string())],
            ),
            Schema::as_unknown(Schema::number()),
        )),
        vec![],
    );
    assert!(matches!(schema.kind(), SchemaKind::Record(..)));
    assert!(is_ok(&schema, obj(&[("a", Unknown::Number(1.0))])));
}

// 220: StructWithRest revives. The pin keys the catch-all by Symbol/BigInt,
// which have no domain here, so a string-keyed catch-all carries the row.
#[test]
fn revives_struct_with_rest() {
    let schema = assert_roundtrip(
        Schema::as_unknown(Schema::struct_with_rest(
            vec![("value".to_string(), Schema::as_unknown(Schema::number()))],
            Schema::as_unknown(Schema::record2(
                Schema::as_unknown(Schema::string()),
                Schema::as_unknown(Schema::string()),
            )),
        )),
        vec![],
    );
    assert!(is_ok(
        &schema,
        obj(&[("value", Unknown::Number(1.0)), ("extra", text("kept"))])
    ));
}

// 226: Union revives and keeps member order for decoding.
#[test]
fn revives_union() {
    let schema = assert_roundtrip(
        Schema::union_with_mode(
            vec![
                Schema::as_unknown(Schema::string()),
                Schema::as_unknown(Schema::number()),
            ],
            "anyOf",
        ),
        vec![],
    );
    assert!(is_ok(&schema, text("value")));
    assert!(is_ok(&schema, Unknown::Number(1.0)));
    assert!(!is_ok(&schema, Unknown::Bool(true)));
}

// 233: Union options survive the live and JSON round trips.
#[test]
fn revives_union_options_through_live_and_json_roundtrips() {
    let representation = json!({
        "_tag": "Union",
        "types": [{ "_tag": "String", "checks": [] }, { "_tag": "Number", "checks": [] }],
        "options": { "mode": "oneOf" },
        "checks": [],
    });
    let document = Document {
        representation: representation.clone(),
        references: BTreeMap::new(),
    };
    let revived = from_representation(&document, &[]).expect("revives");
    let round = to_json(&to_representation(&revived).expect("re-converts")).expect("serializes");
    assert_eq!(round["representation"], representation);
    let from_json = from_representation(
        &from_json(&to_json(&document).expect("serializes")).expect("parses"),
        &[],
    )
    .expect("revives");
    let round = to_json(&to_representation(&from_json).expect("re-converts")).expect("serializes");
    assert_eq!(round["representation"], representation);
}

// 252: an empty Union revives as Never.
#[test]
fn revives_an_empty_union_as_never() {
    let document = Document {
        representation: json!({ "_tag": "Union", "types": [], "options": { "mode": "anyOf" }, "checks": [] }),
        references: BTreeMap::new(),
    };
    let schema = from_representation(&document, &[]).expect("revives");
    assert!(!is_ok(&schema, Unknown::Null));
    assert!(!is_ok(&schema, text("value")));
}

// 261: Suspend revives with a lazy back edge. The built schema cannot convert
// (a thunk is a closure), so the row starts from the document the pin emits.
#[test]
fn revives_suspend() {
    let category_body: Value = json!({
        "_tag": "Objects",
        "annotations": { "identifier": "Category" },
        "propertySignatures": [
            {
                "name": "value",
                "type": { "_tag": "Number", "checks": [] },
                "isOptional": false,
                "isMutable": false
            },
            {
                "name": "next",
                "type": {
                    "_tag": "Suspend",
                    "thunk": { "_tag": "Reference", "$ref": "Category" },
                    "checks": []
                },
                "isOptional": true,
                "isMutable": false
            }
        ],
        "indexSignatures": [],
        "checks": []
    });
    let document = Document {
        representation: json!({ "_tag": "Reference", "$ref": "Category" }),
        references: BTreeMap::from([("Category".to_string(), category_body.clone())]),
    };
    let schema = from_representation(&document, &[]).expect("revives");
    let fields = schema.fields().expect("recursive root is a struct");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[1].name(), "next");
    assert!(matches!(
        fields[1].inner().kind(),
        SchemaKind::Opaque("suspend")
    ));
    assert!(is_ok(
        &schema,
        obj(&[
            ("value", Unknown::Number(1.0)),
            ("next", obj(&[("value", Unknown::Number(2.0))]))
        ])
    ));
    // The revived root converts back to the reference it arrived as.
    let back = to_representation(&schema).expect("re-converts");
    assert_eq!(
        back.representation,
        json!({ "_tag": "Reference", "$ref": "Category" })
    );
    assert_eq!(back.references.get("Category"), Some(&category_body));
}

// 293: node annotations restore onto the revived schema.
#[test]
fn restores_node_annotations() {
    let schema = revive(
        Schema::annotated_with(
            Schema::as_unknown(Schema::string()),
            vec![("title".to_string(), "Name".to_string())],
        ),
        vec![],
    );
    assert_eq!(
        schema.resolve_annotations(),
        Some(vec![("title".to_string(), "Name".to_string())])
    );
}

// 298: node annotations restore before checks, each on its own level.
#[test]
fn restores_node_annotations_before_checks() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "annotations": { "title": "node" },
            "checks": [{
                "_tag": "Filter",
                "representation": { "id": FILTER_ID, "payload": { "minimum": 2 } },
                "annotations": { "description": "check" },
                "aborted": false
            }],
        }),
        references: BTreeMap::new(),
    };
    let schema = from_representation(&document, &[min_length_reviver()]).expect("revives");
    // The port keeps one annotation slot per schema, so the node/check split
    // the pin reads off the AST is asserted on the re-emitted document: the
    // title sits on the node, the description on the check, never crossed.
    let back = to_json(&to_representation(&schema).expect("re-converts")).expect("serializes");
    assert_eq!(back["representation"], document.representation);
    assert_eq!(
        back["representation"]["annotations"]["title"],
        json!("node")
    );
    assert_eq!(
        back["representation"]["checks"][0]["annotations"]["description"],
        json!("check")
    );
    assert!(back["representation"]["checks"][0]["annotations"]
        .get("title")
        .is_none());
    assert!(!is_ok(&schema, text("a")));
    assert!(is_ok(&schema, text("ab")));
}

// 311: tuple element annotations restore onto the position, not the type.
#[test]
fn restores_tuple_element_annotations() {
    let schema = revive(
        Schema::as_unknown(Schema::tuple_of(vec![Schema::annotate_key(
            Schema::as_unknown(Schema::string()),
            vec![("description".to_string(), "element".to_string())],
        )])),
        vec![],
    );
    let back = to_json(&to_representation(&schema).expect("re-converts")).expect("serializes");
    assert_eq!(
        back["representation"]["elements"][0]["annotations"],
        json!({ "description": "element" })
    );
    assert!(back["representation"]["elements"][0]["type"]
        .get("annotations")
        .is_none());
}

// 320: property annotations restore onto the property, not the type.
#[test]
fn restores_property_annotations() {
    let schema = revive(
        Schema::as_unknown(Schema::struct_shape(vec![Field::required(
            "value",
            Schema::annotate_key(
                Schema::as_unknown(Schema::string()),
                vec![("description".to_string(), "property".to_string())],
            ),
        )])),
        vec![],
    );
    let back = to_json(&to_representation(&schema).expect("re-converts")).expect("serializes");
    assert_eq!(
        back["representation"]["propertySignatures"][0]["annotations"],
        json!({ "description": "property" })
    );
}

// 329: checks added by fromBrand restore through the built-in revivers.
#[test]
fn restores_checks_added_by_from_brand() {
    let branded = Schema::checked(
        Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isInt",
            vec![],
            vec![],
        ),
        "isGreaterThan",
        vec!["0"],
        vec![],
    );
    let restored = assert_roundtrip(branded, vec![is_int_reviver(), is_greater_than_reviver()]);
    assert!(is_ok(&restored, Unknown::Number(1.0)));
    assert!(!is_ok(&restored, Unknown::Number(1.2)));
    assert!(!is_ok(&restored, Unknown::Number(-1.0)));
}

// 348: a node representation annotation without schema dependencies round-trips.
#[test]
fn restores_a_node_representation_annotation_without_schema_dependencies() {
    let schema = assert_roundtrip(
        annotate_with_representation(
            Schema::as_unknown(Schema::string()),
            "acme/schema/String",
            Value::Null,
            vec![],
        )
        .expect("annotates"),
        vec![],
    );
    assert!(schema
        .resolve_annotations()
        .expect("annotations")
        .iter()
        .any(|(key, value)| key == "representation" && value.contains("acme/schema/String")));
}

// 354: a node representation annotation with schema dependencies round-trips.
#[test]
fn restores_a_node_representation_annotation_with_schema_dependencies() {
    let schema = assert_roundtrip(
        annotate_with_representation(
            Schema::as_unknown(Schema::string()),
            "acme/schema/String",
            Value::Null,
            vec![Schema::as_unknown(Schema::number())],
        )
        .expect("annotates"),
        vec![],
    );
    assert!(is_ok(&schema, text("value")));
    let back = to_json(&to_representation(&schema).expect("re-converts")).expect("serializes");
    assert_eq!(
        back["representation"]["representation"]["schemas"],
        json!([{ "_tag": "Number", "checks": [] }])
    );
}

// 364: a Filter revives with its predicate, annotation and abort flag.
#[test]
fn revives_a_filter() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{
                "_tag": "Filter",
                "representation": { "id": FILTER_ID, "payload": { "minimum": 2 } },
                "annotations": { "description": "at least two" },
                "aborted": true
            }],
        }),
        references: BTreeMap::new(),
    };
    let schema = from_representation(&document, &[min_length_reviver()]).expect("revives");
    assert!(!is_ok(&schema, text("a")));
    assert!(is_ok(&schema, text("ab")));
    let back = to_json(&to_representation(&schema).expect("re-converts")).expect("serializes");
    assert_eq!(back["representation"], document.representation);
}

// 375: a FilterGroup without an identity revives from its children.
#[test]
fn revives_a_filter_group_without_an_identity_from_its_children() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{
                "_tag": "FilterGroup",
                "annotations": { "description": "both" },
                "checks": [
                    {
                        "_tag": "Filter",
                        "representation": { "id": FILTER_ID, "payload": { "minimum": 2 } },
                        "aborted": false
                    },
                    {
                        "_tag": "Filter",
                        "representation": { "id": FILTER_ID, "payload": { "minimum": 3 } },
                        "aborted": false
                    }
                ]
            }],
        }),
        references: BTreeMap::new(),
    };
    let schema = from_representation(&document, &[min_length_reviver()]).expect("revives");
    assert!(!is_ok(&schema, text("ab")));
    assert!(is_ok(&schema, text("abc")));
    let back = to_json(&to_representation(&schema).expect("re-converts")).expect("serializes");
    assert_eq!(back["representation"], document.representation);
}

// 382: an identified FilterGroup reviver wins over the persisted children.
#[test]
fn uses_an_identified_filter_group_reviver_instead_of_its_persisted_children() {
    let group_id = "acme/schema/group";
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{
                "_tag": "FilterGroup",
                "representation": { "id": group_id, "payload": Value::Null },
                "checks": [{
                    "_tag": "Filter",
                    "representation": { "id": FILTER_ID, "payload": { "minimum": 1 } },
                    "aborted": false
                }]
            }],
        }),
        references: BTreeMap::new(),
    };
    let reviver = Reviver::group(group_id, Schema::as_unknown(Schema::null()), |_| {
        RevivedCheck::Custom {
            predicate: Arc::new(
                |value| !matches!(value, Unknown::Text(blocked) if blocked == "blocked"),
            ),
        }
    });
    let parsed = from_json(&to_json(&document).expect("serializes")).expect("parses");
    let schema = from_representation(&parsed, &[reviver]).expect("revives");
    assert!(is_ok(&schema, text("allowed")));
    assert!(!is_ok(&schema, text("blocked")));
}

// 430: a missing reviver reports its identity and path.
#[test]
fn reports_a_missing_reviver() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{
                "_tag": "Filter",
                "representation": { "id": FILTER_ID, "payload": { "minimum": 2 } },
                "aborted": false
            }],
        }),
        references: BTreeMap::new(),
    };
    let parsed = from_json(&to_json(&document).expect("serializes")).expect("parses");
    assert_eq!(
        from_representation(&parsed, &[]).err().expect("fails").to_string(),
        "Missing reviver for acme/schema/minLength\n  at [\"representation\"][\"checks\"][0][\"representation\"]"
    );
}

// 440: duplicate reviver IDs are rejected.
#[test]
fn rejects_duplicate_reviver_ids() {
    let document = Document {
        representation: json!({ "_tag": "String", "checks": [] }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        from_representation(&document, &[min_length_reviver(), min_length_reviver()])
            .err()
            .expect("fails")
            .to_string(),
        "Duplicate reviver for acme/schema/minLength\n  at [\"revivers\"][1][\"id\"]"
    );
}

// 452: an invalid reviver payload reports its path.
#[test]
fn rejects_an_invalid_reviver_payload() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{
                "_tag": "Filter",
                "representation": { "id": FILTER_ID, "payload": { "minimum": "two" } },
                "aborted": false
            }],
        }),
        references: BTreeMap::new(),
    };
    let parsed = from_json(&to_json(&document).expect("serializes")).expect("parses");
    assert_eq!(
        from_representation(&parsed, &[min_length_reviver()]).err().expect("fails").to_string(),
        "Invalid representation payload for acme/schema/minLength\n  at [\"representation\"][\"checks\"][0][\"representation\"][\"payload\"]"
    );
}

// 483: a Filter without an identity reports the missing annotation.
#[test]
fn requires_a_representation_identity_on_a_filter() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{ "_tag": "Filter", "aborted": false }],
        }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        from_representation(&document, &[]).err().expect("fails").to_string(),
        "Missing representation annotation\n  at [\"representation\"][\"checks\"][0][\"representation\"]"
    );
}

// 498: a Declaration without an identity reports the missing annotation.
#[test]
fn requires_a_representation_identity_on_a_declaration() {
    let document = Document {
        representation: json!({ "_tag": "Declaration", "typeParameters": [], "checks": [] }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        from_representation(&document, &[])
            .err()
            .expect("fails")
            .to_string(),
        "Missing representation annotation\n  at [\"representation\"][\"representation\"]"
    );
}

// 513: an invalid reference reports its name and path.
#[test]
fn reports_an_invalid_reference() {
    let document = Document {
        representation: json!({ "_tag": "Reference", "$ref": "Missing" }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        from_representation(&document, &[])
            .err()
            .expect("fails")
            .to_string(),
        "Invalid reference Missing\n  at [\"representation\"][\"$ref\"]"
    );
}
