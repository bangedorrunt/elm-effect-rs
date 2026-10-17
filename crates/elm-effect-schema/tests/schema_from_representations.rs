//! Revived representation documents (`fromRepresentations`).
//! Ports packages/effect/test/schema/representation/fromRepresentations.test.ts by exact call.

use elm_effect_schema::{SchemaKind, Unknown, from_representations};
use std::collections::BTreeMap;

fn tag(tag: &str) -> serde_json::Value {
    serde_json::json!({ "_tag": tag, "checks": [] })
}

fn reference(name: &str) -> serde_json::Value {
    serde_json::json!({ "_tag": "Reference", "$ref": name })
}

fn objects(properties: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "_tag": "Objects",
        "checks": [],
        "propertySignatures": properties,
        "indexSignatures": []
    })
}

fn optional_prop(name: &str, ty: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "type": ty,
        "isOptional": true,
        "isMutable": false
    })
}

fn required_prop(name: &str, ty: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "type": ty,
        "isOptional": false,
        "isMutable": false
    })
}

fn refs(pairs: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(name, doc)| (name.to_string(), doc.clone()))
        .collect()
}

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

// 9: root documents revive in order, each decoding its own shape.
#[test]
fn preserves_root_order() {
    let schemas =
        from_representations(vec![tag("String"), tag("Boolean"), tag("Number")], BTreeMap::new())
            .expect("roots revive");
    assert_eq!(schemas.len(), 3);
    assert_eq!(
        schemas[0].decode(Unknown::Text("value".to_string())),
        Ok(Unknown::Text("value".to_string()))
    );
    assert_eq!(schemas[1].decode(Unknown::Bool(true)), Ok(Unknown::Bool(true)));
    assert_eq!(
        schemas[2].decode(Unknown::Number(1.0)),
        Ok(Unknown::Number(1.0))
    );
    assert!(schemas[0].decode(Unknown::Bool(true)).is_err());
    assert!(schemas[2].decode(Unknown::Text("value".to_string())).is_err());
}

// 24: definitions no root reaches are never revived, so an unrevivable
// unused definition cannot fail the import.
#[test]
fn does_not_revive_unreachable_references() {
    let schemas = from_representations(
        vec![tag("String")],
        refs(&[(
            "Unused",
            serde_json::json!({
                "_tag": "Declaration",
                "typeParameters": [],
                "representation": { "id": "missing", "payload": serde_json::Value::Null },
                "checks": []
            }),
        )]),
    )
    .expect("unreachable definitions are skipped");
    assert_eq!(
        schemas[0].decode(Unknown::Text("value".to_string())),
        Ok(Unknown::Text("value".to_string()))
    );
}

// 40: an alias chain normalizes to the target shape while the outer
// reference name stays on the revived schema.
#[test]
fn normalizes_aliases_while_preserving_the_outer_reference() {
    let schemas = from_representations(
        vec![reference("Alias")],
        refs(&[
            ("Alias", reference("Value")),
            ("Value", tag("String")),
        ]),
    )
    .expect("alias chain revives");
    assert_eq!(
        schemas[0].decode(Unknown::Text("value".to_string())),
        Ok(Unknown::Text("value".to_string()))
    );
    assert!(schemas[0].decode(Unknown::Number(1.0)).is_err());
    assert_eq!(
        schemas[0].resolve_annotations(),
        Some(vec![("identifier".to_string(), "Alias".to_string())])
    );
}

