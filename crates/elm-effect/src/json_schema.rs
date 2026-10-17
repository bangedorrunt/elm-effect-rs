//! `JsonSchema.ts` behavior port: dialect conversion for JSON Schema.
//!
//! Two directions. `from_schema_*` lifts a foreign document into the canonical
//! draft-2020-12 shape; `to_document_*` lowers a canonical document back to
//! draft-07, draft-04, or OpenAPI 3.1. [`rewrite_refs`], [`get_reference_key`],
//! [`sanitize_open_api_components_schemas_key`] and
//! [`is_valid_open_api_components_schemas_key`] are the small helpers the
//! emitters and the OpenAPI surface share.
//!
//! Three things carry the weight:
//!
//! - **A reference is not written until the walk ends.** A converter cannot
//!   know where a pointer lands until every target path is recorded, so a
//!   pending `$ref` is written as a sentinel string and resolved in
//!   [`Converter::resolve_pending`]. The sentinel holds a NUL byte, which no
//!   JSON document at the pin can contain.
//! - **The walk tracks resource scopes.** A non-empty `$id` that does not
//!   start with `#` opens a new JSON Schema resource, and a pointer inside it
//!   resolves against that resource's URI. `locations` maps
//!   `(resource, source pointer)` to the target pointer, which is how a moved
//!   subtree keeps its references valid.
//! - **Lifting and lowering are different adapters.** Draft-07 and OpenAPI 3.0
//!   each have a lifting adapter (into 2020-12) and draft-07 and draft-04 each
//!   have a lowering adapter (out of 2020-12); they share the engine but not
//!   the keyword table, so they are four separate cases rather than one
//!   parameterized walk.
//!
//! Deliberate residuals (thread, not beads): WHATWG URL parsing is
//! implemented for the special schemes the tests use (http/https/ws/wss/ftp),
//! not for the full URL Standard, so a non-special-scheme reference with a
//! relative path is resolved per RFC 3986 and may differ from a browser.
// This file belongs to the parked port campaign: a test reaches it through a
// `#[path]` include to port specific rows, so most of its surface is unused by
// any one test target. Warnings about the unexercised remainder describe the
// test's coverage, not a defect in the crate, so they are allowed here rather
// than fixed one by one.
#![allow(dead_code)]

use std::collections::BTreeMap;

use super::json::Json;
use super::json_pointer::{format_uri_fragment, parse_uri_fragment};

pub const META_SCHEMA_URI_DRAFT_04: &str = "http://json-schema.org/draft-04/schema#";
pub const META_SCHEMA_URI_DRAFT_07: &str = "http://json-schema.org/draft-07/schema#";
pub const META_SCHEMA_URI_DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
const META_SCHEMA_URI_OPEN_API_3_1: &str = "https://spec.openapis.org/oas/3.1/dialect/base";

/// The dialect a document is expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Draft04,
    Draft07,
    Draft2020_12,
    OpenApi3_1,
    OpenApi3_0,
}

impl Dialect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft04 => "draft-04",
            Self::Draft07 => "draft-07",
            Self::Draft2020_12 => "draft-2020-12",
            Self::OpenApi3_1 => "openapi-3.1",
            Self::OpenApi3_0 => "openapi-3.0",
        }
    }
}

/// One converted schema plus the definitions it references.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub dialect: Dialect,
    pub schema: Json,
    pub definitions: BTreeMap<String, Json>,
}

/// Several converted schemas sharing one definitions pool.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiDocument {
    pub dialect: Dialect,
    pub schemas: Vec<Json>,
    pub definitions: BTreeMap<String, Json>,
}

/// A refused conversion. The message is the pin's own wording, so a test can
/// assert the same text it asserts upstream.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionError(pub String);

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConversionError {}

type Outcome = Result<Json, ConversionError>;

fn refuse(message: String) -> ConversionError {
    ConversionError(message)
}

/// The dialect name the pin prints in a refusal.
fn dialect_label(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Draft04 => "Draft-04",
        Dialect::Draft07 => "Draft-07",
        Dialect::Draft2020_12 => "Draft 2020-12",
        Dialect::OpenApi3_1 => "OpenAPI 3.1",
        Dialect::OpenApi3_0 => "OpenAPI 3.0",
    }
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

/// The pin's `Predicate.isObject`: a non-null object that is not an array.
fn is_object(value: &Json) -> bool {
    matches!(value, Json::Obj(_))
}

fn as_object(value: &Json) -> Option<&BTreeMap<String, Json>> {
    match value {
        Json::Obj(map) => Some(map),
        _ => None,
    }
}

fn field<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    as_object(value)?.get(key)
}

fn field_str<'a>(value: &'a Json, key: &str) -> Option<&'a str> {
    match field(value, key) {
        Some(Json::Str(text)) => Some(text),
        _ => None,
    }
}

fn has(value: &Json, key: &str) -> bool {
    as_object(value).is_some_and(|map| map.contains_key(key))
}

/// The pin accepts both spellings of a meta-schema URI: with and without the
/// trailing empty fragment.
fn is_meta_schema_uri(value: Option<&Json>, uri: &str) -> bool {
    let Some(Json::Str(text)) = value else {
        return false;
    };
    if text == uri {
        return true;
    }
    let alternative = match uri.strip_suffix('#') {
        Some(trimmed) => trimmed.to_string(),
        None => format!("{uri}#"),
    };
    *text == alternative
}

/// `InternalRecord.assignProperty` with a plain insert: a `BTreeMap` has no
/// prototype to poison.
fn assign(out: &mut BTreeMap<String, Json>, key: &str, value: Json) {
    out.insert(key.to_string(), value);
}

fn text(value: &str) -> Json {
    Json::Str(value.to_string())
}

fn obj_of(pairs: &[(&str, Json)]) -> Json {
    let mut map = BTreeMap::new();
    for (key, value) in pairs {
        assign(&mut map, key, value.clone());
    }
    Json::Obj(map)
}

// ---------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------

/// The resolver the converter needs: enough of the URL Standard to compare a
/// reference's resource against a base, not a general-purpose parser.
#[derive(Clone, Debug, PartialEq)]
struct Url {
    scheme: String,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Url {
    /// Serialize without the fragment, which is how the pin compares
    /// resources (`url.hash = ""` then `url.href`).
    fn href_without_fragment(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.scheme);
        out.push(':');
        if let Some(authority) = &self.authority {
            out.push_str("//");
            out.push_str(authority);
        }
        out.push_str(&self.path);
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(query);
        }
        out
    }
}

/// True for the schemes the URL Standard calls special: they get an empty
/// path turned into `/`, and a lowercased host.
fn is_special_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

/// RFC 3986 §5.2.4, applied only when the path has dot segments, so an
/// ordinary path is returned untouched.
fn remove_dot_segments(path: &str) -> String {
    let has_dots = path == "."
        || path == ".."
        || path.contains("./")
        || path.contains("/.")
        || path.ends_with("/..");
    if !has_dots {
        return path.to_string();
    }
    let mut out: Vec<&str> = Vec::new();
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    let mut joined = String::new();
    if absolute {
        joined.push('/');
    }
    joined.push_str(&out.join("/"));
    if trailing && !joined.ends_with('/') {
        joined.push('/');
    }
    joined
}

/// The scheme of an absolute reference, if it has one:
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )` then `:`.
fn scheme_end(reference: &str) -> Option<usize> {
    let mut chars = reference.char_indices();
    match chars.next() {
        Some((_, first)) if first.is_ascii_alphabetic() => {}
        _ => return None,
    }
    for (index, ch) in chars {
        if ch == ':' {
            return Some(index);
        }
        if !(ch.is_ascii_alphanumeric() || ch == '+' || ch == '-' || ch == '.') {
            return None;
        }
    }
    None
}

/// `//host/path` splits into its authority and its path.
fn split_authority(rest: &str) -> (Option<String>, String) {
    let remainder = &rest[2..];
    match remainder.find('/') {
        Some(index) => (
            Some(remainder[..index].to_string()),
            remainder[index..].to_string(),
        ),
        None => (Some(remainder.to_string()), String::new()),
    }
}

