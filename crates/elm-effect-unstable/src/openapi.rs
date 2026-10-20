//! Minimal OpenAPI 3.1 assembly: path templates, operation registry, security schemes.
//!
//! Ports the document-shape half of `packages/effect/test/httpapi/OpenApi.test.ts`
//! plus the representation-consumer half of
//! `packages/effect/test/httpapi/OpenApiRepresentation.test.ts`: `:param`
//! templates with literal `:action` suffixes, duplicate method+path /
//! operationId rejection, equivalent security-scheme dedup, GET+QUERY merging,
//! additional declaration schemas (with the `Encoded` alias pair), a reference
//! policy for inlining, query structs decomposed into parameters without
//! emitting the container, and a spec cache keyed by API and options identity.
//! Schemas arrive as `elm_effect_schema` values and compile through
//! `json_schema` (identifiers hoist once into `components.schemas`, shared
//! across every endpoint).
//!
//! Out of scope (their rows stay excused with reasons): encoded-side
//! projection beyond what the compiler spells (`fromJsonString`, SSE event
//! envelopes), custom compiler callbacks (`toJsonSchema`), codec modes,
//! middleware error sets, header wire schemas
//! (`WithHeaders`/`encodeToWithHeaders`), stream and text/form bodies, and
//! `rawJSON`/override identity. None of that machinery exists yet; every
//! assertion here holds on the routing/registry layer.

use elm_effect_schema::json_schema::{
    CompileOptions, OnExcessProperty, compile_multi_with_options,
};
use elm_effect_schema::{Schema, Unknown};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

/// A templated path: `:name` segments become `{name}`; a segment that does not
/// start with `:` (like `offers:resolve`) and the `:action` tail after a param
/// (like `:id:wait`) stay literal.
pub fn template_path(route: &str) -> String {
    route
        .split('/')
        .map(|segment| match segment.strip_prefix(':') {
            Some(rest) => {
                let end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                if end == 0 {
                    segment.to_owned()
                } else {
                    format!("{{{}}}{}", &rest[..end], &rest[end..])
                }
            }
            None => segment.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The duplicate-detection shape of a route: every `:name` becomes `{}` so
/// `/users/:id` and `/users/:userId` collide.
fn canonical_template(route: &str) -> String {
    route
        .split('/')
        .map(|segment| match segment.strip_prefix(':') {
            Some(rest) => {
                let end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                if end == 0 {
                    segment.to_owned()
                } else {
                    format!("{{}}{}", &rest[end..])
                }
            }
            None => segment.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// An endpoint method. `Query` is not a standard OpenAPI method: its operation
/// lands under `x-oai-additionalOperations`, mirroring the pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// GET.
    Get,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// DELETE.
    Delete,
    /// PATCH.
    Patch,
    /// HEAD.
    Head,
    /// OPTIONS.
    Options,
    /// QUERY (safe retrieval carrying a request body).
    Query,
}

impl Method {
    /// The uppercase method name used in duplicate-operation reports.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
            Self::Query => "QUERY",
        }
    }

    /// The path-item slot: lowercase for standard methods, the additional
    /// operations map for QUERY.
    fn slot(&self) -> &'static str {
        match self {
            Self::Query => "x-oai-additionalOperations",
            _ => match self {
                Self::Get => "get",
                Self::Post => "post",
                Self::Put => "put",
                Self::Delete => "delete",
                Self::Patch => "patch",
                Self::Head => "head",
                Self::Options => "options",
                Self::Query => unreachable!(),
            },
        }
    }
}

/// A parameter location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamIn {
    /// Path template parameter.
    Path,
    /// Query parameter.
    Query,
    /// Header parameter.
    Header,
}

impl ParamIn {
    fn name(&self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Query => "query",
            Self::Header => "header",
        }
    }
}

