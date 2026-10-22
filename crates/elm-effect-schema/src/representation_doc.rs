//! Singular representation documents (`toRepresentation` / `fromRepresentation`).
//!
//! A document is the pin's `{ representation, references }` pair: the converted
//! schema plus the identifier-hoisted definitions it points at. Conversion runs
//! over `SchemaKind`, so anything the kind cannot see cannot be converted: a
//! `suspend` thunk is a closure the kind does not carry, `TemplateLiteral`
//! parts are folded into an opaque marker, and `Symbol` / `BigInt` /
//! `Undefined` have no domain in `Unknown`. Those shapes fail the conversion
//! instead of guessing.
//!
//! Custom filters survive through a reviver registry. A reviver either names a
//! built-in check the port already enforces (so the revived schema decodes
//! exactly like the original) or supplies a predicate, in which case the
//! document identity rides a reserved annotation pair the converter reads back.
//! The reserved key starts with NUL, which no legitimate annotation can
//! contain, so user pairs never collide with it and `to_json` strips it.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::filters::{FilterOutcome, FilterSpec};
use crate::{Schema, SchemaKind, Unknown};

/// The converted pair: a representation plus its hoisted definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub representation: Value,
    pub references: BTreeMap<String, Value>,
}

/// A conversion failure: the pin's message plus the path it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReprError {
    pub message: String,
    pub path: String,
}

impl ReprError {
    fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: path.into(),
        }
    }
}

impl std::fmt::Display for ReprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n  at {}", self.message, self.path)
    }
}

impl std::error::Error for ReprError {}

/// One named segment below a path: `["representation"]` below the root.
fn seg(path: &str, key: &str) -> String {
    format!("{path}[\"{key}\"]")
}

/// One indexed segment below a path: `[0]` below its array.
fn idx(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

impl From<ReprError> for crate::SchemaError {
    fn from(error: ReprError) -> Self {
        crate::SchemaError::InvalidJson {
            path: error.path,
            message: error.message,
        }
    }
}

/// What a reviver hands back: either a built-in check the port enforces by
/// name, or a custom predicate carried by `Schema::filtered`.
pub enum RevivedCheck {
    Named {
        name: String,
        args: Vec<String>,
    },
    Custom {
        predicate: Arc<dyn Fn(&Unknown) -> bool + Send + Sync>,
    },
}

/// The input a reviver sees: the document's annotation pairs plus the decoded
/// payload the document carried.
pub struct ReviveInput {
    pub annotations: Vec<(String, String)>,
    pub payload: Unknown,
}

/// One reviver: a filter or filter-group identity plus the schema its payload
/// must decode through and the function rebuilding the check.
pub struct Reviver {
    pub id: String,
    pub payload_schema: Schema<Unknown>,
    pub revive: Arc<dyn Fn(ReviveInput) -> RevivedCheck + Send + Sync>,
    pub group: bool,
}

impl Reviver {
    /// A filter reviver, like the pin's `FilterReviver`.
    pub fn filter(
        id: &str,
        payload_schema: Schema<Unknown>,
        revive: impl Fn(ReviveInput) -> RevivedCheck + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.to_string(),
            payload_schema,
            revive: Arc::new(revive),
            group: false,
        }
    }

    /// A filter-group reviver, used instead of the group's persisted children.
    pub fn group(
        id: &str,
        payload_schema: Schema<Unknown>,
        revive: impl Fn(ReviveInput) -> RevivedCheck + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.to_string(),
            payload_schema,
            revive: Arc::new(revive),
            group: true,
        }
    }
}

/// The built-in `isInt` reviver: the pin's `isIntReviver` with a null payload.
pub fn is_int_reviver() -> Reviver {
    Reviver::filter(
        "effect/schema/isInt",
        Schema::as_unknown(Schema::null()),
        |_| RevivedCheck::Named {
            name: "isInt".to_string(),
            args: Vec::new(),
        },
    )
}

/// The built-in `isGreaterThan` reviver: the payload carries the bound the
/// port's own conversion wrote.
pub fn is_greater_than_reviver() -> Reviver {
    Reviver::filter(
        "effect/schema/isGreaterThan",
        Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::number()))),
        |input| {
            let bound = match &input.payload {
                Unknown::Object(fields) => match fields.get("greaterThan") {
                    Some(Unknown::Number(bound)) => render_number(*bound),
                    _ => "0".to_string(),
                },
                _ => "0".to_string(),
            };
            RevivedCheck::Named {
                name: "isGreaterThan".to_string(),
                args: vec![bound],
            }
        },
    )
}

/// The reserved annotation key carrying a revived filter's document identity.
/// NUL-prefixed, so no user annotation can collide with it.
const BLOB_KEY: &str = "\u{0}representation";

/// Convert one schema to its document, hoisting identifier annotations into
/// references. Fails on shapes the kind cannot see (suspend thunks, template
/// parts, symbols, bigints, undefined).
pub fn to_representation(schema: &Schema<Unknown>) -> Result<Document, ReprError> {
    let mut state = Convert {
        references: BTreeMap::new(),
        reserved: std::collections::BTreeSet::new(),
        identities: Vec::new(),
    };
    let representation = convert_schema(schema, &mut state, true, "[\"representation\"]")?;
    Ok(Document {
        representation,
        references: state.references,
    })
}

/// Several roots converted against one shared reference map, in input
/// order. This is the port's `toRepresentations`: the first root claims each
/// identifier (suffixed on collision, in encounter order), and later roots
/// reuse the same entry.
///
/// Identity is the shared `decode` allocation: clones of one schema reuse its
/// reference, while separately built schemas suffix even when their bodies
/// match. That is the portable half of the pin's owner rule; the other half
/// (`Context` copies, `referencePolicy`, encoded-side projections) has no
/// counterpart here, so those rows stay excused.
#[derive(Debug, Clone, PartialEq)]
pub struct Documents {
    pub representations: Vec<Value>,
    pub references: BTreeMap<String, Value>,
}

/// Convert several roots against one shared reference map.
pub fn to_representations(schemas: &[Schema<Unknown>]) -> Result<Documents, ReprError> {
    let mut state = Convert {
        references: BTreeMap::new(),
        reserved: std::collections::BTreeSet::new(),
        identities: Vec::new(),
    };
    let mut representations = Vec::with_capacity(schemas.len());
    for (index, schema) in schemas.iter().enumerate() {
        representations.push(convert_schema(
            schema,
            &mut state,
            true,
            &format!("[\"representations\"][{index}]"),
        )?);
    }
    Ok(Documents {
        representations,
        references: state.references,
    })
}
/// Annotate a schema with a structured node identity, like the pin's
/// `annotate({ representation: { id, payload, schemas } })`. Dependency schemas
/// convert beside it, so a failure there fails the annotation too.
pub fn annotate_with_representation(
    schema: Schema<Unknown>,
    id: &str,
    payload: Value,
    dependencies: Vec<Schema<Unknown>>,
) -> Result<Schema<Unknown>, ReprError> {
    let mut schemas = Vec::with_capacity(dependencies.len());
    for dependency in &dependencies {
        schemas.push(to_representation(dependency)?.representation);
    }
    let mut identity = serde_json::Map::new();
    identity.insert("id".to_string(), Value::String(id.to_string()));
    identity.insert("payload".to_string(), payload);
    if !schemas.is_empty() {
        identity.insert("schemas".to_string(), Value::Array(schemas));
    }
    Ok(Schema::annotated_with(
        schema,
        vec![(
            "representation".to_string(),
            Value::Object(identity).to_string(),
        )],
    ))
}

/// Revive one schema per document root is out of scope here; this revives the
/// single root the singular API converts.
pub fn from_representation(
    document: &Document,
    revivers: &[Reviver],
) -> Result<Schema<Unknown>, ReprError> {
    from_representation_with_declarations(document, revivers, &[])
}

/// [`from_representation`] with declaration revivers: a `Declaration` node is
/// rebuilt by the reviver whose id it names, which is how a persisted
/// `Schema.Option` comes back as an Option rather than as a union of tags.
pub fn from_representation_with_declarations(
    document: &Document,
    revivers: &[Reviver],
    declarations: &[DeclarationReviver],
) -> Result<Schema<Unknown>, ReprError> {
    for (index, first) in revivers.iter().enumerate() {
        if revivers[..index].iter().any(|seen| seen.id == first.id) {
            return Err(ReprError::at(
                format!("[\"revivers\"][{index}][\"id\"]"),
                format!("Duplicate reviver for {}", first.id),
            ));
        }
    }
    for (index, first) in declarations.iter().enumerate() {
        if declarations[..index].iter().any(|seen| seen.id == first.id) {
            return Err(ReprError::at(
                format!("[\"revivers\"][{index}][\"id\"]"),
                format!("Duplicate reviver for {}", first.id),
            ));
        }
    }
    let mut state = Revive {
        references: &document.references,
        bodies: BTreeMap::new(),
        slots: BTreeMap::new(),
        resolving: Vec::new(),
        revivers,
        declarations,
    };
    revive_named(&mut state, &document.representation, "[\"representation\"]")
}