// 57: two roots naming one definition share the resolved schema.
#[test]
fn shares_a_resolved_reference_between_roots() {
    let schemas = from_representations(
        vec![reference("Shared"), reference("Shared")],
        refs(&[("Shared", tag("Number"))]),
    )
    .expect("shared definition revives");
    assert_eq!(schemas.len(), 2);
    assert_eq!(schemas[0].kind(), schemas[1].kind());
    for schema in &schemas {
        // The pin annotates the resolved body without changing its shape,
        // so the shared root decodes as a Number carrying its definition name.
        let SchemaKind::Annotated { inner, .. } = schema.kind() else {
            panic!("a resolved reference keeps its annotated shape");
        };
        assert!(matches!(*inner, SchemaKind::Number));
        assert_eq!(
            schema.resolve_annotations(),
            Some(vec![("identifier".to_string(), "Shared".to_string())])
        );
        assert_eq!(
            schema.decode(Unknown::Number(1.0)),
            Ok(Unknown::Number(1.0))
        );
        assert!(schema.decode(Unknown::Text("value".to_string())).is_err());
    }
    assert_eq!(
        schemas[0].decode(Unknown::Number(1.0)),
        schemas[1].decode(Unknown::Number(1.0))
    );
}

// 72: a definition literally named `__proto__` resolves like any other key.
#[test]
fn resolves_a_reachable_proto_reference() {
    let schemas = from_representations(vec![reference("__proto__")], refs(&[("__proto__", tag("String"))]))
        .expect("__proto__ resolves");
    assert_eq!(
        schemas[0].decode(Unknown::Text("value".to_string())),
        Ok(Unknown::Text("value".to_string()))
    );
}

// 86: a definition naming itself revives with a lazy back edge.
#[test]
fn revives_recursive_definitions() {
    let schemas = from_representations(
        vec![reference("Recursive")],
        refs(&[(
            "Recursive",
            objects(serde_json::json!([
                required_prop("value", tag("Number")),
                optional_prop("next", reference("Recursive")),
            ])),
        )]),
    )
    .expect("recursive definition revives");
    let fields = schemas[0].fields().expect("recursive root is a struct");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name(), "value");
    assert_eq!(fields[1].name(), "next");
    assert!(matches!(
        fields[1].inner().kind(),
        SchemaKind::Opaque("suspend")
    ));
    assert_eq!(
        schemas[0].decode(obj(&[
            ("value", Unknown::Number(1.0)),
            ("next", obj(&[("value", Unknown::Number(2.0))]))
        ])),
        Ok(obj(&[
            ("value", Unknown::Number(1.0)),
            ("next", obj(&[("value", Unknown::Number(2.0))]))
        ]))
    );
    assert_eq!(
        schemas[0].decode(obj(&[("value", Unknown::Number(1.0))])),
        Ok(obj(&[("value", Unknown::Number(1.0))]))
    );
}

// 128: two definitions naming each other revive with concrete roots and a
// lazy back edge on one side.
#[test]
fn revives_mutually_recursive_definitions_with_concrete_roots() {
    let schemas = from_representations(
        vec![reference("A"), reference("B")],
        refs(&[
            (
                "A",
                objects(serde_json::json!([optional_prop("b", reference("B"))])),
            ),
            (
                "B",
                objects(serde_json::json!([optional_prop("a", reference("A"))])),
            ),
        ]),
    )
    .expect("mutually recursive definitions revive");
    let fields_a = schemas[0].fields().expect("root A is a struct");
    let fields_b = schemas[1].fields().expect("root B is a struct");
    assert_eq!(fields_a.len(), 1);
    assert_eq!(fields_b.len(), 1);
    let back_edge_is_lazy = matches!(fields_a[0].inner().kind(), SchemaKind::Opaque("suspend"))
        || matches!(fields_b[0].inner().kind(), SchemaKind::Opaque("suspend"));
    assert!(back_edge_is_lazy, "one back edge stays lazy");
    assert_eq!(
        schemas[0].decode(obj(&[("b", obj(&[("a", obj(&[]))]))])),
        Ok(obj(&[("b", obj(&[("a", obj(&[]))]))]))
    );
    assert_eq!(
        schemas[1].decode(obj(&[("a", obj(&[("b", obj(&[]))]))])),
        Ok(obj(&[("a", obj(&[("b", obj(&[]))]))]))
    );
}