/// Resolve `reference` against `base`, both as URI references.
fn resolve_url(reference: &str, base: &Url) -> Option<Url> {
    let (before_fragment, fragment) = match reference.find('#') {
        Some(index) => (
            &reference[..index],
            Some(reference[index + 1..].to_string()),
        ),
        None => (reference, None),
    };
    let (before_query, query) = match before_fragment.find('?') {
        Some(index) => (
            &before_fragment[..index],
            Some(before_fragment[index + 1..].to_string()),
        ),
        None => (before_fragment, None),
    };

    let (scheme, rest) = match scheme_end(before_query) {
        Some(index) => (
            Some(before_query[..index].to_ascii_lowercase()),
            &before_query[index + 1..],
        ),
        None => (None, before_query),
    };

    // An absolute reference ignores the base entirely.
    if let Some(scheme) = scheme {
        let (authority, path) = if rest.starts_with("//") {
            split_authority(rest)
        } else {
            (None, rest.to_string())
        };
        let authority = authority.map(|host| {
            if is_special_scheme(&scheme) {
                host.to_ascii_lowercase()
            } else {
                host
            }
        });
        let mut path = remove_dot_segments(&path);
        if path.is_empty() && authority.is_some() && is_special_scheme(&scheme) {
            path.push('/');
        }
        return Some(Url {
            scheme,
            authority,
            path,
            query,
            fragment,
        });
    }

    if rest.starts_with("//") {
        let (authority, path) = split_authority(rest);
        let mut path = remove_dot_segments(&path);
        if path.is_empty() && is_special_scheme(&base.scheme) {
            path.push('/');
        }
        return Some(Url {
            scheme: base.scheme.clone(),
            authority: authority.map(|host| {
                if is_special_scheme(&base.scheme) {
                    host.to_ascii_lowercase()
                } else {
                    host
                }
            }),
            path,
            query,
            fragment,
        });
    }
    if rest.is_empty() {
        return Some(Url {
            scheme: base.scheme.clone(),
            authority: base.authority.clone(),
            path: base.path.clone(),
            query: query.or_else(|| base.query.clone()),
            fragment,
        });
    }
    let path = if rest.starts_with('/') {
        remove_dot_segments(rest)
    } else {
        // §5.2.3 merge: drop the base's last segment, then append.
        let mut merged = match base.path.rfind('/') {
            Some(index) => base.path[..=index].to_string(),
            None => String::new(),
        };
        merged.push_str(rest);
        remove_dot_segments(&merged)
    };
    Some(Url {
        scheme: base.scheme.clone(),
        authority: base.authority.clone(),
        path,
        query,
        fragment,
    })
}

/// The base every conversion resolves against. The pin uses a reserved
/// `.invalid` host precisely so a relative `$id` cannot reach the network.
const ROOT_URI: &str = "https://effect.invalid/.json-schema/";

/// Parse an absolute reference. A reference with no scheme is not one.
fn parse_url(value: &str) -> Option<Url> {
    scheme_end(value)?;
    let empty = Url {
        scheme: String::new(),
        authority: None,
        path: String::new(),
        query: None,
        fragment: None,
    };
    resolve_url(value, &empty)
}

fn root_href() -> String {
    ROOT_URI.to_string()
}

/// `resolveResourceUri`: a string `$id` resolved against the enclosing
/// resource, with the fragment dropped.
fn resolve_resource_uri(value: Option<&Json>, base: &str) -> Option<String> {
    let Json::Str(text) = value? else {
        return None;
    };
    let base = parse_url(base)?;
    let resolved = resolve_url(text, &base)?;
    Some(resolved.href_without_fragment())
}

fn creates_resource(id: Option<&Json>) -> bool {
    match id {
        Some(Json::Str(text)) => !text.is_empty() && !text.starts_with('#'),
        _ => false,
    }
}

/// `locationKey`: the resource plus the source pointer, as one key.
fn location_key(resource: &str, pointer: &[String]) -> String {
    let mut out = String::with_capacity(resource.len() + pointer.len() * 8);
    out.push_str(resource);
    out.push('\u{0}');
    out.push('[');
    for (index, token) in pointer.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_string(token));
    }
    out.push(']');
    out
}

/// `JSON.stringify` for one string, which is all `locationKey` needs.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Move a reference onto the path its target moved to, keeping any URI part.
fn relocate_reference(reference: &str, target_path: &[String]) -> String {
    let index = reference.find('#');
    if index.is_none() && target_path.is_empty() {
        return reference.to_string();
    }
    let uri = match index {
        None => reference,
        Some(index) => &reference[..index],
    };
    format!("{}{}", uri, format_uri_fragment(target_path))
}

// ---------------------------------------------------------------------------
// Structural walk (`transformSchema`)
// ---------------------------------------------------------------------------

/// Keywords whose value is a map of schemas.
const SCHEMA_MAP_KEYWORDS: [&str; 2] = ["properties", "patternProperties"];

/// Keywords whose value is an array of schemas.
const SCHEMA_ARRAY_KEYWORDS: [&str; 3] = ["allOf", "anyOf", "oneOf"];

/// Keywords whose value is one schema, in the dialect-neutral walk.
const JSON_SCHEMA_SINGLE_KEYWORDS: [&str; 8] = [
    "not",
    "additionalProperties",
    "propertyNames",
    "contains",
    "if",
    "then",
    "else",
    "contentSchema",
];

fn is_map_of_schemas(key: &str) -> bool {
    matches!(
        key,
        "$defs" | "properties" | "patternProperties" | "dependentSchemas"
    )
}

fn is_array_of_schemas(key: &str) -> bool {
    matches!(key, "allOf" | "anyOf" | "oneOf" | "prefixItems")
}

fn is_single_schema(key: &str) -> bool {
    matches!(
        key,
        "not"
            | "additionalProperties"
            | "propertyNames"
            | "unevaluatedProperties"
            | "items"
            | "contains"
            | "unevaluatedItems"
            | "if"
            | "then"
            | "else"
            | "contentSchema"
    )
}

fn map_object(value: &Json, f: &mut dyn FnMut(&Json, &str) -> Json) -> Option<Json> {
    let map = as_object(value)?;
    let mut out = BTreeMap::new();
    for (key, item) in map {
        assign(&mut out, key, f(item, key));
    }
    Some(Json::Obj(out))
}

/// Walk every schema position, rebuilding each object and letting `transform`
/// rewrite it. `in_embedded_resource` says whether an enclosing node already
/// opened a resource, which the OpenAPI 3.1 conversion needs.
fn transform_schema(
    node: &Json,
    transform: &mut dyn FnMut(&mut BTreeMap<String, Json>, bool),
) -> Json {
    fn walk(
        node: &Json,
        inherited_resource: bool,
        is_root: bool,
        transform: &mut dyn FnMut(&mut BTreeMap<String, Json>, bool),
    ) -> Json {
        let Some(map) = as_object(node) else {
            return node.clone();
        };
        let in_embedded_resource =
            inherited_resource || (!is_root && creates_resource(map.get("$id")));
        let mut out = BTreeMap::new();
        for (key, value) in map {
            let transformed = if is_map_of_schemas(key) {
                map_object(value, &mut |item, _| {
                    walk(item, in_embedded_resource, false, transform)
                })
                .unwrap_or_else(|| value.clone())
            } else if is_array_of_schemas(key) {
                match value {
                    Json::Arr(items) => Json::Arr(
                        items
                            .iter()
                            .map(|item| walk(item, in_embedded_resource, false, transform))
                            .collect(),
                    ),
                    other => other.clone(),
                }
            } else if is_single_schema(key) {
                walk(value, in_embedded_resource, false, transform)
            } else {
                value.clone()
            };
            assign(&mut out, key, transformed);
        }
        transform(&mut out, in_embedded_resource);
        Json::Obj(out)
    }
    walk(node, false, true, transform)
}

/// Rewrite every `$ref` / `$dynamicRef` in every schema position.
pub fn rewrite_refs(schema: &Json, rewrite: &dyn Fn(&str) -> String) -> Json {
    transform_schema(schema, &mut |out, _| {
        rewrite_schema_ref(out, &mut |reference, _| rewrite(reference));
    })
}

fn rewrite_schema_ref(
    schema: &mut BTreeMap<String, Json>,
    rewrite: &mut dyn FnMut(&str, &str) -> String,
) {
    if let Some(Json::Str(reference)) = schema.get("$ref") {
        let next = rewrite(reference, "$ref");
        assign(schema, "$ref", text(&next));
    }
    if let Some(Json::Str(reference)) = schema.get("$dynamicRef") {
        let next = rewrite(reference, "$dynamicRef");
        assign(schema, "$dynamicRef", text(&next));
    }
}

// ---------------------------------------------------------------------------
// OpenAPI component keys
// ---------------------------------------------------------------------------

const OPEN_API_31_TARGET_COLLISIONS: [&str; 4] =
    ["example", "discriminator", "xml", "externalDocs"];