/// Validate a document and normalize it to its JSON form: reserved pairs are
/// stripped, everything else passes through.
pub fn to_json(document: &Document) -> Result<Value, ReprError> {
    let representation = to_json_value(&document.representation, "[\"representation\"]")?;
    let mut references = serde_json::Map::new();
    for (name, doc) in &document.references {
        references.insert(
            name.clone(),
            to_json_value(doc, &format!("[\"references\"][\"{name}\"]"))?,
        );
    }
    Ok(serde_json::json!({ "representation": representation, "references": references }))
}

/// Read a JSON form back into a document.
pub fn from_json(value: &Value) -> Result<Document, ReprError> {
    let representation = value
        .get("representation")
        .cloned()
        .ok_or_else(|| ReprError::at("representation", "a document carries a `representation`"))?;
    let references = match value.get("references") {
        None => BTreeMap::new(),
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(_) => {
            return Err(ReprError::at(
                "references",
                "a document carries a `references` object",
            ));
        }
    };
    Ok(Document {
        representation,
        references,
    })
}

/// A JSON annotation value back into the string pair the port stores: strings
/// stay strings, anything else keeps its canonical JSON text.
fn pair_from_json(key: &str, value: &Value) -> (String, String) {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    (key.to_string(), text)
}

/// A whole number keeps no `.0`, because that is what the pin prints.
fn render_number(value: f64) -> String {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        return format!("{}", value as i64);
    }
    format!("{value}")
}

/// A check argument back to a JSON number.
fn number_value(args: &[String]) -> Option<Value> {
    let parsed: f64 = args.first()?.parse().ok()?;
    if parsed.fract() == 0.0 && parsed >= i64::MIN as f64 && parsed <= i64::MAX as f64 {
        return Some(Value::Number(serde_json::Number::from(parsed as i64)));
    }
    serde_json::Number::from_f64(parsed).map(Value::Number)
}

/// The source text out of an `isPattern` argument's `new RegExp("...", "u")`.
fn pattern_source(arg: &str) -> Option<String> {
    let inner = arg
        .strip_prefix("new RegExp(")?
        .strip_suffix(')')
        .and_then(|call| call.strip_suffix(", \"u\""))
        .unwrap_or_else(|| arg.strip_prefix("new RegExp(").unwrap_or(arg));
    serde_json::from_str::<String>(inner.trim()).ok()
}

/// A built-in check name to its representation identity plus payload. Only the
/// checks the carried rows need are here; anything else converts without an
/// identity, which `to_json` then refuses to persist.
fn builtin_identity(name: &str, args: &[String]) -> Option<(String, Value)> {
    let payload = match name {
        "isInt" => Value::Null,
        "isGreaterThan" => serde_json::json!({ "greaterThan": number_value(args)? }),
        "isGreaterThanOrEqualTo" => {
            serde_json::json!({ "greaterThanOrEqualTo": number_value(args)? })
        }
        "isLessThan" => serde_json::json!({ "lessThan": number_value(args)? }),
        "isLessThanOrEqualTo" => {
            serde_json::json!({ "lessThanOrEqualTo": number_value(args)? })
        }
        "isMinLength" => serde_json::json!({ "minLength": number_value(args)? }),
        "isMaxLength" => serde_json::json!({ "maxLength": number_value(args)? }),
        "isMinProperties" => serde_json::json!({ "minProperties": number_value(args)? }),
        "isMaxProperties" => serde_json::json!({ "maxProperties": number_value(args)? }),
        "isPattern" => serde_json::json!({
            "source": pattern_source(args.first()?)?,
            "flags": "u",
        }),
        _ => return None,
    };
    Some((format!("effect/schema/{name}"), payload))
}

struct Convert {
    references: BTreeMap<String, Value>,
    reserved: std::collections::BTreeSet<String>,
    /// Hoisted schema identities: the `decode` allocation address per
    /// reference name, so clones of one schema reuse its entry while
    /// separately built schemas suffix. Only the schema-holding paths record
    /// here; kind-only traversals always allocate.
    identities: Vec<(usize, String)>,
}

/// The identity of a schema object: its shared `decode` allocation. Clones
/// share it, separate constructions do not.
fn identity_of(schema: &Schema<Unknown>) -> usize {
    std::sync::Arc::as_ptr(&schema.decode) as *const () as usize
}

/// A recorded reference for this schema object, when it already hoisted one.
fn known_reference(state: &Convert, schema: &Schema<Unknown>) -> Option<Value> {
    let address = identity_of(schema);
    state
        .identities
        .iter()
        .find(|(seen, _)| *seen == address)
        .map(|(_, name)| serde_json::json!({ "_tag": "Reference", "$ref": name }))
}

/// An annotation pair value back into JSON: objects and arrays stored as
/// canonical text emit as JSON again, everything else stays a string. Only
/// containers parse, so a plain string that merely looks numeric or quoted
/// never changes meaning across the round trip.
fn annotation_json(value: &str) -> Value {
    let trimmed = value.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Value>(value) {
            return parsed;
        }
    }
    Value::String(value.to_string())
}

/// Insert annotation pairs into a list, replacing by key: the later value wins
/// in the earlier key's position, which is how layered annotations merge.
fn merge_pairs(into: &mut Vec<(String, String)>, pairs: Vec<(String, String)>) {
    for (key, value) in pairs {
        match into.iter_mut().find(|(seen, _)| *seen == key) {
            Some(slot) => slot.1 = value,
            None => into.push((key, value)),
        }
    }
}

/// Reserve the reference name for the annotated node being converted: the
/// first occurrence keeps the name, later ones take suffixed names. Names are
/// reserved before the body converts so a nested node under a taken name
/// cannot claim the name its outer node is still building.
fn allocate(state: &mut Convert, id: &str) -> String {
    if !state.references.contains_key(id) && !state.reserved.contains(id) {
        state.reserved.insert(id.to_string());
        return id.to_string();
    }
    let mut counter = 1;
    loop {
        let candidate = format!("{id}_{counter}");
        if !state.references.contains_key(&candidate) && !state.reserved.contains(&candidate) {
            state.reserved.insert(candidate.clone());
            return candidate;
        }
        counter += 1;
    }
}

/// Rewrite a hoisted body's identifier annotation to its allocated name, like
/// the pin's `Node_1` keeps `identifier: "Node_1"`. Only the plain identifier
/// is rewritten; a fallback marker keeps its own name.
fn set_identifier(body: &mut Value, name: &str) {
    if let Some(annotations) = body
        .as_object_mut()
        .and_then(|map| map.get_mut("annotations"))
        .and_then(Value::as_object_mut)
    {
        if annotations.contains_key("identifier") {
            annotations.insert("identifier".to_string(), Value::String(name.to_string()));
        }
    }
}

fn keyword(tag: &str) -> Value {
    serde_json::json!({ "_tag": tag, "checks": [] })
}

fn convert_schema(
    schema: &Schema<Unknown>,
    state: &mut Convert,
    hoist_identifiers: bool,
    path: &str,
) -> Result<Value, ReprError> {
    convert_schema_enclosing(schema, state, hoist_identifiers, None, path)
}

fn convert_schema_enclosing(
    schema: &Schema<Unknown>,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    // A declaration converts to its own node, not to the shape it encodes
    // through: that is what makes a persisted `Schema.Option(String)` read back
    // as an Option rather than as a union of tag structs.
    if let Some(declaration) = schema.declaration() {
        let mut parameters = Vec::with_capacity(declaration.parameters.len());
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            parameters.push(convert_schema(
                parameter,
                state,
                hoist_identifiers,
                &idx(&seg(path, "typeParameters"), index),
            )?);
        }
        return Ok(serde_json::json!({
            "_tag": "Declaration",
            "representation": { "id": declaration.id, "payload": declaration.payload },
            "typeParameters": parameters,
        }));
    }
    // An annotated wrapper converts before the accessors: the wrapper owns
    // the identifier hoisting, and the accessors alone would drop it (a
    // revived struct keeps its fields beside its identifier annotation).
    if let SchemaKind::Annotated { pairs, inner } = schema.repr_kind() {
        return convert_annotated_schema(
            schema,
            &pairs,
            &inner,
            state,
            hoist_identifiers,
            enclosing,
            path,
        );
    }
    // Structs and tuples convert from the field/element lists, which still
    // carry key annotations and optionality the kind may have folded away. A
    // kind-driven struct drops key annotations, which only the accessors keep.
    if let Some(fields) = schema.fields() {
        return convert_field_list(fields, None, state, hoist_identifiers, enclosing, path);
    }
    if let Some(elements) = schema.elements() {
        return convert_element_list(elements, None, state, enclosing, path);
    }
    convert_kind(
        &schema.repr_kind(),
        state,
        hoist_identifiers,
        enclosing,
        path,
    )
}

