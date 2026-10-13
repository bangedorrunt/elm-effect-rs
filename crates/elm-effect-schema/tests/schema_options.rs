//! Slice 4: options/eagerness semantics.
//! Binds Schema.test.ts:131 (`does not interpret custom parseOptions
//! metadata`) and :5945 (`tagDefaultOmit`). The other two tests here are
//! port-local: their pin rows (:59 eagerness, :735 `__proto__`) assert
//! JavaScript surfaces this port does not have, and the ledger carries the
//! reason rather than a witness.

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

/// `Schema.Number.check(Schema.isGreaterThan(0), Schema.isInt())` — the base the
/// metadata row annotates. Rebuilt per call so both spellings stay independent.
fn bounded_int() -> Schema<Unknown> {
    Schema::checked(
        Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isGreaterThan",
            vec!["0"],
            vec![],
        ),
        "isInt",
        vec![],
        vec![],
    )
}

/// Schema.test.ts:131 `does not interpret custom parseOptions metadata`.
/// The annotation rides the codec: the annotated schema rejects and accepts
/// exactly what the bare one does, so `errors: "all"` changes nothing here. The
/// pin's expected text names the failing check; this port's check errors are
/// generic `TypeMismatch`es, so the claim covers which value fails.
#[test]
fn custom_metadata_is_not_interpreted() {
    let plain = bounded_int();
    let annotated = Schema::annotated_with(
        bounded_int(),
        vec![("parseOptions", r#"{"errors": "all"}"#)],
    );

    let rejected = Unknown::Number(-1.2);
    assert!(plain.decode(rejected.clone()).is_err());
    assert!(annotated.decode(rejected).is_err());

    let accepted = Unknown::Number(1.0);
    assert_eq!(annotated.decode(accepted.clone()), plain.decode(accepted));
    // A non-integer inside the bound still fails, so the annotation did not
    // relax the second check either.
    assert!(annotated.decode(Unknown::Number(1.5)).is_err());
}

/// Port-local (pin row Schema.test.ts:735 is not-applicable): with no prototype
/// chain, `__proto__` is an ordinary key that decodes as its own field.
#[test]
fn proto_key_is_ordinary_data() {
    let schema = Schema::struct_shape(vec![Field::required(
        "__proto__",
        Schema::as_unknown(Schema::string()),
    )]);
    let out: BTreeMap<String, Unknown> = schema
        .decode(obj(vec![("__proto__", Unknown::Text("polluted".to_string()))]))
        .unwrap();
    assert_eq!(
        out.get("__proto__"),
        Some(&Unknown::Text("polluted".to_string()))
    );
    assert!(schema.decode(obj(vec![("other", Unknown::Text("a".to_string()))])).is_err());
}

/// Schema.test.ts:5945 `tagDefaultOmit`.
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
    // A tag other than the default fails decoding.
    assert!(
        schema
            .decode(obj(vec![
                ("_tag", Unknown::Text("c".to_string())),
                ("a", Unknown::Text("1".to_string()))
            ]))
            .is_err()
    );
    // encoding omits it, and leaves the payload field.
    let encoded = schema.encode(&decoded);
    assert!(matches!(encoded, Unknown::Object(ref m) if !m.contains_key("_tag")));
    assert!(matches!(encoded, Unknown::Object(ref m) if m.contains_key("a")));
    let encoded_made = schema.encode(&made);
    assert!(matches!(encoded_made, Unknown::Object(ref m) if !m.contains_key("_tag")));
}