/// `^[a-zA-Z0-9.\-_]+$`.
pub fn is_valid_open_api_components_schemas_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_')
}

pub fn sanitize_open_api_components_schemas_key(value: &str) -> String {
    if value.is_empty() {
        return "_".to_string();
    }
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn get_reference_key(reference: &str) -> Option<String> {
    if !reference.starts_with('#') {
        return None;
    }
    let path = parse_uri_fragment(reference)?;
    if path.len() == 2 && path[0] == "$defs" {
        Some(path[1].clone())
    } else {
        None
    }
}

/// `#/components/schemas/A` becomes `#/$defs/A`; anything else is untouched.
fn rewrite_open_api_components_reference(reference: &str) -> String {
    let path = if reference.starts_with('#') {
        parse_uri_fragment(reference)
    } else {
        None
    };
    match path {
        Some(path) if path.len() >= 2 && path[0] == "components" && path[1] == "schemas" => {
            let mut tokens = vec!["$defs".to_string()];
            tokens.extend(path[2..].iter().cloned());
            format_uri_fragment(&tokens)
        }
        _ => reference.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// `fromSchemaDraft2020_12`: the canonical shape, so only `$defs` moves.
pub fn from_schema_draft2020_12(js: &Json) -> Document {
    let mut schema = BTreeMap::new();
    let mut definitions = BTreeMap::new();
    if let Some(map) = as_object(js) {
        for (key, value) in map {
            if key == "$defs" {
                if let Some(defs) = as_object(value) {
                    definitions = defs.clone();
                }
            } else {
                assign(&mut schema, key, value.clone());
            }
        }
    }
    Document {
        dialect: Dialect::Draft2020_12,
        schema: Json::Obj(schema),
        definitions,
    }
}

pub fn from_schema_draft07(js: &Json) -> Result<Document, ConversionError> {
    Ok(from_schema_draft2020_12(&convert_draft07(js)?))
}

pub fn from_schema_openapi3_0(js: &Json) -> Result<Document, ConversionError> {
    Ok(from_schema_draft2020_12(&convert_open_api30(js)?))
}

/// Nothing here can refuse: the OpenAPI 3.1 lift only relocates references
/// and merges `example` into `examples`, so the `Result` exists for callers
/// that treat every entry point alike.
pub fn from_schema_openapi3_1(js: &Json) -> Result<Document, ConversionError> {
    let is_root_resource = creates_resource(field(js, "$id"));
    let schema = transform_schema(js, &mut |schema, in_embedded_resource| {
        if !is_root_resource && !in_embedded_resource {
            rewrite_schema_ref(schema, &mut |reference, _| {
                rewrite_open_api_components_reference(reference)
            });
        }
        if is_meta_schema_uri(schema.get("$schema"), META_SCHEMA_URI_OPEN_API_3_1) {
            assign(schema, "$schema", text(META_SCHEMA_URI_DRAFT_2020_12));
        }
        if schema.contains_key("example") {
            let examples = schema.get("examples");
            let mergeable = examples.is_none() || matches!(examples, Some(Json::Arr(_)));
            if mergeable {
                let mut merged = vec![schema.get("example").cloned().unwrap_or(Json::Null)];
                if let Some(Json::Arr(items)) = examples {
                    merged.extend(items.iter().cloned());
                }
                assign(schema, "examples", Json::Arr(merged));
                schema.remove("example");
            }
        }
    });
    Ok(from_schema_draft2020_12(&schema))
}

pub fn to_document_draft07(document: &Document) -> Result<Document, ConversionError> {
    let (schema, definitions) = convert_document(
        document,
        Adapter::Draft2020ToDraft07,
        ConversionOptions::default(),
    )?;
    Ok(Document {
        dialect: Dialect::Draft07,
        schema,
        definitions,
    })
}

/// Draft-04 has no boolean schemas, so `true` becomes `{}` and `false`
/// becomes `{ not: {} }`.
fn draft04_boolean(schema: bool) -> Json {
    if schema {
        obj_of(&[])
    } else {
        obj_of(&[("not", obj_of(&[]))])
    }
}

pub fn to_document_draft04(document: &Document) -> Result<Document, ConversionError> {
    let options = ConversionOptions {
        boolean_adapter: Some(draft04_boolean),
        ..ConversionOptions::default()
    };
    let (schema, definitions) = convert_document(document, Adapter::Draft2020ToDraft04, options)?;
    Ok(Document {
        dialect: Dialect::Draft04,
        schema,
        definitions,
    })
}

pub fn to_multi_document_open_api3_1(
    multi_document: &MultiDocument,
) -> Result<MultiDocument, ConversionError> {
    let definition_keys: Vec<String> = multi_document.definitions.keys().cloned().collect();
    let mut key_map: BTreeMap<String, String> = BTreeMap::new();
    let mut used_keys: Vec<String> = definition_keys
        .iter()
        .filter(|key| is_valid_open_api_components_schemas_key(key))
        .cloned()
        .collect();
    let mut invalid_keys: Vec<(String, String)> = definition_keys
        .iter()
        .filter(|key| !is_valid_open_api_components_schemas_key(key))
        .map(|key| (key.clone(), sanitize_open_api_components_schemas_key(key)))
        .collect();
    // Sorted, so which key wins a sanitized collision does not depend on the
    // order the caller happened to build the map in.
    invalid_keys.sort();
    for (key, base) in &invalid_keys {
        if used_keys.iter().any(|used| used == base) {
            continue;
        }
        used_keys.push(base.clone());
        key_map.insert(key.clone(), base.clone());
    }
    for (key, base) in &invalid_keys {
        if key_map.contains_key(key) {
            continue;
        }
        let mut suffix = 0usize;
        let candidate = loop {
            suffix += 1;
            let candidate = format!("{base}_{suffix}");
            if !used_keys.iter().any(|used| *used == candidate) {
                break candidate;
            }
        };
        used_keys.push(candidate.clone());
        key_map.insert(key.clone(), candidate);
    }

    let rewrite = |schema: &Json,
                   reject_shared_definition_refs: bool|
     -> Result<Json, ConversionError> {
        let is_root_resource = creates_resource(field(schema, "$id"));
        let local_definitions = as_object(schema).and_then(|map| map.get("$defs"));
        let mut error: Option<ConversionError> = None;
        let out = transform_schema(schema, &mut |node, in_embedded_resource| {
            if error.is_some() {
                return;
            }
            let node_object = Json::Obj(node.clone());
            if let Err(collision) = reject_keyword_collisions(
                &node_object,
                &OPEN_API_31_TARGET_COLLISIONS,
                "OpenAPI 3.1",
                "Draft 2020-12",
            ) {
                error = Some(collision);
                return;
            }
            let mut ref_error: Option<ConversionError> = None;
            rewrite_schema_ref(node, &mut |reference, keyword| {
                if ref_error.is_some() {
                    return reference.to_string();
                }
                let Some(path) = (if reference.starts_with('#') {
                    parse_uri_fragment(reference)
                } else {
                    None
                }) else {
                    return reference.to_string();
                };
                if path.len() < 2 || path[0] != "$defs" {
                    return reference.to_string();
                }
                let key = path[1].clone();
                if is_root_resource {
                    let shared = !in_embedded_resource
                        && multi_document.definitions.contains_key(&key)
                        && match local_definitions.and_then(|value| as_object(value)) {
                            Some(defs) => !defs.contains_key(&key),
                            None => true,
                        };
                    if reject_shared_definition_refs && shared {
                        ref_error = Some(refuse(format!(
                            "Cannot convert JSON Schema keyword \"{keyword}\" to OpenAPI 3.1: a schema resource cannot reference the shared definitions pool"
                        )));
                    }
                    return reference.to_string();
                }
                if in_embedded_resource {
                    return reference.to_string();
                }
                let mut tokens = vec!["components".to_string(), "schemas".to_string()];
                tokens.push(key_map.get(&key).cloned().unwrap_or_else(|| key.clone()));
                tokens.extend(path[2..].iter().cloned());
                format_uri_fragment(&tokens)
            });
            if let Some(ref_error) = ref_error {
                error = Some(ref_error);
            }
        });
        match error {
            Some(error) => Err(error),
            None => Ok(out),
        }
    };

    let mut schemas = Vec::with_capacity(multi_document.schemas.len());
    for schema in &multi_document.schemas {
        schemas.push(rewrite(schema, true)?);
    }
    let mut definitions = BTreeMap::new();
    for key in &definition_keys {
        let original = multi_document
            .definitions
            .get(key)
            .expect("the key came from this map");
        let target = key_map.get(key).cloned().unwrap_or_else(|| key.clone());
        assign(&mut definitions, &target, rewrite(original, false)?);
    }

    Ok(MultiDocument {
        dialect: Dialect::OpenApi3_1,
        schemas,
        definitions,
    })
}

// ---------------------------------------------------------------------------
// The converter engine
// ---------------------------------------------------------------------------

/// A resource opened by a non-fragment `$id`.
#[derive(Clone, Debug)]
struct ResourceScope {
    parent: Option<Box<ResourceScope>>,
    source_root: Vec<String>,
    target_root: Vec<String>,
    uri: String,
}

/// Which keyword table a conversion runs: two lifts into 2020-12 and two
/// lowerings out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Adapter {
    Draft07To2020,
    Draft2020ToDraft07,
    Draft2020ToDraft04,
    OpenApi30To2020,
}

/// A `$ref` whose final text is not known until the walk ends.
#[derive(Clone, Debug)]
struct PendingReference {
    value: String,
    resource: String,
}

/// The sentinel prefix for a not-yet-resolved `$ref`. A NUL byte cannot occur
/// in a JSON document the pin's tests build, so the marker is unambiguous.
const PENDING_PREFIX: &str = "\u{0}json-schema-pending-ref:";

fn pending_token(index: usize) -> String {
    format!("{PENDING_PREFIX}{index}")
}

/// Replace every pending sentinel in the finished output with its resolved
/// reference. Ordinary strings are left alone unless they are a token, which
/// cannot happen for a document the converter itself did not write.
fn replace_tokens(value: &mut Json, resolved: &BTreeMap<String, String>) {
    match value {
        Json::Str(text) => {
            if let Some(replacement) = resolved.get(text) {
                *text = replacement.clone();
            }
        }
        Json::Arr(items) => {
            for item in items {
                replace_tokens(item, resolved);
            }
        }
        Json::Obj(map) => {
            for item in map.values_mut() {
                replace_tokens(item, resolved);
            }
        }
        _ => {}
    }
}

/// The options `runConverter` carries.
#[derive(Clone, Copy, Default)]
struct ConversionOptions {
    boolean_adapter: Option<fn(bool) -> Json>,
    track_ids: bool,
    ignore_ref_siblings: bool,
}

struct Converter {
    locations: BTreeMap<String, Vec<String>>,
    pending: Vec<PendingReference>,
    adapter: Adapter,
    options: ConversionOptions,
}

/// Where a node sits and which resource encloses it.
struct Context<'a> {
    converter: &'a mut Converter,
    source_path: Vec<String>,
    target_path: Vec<String>,
    scope: ResourceScope,
    current_resource: String,
}

impl Context<'_> {
    fn is_document_root(&self) -> bool {
        self.source_path.is_empty()
    }

    fn schema(&mut self, value: &Json, source_key: &str, target_key: &str) -> Outcome {
        let mut source_path = self.source_path.clone();
        source_path.push(source_key.to_string());
        let mut target_path = self.target_path.clone();
        target_path.push(target_key.to_string());
        let scope = self.scope.clone();
        self.converter
            .loop_node(value, &source_path, &target_path, &scope)
    }

    fn schema_at(
        &mut self,
        value: &Json,
        source_suffix: &[&str],
        target_suffix: &[&str],
    ) -> Outcome {
        let mut source_path = self.source_path.clone();
        source_path.extend(source_suffix.iter().map(|token| (*token).to_string()));
        let mut target_path = self.target_path.clone();
        target_path.extend(target_suffix.iter().map(|token| (*token).to_string()));
        let scope = self.scope.clone();
        self.converter
            .loop_node(value, &source_path, &target_path, &scope)
    }

    fn schema_array(&mut self, value: &Json, source_key: &str, target_key: &str) -> Outcome {
        match value {
            Json::Arr(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let mut source_path = self.source_path.clone();
                    source_path.push(source_key.to_string());
                    source_path.push(index.to_string());
                    let mut target_path = self.target_path.clone();
                    target_path.push(target_key.to_string());
                    target_path.push(index.to_string());
                    let scope = self.scope.clone();
                    out.push(
                        self.converter
                            .loop_node(item, &source_path, &target_path, &scope)?,
                    );
                }
                Ok(Json::Arr(out))
            }
            other => Ok(other.clone()),
        }
    }

    fn schema_map(&mut self, value: &Json, source_key: &str, target_key: &str) -> Outcome {
        let Some(map) = as_object(value) else {
            return Ok(value.clone());
        };
        let mut out = BTreeMap::new();
        for (key, item) in map {
            let mut source_path = self.source_path.clone();
            source_path.push(source_key.to_string());
            source_path.push(key.clone());
            let mut target_path = self.target_path.clone();
            target_path.push(target_key.to_string());
            target_path.push(key.clone());
            let scope = self.scope.clone();
            let converted = self
                .converter
                .loop_node(item, &source_path, &target_path, &scope)?;
            assign(&mut out, key, converted);
        }
        Ok(Json::Obj(out))
    }

    /// Record a reference. A string is deferred until every target path is
    /// known; anything else is written immediately, as the pin does.
    fn reference(&mut self, out: &mut BTreeMap<String, Json>, value: &Json) {
        match value {
            Json::Str(reference) => {
                let index = self.converter.pending.len();
                self.converter.pending.push(PendingReference {
                    value: reference.clone(),
                    resource: self.current_resource.clone(),
                });
                assign(out, "$ref", text(&pending_token(index)));
            }
            other => assign(out, "$ref", other.clone()),
        }
    }
}

