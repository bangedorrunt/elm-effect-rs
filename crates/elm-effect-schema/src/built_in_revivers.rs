//! Built-in revivers: mapping from representation node kinds to the constructors
//! that recreate the corresponding schema values.
//!
//! `from_representations` revives the serializable document the pin's
//! `SchemaRepresentation` writes: primitive tags, `Reference` links into a
//! shared `references` table, and `Objects` shapes. Only definitions a root
//! reaches are revived, an alias chain normalizes to its target while the
//! outer name stays on as the `identifier` annotation, and a reference met
//! while its own definition is still being built becomes a lazy `suspend`
//! back edge. Anything else (`Declaration`, checks, index signatures, mutable
//! fields) fails the import instead of guessing.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use crate::{Field, Schema, SchemaError, Unknown};

/// Revive one schema per root representation, sharing the definitions in
/// `references`. The pin's `revivers` option is the empty list in every row
/// this covers, so reviver-backed tags fail rather than resolve elsewhere.
pub fn from_representations(
    representations: Vec<serde_json::Value>,
    references: BTreeMap<String, serde_json::Value>,
) -> Result<Vec<Schema<Unknown>>, SchemaError> {
    let mut state = Revive {
        references: &references,
        bodies: BTreeMap::new(),
        slots: BTreeMap::new(),
        resolving: Vec::new(),
    };
    representations
        .iter()
        .enumerate()
        .map(|(index, doc)| {
            revive_doc(&mut state, doc, &format!("representations[{index}]"))
        })
        .collect()
}

struct Revive<'a> {
    references: &'a BTreeMap<String, serde_json::Value>,
    bodies: BTreeMap<String, Schema<Unknown>>,
    slots: BTreeMap<String, Arc<OnceLock<Schema<Unknown>>>>,
    resolving: Vec<String>,
}

fn invalid(path: &str, message: impl Into<String>) -> SchemaError {
    SchemaError::InvalidJson {
        path: path.to_string(),
        message: message.into(),
    }
}

fn tag_of<'a>(doc: &'a serde_json::Value, path: &str) -> Result<&'a str, SchemaError> {
    doc.get("_tag")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid(path, "a representation carries a string `_tag`"))
}

fn no_checks(doc: &serde_json::Value, path: &str) -> Result<(), SchemaError> {
    match doc.get("checks") {
        None => Ok(()),
        Some(serde_json::Value::Array(checks)) if checks.is_empty() => Ok(()),
        _ => Err(invalid(path, "checks are not revived by this port")),
    }
}

/// The pin annotates every resolved reference with its own name, so an alias
/// chain keeps the outer name while decoding as its target.
fn resolve_reference(
    state: &mut Revive,
    key: &str,
    path: &str,
) -> Result<Schema<Unknown>, SchemaError> {
    if let Some(body) = state.bodies.get(key) {
        return Ok(body.clone());
    }
    // A definition met while it is still being built is a cycle, which a
    // lazy back edge breaks: the wrapper reads the finished body on first
    // decode, once the whole import has returned.
    if state.resolving.iter().any(|name| name == key) {
        let slot = state.slots.get(key).cloned().ok_or_else(|| {
            invalid(path, format!("reference `{key}` is resolving without a slot"))
        })?;
        return Ok(Schema::<Unknown>::suspend(move || {
            slot.get()
                .cloned()
                .expect("a revived reference resolves before it decodes")
        }));
    }
    let doc = state
        .references
        .get(key)
        .ok_or_else(|| invalid(path, format!("invalid reference `{key}`")))?
        .clone();
    state
        .slots
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(OnceLock::new()));
    state.resolving.push(key.to_string());
    let schema = revive_doc(state, &doc, &format!("references[{key}]"));
    state.resolving.pop();
    let schema = schema.map(|schema| {
        Schema::annotated_with(schema, vec![("identifier".to_string(), key.to_string())])
    })?;
    state.bodies.insert(key.to_string(), schema.clone());
    // A back edge created while this body was being built reads this slot,
    // so it must hold the finished body before any decode can reach it.
    if let Some(slot) = state.slots.get(key) {
        let _ = slot.set(schema.clone());
    }
    Ok(schema)
}

fn revive_doc(
    state: &mut Revive,
    doc: &serde_json::Value,
    path: &str,
) -> Result<Schema<Unknown>, SchemaError> {
    no_checks(doc, path)?;
    match tag_of(doc, path)? {
        "String" => Ok(Schema::as_unknown(Schema::string())),
        "Boolean" => Ok(Schema::as_unknown(Schema::boolean())),
        "Number" => Ok(Schema::as_unknown(Schema::number())),
        "Reference" => {
            let target = doc
                .get("$ref")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| invalid(path, "a `Reference` carries a string `$ref`"))?;
            resolve_reference(state, target, path)
        }
        "Objects" => {
            let signatures = doc
                .get("propertySignatures")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| invalid(path, "an `Objects` carries `propertySignatures`"))?;
            if doc
                .get("indexSignatures")
                .is_some_and(|signatures| signatures != &serde_json::Value::Array(Vec::new()))
            {
                return Err(invalid(path, "index signatures are not revived by this port"));
            }
            let mut fields = Vec::with_capacity(signatures.len());
            for (index, property) in signatures.iter().enumerate() {
                let field_path = format!("{path}[propertySignatures][{index}]");
                let name = property
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| invalid(&field_path, "a property carries a string `name`"))?;
                let ty = property.get("type").ok_or_else(|| {
                    invalid(&field_path, "a property carries a `type` representation")
                })?;
                if property
                    .get("isMutable")
                    .is_some_and(|mutable| mutable != &serde_json::Value::Bool(false))
                {
                    return Err(invalid(&field_path, "mutable fields are not revived by this port"));
                }
                let schema = revive_doc(state, ty, &format!("{field_path}[type]"))?;
                fields.push(if property.get("isOptional") == Some(&serde_json::Value::Bool(true)) {
                    Field::optional_key(name, schema)
                } else {
                    Field::required(name, schema)
                });
            }
            Ok(Schema::as_unknown(Schema::struct_shape(fields)))
        }
        other => Err(invalid(
            path,
            format!("no reviver for `{other}` without the pin's reviver set"),
        )),
    }
}
