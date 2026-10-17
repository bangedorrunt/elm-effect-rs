//! Schema to JSON Schema compiler, phase 1: core compilation.
//!
//! Compiles the port's `SchemaKind` tree to JSON Schema documents: scalar
//! types, constrained strings/numbers/arrays/objects, enums and consts,
//! `oneOf`/`anyOf`/`allOf` unions, tuple `prefixItems`, and `$defs` hoisting
//! for identifier-annotated schemas (the same hoisting the Standard JSON
//! Schema documents use).
//!
//! Out of scope for phase 1: the encoded-side projection, custom compiler
//! hooks, the reference policy, and multi-root orchestration. Checks the
//! compiler cannot spell as a keyword (`isFinite`, `isTrimmed`,
//! `isPropertyNames`, unknown filters) degrade to the unconstrained base,
//! the same pass-through the decoder uses for a check it does not know.

use super::{Schema, SchemaKind, Unknown, kind_type_name};

/// Several roots compiled against one definition map: shared identifiers are
/// emitted once and every use is a `$ref`.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiDocument {
    /// One compiled schema per root, in input order.
    pub schemas: Vec<serde_json::Value>,
    /// Definitions hoisted from identifier annotations, shared by all roots.
    pub definitions: serde_json::Map<String, serde_json::Value>,
}

/// Compile several roots against one shared definition map. Phase 2: the
/// multi-document orchestration the harness owns; the core stays
/// single-document until then.

/// Compile one schema to a JSON Schema document, hoisting identifier
/// annotations into a `$defs` entry and emitting a `$ref` at each use.
pub fn compile(schema: &Schema<Unknown>) -> serde_json::Value {
    let mut defs = serde_json::Map::new();
    let body = compile_kind(&schema.kind(), &mut defs);
    if defs.is_empty() {
        return body;
    }
    match body {
        serde_json::Value::Object(mut map) => {
            map.insert("$defs".to_string(), serde_json::Value::Object(defs));
            serde_json::Value::Object(map)
        }
        // Only a primitive root is not an object, and a primitive root holds
        // no identifiers, so this arm is unreachable through `compile_kind`;
        // it keeps the definitions rather than dropping them.
        other => serde_json::json!({"allOf": [other], "$defs": defs}),
    }
}

/// Compile several roots against one shared definition map: an identifier
/// used by more than one root is emitted once and referenced everywhere.
pub fn compile_multi(roots: &[Schema<Unknown>]) -> MultiDocument {
    let mut definitions = serde_json::Map::new();
    let schemas = roots
        .iter()
        .map(|root| compile_kind(&root.kind(), &mut definitions))
        .collect();
    MultiDocument {
        schemas,
        definitions,
    }
}

/// Walk through annotation and check wrappers to the shaped base.
fn strip(kind: &SchemaKind) -> &SchemaKind {
    let mut current = kind;
    loop {
        match current {
            SchemaKind::Annotated { inner, .. } => current = inner,
            SchemaKind::Check { base, .. } => current = base,
            _ => return current,
        }
    }
}

/// Whether a kind is a wrapper the required computation looks through: an
/// absent-tolerant position, whether spelled `optional` or `optionalKey`.
fn is_absent_tolerant(kind: &SchemaKind) -> bool {
    matches!(strip(kind), SchemaKind::Optional(_) | SchemaKind::OptionalKey(_))
}

/// Unwrap one absent-tolerant layer for element compilation.
fn unwrap_optional(kind: &SchemaKind) -> &SchemaKind {
    match strip(kind) {
        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => inner,
        _ => kind,
    }
}

/// A check argument printed by `format_number` back to JSON. Whole values
/// keep their integer form so a document round-trips textually (`2`, not
/// `2.0`); anything else goes through the float conversion.
fn number_arg(args: &[String]) -> Option<serde_json::Value> {
    let parsed: f64 = args.first()?.parse().ok()?;
    if parsed.fract() == 0.0 && parsed >= i64::MIN as f64 && parsed <= i64::MAX as f64 {
        return Some(serde_json::Value::Number(serde_json::Number::from(parsed as i64)));
    }
    serde_json::Number::from_f64(parsed).map(serde_json::Value::Number)
}

