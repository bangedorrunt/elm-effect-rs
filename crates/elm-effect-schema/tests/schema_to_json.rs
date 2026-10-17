//! `toJson` JSON forms
//! (packages/effect/test/schema/representation/toJson.test.ts).
//!
//! `toJson` validates a document and normalizes it to plain JSON: literals
//! that spell non-finite numbers travel in typed wrappers, everything else
//! passes through. Annotations in this port are string pairs, so JSON-valued
//! ones ride as canonical text and re-emit as JSON; function-valued
//! annotations cannot exist here, which settles the pruning rows.
//!
//! Excused, with reasons:
//! - `:72/:110 live callbacks` (2 rows): no function-valued callbacks exist to
//!   strip; JSON annotations are preserved instead.
//! - `:150/:172/:200 pruning` (3 rows): vacuous, every stored annotation is
//!   already JSON-safe; only the reserved port-internal pairs are stripped.
//! - `:282 cyclic`, `:299 sparse array`: `serde_json` values cannot be cyclic
//!   or sparse.
//! - `:315 bigint`, `:449 bigint structural values`: no bigint domain.
//! - `:330 undefined`: no undefined domain.
//! - `:345 accessors`: no getters.
//! - `:365 filter groups without an identity`: building a group needs custom
//!   callbacks plus a group constructor neither side has.
//! - `:463 ambiguous Enum values`: needs the Enum constructor.
//! - `:488 ambiguous property names`: needs symbol property names.
//! - `:528 global symbols`, `:544/:554 local symbols` (3 rows): no symbol
//!   domain to encode or reject.
//! - `:572 recursive references`: the built suspend cannot convert (a thunk is
//!   a closure the kind does not carry).

use std::collections::BTreeMap;

use elm_effect_schema::representation_doc::{
    from_json, from_representation, to_json, to_representation, Document,
};
use elm_effect_schema::{Field, Schema, Unknown};
use serde_json::{json, Value};

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn roundtrip(representation: Value) -> Value {
    let document = Document {
        representation,
        references: BTreeMap::new(),
    };
    let json = to_json(&document).expect("serializes");
    let back = from_json(&json).expect("parses");
    assert_eq!(back.representation, document.representation);
    assert!(back.references.is_empty());
    json
}