impl Converter {
    fn new(adapter: Adapter, options: ConversionOptions) -> Self {
        Self {
            locations: BTreeMap::new(),
            pending: Vec::new(),
            adapter,
            options,
        }
    }

    /// Seed the root resource from the document's own `$id` when the pin
    /// tracks identifiers, and hand back the scope every root node shares.
    fn root_scope(&self, root: &Json) -> ResourceScope {
        let uri = if self.options.track_ids {
            resolve_resource_uri(field(root, "$id"), &root_href()).unwrap_or_else(root_href)
        } else {
            root_href()
        };
        ResourceScope {
            parent: None,
            source_root: Vec::new(),
            target_root: Vec::new(),
            uri,
        }
    }

    fn loop_node(
        &mut self,
        node: &Json,
        source_path: &[String],
        target_path: &[String],
        scope: &ResourceScope,
    ) -> Outcome {
        if let Json::Bool(flag) = node {
            self.record_locations(source_path, target_path, scope);
            return Ok(match self.options.boolean_adapter {
                Some(adapter) => adapter(*flag),
                None => node.clone(),
            });
        }
        if !is_object(node) {
            return Ok(node.clone());
        }
        let resource_id = self.resource_id(node);
        let mut current = scope.clone();
        if !source_path.is_empty() && self.options.track_ids && creates_resource(resource_id) {
            if let Some(uri) = resolve_resource_uri(resource_id, &scope.uri) {
                current = ResourceScope {
                    parent: Some(Box::new(scope.clone())),
                    source_root: source_path.to_vec(),
                    target_root: target_path.to_vec(),
                    uri,
                };
            }
        }
        self.record_locations(source_path, target_path, &current);
        let current_resource = current.uri.clone();
        let adapter = self.adapter;
        let mut context = Context {
            converter: self,
            source_path: source_path.to_vec(),
            target_path: target_path.to_vec(),
            scope: current,
            current_resource,
        };
        adapter.convert(node, &mut context)
    }

    /// `getResourceId`: with `ignoreRefSiblings`, an `$id` beside a `$ref` on
    /// the same node is discarded, because the pin treats that node as a bare
    /// reference rather than a resource.
    fn resource_id<'a>(&self, schema: &'a Json) -> Option<&'a Json> {
        if self.options.ignore_ref_siblings && field_str(schema, "$ref").is_some() {
            return None;
        }
        field(schema, "$id")
    }

    fn record_locations(
        &mut self,
        source_path: &[String],
        target_path: &[String],
        scope: &ResourceScope,
    ) {
        if let Some(parent) = &scope.parent {
            self.record_locations(source_path, target_path, parent);
        }
        let source_tail = &source_path[scope.source_root.len().min(source_path.len())..];
        let target_tail = &target_path[scope.target_root.len().min(target_path.len())..];
        self.locations
            .insert(location_key(&scope.uri, source_tail), target_tail.to_vec());
    }

    /// Resolve every deferred reference now that all target paths are known.
    fn resolve_pending(&mut self) -> BTreeMap<String, String> {
        let pending = std::mem::take(&mut self.pending);
        let mut resolved = BTreeMap::new();
        for (index, item) in pending.iter().enumerate() {
            let mut reference = item.value.clone();
            let target = parse_url(&item.resource).and_then(|base| resolve_url(&item.value, &base));
            if let Some(url) = target {
                let pointer = url
                    .fragment
                    .as_deref()
                    .and_then(|fragment| parse_uri_fragment(&format!("#{fragment}")));
                if let Some(pointer) = pointer {
                    let key = location_key(&url.href_without_fragment(), &pointer);
                    if let Some(target) = self.locations.get(&key) {
                        reference = relocate_reference(&item.value, target);
                    }
                }
            }
            resolved.insert(pending_token(index), reference);
        }
        resolved
    }
}