/// An annotated schema: flatten nested annotation layers (the last applied
/// wins each key), hoist identifiers, and convert the content from the
/// accessors when they describe it.
fn convert_annotated_schema(
    schema: &Schema<Unknown>,
    pairs: &[(String, String)],
    inner: &SchemaKind,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    let mut merged: Vec<(String, String)> = Vec::new();
    let mut content = inner;
    // Collect layers outside-in, then fold inside-out so the last applied
    // annotation wins each key while keeping its first position.
    let mut layers: Vec<&[(String, String)]> = vec![pairs];
    while let SchemaKind::Annotated {
        pairs: inner_pairs,
        inner: next,
    } = content
    {
        layers.push(inner_pairs);
        content = next;
    }
    for layer in layers.into_iter().rev() {
        merge_pairs(&mut merged, layer.to_vec());
    }
    if hoist_identifiers {
        if let Some((_, id)) = merged.iter().find(|(key, _)| key == "identifier") {
            if let Some(reused) = known_reference(state, schema) {
                return Ok(reused);
            }
            let name = allocate(state, id);
            let body = convert_annotated_body(schema, content, state, true, Some(&name), path)?;
            let mut body = with_annotations(body, &merged, &[]);
            set_identifier(&mut body, &name);
            state.references.insert(name.clone(), body);
            state.identities.push((identity_of(schema), name.clone()));
            return Ok(serde_json::json!({ "_tag": "Reference", "$ref": name }));
        }
        if let Some((_, name)) = merged.iter().find(|(key, _)| key == "~identifier") {
            if let Some(reused) = known_reference(state, schema) {
                return Ok(reused);
            }
            let synthetic = format!("{name}Encoded");
            let placed = allocate(state, &synthetic);
            let body = convert_annotated_body(schema, content, state, true, Some(&placed), path)?;
            let body = with_annotations(body, &merged, &[]);
            state.references.insert(placed.clone(), body);
            state.identities.push((identity_of(schema), placed.clone()));
            return Ok(serde_json::json!({ "_tag": "Reference", "$ref": placed }));
        }
    }
    let mut node =
        convert_annotated_body(schema, content, state, hoist_identifiers, enclosing, path)?;
    // Reaching here means nothing hoisted (hoists return early), so every
    // pair stays: only the reserved blob is port-internal.
    for (key, value) in &merged {
        if key == BLOB_KEY {
            continue;
        }
        if key == "representation" {
            node["representation"] = serde_json::from_str(value).unwrap_or(Value::Null);
            continue;
        }
        node["annotations"][key] = annotation_json(value);
    }
    Ok(node)
}

/// The content below flattened annotation layers: the accessors when they
/// describe it, the kind otherwise.
fn convert_annotated_body(
    schema: &Schema<Unknown>,
    content: &SchemaKind,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    if let Some(fields) = schema.fields() {
        let rest = match content {
            SchemaKind::StructWithRest(_, rest) => Some(rest.as_ref()),
            _ => None,
        };
        return convert_field_list(fields, rest, state, hoist_identifiers, enclosing, path);
    }
    if let Some(elements) = schema.elements() {
        let rest = match content {
            SchemaKind::TupleWithRest(_, rest) => Some(rest.as_slice()),
            _ => None,
        };
        return convert_element_list(elements, rest, state, enclosing, path);
    }
    convert_kind(content, state, hoist_identifiers, enclosing, path)
}

fn convert_kind(
    kind: &SchemaKind,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    match kind {
        SchemaKind::String => Ok(keyword("String")),
        SchemaKind::Number => Ok(keyword("Number")),
        SchemaKind::Boolean => Ok(keyword("Boolean")),
        SchemaKind::Opaque("null") => Ok(keyword("Null")),
        SchemaKind::Opaque("void") => Ok(keyword("Void")),
        SchemaKind::Opaque("never") => Ok(keyword("Never")),
        SchemaKind::Opaque("any") => Ok(keyword("Any")),
        SchemaKind::Opaque("unknown") => Ok(keyword("Unknown")),
        SchemaKind::Opaque("objectKeyword") => Ok(keyword("ObjectKeyword")),
        SchemaKind::Opaque("numberFromString") | SchemaKind::Opaque("finiteFromString") => {
            Ok(keyword("String"))
        }
        // A suspend thunk is a closure the kind does not carry, so only a
        // thunk directly under the identifier being hoisted converts: that is
        // direct recursion, and the thunk is the enclosing reference. Anything
        // else (a standalone or nested-indirect suspend) fails instead of
        // guessing.
        SchemaKind::Opaque("suspend") => match enclosing {
            Some(id) => Ok(serde_json::json!({
                "_tag": "Suspend",
                "thunk": { "_tag": "Reference", "$ref": id },
                "checks": [],
            })),
            None => Err(ReprError::at(
                path,
                "a suspend thunk is a closure the kind does not carry",
            )),
        },
        SchemaKind::Opaque(name) => Err(ReprError::at(
            path,
            format!("`{name}` has no representation form"),
        )),
        SchemaKind::Literal(value) => Ok(serde_json::json!({
            "_tag": "Literal",
            "literal": value.to_json(),
            "checks": [],
        })),
        SchemaKind::Literals(_) => Err(ReprError::at(
            path,
            "literal unions need a name-preserving Enum schema this port does not build",
        )),
        SchemaKind::TemplateLiteral(parts) => {
            let converted = parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    convert_kind(
                        part,
                        state,
                        hoist_identifiers,
                        enclosing,
                        &idx(&seg(path, "parts"), index),
                    )
                })
                .collect::<Result<Vec<_>, ReprError>>()?;
            Ok(serde_json::json!({ "_tag": "TemplateLiteral", "parts": converted, "checks": [] }))
        }
        SchemaKind::Reference(name) => Ok(serde_json::json!({ "_tag": "Reference", "$ref": name })),
        SchemaKind::Optional(_) | SchemaKind::OptionalKey(_) | SchemaKind::Rest(_) => {
            Err(ReprError::at(
                path,
                "optional and rest positions only convert inside a tuple",
            ))
        }
        SchemaKind::Array(items) => {
            let rest = convert_kind(items, state, hoist_identifiers, enclosing, path)?;
            Ok(
                serde_json::json!({ "_tag": "Arrays", "elements": [], "rest": [rest], "checks": [] }),
            )
        }
        SchemaKind::Tuple(items) => {
            let elements = items
                .iter()
                .map(|item| {
                    let optional =
                        matches!(item, SchemaKind::Optional(_) | SchemaKind::OptionalKey(_));
                    let inner = match item {
                        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => inner,
                        _ => item,
                    };
                    let doc = convert_kind(inner, state, hoist_identifiers, enclosing, path)?;
                    let mut element = serde_json::Map::new();
                    element.insert("type".to_string(), doc);
                    element.insert("isOptional".to_string(), Value::Bool(optional));
                    Ok(Value::Object(element))
                })
                .collect::<Result<Vec<_>, ReprError>>()?;
            Ok(serde_json::json!({
                "_tag": "Arrays", "elements": elements, "rest": [], "checks": [],
            }))
        }
        SchemaKind::TupleWithRest(items, rest) => {
            let elements = items
                .iter()
                .map(|item| {
                    let optional =
                        matches!(item, SchemaKind::Optional(_) | SchemaKind::OptionalKey(_));
                    let inner = match item {
                        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => inner,
                        _ => item,
                    };
                    let doc = convert_kind(inner, state, hoist_identifiers, enclosing, path)?;
                    let mut element = serde_json::Map::new();
                    element.insert("type".to_string(), doc);
                    element.insert("isOptional".to_string(), Value::Bool(optional));
                    Ok(Value::Object(element))
                })
                .collect::<Result<Vec<_>, ReprError>>()?;
            let rest = rest
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    convert_kind(
                        item,
                        state,
                        hoist_identifiers,
                        enclosing,
                        &idx(&seg(path, "rest"), index),
                    )
                })
                .collect::<Result<Vec<_>, ReprError>>()?;
            Ok(serde_json::json!({
                "_tag": "Arrays", "elements": elements, "rest": rest, "checks": [],
            }))
        }
        SchemaKind::Struct(fields) => {
            convert_struct_fields(fields, None, state, hoist_identifiers, enclosing, path)
        }
        SchemaKind::StructWithRest(fields, rest) => convert_struct_fields(
            fields,
            Some(rest),
            state,
            hoist_identifiers,
            enclosing,
            path,
        ),
        SchemaKind::Record(key, value) => {
            let parameter = convert_kind(key, state, hoist_identifiers, enclosing, path)?;
            let target = convert_kind(value, state, hoist_identifiers, enclosing, path)?;
            Ok(serde_json::json!({
                "_tag": "Objects",
                "propertySignatures": [],
                "indexSignatures": [{ "parameter": parameter, "type": target }],
                "checks": [],
            }))
        }
        SchemaKind::Union { members, mode } => {
            let types = members
                .iter()
                .map(|member| convert_kind(member, state, hoist_identifiers, enclosing, path))
                .collect::<Result<Vec<_>, ReprError>>()?;
            let mut doc = serde_json::json!({ "_tag": "Union", "types": types, "checks": [] });
            if *mode != "anyOf" {
                doc["options"] = serde_json::json!({ "mode": mode });
            }
            Ok(doc)
        }
        SchemaKind::Check {
            base,
            name,
            args,
            annotation,
        } => {
            // An identifier below a check stays inline: the pin only extracts
            // references above the check that names them.
            let mut node = convert_kind(base, state, false, enclosing, path)?;
            let filter = convert_check(name, args, annotation)?;
            node["checks"] = match node.get("checks") {
                Some(Value::Array(existing)) => {
                    let mut checks = existing.clone();
                    checks.push(filter);
                    Value::Array(checks)
                }
                _ => Value::Array(vec![filter]),
            };
            Ok(node)
        }
        SchemaKind::Annotated { pairs, inner } => {
            if hoist_identifiers {
                if let Some((_, id)) = pairs.iter().find(|(key, _)| key == "identifier") {
                    let name = allocate(state, id);
                    let body = convert_kind(inner, state, true, Some(&name), path)?;
                    let mut body = with_annotations(body, pairs, &[]);
                    set_identifier(&mut body, &name);
                    state.references.insert(name.clone(), body);
                    return Ok(serde_json::json!({ "_tag": "Reference", "$ref": name }));
                }
                if let Some((_, name)) = pairs.iter().find(|(key, _)| key == "~identifier") {
                    let synthetic = format!("{name}Encoded");
                    let placed = allocate(state, &synthetic);
                    let body = convert_kind(inner, state, true, Some(&placed), path)?;
                    let body = with_annotations(body, pairs, &[]);
                    state.references.insert(placed.clone(), body);
                    return Ok(serde_json::json!({ "_tag": "Reference", "$ref": placed }));
                }
            }
            let mut node = convert_kind(inner, state, hoist_identifiers, enclosing, path)?;
            // A structured node annotation travels as a real field, not as a
            // string pair; the revive side restores the same pair. Reaching
            // here means nothing hoisted, so identifiers stay inline.
            for (key, value) in pairs {
                if key == BLOB_KEY {
                    continue;
                }
                if key == "representation" {
                    let parsed: Value = serde_json::from_str(value).unwrap_or(Value::Null);
                    node["representation"] = parsed;
                    continue;
                }
                node["annotations"][key] = annotation_json(value);
            }
            Ok(node)
        }
        SchemaKind::Object | SchemaKind::Json => Err(ReprError::at(
            path,
            "open shapes have no representation form",
        )),
    }
}