/// One documented parameter: its schema compiles with the rest of the spec.
pub struct Parameter {
    /// Parameter name.
    pub name: String,
    /// Parameter location.
    pub location: ParamIn,
    /// Whether the parameter is required.
    pub required: bool,
    /// Parameter schema.
    pub schema: Schema<Unknown>,
}

/// One response entry: success defaults to 200, errors to 500.
pub struct ResponseEntry {
    /// Status override; the set default applies when absent.
    pub status: Option<u16>,
    /// Response content type.
    pub content_type: String,
    /// Response schema.
    pub schema: Schema<Unknown>,
}

/// A security scheme: an API key or an HTTP scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityScheme {
    /// API key in a header or query parameter.
    ApiKey {
        /// Key name as declared.
        name: String,
        /// Key location (`header` or `query`).
        location: String,
    },
    /// HTTP authorization scheme.
    Http {
        /// Scheme name as declared.
        scheme: String,
    },
}

impl SecurityScheme {
    /// Whether two schemes for one name describe the same requirement. API key
    /// header names and HTTP schemes compare case-insensitively (the pin keeps
    /// the first declaration); anything else must match exactly.
    fn equivalent(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::ApiKey { name, location },
                Self::ApiKey {
                    name: other_name,
                    location: other_location,
                },
            ) => {
                location.eq_ignore_ascii_case(other_location)
                    && (name == other_name
                        || (location.eq_ignore_ascii_case("header")
                            && name.eq_ignore_ascii_case(other_name)))
            }
            (
                Self::Http { scheme },
                Self::Http {
                    scheme: other_scheme,
                },
            ) => scheme.eq_ignore_ascii_case(other_scheme),
            _ => false,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::ApiKey { name, location } => serde_json::json!({
                "type": "apiKey",
                "name": name,
                "in": location.to_ascii_lowercase(),
            }),
            Self::Http { scheme } => serde_json::json!({
                "type": "http",
                "scheme": scheme,
            }),
        }
    }
}

/// One documented endpoint: routing plus already-chosen schemas.
pub struct ApiEndpoint {
    /// Endpoint identifier (the default operationId).
    pub id: String,
    /// Endpoint method.
    pub method: Method,
    /// Route path with `:param` segments.
    pub path: String,
    /// Operation identifier; absent means none is emitted.
    pub operation_id: Option<String>,
    /// Documented parameters in declaration order.
    pub parameters: Vec<Parameter>,
    /// Request payloads: declared content type plus schema.
    pub payloads: Vec<(String, Schema<Unknown>)>,
    /// Success responses.
    pub success: Vec<ResponseEntry>,
    /// Error responses.
    pub error: Vec<ResponseEntry>,
    /// Security schemes attached to this endpoint, keyed by scheme name.
    pub security: Vec<(String, SecurityScheme)>,
    /// Operation transform, applied to the assembled operation object.
    pub transform: Option<fn(serde_json::Value) -> serde_json::Value>,
    /// Query structs decomposed into parameters: each must compile to an
    /// object (or a `$ref` to one); the container itself is never emitted.
    pub query_structs: Vec<Schema<Unknown>>,
}

impl ApiEndpoint {
    /// A documented endpoint with the identifier as its operationId.
    pub fn new(id: &str, method: Method, path: &str) -> Self {
        Self {
            id: id.to_owned(),
            method,
            path: path.to_owned(),
            operation_id: Some(id.to_owned()),
            parameters: Vec::new(),
            payloads: Vec::new(),
            success: Vec::new(),
            error: Vec::new(),
            security: Vec::new(),
            transform: None,
            query_structs: Vec::new(),
        }
    }
}

/// Assembly failure: duplicate operations, duplicate operationIds, and
/// conflicting security schemes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenApiError {
    /// Two endpoints share a method and an equivalent templated path.
    DuplicateOperation {
        /// Uppercase method name.
        method: String,
        /// Templated path of the colliding endpoint.
        path: String,
    },
    /// Two operations share one operationId.
    DuplicateOperationId {
        /// The repeated identifier.
        id: String,
    },
    /// Two incompatible schemes share one security-scheme name.
    ConflictingSecurityScheme {
        /// The conflicting name.
        name: String,
    },
}