fn convert_schema(root: &Json, adapter: Adapter, options: ConversionOptions) -> Outcome {
    let mut converter = Converter::new(adapter, options);
    let scope = converter.root_scope(root);
    let mut out = converter.loop_node(root, &[], &[], &scope)?;
    let resolved = converter.resolve_pending();
    replace_tokens(&mut out, &resolved);
    Ok(out)
}

/// Convert one document, tracking its `$defs` as a `definitions` map.
fn convert_document(
    document: &Document,
    adapter: Adapter,
    options: ConversionOptions,
) -> Result<(Json, BTreeMap<String, Json>), ConversionError> {
    let options = ConversionOptions {
        track_ids: true,
        ..options
    };
    let mut converter = Converter::new(adapter, options);
    let scope = converter.root_scope(&document.schema);
    let mut schema_out = converter.loop_node(&document.schema, &[], &[], &scope)?;
    let mut definitions_out: BTreeMap<String, Json> = BTreeMap::new();
    for (key, definition) in &document.definitions {
        let source_path = vec!["$defs".to_string(), key.clone()];
        let target_path = vec!["definitions".to_string(), key.clone()];
        let value = converter.loop_node(definition, &source_path, &target_path, &scope)?;
        assign(&mut definitions_out, key, value);
    }
    let resolved = converter.resolve_pending();
    replace_tokens(&mut schema_out, &resolved);
    for value in definitions_out.values_mut() {
        replace_tokens(value, &resolved);
    }
    Ok((schema_out, definitions_out))
}

// ---------------------------------------------------------------------------
// Adapters
// ---------------------------------------------------------------------------

const OPEN_API_30_SCHEMA_MAP_KEYWORDS: [&str; 1] = ["properties"];
const OPEN_API_30_SCHEMA_SINGLE_KEYWORDS: [&str; 3] = ["not", "items", "additionalProperties"];
const DRAFT_04_SCHEMA_SINGLE_KEYWORDS: [&str; 3] = ["not", "additionalProperties", "contentSchema"];

fn convert_subschema_keyword(
    out: &mut BTreeMap<String, Json>,
    key: &str,
    value: &Json,
    context: &mut Context<'_>,
    single_keywords: &[&str],
    map_keywords: &[&str],
) -> Result<bool, ConversionError> {
    let converted = if map_keywords.contains(&key) {
        context.schema_map(value, key, key)?
    } else if SCHEMA_ARRAY_KEYWORDS.contains(&key) {
        context.schema_array(value, key, key)?
    } else if single_keywords.contains(&key) {
        context.schema(value, key, key)?
    } else {
        return Ok(false);
    };
    assign(out, key, converted);
    Ok(true)
}

fn unsupported(keyword: &str, dialect: &str, details: &str) -> ConversionError {
    refuse(format!(
        "Cannot convert JSON Schema keyword \"{keyword}\" to {dialect}: {details}"
    ))
}

fn reject_keyword_collisions(
    source: &Json,
    keywords: &[&str],
    target_dialect: &str,
    source_dialect: &str,
) -> Result<(), ConversionError> {
    for keyword in keywords {
        if has(source, keyword) {
            return Err(unsupported(
                keyword,
                target_dialect,
                &format!(
                    "it is not active in {source_dialect} but would become active in the target"
                ),
            ));
        }
    }
    Ok(())
}

/// The keywords a pre-2020 dialect does not define but 2020-12 does, so a
/// draft-07 document that spells one is ambiguous rather than merely unknown.
const PRE_2020_TO_2020_COLLISIONS: [&str; 13] = [
    "$anchor",
    "$defs",
    "$dynamicAnchor",
    "$dynamicRef",
    "$vocabulary",
    "contentSchema",
    "dependentRequired",
    "dependentSchemas",
    "maxContains",
    "minContains",
    "prefixItems",
    "unevaluatedItems",
    "unevaluatedProperties",
];

const DRAFT_07_TO_2020_COLLISIONS: [&str; 14] = [
    "$anchor",
    "$defs",
    "$dynamicAnchor",
    "$dynamicRef",
    "$vocabulary",
    "contentSchema",
    "dependentRequired",
    "dependentSchemas",
    "maxContains",
    "minContains",
    "prefixItems",
    "unevaluatedItems",
    "unevaluatedProperties",
    "deprecated",
];

const OPEN_API_30_TO_2020_COLLISIONS: [&str; 26] = [
    "$anchor",
    "$defs",
    "$dynamicAnchor",
    "$dynamicRef",
    "$vocabulary",
    "contentSchema",
    "dependentRequired",
    "dependentSchemas",
    "maxContains",
    "minContains",
    "prefixItems",
    "unevaluatedItems",
    "unevaluatedProperties",
    "$comment",
    "$id",
    "$schema",
    "const",
    "contains",
    "contentEncoding",
    "contentMediaType",
    "else",
    "examples",
    "if",
    "patternProperties",
    "propertyNames",
    "then",
];

const DRAFT_07_TARGET_COLLISIONS: [&str; 3] = ["additionalItems", "definitions", "dependencies"];
const DRAFT_04_TARGET_COLLISIONS: [&str; 4] =
    ["additionalItems", "definitions", "dependencies", "id"];

/// `^[A-Za-z_][-A-Za-z0-9._]*$`.
fn is_anchor(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_'))
}

/// `^[A-Za-z][-A-Za-z0-9._:]*$`.
fn is_legacy_id_fragment(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | ':'))
}

impl Adapter {
    fn convert(self, source: &Json, context: &mut Context<'_>) -> Outcome {
        match self {
            Self::Draft07To2020 => draft07_to_2020_adapter(source, context),
            Self::Draft2020ToDraft07 => draft2020_to_draft07_adapter(source, context),
            Self::Draft2020ToDraft04 => draft2020_to_draft04_adapter(source, context),
            Self::OpenApi30To2020 => open_api30_to_2020_adapter(source, context),
        }
    }
}

/// `convertDraft07`: lift draft-07 into 2020-12.
fn convert_draft07(root: &Json) -> Outcome {
    let options = ConversionOptions {
        track_ids: true,
        ignore_ref_siblings: true,
        ..ConversionOptions::default()
    };
    convert_schema(root, Adapter::Draft07To2020, options)
}

/// `convertOpenApi30`: lift OpenAPI 3.0 into 2020-12.
fn convert_open_api30(root: &Json) -> Outcome {
    convert_schema(root, Adapter::OpenApi30To2020, ConversionOptions::default())
}