// 35: an empty reference is rejected with its path.
#[test]
fn rejects_invalid_documents() {
    let document = Document {
        representation: json!({ "_tag": "Reference", "$ref": "" }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        to_json(&document).expect_err("fails").to_string(),
        "Expected a value with a length of at least 1\n  at [\"representation\"][\"$ref\"]"
    );
}

// 46: checks on a Suspend are excess properties.
#[test]
fn rejects_checks_on_suspend_representations() {
    let document = Document {
        representation: json!({
            "_tag": "Suspend",
            "checks": [Value::Null],
            "thunk": { "_tag": "String", "checks": [] },
        }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        to_json(&document).expect_err("fails").to_string(),
        "Expected no excess property\n  at [\"representation\"][\"checks\"][0]"
    );
}

// 61: persisting a Filter without an identity reports the missing key.
#[test]
fn requires_representation_when_persisting_a_filter() {
    let document = Document {
        representation: json!({
            "_tag": "String",
            "checks": [{ "_tag": "Filter", "aborted": false }],
        }),
        references: BTreeMap::new(),
    };
    assert_eq!(
        to_json(&document).expect_err("fails").to_string(),
        "Missing key\n  at [\"representation\"][\"checks\"][0][\"representation\"]"
    );
}

// 135: JSON annotations survive the conversion.
#[test]
fn preserves_json_annotations() {
    let schema = Schema::annotated_with(
        string(),
        vec![(
            "values".to_string(),
            json!(["1", "Symbol(a)", "NaN"]).to_string(),
        )],
    );
    let document = to_representation(&schema).expect("converts");
    assert_eq!(
        to_json(&document).expect("serializes")["representation"],
        json!({
            "_tag": "String",
            "annotations": { "values": ["1", "Symbol(a)", "NaN"] },
            "checks": [],
        })
    );
}

// 161: an absent annotations map stays absent.
#[test]
fn omits_undefined_annotations() {
    let json = roundtrip(json!({ "_tag": "String", "checks": [] }));
    assert!(json["representation"].get("annotations").is_none());
}

// 240: `__proto__` is a plain data key in this port's maps.
#[test]
fn preserves_proto_annotations_as_data_properties() {
    let json = roundtrip(json!({
        "_tag": "String",
        "annotations": { "__proto__": "safe" },
        "checks": [],
    }));
    assert_eq!(
        json["representation"]["annotations"],
        json!({ "__proto__": "safe" })
    );
}

// 261: one JSON text shared by two pairs emits twice, like shared values do.
#[test]
fn preserves_shared_json_annotation_values() {
    let shared = json!({ "value": "shared" }).to_string();
    let schema = Schema::annotated_with(
        string(),
        vec![
            ("left".to_string(), shared.clone()),
            ("right".to_string(), shared),
        ],
    );
    let document = to_representation(&schema).expect("converts");
    let json = to_json(&document).expect("serializes");
    assert_eq!(
        json["representation"]["annotations"],
        json!({ "left": { "value": "shared" }, "right": { "value": "shared" } })
    );
}

// 401: tuple element annotations persist without the live half, which this
// port cannot build. The description (the JSON half) carries the row.
#[test]
fn preserves_tuple_element_annotations_independently() {
    let schema = Schema::as_unknown(Schema::tuple_of(vec![Schema::annotate_key(
        string(),
        vec![("description".to_string(), "element".to_string())],
    )]));
    let document = to_representation(&schema).expect("converts");
    assert_eq!(
        to_json(&document).expect("serializes")["representation"],
        json!({
            "_tag": "Arrays",
            "elements": [{
                "type": { "_tag": "String", "checks": [] },
                "isOptional": false,
                "annotations": { "description": "element" }
            }],
            "rest": [],
            "checks": [],
        })
    );
}

// 424: property annotations persist the same way.
#[test]
fn preserves_property_annotations_independently() {
    let schema = Schema::as_unknown(Schema::struct_shape(vec![Field::required(
        "value",
        Schema::annotate_key(
            string(),
            vec![("description".to_string(), "property".to_string())],
        ),
    )]));
    let document = to_representation(&schema).expect("converts");
    assert_eq!(
        to_json(&document).expect("serializes")["representation"],
        json!({
            "_tag": "Objects",
            "propertySignatures": [{
                "name": "value",
                "type": { "_tag": "String", "checks": [] },
                "isOptional": false,
                "isMutable": false,
                "annotations": { "description": "property" }
            }],
            "indexSignatures": [],
            "checks": [],
        })
    );
}

// 516: string literals that spell non-finite numbers travel wrapped, and
// revive back to the same literal.
#[test]
fn preserves_string_literals_resembling_non_finite_numbers() {
    for literal in ["NaN", "Infinity", "-Infinity"] {
        let schema = Schema::as_unknown(Schema::literal(Unknown::Text(literal.to_string())));
        let document = to_representation(&schema).expect("converts");
        let json = to_json(&document).expect("serializes");
        assert_eq!(
            json["representation"],
            json!({
                "_tag": "Literal",
                "literal": { "type": "string", "value": literal },
                "checks": [],
            }),
            "literal {literal}"
        );
        let revived =
            from_representation(&from_json(&json).expect("parses"), &[]).expect("revives");
        assert_eq!(
            revived.decode(Unknown::Text(literal.to_string())),
            Ok(Unknown::Text(literal.to_string()))
        );
    }
    // Ordinary literals stay plain.
    let schema = Schema::as_unknown(Schema::literal(Unknown::Text("value".to_string())));
    let json = to_json(&to_representation(&schema).expect("converts")).expect("serializes");
    assert_eq!(json["representation"]["literal"], json!("value"));
}

// 591: encoded-form annotations persist. There is no fromJsonString
// constructor in this port, so the annotation shape is built directly.
#[test]
fn encodes_from_json_string_annotations() {
    let schema = Schema::annotated_with(
        string(),
        vec![
            (
                "contentMediaType".to_string(),
                "application/json".to_string(),
            ),
            (
                "expected".to_string(),
                "a string that will be decoded as JSON".to_string(),
            ),
        ],
    );
    let document = to_representation(&schema).expect("converts");
    assert_eq!(
        to_json(&document).expect("serializes")["representation"],
        json!({
            "_tag": "String",
            "annotations": {
                "contentMediaType": "application/json",
                "expected": "a string that will be decoded as JSON"
            },
            "checks": [],
        })
    );
}