impl fmt::Display for OpenApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOperation { method, path } => {
                write!(f, "Duplicate OpenAPI operation for {method} {path}")
            }
            Self::DuplicateOperationId { id } => {
                write!(f, "Duplicate OpenAPI operationId: {id}")
            }
            Self::ConflictingSecurityScheme { name } => {
                write!(f, "Conflicting OpenAPI security scheme: {name}")
            }
        }
    }
}

impl std::error::Error for OpenApiError {}

/// An OpenAPI 3.1 assembler: endpoints in, one document out.
pub struct SpecBuilder {
    title: String,
    version: String,
    endpoints: Vec<ApiEndpoint>,
    compile_options: CompileOptions,
    additional_schemas: Vec<Schema<Unknown>>,
    reference_policy: Option<fn(&str) -> bool>,
}

impl SpecBuilder {
    /// An assembler for an API with this title (version defaults to `0.0.1`,
    /// mirroring the pin).
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            version: "0.0.1".to_owned(),
            endpoints: Vec::new(),
            compile_options: CompileOptions {
                on_excess_property: OnExcessProperty::Error,
                generate_descriptions: false,
                include_annotation_key: None,
            },
            additional_schemas: Vec::new(),
            reference_policy: None,
        }
    }

    /// One more endpoint, in declaration order.
    pub fn endpoint(mut self, endpoint: ApiEndpoint) -> Self {
        self.endpoints.push(endpoint);
        self
    }

    /// Schema compiler options for every endpoint schema.
    pub fn compile_options(mut self, options: CompileOptions) -> Self {
        self.compile_options = options;
        self
    }

    /// One more additional declaration schema: it lands in
    /// `components.schemas` even when no endpoint uses it.
    pub fn additional_schema(mut self, schema: Schema<Unknown>) -> Self {
        self.additional_schemas.push(schema);
        self
    }

    /// Whether the definition for `identifier` stays a `$ref` (true) or is
    /// inlined at every use (false). Absent, every definition is referenced.
    pub fn reference_policy(mut self, policy: fn(&str) -> bool) -> Self {
        self.reference_policy = Some(policy);
        self
    }

    /// Whether the definition for `identifier` stays referenced under the
    /// current policy (absent policy keeps every reference).
    fn keeps_reference(&self, identifier: &str) -> bool {
        self.reference_policy
            .is_none_or(|policy| policy(identifier))
    }

    /// The assembled document: `openapi`, `info`, `paths`, and `components`.
    /// Every call compiles fresh values, so mutating one document never leaks
    /// into the next.
    pub fn build(&self) -> Result<serde_json::Value, OpenApiError> {
        // OperationIds collide before anything compiles: the report names the id.
        let mut seen_ids = std::collections::HashSet::new();
        for endpoint in &self.endpoints {
            if let Some(id) = &endpoint.operation_id
                && !seen_ids.insert(id.clone())
            {
                return Err(OpenApiError::DuplicateOperationId { id: id.clone() });
            }
        }
        // One shared definition map: an identifier used by several endpoints is
        // emitted once and referenced everywhere. Query structs and additional
        // schemas join the same compilation so shared identifiers stay shared.
        let roots: Vec<Schema<Unknown>> = self
            .endpoints
            .iter()
            .flat_map(|endpoint| {
                endpoint
                    .parameters
                    .iter()
                    .map(|param| param.schema.clone())
                    .chain(endpoint.payloads.iter().map(|(_, schema)| schema.clone()))
                    .chain(endpoint.success.iter().map(|entry| entry.schema.clone()))
                    .chain(endpoint.error.iter().map(|entry| entry.schema.clone()))
                    .chain(endpoint.query_structs.iter().cloned())
            })
            .chain(self.additional_schemas.iter().cloned())
            .collect();
        let compiled = compile_multi_with_options(&roots, &self.compile_options);
        let mut docs = compiled.schemas.into_iter();
        let mut definitions = compiled.definitions;
        // Additional declaration schemas become components, never route
        // documents. An identified declaration aliases its `Encoded` form (the
        // canonical JSON codec definition), exactly like the pin; an inlined
        // identifier lands under its own name instead.
        let mut additional_components: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let mut additional_docs = Vec::new();
        for _ in 0..self.additional_schemas.len() {
            additional_docs.push(docs.next_back());
        }
        additional_docs.reverse();
        for doc in additional_docs.into_iter().flatten() {
            if let Some(id) = ref_target(&doc) {
                if self.keeps_reference(&id) {
                    if let Some(definition) = definitions.remove(&id) {
                        additional_components.insert(
                            id.clone(),
                            serde_json::json!({
                                "$ref": format!("#/components/schemas/{id}Encoded")
                            }),
                        );
                        additional_components.insert(format!("{id}Encoded"), definition);
                    }
                } else if let Some(definition) = definitions.get(&id) {
                    // Inlined identifiers keep their definition in place so the
                    // inline pass below rewrites every use; the component is a
                    // copy of the same document.
                    additional_components.insert(id, definition.clone());
                }
            }
            // An unidentified declaration has no component key; nothing to emit.
        }
        let mut operations: Vec<serde_json::Value> = Vec::new();
        // Query structs decompose into parameters: each property becomes one
        // query parameter, and a `$ref` container never emits itself.
        let mut endpoint_query_params: Vec<Vec<serde_json::Value>> = Vec::new();
        for endpoint in &self.endpoints {
            let mut take = |n: usize| docs.by_ref().take(n).collect::<Vec<_>>();
            let param_docs = take(endpoint.parameters.len());
            let payload_docs = take(endpoint.payloads.len());
            let success_docs = take(endpoint.success.len());
            let error_docs = take(endpoint.error.len());
            let query_docs = take(endpoint.query_structs.len());
            let mut extra_params = Vec::new();
            for query_doc in &query_docs {
                extra_params.extend(query_parameters(query_doc, &mut definitions));
            }
            endpoint_query_params.push(extra_params);
            operations.push(operation_document(
                endpoint,
                &param_docs,
                &payload_docs,
                &success_docs,
                &error_docs,
                &endpoint_query_params[endpoint_query_params.len() - 1],
            ));
        }
        // The reference policy inlines definitions: every `$ref` to an inlined
        // identifier becomes the definition, in operations and in the
        // definitions that remain. Components keep every definition the
        // compilation hoisted.
        if let Some(policy) = self.reference_policy {
            let inline: Vec<(String, serde_json::Value)> = definitions
                .iter()
                .filter(|(id, _)| !policy(id))
                .map(|(id, definition)| (id.clone(), definition.clone()))
                .collect();
            for (id, definition) in &inline {
                let target = format!("#/$defs/{id}");
                for operation in &mut operations {
                    inline_refs(operation, &target, definition);
                }
                for remaining in definitions.values_mut() {
                    inline_refs(remaining, &target, definition);
                }
            }
        }
        let mut paths: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let mut seen_operations = std::collections::HashSet::new();
        for (endpoint, mut operation) in self.endpoints.iter().zip(operations) {
            let path = template_path(&endpoint.path);
            let canonical = format!(
                "{} {}",
                endpoint.method.name(),
                canonical_template(&endpoint.path)
            );
            if !seen_operations.insert(canonical) {
                return Err(OpenApiError::DuplicateOperation {
                    method: endpoint.method.name().to_owned(),
                    path,
                });
            }
            if let Some(transform) = endpoint.transform {
                operation = transform(operation);
            }
            let item = paths.entry(path).or_insert_with(|| serde_json::json!({}));
            if endpoint.method == Method::Query {
                let additional = item
                    .as_object_mut()
                    .expect("path item")
                    .entry("x-oai-additionalOperations")
                    .or_insert_with(|| serde_json::json!({}));
                additional
                    .as_object_mut()
                    .expect("additional operations")
                    .insert("QUERY".to_owned(), operation);
            } else {
                item.as_object_mut()
                    .expect("path item")
                    .insert(endpoint.method.slot().to_owned(), operation);
            }
        }
        let mut components = serde_json::json!({});
        let mut schemas: serde_json::Map<String, serde_json::Value> =
            definitions.into_iter().collect::<serde_json::Map<_, _>>();
        for (name, component) in additional_components {
            schemas.insert(name, component);
        }
        if !schemas.is_empty() {
            components["schemas"] = serde_json::Value::Object(schemas);
        }
        let mut schemes: Vec<(String, SecurityScheme)> = Vec::new();
        for endpoint in &self.endpoints {
            for (name, scheme) in &endpoint.security {
                match schemes.iter().find(|(held, _)| held == name) {
                    None => schemes.push((name.clone(), scheme.clone())),
                    Some((_, held)) => {
                        if !held.equivalent(scheme) {
                            return Err(OpenApiError::ConflictingSecurityScheme {
                                name: name.clone(),
                            });
                        }
                    }
                }
            }
        }
        if !schemes.is_empty() {
            components["securitySchemes"] = serde_json::Value::Object(
                schemes
                    .into_iter()
                    .map(|(name, scheme)| (name, scheme.to_json()))
                    .collect(),
            );
        }
        let mut spec = serde_json::json!({
            "openapi": "3.1.0",
            "info": {"title": self.title, "version": self.version},
            "paths": serde_json::Value::Object(
                paths.into_iter().collect::<serde_json::Map<_, _>>()
            ),
            "components": components,
        });
        // Definitions hoist as `#/$defs/...`; the document spells components.
        let text = serde_json::to_string(&spec).expect("spec serializes");
        spec = serde_json::from_str(&text.replace("#/$defs/", "#/components/schemas/"))
            .expect("spec parses");
        Ok(spec)
    }
}