fn draft07_to_2020_adapter(source: &Json, context: &mut Context<'_>) -> Outcome {
    let mut out: BTreeMap<String, Json> = BTreeMap::new();

    // A node that is only a reference moves wholesale: the pin refuses to
    // rewrite its siblings, because a draft-07 `$ref` has none.
    if let Some(Json::Str(reference)) = field(source, "$ref") {
        context.reference(&mut out, &text(reference));
        if has(source, "definitions") {
            let converted = context.schema_map(
                field(source, "definitions").expect("checked"),
                "definitions",
                "$defs",
            )?;
            assign(&mut out, "$defs", converted);
        }
        return Ok(Json::Obj(out));
    }

    reject_keyword_collisions(
        source,
        &DRAFT_07_TO_2020_COLLISIONS,
        "Draft 2020-12",
        "Draft-07",
    )?;

    let mut items: Option<Json> = None;
    let mut additional_items: Option<Json> = None;

    let map = as_object(source).expect("the walk only reaches objects");
    for (key, value) in map {
        if convert_subschema_keyword(
            &mut out,
            key,
            value,
            context,
            &JSON_SCHEMA_SINGLE_KEYWORDS,
            &SCHEMA_MAP_KEYWORDS,
        )? {
            continue;
        }
        match key.as_str() {
            "$schema" => {
                let rewritten = if is_meta_schema_uri(Some(value), META_SCHEMA_URI_DRAFT_07) {
                    text(META_SCHEMA_URI_DRAFT_2020_12)
                } else {
                    value.clone()
                };
                assign(&mut out, "$schema", rewritten);
            }
            "$id" => convert_draft07_id(&mut out, value)?,
            "definitions" => {
                let converted = context.schema_map(value, key, "$defs")?;
                assign(&mut out, "$defs", converted);
            }
            "dependencies" => {
                let Some(dependencies) = as_object(value) else {
                    assign(&mut out, key, value.clone());
                    continue;
                };
                let mut dependent_required: BTreeMap<String, Json> = BTreeMap::new();
                let mut dependent_schemas: BTreeMap<String, Json> = BTreeMap::new();
                for (dependency, entry) in dependencies {
                    if matches!(entry, Json::Arr(_)) {
                        assign(&mut dependent_required, dependency, entry.clone());
                    } else {
                        let converted = context.schema_at(
                            entry,
                            &["dependencies", dependency],
                            &["dependentSchemas", dependency],
                        )?;
                        assign(&mut dependent_schemas, dependency, converted);
                    }
                }
                if !dependent_required.is_empty() {
                    assign(&mut out, "dependentRequired", Json::Obj(dependent_required));
                }
                if !dependent_schemas.is_empty() {
                    assign(&mut out, "dependentSchemas", Json::Obj(dependent_schemas));
                }
            }
            "items" => items = Some(value.clone()),
            "additionalItems" => additional_items = Some(value.clone()),
            _ => assign(&mut out, key, value.clone()),
        }
    }

    if let Some(items) = items {
        if matches!(items, Json::Arr(_)) {
            let converted = context.schema_array(&items, "items", "prefixItems")?;
            assign(&mut out, "prefixItems", converted);
            if let Some(additional_items) = additional_items {
                let converted = context.schema(&additional_items, "additionalItems", "items")?;
                assign(&mut out, "items", converted);
            }
        } else {
            let converted = context.schema(&items, "items", "items")?;
            assign(&mut out, "items", converted);
        }
    }

    Ok(Json::Obj(out))
}

/// `convertOpenApi30`: lift OpenAPI 3.0 into 2020-12.
fn open_api30_to_2020_adapter(source: &Json, context: &mut Context<'_>) -> Outcome {
    let mut out: BTreeMap<String, Json> = BTreeMap::new();

    if let Some(Json::Str(reference)) = field(source, "$ref") {
        let rewritten = rewrite_open_api_components_reference(reference);
        context.reference(&mut out, &text(&rewritten));
        return Ok(Json::Obj(out));
    }
    reject_keyword_collisions(
        source,
        &OPEN_API_30_TO_2020_COLLISIONS,
        "Draft 2020-12",
        "OpenAPI 3.0",
    )?;

    let map = as_object(source).expect("the walk only reaches objects");
    for (key, value) in map {
        if convert_subschema_keyword(
            &mut out,
            key,
            value,
            context,
            &OPEN_API_30_SCHEMA_SINGLE_KEYWORDS,
            &OPEN_API_30_SCHEMA_MAP_KEYWORDS,
        )? {
            continue;
        }
        match key.as_str() {
            "example" => assign(&mut out, "examples", Json::Arr(vec![value.clone()])),
            "nullable" | "exclusiveMinimum" | "exclusiveMaximum" => {}
            _ => assign(&mut out, key, value.clone()),
        }
    }

    convert_open_api_exclusive_bound(source, &mut out, "minimum");
    convert_open_api_exclusive_bound(source, &mut out, "maximum");

    if field(source, "nullable") == Some(&Json::Bool(true)) {
        if let Some(Json::Str(kind)) = field(source, "type") {
            assign(&mut out, "type", Json::Arr(vec![text(kind), text("null")]));
        }
    }

    Ok(Json::Obj(out))
}

/// `draft07Adapter`: lower 2020-12 into draft-07.
fn draft2020_to_draft07_adapter(source: &Json, context: &mut Context<'_>) -> Outcome {
    reject_keyword_collisions(
        source,
        &DRAFT_07_TARGET_COLLISIONS,
        dialect_label(Dialect::Draft07),
        dialect_label(Dialect::Draft2020_12),
    )?;
    let mut out: BTreeMap<String, Json> = BTreeMap::new();
    let mut reference: Option<Json> = None;
    let mut prefix_items: Option<Json> = None;
    let mut items: Option<Json> = None;

    let map = as_object(source).expect("the walk only reaches objects");
    for (key, value) in map {
        if convert_subschema_keyword(
            &mut out,
            key,
            value,
            context,
            &JSON_SCHEMA_SINGLE_KEYWORDS,
            &SCHEMA_MAP_KEYWORDS,
        )? {
            continue;
        }
        match key.as_str() {
            "$ref" => reference = Some(value.clone()),
            "$schema" => convert_meta_schema_keyword(
                &mut out,
                value,
                context,
                META_SCHEMA_URI_DRAFT_07,
                dialect_label(Dialect::Draft07),
            )?,
            "$id" | "$anchor" => {}
            "$defs" => {
                let converted = context.schema_map(value, key, "definitions")?;
                assign(&mut out, "definitions", converted);
            }
            "prefixItems" => prefix_items = Some(value.clone()),
            "items" => items = Some(value.clone()),
            "dependentRequired" | "dependentSchemas" | "minContains" | "maxContains" => {}
            "$dynamicRef"
            | "$dynamicAnchor"
            | "$vocabulary"
            | "unevaluatedProperties"
            | "unevaluatedItems" => {
                return Err(unsupported(
                    key,
                    dialect_label(Dialect::Draft07),
                    "the target dialect has no equivalent",
                ));
            }
            "required" => {
                // draft-07 has no `required: []`; an empty list is dropped
                // rather than emitted as a no-op.
                if matches!(value, Json::Arr(list) if list.is_empty()) {
                    continue;
                }
                assign(&mut out, key, value.clone());
            }
            _ => assign(&mut out, key, value.clone()),
        }
    }

    convert_tuple(&mut out, prefix_items.as_ref(), items.as_ref(), context)?;

    if has(source, "contains") {
        let min_contains = field(source, "minContains");
        let max_contains = field(source, "maxContains");
        let min_is_custom = matches!(min_contains, Some(Json::Num(value)) if *value != 1.0);
        if min_is_custom || max_contains.is_some() {
            return Err(unsupported(
                "minContains/maxContains",
                dialect_label(Dialect::Draft07),
                "contains cardinality cannot be represented",
            ));
        }
        if let Some(value) = min_contains {
            assign(&mut out, "minContains", value.clone());
        }
    } else {
        if let Some(value) = field(source, "minContains") {
            assign(&mut out, "minContains", value.clone());
        }
        if let Some(value) = field(source, "maxContains") {
            assign(&mut out, "maxContains", value.clone());
        }
    }

    convert_dependencies(source, &mut out, context, Dialect::Draft07)?;
    convert_legacy_id(source, &mut out, "$id", Dialect::Draft07)?;
    convert_reference(&mut out, reference.as_ref(), context);

    Ok(Json::Obj(out))
}