/// A pattern check argument back to its source text. The argument is the
/// `new RegExp("...", "u")` literal the repr prints, so parsing reads the
/// JSON string back out of it.
fn pattern_arg(arg: &str) -> Option<String> {
    let inner = arg
        .strip_prefix("new RegExp(")?
        .strip_suffix(')')?
        .strip_suffix(", \"u\"")
        .unwrap_or_else(|| arg.strip_prefix("new RegExp(").unwrap_or(arg));
    serde_json::from_str::<String>(inner.trim()).ok()
}

/// Apply one named check to a document already compiled from its base. The
/// base type decides which keyword family a shared check name lands in
/// (`isMinLength` is `minLength` on a string, `minItems` on an array).
fn apply_check(
    doc: &mut serde_json::Value,
    base_type: Option<&str>,
    name: &str,
    args: &[String],
) {
    let map = match doc.as_object_mut() {
        Some(map) => map,
        None => return,
    };
    let set = |map: &mut serde_json::Map<String, serde_json::Value>, key: &str, value: serde_json::Value| {
        map.insert(key.to_string(), value);
    };
    match (base_type, name) {
        (Some("string"), "isMinLength" | "isMinCodePoints") => {
            if let Some(value) = number_arg(args) {
                set(map, "minLength", value);
            }
        }
        (Some("string"), "isMaxLength" | "isMaxCodePoints") => {
            if let Some(value) = number_arg(args) {
                set(map, "maxLength", value);
            }
        }
        (Some("string"), "isPattern") => {
            if let Some(pattern) = args.first().and_then(|arg| pattern_arg(arg)) {
                set(map, "pattern", serde_json::Value::String(pattern));
            }
        }
        (Some("number"), "isInt") => {
            set(map, "type", serde_json::Value::String("integer".to_string()));
        }
        (Some("number"), "isGreaterThanOrEqualTo") => {
            if let Some(value) = number_arg(args) {
                set(map, "minimum", value);
            }
        }
        (Some("number"), "isLessThanOrEqualTo") => {
            if let Some(value) = number_arg(args) {
                set(map, "maximum", value);
            }
        }
        (Some("number"), "isGreaterThan") => {
            if let Some(value) = number_arg(args) {
                set(map, "exclusiveMinimum", value);
            }
        }
        (Some("number"), "isLessThan") => {
            if let Some(value) = number_arg(args) {
                set(map, "exclusiveMaximum", value);
            }
        }
        (Some("number"), "isMultipleOf") => {
            if let Some(value) = number_arg(args) {
                set(map, "multipleOf", value);
            }
        }
        (Some("array"), "isMinLength") => {
            if let Some(value) = number_arg(args) {
                set(map, "minItems", value);
            }
        }
        (Some("array"), "isMaxLength") => {
            if let Some(value) = number_arg(args) {
                set(map, "maxItems", value);
            }
        }
        (Some("array"), "isUnique") => {
            set(map, "uniqueItems", serde_json::Value::Bool(true));
        }
        (Some("object"), "isMinProperties") => {
            if let Some(value) = number_arg(args) {
                set(map, "minProperties", value);
            }
        }
        (Some("object"), "isMaxProperties") => {
            if let Some(value) = number_arg(args) {
                set(map, "maxProperties", value);
            }
        }
        // `isFinite` is implied by `type: number`; `isTrimmed`,
        // `isPropertyNames` (whose argument is rendered code, not a schema),
        // grouped ranges and unknown filters have no keyword and degrade to
        // the unconstrained base.
        _ => {}
    }
}

/// Whether a record value shape is the open-JSON catch-all, which an object
/// document leaves implicit rather than spelling out.
fn is_open_value(kind: &SchemaKind) -> bool {
    matches!(strip(kind), SchemaKind::Json)
}

/// Whether a kind is the shape nothing validates against.
fn is_never(kind: &SchemaKind) -> bool {
    matches!(strip(kind), SchemaKind::Opaque("never"))
}