/// A converted body plus every annotation pair except the skipped keys.
fn with_annotations(mut body: Value, pairs: &[(String, String)], skip: &[&str]) -> Value {
    for (key, value) in pairs {
        if skip.contains(&key.as_str()) || *key == BLOB_KEY {
            continue;
        }
        if key == "representation" {
            body["representation"] = serde_json::from_str(value).unwrap_or(Value::Null);
            continue;
        }
        body["annotations"][key] = annotation_json(value);
    }
    body
}

fn convert_check(
    name: &str,
    args: &[String],
    annotation: &[(String, String)],
) -> Result<Value, ReprError> {
    // A revived custom filter or group carries its whole document identity in
    // the reserved pair, which re-emits exactly what was revived.
    if let Some((_, blob)) = annotation.iter().find(|(key, _)| key == BLOB_KEY) {
        let parsed: Value = serde_json::from_str(blob)
            .map_err(|_| ReprError::at("checks", "a revived filter carries valid JSON"))?;
        return revive_blob_to_doc(&parsed, annotation);
    }
    let user: serde_json::Map<String, Value> = annotation
        .iter()
        .filter(|(key, _)| *key != BLOB_KEY)
        .map(|(key, value)| (key.clone(), annotation_json(value)))
        .collect();
    let mut filter = serde_json::Map::new();
    filter.insert("_tag".to_string(), Value::String("Filter".to_string()));
    if let Some((id, payload)) = builtin_identity(name, args) {
        let mut identity = serde_json::Map::new();
        identity.insert("id".to_string(), Value::String(id));
        identity.insert("payload".to_string(), payload);
        filter.insert("representation".to_string(), Value::Object(identity));
    }
    if !user.is_empty() {
        filter.insert("annotations".to_string(), Value::Object(user));
    }
    filter.insert("aborted".to_string(), Value::Bool(false));
    Ok(Value::Object(filter))
}