/// `draft04Adapter`: lower 2020-12 into draft-04.
fn draft2020_to_draft04_adapter(source: &Json, context: &mut Context<'_>) -> Outcome {
    reject_keyword_collisions(
        source,
        &DRAFT_04_TARGET_COLLISIONS,
        dialect_label(Dialect::Draft04),
        dialect_label(Dialect::Draft2020_12),
    )?;
    let mut out: BTreeMap<String, Json> = BTreeMap::new();
    let mut reference: Option<Json> = None;
    let mut prefix_items: Option<Json> = None;
    let mut items: Option<Json> = None;
    let mut const_schema: Option<Json> = None;

    let map = as_object(source).expect("the walk only reaches objects");
    for (key, value) in map {
        if convert_subschema_keyword(
            &mut out,
            key,
            value,
            context,
            &DRAFT_04_SCHEMA_SINGLE_KEYWORDS,
            &SCHEMA_MAP_KEYWORDS,
        )? {
            continue;
        }
        match key.as_str() {
            "$ref" => reference = Some(value.clone()),
            "$schema" => convert_meta_schema_keyword(
                &mut out,
                value,
                context,
                META_SCHEMA_URI_DRAFT_04,
                dialect_label(Dialect::Draft04),
            )?,
            "$id" | "$anchor" => {}
            "$defs" => {
                let converted = context.schema_map(value, key, "definitions")?;
                assign(&mut out, "definitions", converted);
            }
            "prefixItems" => prefix_items = Some(value.clone()),
            "items" => items = Some(value.clone()),
            "$dynamicRef"
            | "$dynamicAnchor"
            | "$vocabulary"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "propertyNames" => {
                return Err(unsupported(
                    key,
                    dialect_label(Dialect::Draft04),
                    "the target dialect has no equivalent",
                ));
            }
            // Dropped here and re-derived below, because draft-04 spells
            // exclusive bounds as a boolean beside the inclusive bound, and
            // has no conditionals or `contains` at all.
            "dependentRequired" | "dependentSchemas" | "contains" | "minContains"
            | "maxContains" | "if" | "then" | "else" | "minimum" | "maximum"
            | "exclusiveMinimum" | "exclusiveMaximum" => {}
            "const" => const_schema = Some(obj_of(&[("enum", Json::Arr(vec![value.clone()]))])),
            "required" => {
                if matches!(value, Json::Arr(list) if list.is_empty()) {
                    continue;
                }
                assign(&mut out, key, value.clone());
            }
            _ => assign(&mut out, key, value.clone()),
        }
    }

    convert_tuple(&mut out, prefix_items.as_ref(), items.as_ref(), context)?;
    convert_draft04_exclusive_bound(source, &mut out, "minimum");
    convert_draft04_exclusive_bound(source, &mut out, "maximum");
    convert_dependencies(source, &mut out, context, Dialect::Draft04)?;
    convert_legacy_id(source, &mut out, "id", Dialect::Draft04)?;
    convert_draft04_conditionals(source, &mut out, context)?;
    convert_draft04_contains(source, &mut out, context)?;

    if let Some(const_schema) = const_schema {
        if has(source, "enum") {
            append_all_of(&mut out, const_schema);
        } else if let Some(Json::Arr(values)) = field(&const_schema, "enum") {
            assign(&mut out, "enum", Json::Arr(values.clone()));
        }
    }

    convert_reference(&mut out, reference.as_ref(), context);

    Ok(Json::Obj(out))
}

// ---------------------------------------------------------------------------
// Shared adapter steps
// ---------------------------------------------------------------------------

fn convert_tuple(
    out: &mut BTreeMap<String, Json>,
    prefix_items: Option<&Json>,
    items: Option<&Json>,
    context: &mut Context<'_>,
) -> Result<(), ConversionError> {
    let Some(prefix_items) = prefix_items else {
        if let Some(items) = items {
            let converted = context.schema(items, "items", "items")?;
            assign(out, "items", converted);
        }
        return Ok(());
    };
    let converted = context.schema_array(prefix_items, "prefixItems", "items")?;
    assign(out, "items", converted);
    if let Some(items) = items {
        let converted = context.schema(items, "items", "additionalItems")?;
        assign(out, "additionalItems", converted);
    }
    Ok(())
}

/// A reference that cannot sit beside other keywords becomes an `allOf`
/// member, so the sibling keywords keep their meaning.
fn convert_reference(
    out: &mut BTreeMap<String, Json>,
    reference: Option<&Json>,
    context: &mut Context<'_>,
) {
    let Some(reference) = reference else {
        return;
    };
    if matches!(reference, Json::Str(_)) && !out.is_empty() {
        let mut reference_schema: BTreeMap<String, Json> = BTreeMap::new();
        context.reference(&mut reference_schema, reference);
        append_all_of(out, Json::Obj(reference_schema));
    } else {
        context.reference(out, reference);
    }
}

