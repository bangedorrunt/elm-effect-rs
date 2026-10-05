//! Slice 4: options/eagerness semantics.
//! Ports Schema.test.ts rows 59/131/3601 by exact call.

use elm_effect_schema::{Field, Schema, Unknown};
use std::collections::BTreeMap;

fn obj(pairs: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn sync_decode_and_encode_stay_eager() {
    let schema = Schema::struct_shape(vec![Field::required(
        "n",
        Schema::as_unknown(Schema::number()),
    )]);
    // Inner failures surface inline during the outer call: nothing is
    // deferred.
    assert!(schema.decode(obj(vec![("n", Unknown::Null)])).is_err());
    let map: BTreeMap<String, Unknown> =
        schema.decode(obj(vec![("n", Unknown::Number(1.0))])).unwrap();
    assert_eq!(schema.encode(&map), obj(vec![("n", Unknown::Number(1.0))]));
}

#[test]
fn custom_metadata_is_not_interpreted() {
    let plain = Schema::as_unknown(Schema::string());
    let annotated = Schema::annotated(
        Schema::as_unknown(Schema::string()),
        Some("custom"),
        Some("unknown option payload"),
    );
    // Annotations ride along but never change decoding.
    assert_eq!(
        annotated.decode(Unknown::Text("a".to_string())),
        plain.decode(Unknown::Text("a".to_string()))
    );
    assert_eq!(
        annotated.decode(Unknown::Null).is_err(),
        plain.decode(Unknown::Null).is_err()
    );
}

#[test]
fn proto_key_is_ordinary_data() {
    let schema = Schema::struct_shape(vec![Field::required(
        "__proto__",
        Schema::as_unknown(Schema::string()),
    )]);
    // No prototype chain exists: the key decodes as an own field.
    let out: BTreeMap<String, Unknown> = schema
        .decode(obj(vec![("__proto__", Unknown::Text("polluted".to_string()))]))
        .unwrap();
    assert_eq!(
        out.get("__proto__"),
        Some(&Unknown::Text("polluted".to_string()))
    );
}

#[test]
fn tag_fills_default_and_omits_on_encode() {
    let schema = Schema::struct_shape(vec![
        Field::tag("_tag", Unknown::Text("a".to_string())),
        Field::required("a", Schema::as_unknown(Schema::string())),
    ]);
    // make fills the missing tag.
    let made = schema
        .make(
            [("a".to_string(), Unknown::Text("1".to_string()))]
                .into_iter()
                .collect(),
        )
        .unwrap();
    assert_eq!(made.get("_tag"), Some(&Unknown::Text("a".to_string())));
    // decoding fills it too.
    let decoded = schema
        .decode(obj(vec![("a", Unknown::Text("1".to_string()))]))
        .unwrap();
    assert_eq!(decoded.get("_tag"), Some(&Unknown::Text("a".to_string())));
    // encoding omits it.
    let encoded = schema.encode(&decoded);
    assert!(matches!(encoded, Unknown::Object(ref m) if !m.contains_key("_tag")));
    assert!(matches!(encoded, Unknown::Object(ref m) if m.contains_key("a")));
}