/// An object document for named fields: non-absent-tolerant fields are
/// required, and a plain struct is closed.
fn struct_document(
    fields: &[(String, SchemaKind)],
    open: Option<&SchemaKind>,
    defs: &mut serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, kind) in fields {
        properties.insert(name.clone(), compile_kind(unwrap_optional(kind), defs));
        if !is_absent_tolerant(kind) {
            required.push(serde_json::Value::String(name.clone()));
        }
    }
    let mut doc = serde_json::json!({
        "type": "object",
        "properties": serde_json::Value::Object(properties),
        "required": required,
    });
    match open {
        // No catch-all named: a plain struct is closed.
        None => {
            doc["additionalProperties"] = serde_json::Value::Bool(false);
        }
        // An open catch-all stays implicit; anything else is spelled out.
        Some(rest) if is_open_record_rest(rest) => {}
        Some(rest) if is_never(rest) => {
            doc["additionalProperties"] = serde_json::Value::Bool(false);
        }
        Some(rest) => {
            doc["additionalProperties"] = compile_kind(rest, defs);
        }
    }
    doc
}

/// Whether a struct catch-all is the open-JSON shape the import produces for
/// an object with no `additionalProperties` keyword.
fn is_open_record_rest(kind: &SchemaKind) -> bool {
    match strip(kind) {
        SchemaKind::Record(key, value) => {
            matches!(strip(key), SchemaKind::String) && is_open_value(value)
        }
        _ => false,
    }
}

/// Extract the pattern text from an `isPattern` check argument list.
fn check_pattern(args: &[String]) -> Option<String> {
    pattern_arg(args.first()?)
}

/// A record key split into its base shape plus the checks layered on it.
fn key_checks(kind: &SchemaKind) -> (SchemaKind, Vec<(String, Vec<String>)>) {
    let mut checks = Vec::new();
    let mut current = kind;
    loop {
        match current {
            SchemaKind::Check { base, name, args, .. } => {
                checks.push((name.clone(), args.clone()));
                current = base;
            }
            SchemaKind::Annotated { inner, .. } => {
                current = inner;
            }
            _ => return (current.clone(), checks),
        }
    }
}