fn convert_dependencies(
    source: &Json,
    out: &mut BTreeMap<String, Json>,
    context: &mut Context<'_>,
    target_dialect: Dialect,
) -> Result<(), ConversionError> {
    let dependent_required = field(source, "dependentRequired").filter(|value| is_object(value));
    let dependent_schemas = field(source, "dependentSchemas").filter(|value| is_object(value));
    if dependent_required.is_none() && dependent_schemas.is_none() {
        return Ok(());
    }

    let mut keys: Vec<String> = Vec::new();
    for map in [dependent_required, dependent_schemas]
        .into_iter()
        .flatten()
    {
        if let Some(map) = as_object(map) {
            for key in map.keys() {
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
    }

    let mut dependencies: BTreeMap<String, Json> = BTreeMap::new();
    for key in keys {
        let required = dependent_required.and_then(|map| field(map, &key));
        let dependency = dependent_schemas.and_then(|map| field(map, &key));
        // draft-04 cannot express an empty `required` list at all.
        let omit_required = target_dialect == Dialect::Draft04
            && matches!(required, Some(Json::Arr(list)) if list.is_empty());
        match (dependency, required) {
            // A key that only requires others is a plain list, unless the
            // target dialect cannot spell an empty one.
            (None, required) => {
                if !omit_required {
                    if let Some(required) = required {
                        assign(&mut dependencies, &key, required.clone());
                    }
                }
            }
            // A key with only a schema dependency is that schema.
            (Some(dependency), None) => {
                let converted = context.schema_at(
                    dependency,
                    &["dependentSchemas", &key],
                    &["dependencies", &key],
                )?;
                assign(&mut dependencies, &key, converted);
            }
            (Some(dependency), Some(required)) => {
                if omit_required {
                    let converted = context.schema_at(
                        dependency,
                        &["dependentSchemas", &key],
                        &["dependencies", &key],
                    )?;
                    assign(&mut dependencies, &key, converted);
                } else {
                    // Both kinds on one key: the schema dependency and the
                    // required list become two `allOf` members, because the
                    // target dialect has no combined keyword.
                    let converted = context.schema_at(
                        dependency,
                        &["dependentSchemas", &key],
                        &["dependencies", &key, "allOf", "0"],
                    )?;
                    let combined = obj_of(&[(
                        "allOf",
                        Json::Arr(vec![converted, obj_of(&[("required", required.clone())])]),
                    )]);
                    assign(&mut dependencies, &key, combined);
                }
            }
        }
    }
    if !dependencies.is_empty() {
        assign(out, "dependencies", Json::Obj(dependencies));
    }
    Ok(())
}

fn convert_legacy_id(
    source: &Json,
    out: &mut BTreeMap<String, Json>,
    target_key: &str,
    dialect: Dialect,
) -> Result<(), ConversionError> {
    let id = field(source, "$id");
    let anchor = field(source, "$anchor");
    let label = dialect_label(dialect);
    let Some(anchor) = anchor else {
        if let Some(id) = id {
            assign(out, target_key, id.clone());
        }
        return Ok(());
    };
    let Json::Str(anchor_text) = anchor else {
        return Err(unsupported("$anchor", label, "the anchor is not valid"));
    };
    if !is_anchor(anchor_text) {
        return Err(unsupported("$anchor", label, "the anchor is not valid"));
    }
    if !is_legacy_id_fragment(anchor_text) {
        return Err(unsupported(
            "$anchor",
            label,
            "the anchor cannot be represented as a plain-name fragment identifier",
        ));
    }
    if id.is_none() {
        assign(out, target_key, text(&format!("#{anchor_text}")));
        Ok(())
    } else {
        Err(unsupported(
            "$anchor",
            label,
            "it cannot be combined with the schema $id",
        ))
    }
}

/// Draft-04 has no exclusive numeric bounds: `{ minimum: 5, exclusiveMinimum:
/// true }` carries `exclusiveMinimum: 5`. When both bounds exist the stricter
/// one wins.
fn convert_draft04_exclusive_bound(
    source: &Json,
    out: &mut BTreeMap<String, Json>,
    bound_key: &str,
) {
    let exclusive_key = if bound_key == "minimum" {
        "exclusiveMinimum"
    } else {
        "exclusiveMaximum"
    };
    let bound = field(source, bound_key);
    let exclusive = field(source, exclusive_key);
    match exclusive {
        Some(Json::Num(exclusive)) => {
            let bound_number = match bound {
                Some(Json::Num(value)) => Some(*value),
                _ => None,
            };
            let stricter = match bound_number {
                Some(bound_number) if bound_key == "minimum" => bound_number > *exclusive,
                Some(bound_number) => bound_number < *exclusive,
                None => false,
            };
            match (stricter, bound_number) {
                (true, Some(bound_number)) => assign(out, bound_key, Json::Num(bound_number)),
                _ => {
                    assign(out, bound_key, Json::Num(*exclusive));
                    assign(out, exclusive_key, Json::Bool(true));
                }
            }
        }
        _ => {
            if let Some(bound) = bound {
                assign(out, bound_key, bound.clone());
            }
        }
    }
}

/// OpenAPI 3.0 spells an exclusive bound as a boolean beside an inclusive
/// one; 2020-12 spells it as a number in `exclusiveMinimum` alone.
fn convert_open_api_exclusive_bound(
    source: &Json,
    out: &mut BTreeMap<String, Json>,
    bound_key: &str,
) {
    let exclusive_key = if bound_key == "minimum" {
        "exclusiveMinimum"
    } else {
        "exclusiveMaximum"
    };
    let exclusive = field(source, exclusive_key);
    let Json::Bool(exclusive) = exclusive.unwrap_or(&Json::Null) else {
        if let Some(value) = exclusive {
            assign(out, exclusive_key, value.clone());
        }
        return;
    };
    if *exclusive {
        if let Some(Json::Num(bound)) = field(source, bound_key) {
            assign(out, exclusive_key, Json::Num(*bound));
            out.remove(bound_key);
        }
    }
}

fn convert_meta_schema_keyword(
    out: &mut BTreeMap<String, Json>,
    value: &Json,
    context: &mut Context<'_>,
    target_uri: &str,
    target_dialect: &str,
) -> Result<(), ConversionError> {
    if context.is_document_root() {
        let rewritten = if is_meta_schema_uri(Some(value), META_SCHEMA_URI_DRAFT_2020_12) {
            text(target_uri)
        } else {
            value.clone()
        };
        assign(out, "$schema", rewritten);
        Ok(())
    } else if !is_meta_schema_uri(Some(value), META_SCHEMA_URI_DRAFT_2020_12) {
        Err(unsupported(
            "$schema",
            target_dialect,
            "an embedded resource cannot declare a different dialect",
        ))
    } else {
        Ok(())
    }
}

/// Draft-04 has no `if`/`then`/`else`, so a conditional becomes a two-branch
/// `anyOf`: the keyword holds, or it does not and the `else` branch applies.
fn convert_draft04_conditionals(
    source: &Json,
    out: &mut BTreeMap<String, Json>,
    context: &mut Context<'_>,
) -> Result<(), ConversionError> {
    let has_if = has(source, "if");
    let has_then = has(source, "then");
    let has_else = has(source, "else");
    let label = dialect_label(Dialect::Draft04);
    if !has_if || (!has_then && !has_else) {
        if has_if {
            let converted = context.schema(field(source, "if").expect("checked"), "if", "if")?;
            assign(out, "if", converted);
        }
        if has_then {
            let converted =
                context.schema(field(source, "then").expect("checked"), "then", "then")?;
            assign(out, "then", converted);
        }
        if has_else {
            let converted =
                context.schema(field(source, "else").expect("checked"), "else", "else")?;
            assign(out, "else", converted);
        }
        return Ok(());
    }

    let index = match out.get("allOf") {
        Some(Json::Arr(items)) => items.len(),
        _ => 0,
    };
    let index_text = index.to_string();
    let base = ["allOf", index_text.as_str(), "anyOf"];
    let branch = |context: &mut Context<'_>, key: &str, target: &[&str]| -> Outcome {
        let mut target_suffix: Vec<&str> = base.to_vec();
        target_suffix.extend_from_slice(target);
        context.schema_at(field(source, key).expect("checked"), &[key], &target_suffix)
    };

    let conditional = if has_then && has_else {
        if has_schema_identifier(field(source, "if").expect("checked")) {
            return Err(unsupported(
                "if",
                label,
                "lowering both branches would duplicate a schema identifier",
            ));
        }
        let if_branch = branch(context, "if", &["0", "allOf", "0"])?;
        let then_branch = branch(context, "then", &["0", "allOf", "1"])?;
        let not_if = branch(context, "if", &["1", "allOf", "0", "not"])?;
        let else_branch = branch(context, "else", &["1", "allOf", "1"])?;
        obj_of(&[(
            "anyOf",
            Json::Arr(vec![
                obj_of(&[("allOf", Json::Arr(vec![if_branch, then_branch]))]),
                obj_of(&[(
                    "allOf",
                    Json::Arr(vec![obj_of(&[("not", not_if)]), else_branch]),
                )]),
            ]),
        )])
    } else if has_then {
        let not_if = branch(context, "if", &["0", "not"])?;
        let then_branch = branch(context, "then", &["1"])?;
        obj_of(&[(
            "anyOf",
            Json::Arr(vec![obj_of(&[("not", not_if)]), then_branch]),
        )])
    } else {
        let if_branch = branch(context, "if", &["0"])?;
        let else_branch = branch(context, "else", &["1"])?;
        obj_of(&[("anyOf", Json::Arr(vec![if_branch, else_branch]))])
    };
    append_all_of(out, conditional);
    Ok(())
}

/// Draft-04 has no `contains`, so "at least one item matches" becomes "the
/// array is not the case that no item matches".
fn convert_draft04_contains(
    source: &Json,
    out: &mut BTreeMap<String, Json>,
    context: &mut Context<'_>,
) -> Result<(), ConversionError> {
    let label = dialect_label(Dialect::Draft04);
    if !has(source, "contains") {
        if let Some(value) = field(source, "minContains") {
            assign(out, "minContains", value.clone());
        }
        if let Some(value) = field(source, "maxContains") {
            assign(out, "maxContains", value.clone());
        }
        return Ok(());
    }

    let min_contains = field(source, "minContains");
    let max_contains = field(source, "maxContains");
    let min_is_custom = matches!(min_contains, Some(Json::Num(value)) if *value != 1.0);
    if min_is_custom || max_contains.is_some() {
        return Err(unsupported(
            "minContains/maxContains",
            label,
            "contains cardinality cannot be represented",
        ));
    }

    let index = match out.get("allOf") {
        Some(Json::Arr(items)) => items.len(),
        _ => 0,
    };
    let index_text = index.to_string();
    let contains = context.schema_at(
        field(source, "contains").expect("checked"),
        &["contains"],
        &[
            "allOf",
            index_text.as_str(),
            "anyOf",
            "1",
            "not",
            "items",
            "not",
        ],
    )?;
    append_all_of(
        out,
        obj_of(&[(
            "anyOf",
            Json::Arr(vec![
                obj_of(&[("not", obj_of(&[("type", text("array"))]))]),
                obj_of(&[("not", obj_of(&[("items", obj_of(&[("not", contains)]))]))]),
            ]),
        )]),
    );
    Ok(())
}

fn append_all_of(out: &mut BTreeMap<String, Json>, schema: Json) {
    match out.get_mut("allOf") {
        Some(Json::Arr(items)) => items.push(schema),
        _ => {
            assign(out, "allOf", Json::Arr(vec![schema]));
        }
    }
}

/// Does this subtree declare a URI, an anchor, or either under a keyword a
/// lowered conditional would duplicate?
fn has_schema_identifier(node: &Json) -> bool {
    let Some(map) = as_object(node) else {
        return false;
    };
    if map.contains_key("$id") || map.contains_key("$anchor") {
        return true;
    }
    for (key, value) in map {
        if is_map_of_schemas(key) {
            if let Some(inner) = as_object(value) {
                if inner.values().any(has_schema_identifier) {
                    return true;
                }
            }
        } else if is_array_of_schemas(key) {
            if let Json::Arr(items) = value {
                if items.iter().any(has_schema_identifier) {
                    return true;
                }
            }
        } else if is_single_schema(key) && has_schema_identifier(value) {
            return true;
        }
    }
    false
}

/// `$id` in draft-07 may carry a plain-name fragment; 2020-12 spells that as
/// `$anchor` beside the identifier.
fn convert_draft07_id(
    out: &mut BTreeMap<String, Json>,
    value: &Json,
) -> Result<(), ConversionError> {
    let Json::Str(value) = value else {
        assign(out, "$id", value.clone());
        return Ok(());
    };
    let Some(index) = value.find('#') else {
        assign(out, "$id", text(value));
        return Ok(());
    };
    let id = &value[..index];
    let anchor = &value[index + 1..];
    if anchor.is_empty() {
        if !id.is_empty() {
            assign(out, "$id", text(id));
        }
        return Ok(());
    }
    if !is_anchor(anchor) {
        return Err(unsupported(
            "$id",
            dialect_label(Dialect::Draft2020_12),
            &format!("fragment \"#{anchor}\" is not a valid $anchor"),
        ));
    }
    if !id.is_empty() {
        assign(out, "$id", text(id));
    }
    assign(out, "$anchor", text(anchor));
    Ok(())
}