/// One operation object: operationId (when present), parameters, an optional
/// request body, and status-grouped responses (several schemas for one status
/// and content type become an `anyOf`).
#[allow(clippy::too_many_arguments)]
fn operation_document(
    endpoint: &ApiEndpoint,
    param_docs: &[serde_json::Value],
    payload_docs: &[serde_json::Value],
    success_docs: &[serde_json::Value],
    error_docs: &[serde_json::Value],
    query_params: &[serde_json::Value],
) -> serde_json::Value {
    let mut operation = serde_json::Map::new();
    if let Some(id) = &endpoint.operation_id {
        operation.insert(
            "operationId".to_owned(),
            serde_json::Value::String(id.clone()),
        );
    }
    let mut parameters = endpoint
        .parameters
        .iter()
        .zip(param_docs)
        .map(|(param, schema)| {
            serde_json::json!({
                "name": param.name,
                "in": param.location.name(),
                "required": param.required,
                "schema": schema,
            })
        })
        .collect::<Vec<_>>();
    parameters.extend(query_params.iter().cloned());
    operation.insert(
        "parameters".to_owned(),
        serde_json::Value::Array(parameters),
    );
    if !endpoint.payloads.is_empty() {
        let content = endpoint
            .payloads
            .iter()
            .zip(payload_docs)
            .map(|((content_type, _), schema)| {
                (content_type.clone(), serde_json::json!({"schema": schema}))
            })
            .collect::<serde_json::Map<_, _>>();
        operation.insert(
            "requestBody".to_owned(),
            serde_json::json!({"content": serde_json::Value::Object(content)}),
        );
    }
    let mut responses: BTreeMap<String, BTreeMap<String, Vec<serde_json::Value>>> = BTreeMap::new();
    for (entry, doc) in endpoint
        .success
        .iter()
        .map(|entry| (entry.status.unwrap_or(200), entry))
        .zip(success_docs)
        .chain(
            endpoint
                .error
                .iter()
                .map(|entry| (entry.status.unwrap_or(500), entry))
                .zip(error_docs),
        )
    {
        let ((status, entry), doc) = (entry, doc);
        responses
            .entry(status.to_string())
            .or_default()
            .entry(entry.content_type.clone())
            .or_default()
            .push(doc.clone());
    }
    let responses = responses
        .into_iter()
        .map(|(status, by_content)| {
            let content = by_content
                .into_iter()
                .map(|(content_type, schemas)| {
                    let schema = if schemas.len() == 1 {
                        schemas.into_iter().next().expect("one schema")
                    } else {
                        serde_json::json!({"anyOf": schemas})
                    };
                    (content_type, serde_json::json!({"schema": schema}))
                })
                .collect::<serde_json::Map<_, _>>();
            (
                status,
                serde_json::json!({"content": serde_json::Value::Object(content)}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    operation.insert("responses".to_owned(), serde_json::Value::Object(responses));
    serde_json::Value::Object(operation)
}

/// The definition identifier a document references, when the whole document is
/// exactly one `$ref` to a hoisted definition.
fn ref_target(doc: &serde_json::Value) -> Option<String> {
    let reference = doc.as_object().and_then(|map| {
        (map.len() == 1)
            .then(|| map.get("$ref"))
            .flatten()
            .and_then(|reference| reference.as_str())
    })?;
    reference.strip_prefix("#/$defs/").map(str::to_owned)
}

/// One query parameter per property of a query-struct document: the container
/// (a `$ref` target or an inline object) never emits itself.
fn query_parameters(
    doc: &serde_json::Value,
    definitions: &mut serde_json::Map<String, serde_json::Value>,
) -> Vec<serde_json::Value> {
    let container = ref_target(doc)
        .and_then(|id| definitions.remove(&id))
        .or_else(|| Some(doc.clone()));
    let Some(container) = container else {
        return Vec::new();
    };
    let Some(properties) = container
        .get("properties")
        .and_then(|properties| properties.as_object())
    else {
        return Vec::new();
    };
    let required = container
        .get("required")
        .and_then(|required| required.as_array())
        .map(|required| {
            required
                .iter()
                .filter_map(|name| name.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    properties
        .iter()
        .map(|(name, schema)| {
            serde_json::json!({
                "name": name,
                "in": "query",
                "required": required.iter().any(|held| held == name),
                "schema": schema,
            })
        })
        .collect()
}

/// Replace every `$ref` to `target` with `replacement`. Children resolve
/// before their parent, and an inserted copy is never revisited, so a
/// self-referential definition terminates instead of unfolding forever.
fn inline_refs(value: &mut serde_json::Value, target: &str, replacement: &serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                inline_refs(item, target, replacement);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, member) in map.iter_mut() {
                inline_refs(member, target, replacement);
            }
            if map.len() == 1
                && map.get("$ref").and_then(|reference| reference.as_str()) == Some(target)
            {
                *value = replacement.clone();
            }
        }
        _ => {}
    }
}

/// A spec cache keyed by API and options identity: one build per key, a fresh
/// clone on every hit, so mutating a returned document never poisons the cache.
#[derive(Debug, Default)]
pub struct SpecCache {
    entries: Mutex<BTreeMap<(String, u64), serde_json::Value>>,
}

impl SpecCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached document for (`api`, `options`): `build` runs only on a
    /// miss, and every hit clones the stored document.
    pub fn spec(
        &self,
        api: &str,
        options: u64,
        build: impl FnOnce() -> serde_json::Value,
    ) -> serde_json::Value {
        let mut entries = self.entries.lock().expect("spec cache");
        if let Some(cached) = entries.get(&(api.to_owned(), options)) {
            return cached.clone();
        }
        let spec = build();
        entries.insert((api.to_owned(), options), spec.clone());
        spec
    }
}