/// The stored blob back into its Filter or FilterGroup document.
fn revive_blob_to_doc(blob: &Value, annotation: &[(String, String)]) -> Result<Value, ReprError> {
    let user: serde_json::Map<String, Value> = annotation
        .iter()
        .filter(|(key, _)| *key != BLOB_KEY)
        .map(|(key, value)| (key.clone(), annotation_json(value)))
        .collect();
    let kind = blob.get("kind").and_then(Value::as_str).unwrap_or("filter");
    if kind == "group" {
        let mut group = serde_json::Map::new();
        group.insert("_tag".to_string(), Value::String("FilterGroup".to_string()));
        if !user.is_empty() {
            group.insert("annotations".to_string(), Value::Object(user));
        }
        group.insert(
            "checks".to_string(),
            blob.get("checks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        );
        return Ok(Value::Object(group));
    }
    let mut filter = serde_json::Map::new();
    filter.insert("_tag".to_string(), Value::String("Filter".to_string()));
    let mut identity = serde_json::Map::new();
    if let Some(id) = blob.get("id").and_then(Value::as_str) {
        identity.insert("id".to_string(), Value::String(id.to_string()));
    }
    if let Some(payload) = blob.get("payload") {
        identity.insert("payload".to_string(), payload.clone());
    }
    if let Some(schemas) = blob.get("schemas") {
        identity.insert("schemas".to_string(), schemas.clone());
    }
    if !identity.is_empty() {
        filter.insert("representation".to_string(), Value::Object(identity));
    }
    if !user.is_empty() {
        filter.insert("annotations".to_string(), Value::Object(user));
    }
    filter.insert(
        "aborted".to_string(),
        blob.get("aborted").cloned().unwrap_or(Value::Bool(false)),
    );
    Ok(Value::Object(filter))
}

/// The inner schema of an optional position, when it is one.
fn optional_inner_kind(kind: &SchemaKind) -> Option<&SchemaKind> {
    match kind {
        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => Some(inner),
        _ => None,
    }
}

fn convert_field_list(
    fields: &[crate::Field],
    rest: Option<&SchemaKind>,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    let mut property_signatures = Vec::with_capacity(fields.len());
    for field in fields {
        let optional = field.optionality() != crate::Optionality::Required;
        // An optional field wraps its inner schema in the matching kind; the
        // wrapper carries no document of its own.
        let mut doc = match optional_inner_kind(&field.inner().repr_kind()) {
            Some(inner) => convert_kind(inner, state, hoist_identifiers, enclosing, path)?,
            None => {
                convert_schema_enclosing(field.inner(), state, hoist_identifiers, enclosing, path)?
            }
        };
        let _ = &mut doc;
        let mut property = serde_json::Map::new();
        property.insert("name".to_string(), Value::String(field.name().to_string()));
        property.insert("type".to_string(), doc);
        property.insert("isOptional".to_string(), Value::Bool(optional));
        property.insert("isMutable".to_string(), Value::Bool(false));
        // Key annotations describe the property, not its type.
        if let Some(pairs) = field.inner().resolve_annotations_key() {
            let annotations: serde_json::Map<String, Value> = pairs
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect();
            if !annotations.is_empty() {
                property.insert("annotations".to_string(), Value::Object(annotations));
            }
        }
        property_signatures.push(Value::Object(property));
    }
    finish_objects(
        property_signatures,
        rest,
        state,
        hoist_identifiers,
        enclosing,
        path,
    )
}

#[allow(clippy::too_many_arguments)]
fn convert_struct_fields(
    fields: &[(String, SchemaKind)],
    rest: Option<&SchemaKind>,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    let mut property_signatures = Vec::with_capacity(fields.len());
    for (name, kind) in fields {
        // The kind-driven path has no key annotations to keep: only the field
        // list carries those.
        let (optional, inner) = match optional_inner_kind(kind) {
            Some(inner) => (true, inner),
            None => (false, kind),
        };
        let doc = convert_kind(inner, state, hoist_identifiers, enclosing, path)?;
        let mut property = serde_json::Map::new();
        property.insert("name".to_string(), Value::String(name.clone()));
        property.insert("type".to_string(), doc);
        property.insert("isOptional".to_string(), Value::Bool(optional));
        property.insert("isMutable".to_string(), Value::Bool(false));
        property_signatures.push(Value::Object(property));
    }
    finish_objects(
        property_signatures,
        rest,
        state,
        hoist_identifiers,
        enclosing,
        path,
    )
}

fn finish_objects(
    property_signatures: Vec<Value>,
    rest: Option<&SchemaKind>,
    state: &mut Convert,
    hoist_identifiers: bool,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    let mut index_signatures = Vec::new();
    if let Some(rest) = rest {
        match rest {
            SchemaKind::Record(key, value) => {
                let parameter = convert_kind(key, state, hoist_identifiers, enclosing, path)?;
                let target = convert_kind(value, state, hoist_identifiers, enclosing, path)?;
                index_signatures.push(serde_json::json!({
                    "parameter": parameter, "type": target,
                }));
            }
            other => {
                let target = convert_kind(other, state, hoist_identifiers, enclosing, path)?;
                index_signatures.push(serde_json::json!({
                    "parameter": keyword("String"), "type": target,
                }));
            }
        }
    }
    Ok(serde_json::json!({
        "_tag": "Objects",
        "propertySignatures": property_signatures,
        "indexSignatures": index_signatures,
        "checks": [],
    }))
}

fn convert_element_list(
    elements: &[Schema<Unknown>],
    rest: Option<&[SchemaKind]>,
    state: &mut Convert,
    enclosing: Option<&str>,
    path: &str,
) -> Result<Value, ReprError> {
    let mut converted = Vec::with_capacity(elements.len());
    for element in elements {
        let optional = matches!(
            element.repr_kind(),
            SchemaKind::Optional(_) | SchemaKind::OptionalKey(_)
        );
        let mut doc = match optional_inner_kind(&element.repr_kind()) {
            Some(inner) => convert_kind(inner, state, true, enclosing, path)?,
            None => convert_schema_enclosing(element, state, true, enclosing, path)?,
        };
        let _ = &mut doc;
        let mut entry = serde_json::Map::new();
        entry.insert("type".to_string(), doc);
        entry.insert("isOptional".to_string(), Value::Bool(optional));
        // Key annotations describe the position, not its type.
        if let Some(pairs) = element.resolve_annotations_key() {
            let annotations: serde_json::Map<String, Value> = pairs
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect();
            if !annotations.is_empty() {
                entry.insert("annotations".to_string(), Value::Object(annotations));
            }
        }
        converted.push(Value::Object(entry));
    }
    let mut rest_docs = Vec::new();
    if let Some(rest) = rest {
        for (index, rest) in rest.iter().enumerate() {
            let inner = match optional_inner_kind(rest) {
                Some(inner) => inner,
                None => rest,
            };
            rest_docs.push(convert_kind(
                inner,
                state,
                true,
                enclosing,
                &idx(&seg(path, "rest"), index),
            )?);
        }
    }
    Ok(serde_json::json!({
        "_tag": "Arrays",
        "elements": converted,
        "rest": rest_docs,
        "checks": [],
    }))
}

fn to_json_value(doc: &Value, path: &str) -> Result<Value, ReprError> {
    let tag = doc
        .get("_tag")
        .and_then(Value::as_str)
        .ok_or_else(|| ReprError::at(path, "a representation carries a string `_tag`"))?;
    match tag {
        "Reference" => {
            let name = doc.get("$ref").and_then(Value::as_str).ok_or_else(|| {
                ReprError::at(seg(path, "$ref"), "a `Reference` carries a `$ref`")
            })?;
            if name.is_empty() {
                return Err(ReprError::at(
                    seg(path, "$ref"),
                    "Expected a value with a length of at least 1",
                ));
            }
            Ok(doc.clone())
        }
        "Suspend" => {
            if let Some(Value::Array(checks)) = doc.get("checks") {
                if !checks.is_empty() {
                    return Err(ReprError::at(
                        idx(&seg(path, "checks"), 0),
                        "Expected no excess property",
                    ));
                }
            }
            Ok(doc.clone())
        }
        // A string literal that spells a non-finite number is ambiguous in
        // JSON, so it travels in a typed wrapper; every other literal stays
        // plain, like `toRepresentation` emits it. Checks still validate below.
        "Literal" => {
            let mut out = doc.clone();
            if let Some(Value::String(text)) = doc.get("literal") {
                if matches!(text.as_str(), "NaN" | "Infinity" | "-Infinity") {
                    out["literal"] = serde_json::json!({ "type": "string", "value": text });
                }
            }
            validate_checks(&out, path)?;
            Ok(strip_reserved(&out))
        }
        _ => {
            validate_checks(doc, path)?;
            Ok(strip_reserved(doc))
        }
    }
}

/// Every Filter check carries its representation identity; a bare one cannot
/// persist, which is what makes `toJson` refuse it.
fn validate_checks(doc: &Value, path: &str) -> Result<(), ReprError> {
    if let Some(Value::Array(checks)) = doc.get("checks") {
        for (index, check) in checks.iter().enumerate() {
            let check_tag = check.get("_tag").and_then(Value::as_str);
            if check_tag == Some("Filter") && check.get("representation").is_none() {
                return Err(ReprError::at(
                    seg(&idx(&seg(path, "checks"), index), "representation"),
                    "Missing key",
                ));
            }
        }
    }
    Ok(())
}

/// Reserved pairs are port-internal, never persisted.
fn strip_reserved(doc: &Value) -> Value {
    let mut out = doc.clone();
    if let Some(Value::Object(annotations)) = doc.get("annotations") {
        let kept: serde_json::Map<String, Value> = annotations
            .iter()
            .filter(|(key, _)| !key.starts_with('\u{0}'))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if let Some(map) = out.as_object_mut() {
            if kept.is_empty() {
                map.remove("annotations");
            } else {
                map.insert("annotations".to_string(), Value::Object(kept));
            }
        }
    }
    if let Some(Value::Object(annotation)) = out.get("annotations") {
        if annotation.is_empty() {
            out.as_object_mut().map(|map| map.remove("annotations"));
        }
    }
    out
}

struct Revive<'a> {
    references: &'a BTreeMap<String, Value>,
    bodies: BTreeMap<String, Schema<Unknown>>,
    slots: BTreeMap<String, Arc<std::sync::OnceLock<Schema<Unknown>>>>,
    resolving: Vec<String>,
    revivers: &'a [Reviver],
    declarations: &'a [DeclarationReviver],
}

/// The parameters a declaration reviver rebuilds from, and the payload its
/// constructor was annotated with.
pub struct DeclarationInput {
    pub payload: Value,
    pub parameters: Vec<Schema<Unknown>>,
}

/// A declaration reviver: the pin's `DeclarationReviver`. One per built-in
/// declaration id, plus whatever a caller registers.
pub struct DeclarationReviver {
    pub id: String,
    pub revive: Arc<dyn Fn(DeclarationInput) -> Result<Schema<Unknown>, ReprError> + Send + Sync>,
}

impl DeclarationReviver {
    pub fn new(
        id: &str,
        revive: impl Fn(DeclarationInput) -> Result<Schema<Unknown>, ReprError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.to_string(),
            revive: Arc::new(revive),
        }
    }
}

fn revive_named(state: &mut Revive, doc: &Value, path: &str) -> Result<Schema<Unknown>, ReprError> {
    let mut schema = revive_doc(state, doc, path)?;
    // Checks apply outside-in, so the outermost check ends up outermost. Node
    // annotations wrap last: a check wrapper carries its own annotation slot,
    // and the node's must stay on the node, not under the check.
    if let Some(Value::Array(checks)) = doc.get("checks") {
        for (index, check) in checks.iter().enumerate() {
            schema = revive_check(state, schema, check, &idx(&seg(path, "checks"), index))?;
        }
    }
    // Node annotations restore onto the revived schema; a structured node
    // annotation keeps its canonical text, like any other JSON value.
    if let Some(Value::Object(annotations)) = doc.get("annotations") {
        let pairs: Vec<(String, String)> = annotations
            .iter()
            .filter(|(key, _)| !key.starts_with('\u{0}'))
            .map(|(key, value)| pair_from_json(key, value))
            .collect();
        if !pairs.is_empty() {
            schema = Schema::annotated_with(schema, pairs);
        }
    }
    // A `Declaration` node's `representation` IS its identity, which the reviver
    // already put on the schema. Re-annotating it here would wrap the rebuilt
    // constructor in a pair the document never carried, and the wrapper renders
    // the shape underneath (`Schema.Array(Schema.Any)`) instead of the
    // constructor's own code.
    if schema.declaration().is_none()
        && let Some(representation) = doc.get("representation")
        && !representation.is_null()
    {
        schema = Schema::annotated_with(
            schema,
            vec![("representation".to_string(), representation.to_string())],
        );
    }
    Ok(schema)
}