fn compile_kind(kind: &SchemaKind, defs: &mut serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    match kind {
        SchemaKind::String => serde_json::json!({"type": "string"}),
        SchemaKind::Number => serde_json::json!({"type": "number"}),
        SchemaKind::Boolean => serde_json::json!({"type": "boolean"}),
        SchemaKind::Json => serde_json::json!({}),
        SchemaKind::Object => serde_json::json!({"type": "object"}),
        SchemaKind::Opaque("null") => serde_json::json!({"type": "null"}),
        SchemaKind::Opaque("never") => serde_json::json!({"not": {}}),
        SchemaKind::Opaque(name) => serde_json::json!({"$comment": name}),
        SchemaKind::Array(items) => {
            serde_json::json!({"type": "array", "items": compile_kind(items, defs)})
        }
        SchemaKind::Optional(inner) => {
            serde_json::json!({"anyOf": [compile_kind(inner, defs), {"type": "null"}]})
        }
        SchemaKind::OptionalKey(inner) => compile_kind(inner, defs),
        SchemaKind::Rest(inner) => {
            serde_json::json!({"prefixItems": [compile_kind(inner, defs)]})
        }
        SchemaKind::Struct(fields) => struct_document(fields, None, defs),
        SchemaKind::StructWithRest(fields, rest) => struct_document(fields, Some(rest), defs),
        SchemaKind::Record(key, value) => {
            let (base, checks) = key_checks(key);
            let pattern = match &base {
                SchemaKind::String => checks.iter().find_map(|(name, args)| {
                    (name == "isPattern").then(|| check_pattern(args)).flatten()
                }),
                _ => None,
            };
            match (&base, pattern) {
                (SchemaKind::String, None) if is_open_value(value) => {
                    serde_json::json!({"type": "object"})
                }
                (SchemaKind::String, None) if is_never(value) => {
                    serde_json::json!({"type": "object", "additionalProperties": false})
                }
                (SchemaKind::String, None) => {
                    serde_json::json!({
                        "type": "object",
                        "additionalProperties": compile_kind(value, defs)
                    })
                }
                // A single pattern key is a closed pattern scope, which is
                // what the import accepts; any other key shape has no
                // document form in phase 1 and degrades to an open object.
                (_, Some(pattern)) => {
                    let mut patterns = serde_json::Map::new();
                    patterns.insert(pattern, compile_kind(value, defs));
                    serde_json::json!({
                        "type": "object",
                        "patternProperties": patterns,
                        "additionalProperties": false
                    })
                }
                _ => serde_json::json!({"type": "object"}),
            }
        }
        SchemaKind::Tuple(items) => {
            let compiled: Vec<serde_json::Value> = items
                .iter()
                .map(|item| compile_kind(unwrap_optional(item), defs))
                .collect();
            let leading = items
                .iter()
                .take_while(|item| !is_absent_tolerant(item))
                .count();
            serde_json::json!({
                "type": "array",
                "prefixItems": compiled,
                "minItems": leading,
                "maxItems": compiled.len(),
            })
        }
        SchemaKind::TupleWithRest(items, rest) => {
            let compiled: Vec<serde_json::Value> = items
                .iter()
                .map(|item| compile_kind(unwrap_optional(item), defs))
                .collect();
            let leading = items
                .iter()
                .take_while(|item| !is_absent_tolerant(item))
                .count();
            let mut doc = serde_json::json!({
                "type": "array",
                "prefixItems": compiled,
                "items": compile_kind(rest, defs),
            });
            if leading < compiled.len() {
                doc["minItems"] = serde_json::Value::Number(
                    serde_json::Number::from(leading as u64),
                );
            }
            doc
        }
        SchemaKind::Literal(value) => {
            let mut doc = serde_json::Map::new();
            doc.insert("const".to_string(), value.to_json());
            serde_json::Value::Object(doc)
        }
        SchemaKind::Literals(values) => {
            serde_json::json!({"enum": values.iter().map(Unknown::to_json).collect::<Vec<_>>()})
        }
        SchemaKind::Union { members, mode } => {
            let key = match *mode {
                "oneOf" => "oneOf",
                "allOf" => "allOf",
                _ => "anyOf",
            };
            let mut doc = serde_json::Map::new();
            doc.insert(
                key.to_string(),
                serde_json::Value::Array(
                    members.iter().map(|member| compile_kind(member, defs)).collect(),
                ),
            );
            serde_json::Value::Object(doc)
        }
        SchemaKind::Reference(name) => {
            serde_json::json!({"$ref": format!("#/$defs/{name}")})
        }
        SchemaKind::Annotated { pairs, inner } => {
            if let Some((_, id)) = pairs.iter().find(|(key, _)| key == "identifier") {
                let doc = compile_kind(inner, defs);
                // Explicit identifiers are never deduplicated: a second
                // schema under the same name keeps the first document, which
                // is the phase-1 sharing rule (fallback renaming is phase 2).
                defs.entry(id.clone()).or_insert(doc);
                return serde_json::json!({"$ref": format!("#/$defs/{id}")});
            }
            let mut doc = compile_kind(inner, defs);
            if let serde_json::Value::Object(ref mut map) = doc {
                for (key, value) in pairs {
                    // `expected` is a codec message, not a document keyword.
                    if key == "expected" {
                        continue;
                    }
                    map.entry(key.clone())
                        .or_insert(serde_json::Value::String(value.clone()));
                }
            }
            doc
        }
        SchemaKind::Check { base, name, args, .. } => {
            // Recursion compiles the rest of the chain (and hoists any
            // identifier riding inside it); this level's check lands beside
            // it, so the outermost check wins a repeated keyword.
            let mut doc = compile_kind(base, defs);
            apply_check(&mut doc, kind_type_name(strip(base)), name, args);
            doc
        }
    }
}