fn revive_reference(
    state: &mut Revive,
    key: &str,
    path: &str,
) -> Result<Schema<Unknown>, ReprError> {
    if let Some(body) = state.bodies.get(key) {
        return Ok(body.clone());
    }
    if state.resolving.iter().any(|name| name == key) {
        let slot = state.slots.get(key).cloned().ok_or_else(|| {
            ReprError::at(
                path,
                format!("reference `{key}` is resolving without a slot"),
            )
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
        .ok_or_else(|| ReprError::at(seg(path, "$ref"), format!("Invalid reference {key}")))?
        .clone();
    state
        .slots
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(std::sync::OnceLock::new()));
    state.resolving.push(key.to_string());
    let schema = revive_named(state, &doc, &format!("[\"references\"][\"{key}\"]"));
    state.resolving.pop();
    let schema = schema.map(|schema| {
        // The hoisted body already carries its own identifier annotation, so
        // only a body without one takes the reference name.
        let already = schema.resolve_annotations().is_some_and(|pairs| {
            pairs
                .iter()
                .any(|(name, value)| name == "identifier" && value == key)
        });
        if already {
            schema
        } else {
            Schema::annotated_with(schema, vec![("identifier".to_string(), key.to_string())])
        }
    })?;
    state.bodies.insert(key.to_string(), schema.clone());
    if let Some(slot) = state.slots.get(key) {
        let _ = slot.set(schema.clone());
    }
    Ok(schema)
}

fn revive_doc(state: &mut Revive, doc: &Value, path: &str) -> Result<Schema<Unknown>, ReprError> {
    let tag = doc
        .get("_tag")
        .and_then(Value::as_str)
        .ok_or_else(|| ReprError::at(path, "a representation carries a string `_tag`"))?;
    match tag {
        "String" => Ok(Schema::as_unknown(Schema::string())),
        "Number" => Ok(Schema::as_unknown(Schema::number())),
        "Boolean" => Ok(Schema::as_unknown(Schema::boolean())),
        "Null" => Ok(Schema::as_unknown(Schema::null())),
        "Void" => Ok(Schema::as_unknown(Schema::void())),
        "Never" => Ok(Schema::as_unknown(Schema::never())),
        "Any" => Ok(Schema::as_unknown(Schema::any())),
        "Unknown" => Ok(Schema::as_unknown(Schema::unknown())),
        "ObjectKeyword" => Ok(Schema::as_unknown(Schema::object_keyword())),
        "Undefined" | "BigInt" | "Symbol" | "UniqueSymbol" => Err(ReprError::at(
            path,
            format!("`{tag}` has no domain in this port"),
        )),
        "TemplateLiteral" => {
            let parts = doc
                .get("parts")
                .and_then(Value::as_array)
                .ok_or_else(|| ReprError::at(path, "a `TemplateLiteral` carries `parts`"))?;
            let mut kinds = Vec::with_capacity(parts.len());
            for (index, part) in parts.iter().enumerate() {
                let schema = revive_named(state, part, &idx(&seg(path, "parts"), index))?;
                kinds.push(schema.repr_kind());
            }
            match Schema::template_literal_from_kinds(kinds) {
                Some(schema) => Ok(Schema::as_unknown(schema)),
                None => Err(ReprError::at(
                    path,
                    "a template part spells no template part (BigInt, symbols and opaque shapes cannot)",
                )),
            }
        }
        "Literal" => {
            let literal = doc
                .get("literal")
                .ok_or_else(|| ReprError::at(path, "a `Literal` carries a `literal`"))?;
            // Typed wrappers keep ambiguous values apart; plain scalars are
            // the common shape.
            let value = if literal.is_object() {
                from_typed_value(literal, path)?
            } else {
                from_json_value(literal, path)?
            };
            Ok(Schema::as_unknown(Schema::literal(value)))
        }
        "Enum" => {
            let members = doc
                .get("enums")
                .and_then(Value::as_array)
                .ok_or_else(|| ReprError::at(path, "an `Enum` carries `enums`"))?;
            let mut values = Vec::with_capacity(members.len());
            for (index, member) in members.iter().enumerate() {
                let pair = member.as_array().ok_or_else(|| {
                    ReprError::at(
                        idx(&seg(path, "enums"), index),
                        "an enum member is a name/value pair",
                    )
                })?;
                let value = pair.get(1).ok_or_else(|| {
                    ReprError::at(
                        idx(&seg(path, "enums"), index),
                        "an enum member is a name/value pair",
                    )
                })?;
                let member_path = idx(&seg(path, "enums"), index);
                let value = if value.is_object() {
                    from_typed_value(value, &member_path)?
                } else {
                    from_json_value(value, &member_path)?
                };
                if let Unknown::Number(number) = &value {
                    if !number.is_finite() {
                        let rendered = if number.is_nan() {
                            "NaN".to_string()
                        } else if *number > 0.0 {
                            "Infinity".to_string()
                        } else {
                            "-Infinity".to_string()
                        };
                        return Err(ReprError::at(
                            path,
                            format!("A numeric enum value must be finite, got {rendered}"),
                        ));
                    }
                }
                values.push(value);
            }
            Ok(match values.len() {
                0 => Schema::as_unknown(Schema::never()),
                1 => Schema::as_unknown(Schema::literal(
                    values.into_iter().next().expect("one value"),
                )),
                _ => Schema::as_unknown(Schema::literals(values)),
            })
        }
        "Arrays" => revive_arrays(state, doc, path),
        "Objects" => revive_objects(state, doc, path),
        "Union" => {
            let types = doc
                .get("types")
                .and_then(Value::as_array)
                .ok_or_else(|| ReprError::at(path, "a `Union` carries `types`"))?;
            if types.is_empty() {
                return Ok(Schema::as_unknown(Schema::never()));
            }
            let mode = doc
                .get("options")
                .and_then(|options| options.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("anyOf");
            if !["anyOf", "oneOf", "allOf"].contains(&mode) {
                return Err(ReprError::at(
                    path,
                    "a union mode is `anyOf`, `oneOf` or `allOf`",
                ));
            }
            let mut members = Vec::with_capacity(types.len());
            for (index, member) in types.iter().enumerate() {
                members.push(revive_named(
                    state,
                    member,
                    &idx(&seg(path, "types"), index),
                )?);
            }
            // The mode is one of three static spellings, so no allocation.
            let mode: &'static str = match mode {
                "oneOf" => "oneOf",
                "allOf" => "allOf",
                _ => "anyOf",
            };
            Ok(Schema::union_with_mode(members, mode))
        }
        "Suspend" => {
            let thunk = doc
                .get("thunk")
                .ok_or_else(|| ReprError::at(path, "a `Suspend` carries a `thunk`"))?;
            // A reference thunk stays lazy through the slot table, exactly
            // like a named recursive definition; anything else revives
            // eagerly, which terminates because the thunk is already a value.
            if thunk.get("_tag").and_then(Value::as_str) == Some("Reference") {
                let key = thunk
                    .get("$ref")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ReprError::at(path, "a `Reference` carries a `$ref`"))?;
                return revive_reference(state, key, path);
            }
            let inner = revive_named(state, thunk, &seg(path, "thunk"))?;
            Ok(Schema::<Unknown>::suspend(move || inner.clone()))
        }
        "Reference" => {
            let key = doc
                .get("$ref")
                .and_then(Value::as_str)
                .ok_or_else(|| ReprError::at(path, "a `Reference` carries a `$ref`"))?;
            revive_reference(state, key, path)
        }
        "Declaration" => {
            let identity = doc.get("representation");
            let id = identity
                .and_then(|identity| identity.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ReprError::at(
                        seg(path, "representation"),
                        "Missing representation annotation",
                    )
                })?;
            let parameters = match doc.get("typeParameters") {
                Some(Value::Array(parameters)) => {
                    let mut revived = Vec::with_capacity(parameters.len());
                    for (index, parameter) in parameters.iter().enumerate() {
                        revived.push(revive_named(
                            state,
                            parameter,
                            &idx(&seg(path, "typeParameters"), index),
                        )?);
                    }
                    revived
                }
                None => Vec::new(),
                Some(_) => {
                    return Err(ReprError::at(
                        seg(path, "typeParameters"),
                        "a `Declaration` carries a `typeParameters` array",
                    ));
                }
            };
            let reviver = state
                .declarations
                .iter()
                .find(|reviver| reviver.id == id)
                .ok_or_else(|| {
                    ReprError::at(
                        seg(path, "representation"),
                        format!("Missing reviver for {id}"),
                    )
                })?;
            let payload = identity
                .and_then(|identity| identity.get("payload"))
                .cloned()
                .unwrap_or(Value::Null);
            (reviver.revive)(DeclarationInput { payload, parameters })
        }
        other => Err(ReprError::at(
            path,
            format!("no reviver for `{other}` without the pin's reviver set"),
        )),
    }
}

/// A `schemaToJsonSchemaDocument` document: the dialect, the root schema and
/// the hoisted definitions. The pin spells the definition slot
/// `definitions` here (not `$defs`) and always carries the dialect.
pub fn schema_to_json_schema_document(schema: &Schema<Unknown>) -> Value {
    schema_to_json_schema_document_with_options(
        schema,
        &crate::json_schema::CompileOptions::default(),
    )
}

/// [`schema_to_json_schema_document`] with explicit compiler options.
pub fn schema_to_json_schema_document_with_options(
    schema: &Schema<Unknown>,
    options: &crate::json_schema::CompileOptions,
) -> Value {
    let mut compiled = crate::json_schema::compile_with_options(schema, options);
    let definitions = compiled
        .as_object_mut()
        .and_then(|map| map.remove("$defs"))
        .unwrap_or(Value::Object(Default::default()));
    serde_json::json!({
        "dialect": "draft-2020-12",
        "schema": compiled,
        "definitions": definitions,
    })
}

/// A JSON structural value back into the port's domain. Typed wrappers keep
/// ambiguous values apart; anything outside the domain fails instead of
/// guessing.
fn from_json_value(value: &Value, path: &str) -> Result<Unknown, ReprError> {
    match value {
        Value::Null => Ok(Unknown::Null),
        Value::Bool(flag) => Ok(Unknown::Bool(*flag)),
        Value::Number(number) => number
            .as_f64()
            .map(Unknown::Number)
            .ok_or_else(|| ReprError::at(path, "a JSON number decodes to a finite double")),
        Value::String(text) => Ok(Unknown::Text(text.clone())),
        Value::Array(_) | Value::Object(_) => {
            Err(ReprError::at(path, "a literal is a JSON scalar"))
        }
    }
}

/// A typed `{ type, value }` wrapper back into the port's domain.
fn from_typed_value(wrapper: &Value, path: &str) -> Result<Unknown, ReprError> {
    let kind = wrapper
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ReprError::at(path, "a typed value carries a `type`"))?;
    let value = wrapper
        .get("value")
        .ok_or_else(|| ReprError::at(path, "a typed value carries a `value`"))?;
    match kind {
        "string" => value
            .as_str()
            .map(|text| Unknown::Text(text.to_string()))
            .ok_or_else(|| ReprError::at(path, "a string value carries text")),
        "number" => match value {
            Value::String(text) => match text.as_str() {
                "NaN" => Ok(Unknown::Number(f64::NAN)),
                "Infinity" => Ok(Unknown::Number(f64::INFINITY)),
                "-Infinity" => Ok(Unknown::Number(f64::NEG_INFINITY)),
                _ => Err(ReprError::at(
                    path,
                    "an ambiguous number names NaN or an infinity",
                )),
            },
            Value::Number(number) => number
                .as_f64()
                .map(Unknown::Number)
                .ok_or_else(|| ReprError::at(path, "a number value carries a number")),
            _ => Err(ReprError::at(path, "a number value carries a number")),
        },
        "bigint" | "symbol" => Err(ReprError::at(
            path,
            format!("`{kind}` has no domain in this port"),
        )),
        other => Err(ReprError::at(
            path,
            format!("unknown typed value `{other}`"),
        )),
    }
}

fn revive_arrays(
    state: &mut Revive,
    doc: &Value,
    path: &str,
) -> Result<Schema<Unknown>, ReprError> {
    let elements = doc
        .get("elements")
        .and_then(Value::as_array)
        .ok_or_else(|| ReprError::at(path, "an `Arrays` carries `elements`"))?;
    let rest = doc
        .get("rest")
        .and_then(Value::as_array)
        .ok_or_else(|| ReprError::at(path, "an `Arrays` carries `rest`"))?;
    let mut converted = Vec::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
        let element_path = idx(&seg(path, "elements"), index);
        let inner = element
            .get("type")
            .ok_or_else(|| ReprError::at(&element_path, "an element carries a `type`"))?;
        let mut schema = revive_named(state, inner, &seg(&element_path, "type"))?;
        if element.get("isOptional") == Some(&Value::Bool(true)) {
            schema = Schema::optional_key_shape(schema);
        }
        if let Some(Value::Object(annotations)) = element.get("annotations") {
            let pairs: Vec<(String, String)> = annotations
                .iter()
                .map(|(key, value)| pair_from_json(key, value))
                .collect();
            if !pairs.is_empty() {
                schema = Schema::annotate_key(schema, pairs);
            }
        }
        converted.push(schema);
    }
    if !rest.is_empty() {
        let mut revived = Vec::with_capacity(rest.len());
        for (index, item) in rest.iter().enumerate() {
            revived.push(revive_named(state, item, &idx(&seg(path, "rest"), index))?);
        }
        // A lone rest element with no leading positions is a plain array: the
        // pin's `Arrays` with no elements and one rest reads the same way.
        if converted.is_empty() && revived.len() == 1 {
            let only = revived.pop().expect("one rest element");
            return Ok(Schema::as_unknown(Schema::array(only)));
        }
        return Ok(Schema::as_unknown(Schema::tuple_with_rest(
            converted, revived,
        )));
    }
    Ok(Schema::as_unknown(Schema::tuple_of(converted)))
}

fn revive_objects(
    state: &mut Revive,
    doc: &Value,
    path: &str,
) -> Result<Schema<Unknown>, ReprError> {
    let properties = doc
        .get("propertySignatures")
        .and_then(Value::as_array)
        .ok_or_else(|| ReprError::at(path, "an `Objects` carries `propertySignatures`"))?;
    let index = doc
        .get("indexSignatures")
        .and_then(Value::as_array)
        .ok_or_else(|| ReprError::at(path, "an `Objects` carries `indexSignatures`"))?;
    if index.len() > 1 {
        return Err(ReprError::at(
            path,
            "an `Objects` carries at most one index signature",
        ));
    }
    let mut fields = Vec::with_capacity(properties.len());
    for (index, property) in properties.iter().enumerate() {
        let property_path = idx(&seg(path, "propertySignatures"), index);
        let name = property_name(property.get("name"), &property_path)?;
        if property.get("isMutable") == Some(&Value::Bool(true)) {
            return Err(ReprError::at(
                &property_path,
                "mutable fields are not revived by this port",
            ));
        }
        let inner = property
            .get("type")
            .ok_or_else(|| ReprError::at(&property_path, "a property carries a `type`"))?;
        let mut schema = revive_named(state, inner, &seg(&property_path, "type"))?;
        if let Some(Value::Object(annotations)) = property.get("annotations") {
            let pairs: Vec<(String, String)> = annotations
                .iter()
                .map(|(key, value)| pair_from_json(key, value))
                .collect();
            if !pairs.is_empty() {
                schema = Schema::annotate_key(schema, pairs);
            }
        }
        fields.push(if property.get("isOptional") == Some(&Value::Bool(true)) {
            crate::Field::optional_key(&name, schema)
        } else {
            crate::Field::required(&name, schema)
        });
    }
    if let Some(signature) = index.first() {
        let signature_path = idx(&seg(path, "indexSignatures"), 0);
        let parameter = signature.get("parameter").ok_or_else(|| {
            ReprError::at(&signature_path, "an index signature carries a `parameter`")
        })?;
        let key = revive_named(state, parameter, &seg(&signature_path, "parameter"))?;
        let target = signature
            .get("type")
            .ok_or_else(|| ReprError::at(&signature_path, "an index signature carries a `type`"))?;
        let value = revive_named(state, target, &seg(&signature_path, "type"))?;
        let rest = Schema::as_unknown(Schema::record2(key, value));
        if fields.is_empty() {
            return Ok(rest);
        }
        let plain: Vec<(String, Schema<Unknown>)> = fields
            .into_iter()
            .map(|field| (field.name().to_string(), field.inner().clone()))
            .collect();
        return Ok(Schema::as_unknown(Schema::struct_with_rest(plain, rest)));
    }
    Ok(Schema::as_unknown(Schema::struct_shape(fields)))
}

/// A property name back into text: plain strings stay, a string-typed wrapper
/// decodes, and a number-typed one is refused. The pin's `PropertyKey` is a
/// string or a symbol, so a numeric name is a document the reader rejects
/// rather than one it coerces into a key (`fromJson.test.ts:295`).
fn property_name(name: Option<&Value>, path: &str) -> Result<String, ReprError> {
    match name {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Object(wrapper))
            if wrapper.get("type").and_then(Value::as_str) == Some("string") => wrapper
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| ReprError::at(path, "a string name carries text")),
        Some(Value::Object(_)) => Err(ReprError::at(path, "a property name is a string")),
        _ => Err(ReprError::at(path, "a property carries a string `name`")),
    }
}

fn revive_check(
    state: &mut Revive,
    base: Schema<Unknown>,
    check: &Value,
    path: &str,
) -> Result<Schema<Unknown>, ReprError> {
    let tag = check
        .get("_tag")
        .and_then(Value::as_str)
        .ok_or_else(|| ReprError::at(path, "a check carries a string `_tag`"))?;
    match tag {
        "Filter" => revive_filter(state, base, check, path),
        "FilterGroup" => revive_group(state, base, check, path),
        other => Err(ReprError::at(
            path,
            format!("`{other}` is not a revivable check"),
        )),
    }
}

fn check_annotations(check: &Value) -> Vec<(String, String)> {
    match check.get("annotations") {
        Some(Value::Object(annotations)) => annotations
            .iter()
            .filter(|(key, _)| !key.starts_with('\u{0}'))
            .map(|(key, value)| pair_from_json(key, value))
            .collect(),
        _ => Vec::new(),
    }
}

fn revive_filter(
    state: &mut Revive,
    base: Schema<Unknown>,
    check: &Value,
    path: &str,
) -> Result<Schema<Unknown>, ReprError> {
    let identity = check.get("representation");
    let id = identity
        .and_then(|identity| identity.get("id"))
        .and_then(Value::as_str);
    let Some(id) = id else {
        return Err(ReprError::at(
            seg(path, "representation"),
            "Missing representation annotation",
        ));
    };
    let reviver = state
        .revivers
        .iter()
        .find(|reviver| reviver.id == id && !reviver.group)
        .ok_or_else(|| {
            ReprError::at(
                seg(path, "representation"),
                format!("Missing reviver for {id}"),
            )
        })?;
    let payload = identity
        .and_then(|identity| identity.get("payload"))
        .cloned()
        .unwrap_or(Value::Null);
    let payload = unknown_from_json(&payload).ok_or_else(|| {
        ReprError::at(
            seg(&seg(path, "representation"), "payload"),
            format!("Invalid representation payload for {id}"),
        )
    })?;
    if reviver.payload_schema.decode(payload.clone()).is_err() {
        return Err(ReprError::at(
            seg(&seg(path, "representation"), "payload"),
            format!("Invalid representation payload for {id}"),
        ));
    }
    let aborted = check.get("aborted") == Some(&Value::Bool(true));
    let annotations = check_annotations(check);
    let revived = (reviver.revive)(ReviveInput {
        annotations: annotations.clone(),
        payload,
    });
    match revived {
        RevivedCheck::Named { name, args } => {
            let annotation: Vec<(&str, &str)> = annotations
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            Ok(Schema::checked(base, &name, arg_refs, annotation))
        }
        RevivedCheck::Custom { predicate } => {
            let mut spec = FilterSpec::new(move |value| FilterOutcome::from_bool(predicate(value)));
            spec.annotation = annotations;
            if aborted {
                spec = spec.aborting();
            }
            let blob = custom_blob(id, check, aborted);
            spec.annotation.push((BLOB_KEY.to_string(), blob));
            Ok(Schema::filtered(base, vec![spec], false))
        }
    }
}

/// The document identity of a custom filter, stored beside the predicate so
/// the converter re-emits exactly what was revived.
fn custom_blob(id: &str, check: &Value, aborted: bool) -> String {
    let mut blob = serde_json::Map::new();
    blob.insert("kind".to_string(), Value::String("filter".to_string()));
    blob.insert("id".to_string(), Value::String(id.to_string()));
    blob.insert(
        "payload".to_string(),
        check
            .get("representation")
            .and_then(|identity| identity.get("payload"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    if let Some(schemas) = check
        .get("representation")
        .and_then(|identity| identity.get("schemas"))
    {
        blob.insert("schemas".to_string(), schemas.clone());
    }
    blob.insert("aborted".to_string(), Value::Bool(aborted));
    Value::Object(blob).to_string()
}

fn revive_group(
    state: &mut Revive,
    base: Schema<Unknown>,
    check: &Value,
    path: &str,
) -> Result<Schema<Unknown>, ReprError> {
    let annotations = check_annotations(check);
    let identity = check.get("representation");
    let id = identity
        .and_then(|identity| identity.get("id"))
        .and_then(Value::as_str);
    // An identified group revives from its reviver instead of its children.
    if let Some(id) = id {
        if let Some(reviver) = state
            .revivers
            .iter()
            .find(|reviver| reviver.id == id && reviver.group)
        {
            let payload = identity
                .and_then(|identity| identity.get("payload"))
                .cloned()
                .unwrap_or(Value::Null);
            let payload = unknown_from_json(&payload).ok_or_else(|| {
                ReprError::at(
                    seg(&seg(path, "representation"), "payload"),
                    format!("Invalid representation payload for {id}"),
                )
            })?;
            if reviver.payload_schema.decode(payload.clone()).is_err() {
                return Err(ReprError::at(
                    seg(&seg(path, "representation"), "payload"),
                    format!("Invalid representation payload for {id}"),
                ));
            }
            let revived = (reviver.revive)(ReviveInput {
                annotations: annotations.clone(),
                payload,
            });
            return match revived {
                RevivedCheck::Named { name, args } => {
                    let annotation: Vec<(&str, &str)> = annotations
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str()))
                        .collect();
                    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    Ok(Schema::checked(base, &name, arg_refs, annotation))
                }
                RevivedCheck::Custom { predicate } => {
                    let mut spec =
                        FilterSpec::new(move |value| FilterOutcome::from_bool(predicate(value)));
                    spec.annotation = annotations;
                    let mut blob = serde_json::Map::new();
                    blob.insert("kind".to_string(), Value::String("group".to_string()));
                    blob.insert("id".to_string(), Value::String(id.to_string()));
                    blob.insert(
                        "payload".to_string(),
                        identity
                            .and_then(|identity| identity.get("payload"))
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                    blob.insert(
                        "checks".to_string(),
                        check
                            .get("checks")
                            .cloned()
                            .unwrap_or(Value::Array(Vec::new())),
                    );
                    spec.annotation
                        .push((BLOB_KEY.to_string(), Value::Object(blob).to_string()));
                    Ok(Schema::filtered(base, vec![spec], false))
                }
            };
        }
    }
    let children = check
        .get("checks")
        .and_then(Value::as_array)
        .ok_or_else(|| ReprError::at(path, "a `FilterGroup` carries `checks`"))?;
    let mut specs = Vec::with_capacity(children.len());
    let mut child_docs = Vec::with_capacity(children.len());
    for (index, child) in children.iter().enumerate() {
        let child_path = idx(&seg(path, "checks"), index);
        let child_tag = child.get("_tag").and_then(Value::as_str);
        if child_tag != Some("Filter") {
            return Err(ReprError::at(
                &child_path,
                "a filter group only groups filters",
            ));
        }
        // The predicate comes from the child's own reviver; the document
        // block is kept so the converter re-emits the group exactly.
        let predicate = child_predicate(state, child, &child_path)?;
        let mut spec = FilterSpec::new(move |value| predicate(value));
        spec.annotation = check_annotations(child);
        if child.get("aborted") == Some(&Value::Bool(true)) {
            spec = spec.aborting();
        }
        specs.push(spec);
        child_docs.push(stripped_check(child));
    }
    let mut blob = serde_json::Map::new();
    blob.insert("kind".to_string(), Value::String("group".to_string()));
    if let Some(id) = id {
        blob.insert("id".to_string(), Value::String(id.to_string()));
    }
    blob.insert("checks".to_string(), Value::Array(child_docs));
    let mut group_annotation = annotations;
    group_annotation.push((BLOB_KEY.to_string(), Value::Object(blob).to_string()));
    // The group annotation rides the first spec, which is the one the kind
    // keeps; the predicates ride every spec, which is what a decode runs.
    if let Some(first) = specs.first_mut() {
        first.annotation = group_annotation;
    }
    Ok(Schema::filtered(base, specs, false))
}

/// A child's predicate on its own, for groups revived from their children.
fn child_predicate(
    state: &Revive,
    child: &Value,
    path: &str,
) -> Result<Arc<dyn Fn(&Unknown) -> FilterOutcome + Send + Sync>, ReprError> {
    let identity = child.get("representation");
    let id = identity
        .and_then(|identity| identity.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ReprError::at(
                seg(path, "representation"),
                "Missing representation annotation",
            )
        })?;
    let reviver = state
        .revivers
        .iter()
        .find(|reviver| reviver.id == id && !reviver.group)
        .ok_or_else(|| {
            ReprError::at(
                seg(path, "representation"),
                format!("Missing reviver for {id}"),
            )
        })?;
    let payload = identity
        .and_then(|identity| identity.get("payload"))
        .cloned()
        .unwrap_or(Value::Null);
    let payload = unknown_from_json(&payload).ok_or_else(|| {
        ReprError::at(
            seg(&seg(path, "representation"), "payload"),
            format!("Invalid representation payload for {id}"),
        )
    })?;
    if reviver.payload_schema.decode(payload.clone()).is_err() {
        return Err(ReprError::at(
            seg(&seg(path, "representation"), "payload"),
            format!("Invalid representation payload for {id}"),
        ));
    }
    let annotations = check_annotations(child);
    let revived = (reviver.revive)(ReviveInput {
        annotations,
        payload,
    });
    match revived {
        RevivedCheck::Named { name, args } => {
            let arg = args.first().cloned();
            let check_name = name.clone();
            Ok(Arc::new(move |value: &Unknown| {
                FilterOutcome::from_bool(crate::check_holds_named(
                    &check_name,
                    arg.as_deref(),
                    value,
                ))
            }))
        }
        RevivedCheck::Custom { predicate } => Ok(Arc::new(move |value: &Unknown| {
            FilterOutcome::from_bool(predicate(value))
        })),
    }
}

/// A group child's document without port-internal pairs.
fn stripped_check(check: &Value) -> Value {
    let mut out = check.clone();
    if let Some(Value::Object(annotations)) = check.get("annotations") {
        let kept: serde_json::Map<String, Value> = annotations
            .iter()
            .filter(|(key, _)| !key.starts_with('\u{0}'))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if let Some(map) = out.as_object_mut() {
            if kept.is_empty() {
                map.remove("annotations");
            } else {
                map.insert("annotations".to_string(), Value::Object(kept));
            }
        }
    }
    out
}

/// A JSON payload back into the port's domain. Objects and arrays convert
/// structurally; anything outside JSON scalars fails, which is what makes an
/// invalid reviver payload invalid.
fn unknown_from_json(value: &Value) -> Option<Unknown> {
    match value {
        Value::Null => Some(Unknown::Null),
        Value::Bool(flag) => Some(Unknown::Bool(*flag)),
        Value::Number(number) => number.as_f64().map(Unknown::Number),
        Value::String(text) => Some(Unknown::Text(text.clone())),
        Value::Array(items) => items
            .iter()
            .map(unknown_from_json)
            .collect::<Option<Vec<_>>>()
            .map(Unknown::Array),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| unknown_from_json(value).map(|value| (key.clone(), value)))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(Unknown::Object),
    }
}
