//! Endpoint construction with response validation (`HttpApiEndpoint`).
//!
//! Ports the constructor half of `packages/effect/src/http-api/HttpApiEndpoint.ts`:
//! identifiers, payload content-type maps, and the success/error response
//! validation (streaming rules, `WithHeaders` exclusivity, SSE reserved
//! events, `HEAD` refusals). Schema codecs are out of scope: there is no
//! codec transform here, so `disable_codecs` only marks the stored value as
//! untransformed.

/// Reserved SSE failure event name: a stream failure schema using it is refused.
pub const RESERVED_STREAM_FAILURE_EVENT: &str = "effect/http-api/stream/failure";

/// Normalize a content type for map keys and comparisons: lowercase, parameters stripped.
pub fn normalize_content_type(content_type: &str) -> String {
    content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// Payload encoding kinds (`asJson` / `asText` / form / multipart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    /// JSON encoding.
    Json,
    /// Plain-text encoding.
    Text,
    /// URL-encoded form encoding.
    Form,
    /// Multipart encoding.
    Multipart,
}

/// One payload encoding: the declared content type plus its encoding kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadEncoding {
    /// The content type exactly as declared.
    pub declared_content_type: String,
    /// The encoding kind.
    pub kind: PayloadKind,
}

impl PayloadEncoding {
    /// A payload encoding with this declared content type and kind.
    pub fn new(content_type: &str, kind: PayloadKind) -> Self {
        Self { declared_content_type: content_type.to_owned(), kind }
    }

    /// Map key: normalized content type.
    pub fn key(&self) -> String {
        normalize_content_type(&self.declared_content_type)
    }
}

/// A response body shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseBody {
    /// Buffered schema with this content type.
    Buffered {
        /// Response content type.
        content_type: String,
    },
    /// Server-sent events with this content type and optional failure event.
    Sse {
        /// Response content type.
        content_type: String,
        /// Failure event literal, when pinned.
        failure_event: Option<String>,
    },
    /// Raw bytes with this content type.
    Bytes {
        /// Response content type.
        content_type: String,
    },
    /// Empty response.
    NoContent,
}

impl ResponseBody {
    /// Buffered JSON response.
    pub fn json() -> Self {
        Self::Buffered { content_type: "application/json".to_owned() }
    }

    /// Buffered response with this content type.
    pub fn buffered(content_type: &str) -> Self {
        Self::Buffered { content_type: content_type.to_owned() }
    }

    /// SSE with the default `text/event-stream` content type.
    pub fn sse(content_type: &str, failure_event: Option<&str>) -> Self {
        Self::Sse {
            content_type: content_type.to_owned(),
            failure_event: failure_event.map(str::to_owned),
        }
    }

    /// The body's content type, if it has one.
    pub fn content_type(&self) -> Option<&str> {
        match self {
            Self::Buffered { content_type }
            | Self::Sse { content_type, .. }
            | Self::Bytes { content_type } => Some(content_type),
            Self::NoContent => None,
        }
    }

    fn is_stream(&self) -> bool {
        matches!(self, Self::Sse { .. } | Self::Bytes { .. })
    }

    fn is_no_content(&self) -> bool {
        matches!(self, Self::NoContent)
    }
}

/// One declared response: status override, body, and headers wrapper flag.
///
/// `WithHeaders` / `encodeToWithHeaders` are both `with_headers: true`; the
/// stored body is the inner schema, mirroring how validation unwraps the
/// wrapper before the stream checks and reads the status from the outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseSpec {
    /// Status override; befalls the set default at build.
    pub status: Option<u16>,
    /// Response body.
    pub body: ResponseBody,
    /// Whether this response carries headers.
    pub with_headers: bool,
}

impl ResponseSpec {
    /// Buffered response with an optional status override.
    pub fn buffered(status: Option<u16>, content_type: &str) -> Self {
        Self { status, body: ResponseBody::buffered(content_type), with_headers: false }
    }

    /// SSE response with an optional status override and failure event.
    pub fn sse(status: Option<u16>, content_type: Option<&str>, failure_event: Option<&str>) -> Self {
        Self {
            status,
            body: ResponseBody::sse(
                content_type.unwrap_or("text/event-stream"),
                failure_event,
            ),
            with_headers: false,
        }
    }

    /// Raw-bytes response with an optional status override.
    pub fn bytes(status: Option<u16>, content_type: &str) -> Self {
        Self { status, body: ResponseBody::Bytes { content_type: content_type.to_owned() }, with_headers: false }
    }

    /// Empty response with an optional status override (defaults to 204).
    pub fn no_content(status: Option<u16>) -> Self {
        Self { status, body: ResponseBody::NoContent, with_headers: false }
    }

    /// Wrap this response with headers, with an optional status override.
    pub fn with_headers(inner: ResponseSpec, status: Option<u16>) -> Self {
        Self { status: status.or(inner.status), body: inner.body, with_headers: true }
    }

    /// The inner body (the wrapper is a flag, so this is the stored body).
    pub fn inner_content(&self) -> &ResponseBody {
        &self.body
    }

    /// The body, when present (bodies are always present in this model).
    pub fn body(&self) -> Option<&ResponseBody> {
        Some(&self.body)
    }
}

/// Endpoint HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointMethod {
    /// GET.
    Get,
    /// POST.
    Post,
    /// HEAD.
    Head,
}

/// A validated endpoint: identifier, method, path, payloads, and response sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Endpoint identifier.
    pub identifier: String,
    /// Endpoint method.
    pub method: EndpointMethod,
    /// Route path.
    pub path: String,
    /// Payload encodings in declaration order.
    pub payload: Vec<PayloadEncoding>,
    /// Validated success responses.
    pub success: Vec<ResponseSpec>,
    /// Validated error responses.
    pub error: Vec<ResponseSpec>,
    /// Codecs disabled: stored values are untransformed (no transform exists).
    pub disable_codecs: bool,
    /// Annotations in declaration order.
    pub annotations: Vec<(String, String)>,
}

impl Endpoint {
    fn builder(identifier: &str, method: EndpointMethod, path: &str) -> EndpointBuilder {
        EndpointBuilder {
            identifier: identifier.to_owned(),
            method,
            path: path.to_owned(),
            payload: Vec::new(),
            success: Vec::new(),
            error: Vec::new(),
            disable_codecs: false,
            annotations: Vec::new(),
        }
    }

    /// A GET endpoint builder.
    pub fn get(identifier: &str, path: &str) -> EndpointBuilder {
        Self::builder(identifier, EndpointMethod::Get, path)
    }

    /// A POST endpoint builder.
    pub fn post(identifier: &str, path: &str) -> EndpointBuilder {
        Self::builder(identifier, EndpointMethod::Post, path)
    }

    /// A HEAD endpoint builder.
    pub fn head(identifier: &str, path: &str) -> EndpointBuilder {
        Self::builder(identifier, EndpointMethod::Head, path)
    }

    /// Payload entry for a normalized content-type key, if any.
    pub fn payload_entry(&self, normalized_content_type: &str) -> Option<&PayloadEncoding> {
        self.payload.iter().find(|entry| entry.key() == normalized_content_type)
    }

    /// Whether the success set holds this body with this headers flag.
    pub fn success_has(&self, response: &ResponseSpec) -> bool {
        self.success.iter().any(|stored| {
            stored.body == response.body && stored.with_headers == response.with_headers
        })
    }

    /// A derived endpoint under `scope`: the path gains the prefix, the
    /// identifier, codec flag, and annotations carry over.
    pub fn prefixed(&self, scope: &str) -> Self {
        let mut next = self.clone();
        let scope = scope.trim_end_matches('/');
        next.path = format!("{scope}{}", self.path);
        next
    }

    /// A derived endpoint with one more annotation.
    pub fn annotated(&self, key: &str, value: &str) -> Self {
        let mut next = self.clone();
        next.annotations.push((key.to_owned(), value.to_owned()));
        next
    }
}

/// An endpoint under construction: method, path, and declared schemas.
pub struct EndpointBuilder {
    identifier: String,
    method: EndpointMethod,
    path: String,
    payload: Vec<PayloadEncoding>,
    success: Vec<ResponseSpec>,
    error: Vec<ResponseSpec>,
    disable_codecs: bool,
    annotations: Vec<(String, String)>,
}

impl EndpointBuilder {
    /// Declared payload encodings.
    pub fn payload(mut self, payload: Vec<PayloadEncoding>) -> Self {
        self.payload = payload;
        self
    }

    /// Declared success responses.
    pub fn success(mut self, success: Vec<ResponseSpec>) -> Self {
        self.success = success;
        self
    }

    /// Declared error responses.
    pub fn error(mut self, error: Vec<ResponseSpec>) -> Self {
        self.error = error;
        self
    }

    /// Disable codec transforms: stored schemas stay exactly as declared.
    pub fn disable_codecs(mut self) -> Self {
        self.disable_codecs = true;
        self
    }

    /// Validate payloads and responses, mirroring the pin's constructor errors.
    pub fn build(self) -> Result<Endpoint, String> {
        let mut payload: Vec<PayloadEncoding> = Vec::with_capacity(self.payload.len());
        for encoding in &self.payload {
            if let Some(existing) = payload.iter().find(|entry| entry.key() == encoding.key()) {
                if existing.kind != encoding.kind {
                    return Err(format!(
                        "Multiple payload encodings for content-type: {}",
                        encoding.declared_content_type
                    ));
                }
                if encoding.kind == PayloadKind::Multipart {
                    return Err(format!(
                        "Multiple multipart payloads for content-type: {}",
                        encoding.declared_content_type
                    ));
                }
                continue;
            }
            payload.push(encoding.clone());
        }
        let success = validate_success(&self.success, self.method)?;
        let error = validate_error(&self.error)?;
        Ok(Endpoint {
            identifier: self.identifier,
            method: self.method,
            path: self.path,
            payload,
            success,
            error,
            disable_codecs: self.disable_codecs,
            annotations: self.annotations,
        })
    }
}

fn validate_success(responses: &[ResponseSpec], method: EndpointMethod) -> Result<Vec<ResponseSpec>, String> {
    let mut has_stream = false;
    let mut statuses: std::collections::HashMap<u16, StatusEntry> =
        std::collections::HashMap::new();
    for response in responses {
        let status = response.status.unwrap_or(default_success_status(&response.body));
        if response.body.is_stream() {
            validate_stream(&response.body, method)?;
            if has_stream {
                return Err("Multiple streaming success responses are not supported".to_owned());
            }
            has_stream = true;
            let entry = statuses.entry(status).or_default();
            if entry.no_content {
                return Err(format!(
                    "Cannot combine no-content and streaming success responses for status: {status}"
                ));
            }
            let content_type = normalize_content_type(
                response.body.content_type().unwrap_or(""),
            );
            if entry.buffered_content_types.contains(&content_type) {
                return Err(format!(
                    "Cannot combine buffered and streaming success responses for status {status} and content-type: {}",
                    response.body.content_type().unwrap_or("")
                ));
            }
            entry.stream = true;
            entry.stream_content_types.insert(content_type);
        } else {
            let entry = statuses.entry(status).or_default();
            let no_content = response.body.is_no_content();
            if entry.stream {
                if no_content {
                    return Err(format!(
                        "Cannot combine no-content and streaming success responses for status: {status}"
                    ));
                }
                let content_type = normalize_content_type(
                    response.body.content_type().unwrap_or(""),
                );
                if entry.stream_content_types.contains(&content_type) {
                    return Err(format!(
                        "Cannot combine buffered and streaming success responses for status {status} and content-type: {}",
                        response.body.content_type().unwrap_or("")
                    ));
                }
            }
            if !no_content {
                entry.buffered_content_types.insert(normalize_content_type(
                    response.body.content_type().unwrap_or(""),
                ));
            }
            entry.no_content = entry.no_content || no_content;
        }
    }
    validate_exclusivity(responses, 200)?;
    Ok(responses
        .iter()
        .map(|response| ResponseSpec {
            status: Some(response.status.unwrap_or(default_success_status(&response.body))),
            body: response.body.clone(),
            with_headers: response.with_headers,
        })
        .collect())
}

fn validate_error(responses: &[ResponseSpec]) -> Result<Vec<ResponseSpec>, String> {
    for response in responses {
        if response.body.is_stream() {
            return Err("Streaming schemas are not supported in error responses".to_owned());
        }
    }
    validate_exclusivity(responses, 500)?;
    Ok(responses
        .iter()
        .map(|response| ResponseSpec {
            status: Some(response.status.unwrap_or(default_error_status(&response.body))),
            body: response.body.clone(),
            with_headers: response.with_headers,
        })
        .collect())
}

#[derive(Default)]
struct StatusEntry {
    stream: bool,
    stream_content_types: std::collections::HashSet<String>,
    buffered_content_types: std::collections::HashSet<String>,
    no_content: bool,
}

fn default_success_status(body: &ResponseBody) -> u16 {
    if body.is_no_content() {
        204
    } else {
        200
    }
}

fn default_error_status(body: &ResponseBody) -> u16 {
    if body.is_no_content() {
        204
    } else {
        500
    }
}

fn validate_stream(body: &ResponseBody, method: EndpointMethod) -> Result<(), String> {
    if method == EndpointMethod::Head {
        return Err("HEAD endpoints cannot declare streaming success responses".to_owned());
    }
    if let ResponseBody::Sse { failure_event: Some(event), .. } = body {
        if event == RESERVED_STREAM_FAILURE_EVENT {
            return Err(format!("SSE event name is reserved: {RESERVED_STREAM_FAILURE_EVENT}"));
        }
    }
    Ok(())
}

fn validate_exclusivity(responses: &[ResponseSpec], default_status: u16) -> Result<(), String> {
    let mut statuses: std::collections::HashMap<u16, ExclusivityEntry> =
        std::collections::HashMap::new();
    for response in responses {
        let status = response.status.unwrap_or(default_status_for(response, default_status));
        let content_type = response
            .body
            .content_type()
            .map(normalize_content_type)
            .unwrap_or_default();
        let entry = statuses.entry(status).or_default();
        let display = if content_type.is_empty() {
            "<no content>".to_owned()
        } else {
            content_type.clone()
        };
        if response.with_headers {
            if entry.header_content_type.is_some() {
                return Err(format!(
                    "Cannot declare multiple responses with headers for status {status}"
                ));
            }
            if entry.plain_content_types.contains(&content_type) {
                return Err(format!(
                    "Cannot combine a response with headers with another response for status {status} and content-type: {display}"
                ));
            }
            entry.header_content_type = Some(content_type);
        } else {
            if entry.header_content_type.as_deref() == Some(content_type.as_str()) {
                return Err(format!(
                    "Cannot combine a response with headers with another response for status {status} and content-type: {display}"
                ));
            }
            entry.plain_content_types.insert(content_type);
        }
    }
    Ok(())
}

fn default_status_for(response: &ResponseSpec, default_status: u16) -> u16 {
    response.status.unwrap_or(match &response.body {
        ResponseBody::NoContent => 204,
        _ => default_status,
    })
}

#[derive(Default)]
struct ExclusivityEntry {
    header_content_type: Option<String>,
    plain_content_types: std::collections::HashSet<String>,
}

// --- HttpApiSchema annotation layer ---
//
// Ports the annotation half of `HttpApiSchema.ts`: status codes, response
// encodings, stream metadata, `WithHeaders` branding, and the
// `encodeToWithHeaders` body/headers split. There is no Schema compiler here,
// so a schema is a metadata record and codecs are plain closures plus a
// `serde_json` string codec for the SSE data envelope.

/// Response body encoding tag (`asJson` / `asText` / `asUint8Array`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseEncodingTag {
    /// JSON encoding.
    Json,
    /// Plain-text encoding.
    Text,
    /// Raw-byte encoding.
    Uint8Array,
    /// URL-encoded form encoding.
    FormUrlEncoded,
}

/// Response body encoding: tag plus declared content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseEncoding {
    /// Encoding tag.
    pub tag: ResponseEncodingTag,
    /// Declared content type.
    pub content_type: String,
}

impl ResponseEncoding {
    /// Default JSON encoding.
    pub fn json() -> Self {
        Self::json_ct("application/json")
    }

    /// JSON encoding with this content type.
    pub fn json_ct(content_type: &str) -> Self {
        Self {
            tag: ResponseEncodingTag::Json,
            content_type: content_type.to_owned(),
        }
    }

    /// Default plain-text encoding.
    pub fn text() -> Self {
        Self {
            tag: ResponseEncodingTag::Text,
            content_type: "text/plain".to_owned(),
        }
    }

    /// Raw-byte encoding.
    pub fn uint8array() -> Self {
        Self {
            tag: ResponseEncodingTag::Uint8Array,
            content_type: "application/octet-stream".to_owned(),
        }
    }
}

/// One schema's HTTP metadata: optional status and encoding annotations.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaMeta {
    status: Option<u16>,
    encoding: Option<ResponseEncoding>,
    description: Option<String>,
}

impl SchemaMeta {
    /// A schema with no HTTP annotations.
    pub fn plain() -> Self {
        Self::default()
    }

    /// A void schema with this status (`Empty(code)`).
    pub fn empty(code: u16) -> Self {
        Self {
            status: Some(code),
            encoding: None,
            description: None,
        }
    }

    /// Annotate this status code.
    pub fn status(mut self, code: u16) -> Self {
        self.status = Some(code);
        self
    }

    /// Annotate by status literal (`"Created"` is 201, `"Conflict"` is 409).
    pub fn status_literal(self, name: &str) -> Self {
        match name {
            "Created" => self.status(201),
            "Conflict" => self.status(409),
            _ => panic!("unknown status literal: {name}"),
        }
    }

    /// Mark as JSON (`asJson`).
    pub fn as_json(mut self) -> Self {
        self.encoding = Some(ResponseEncoding::json());
        self
    }

    /// Mark as text (`asText`).
    pub fn as_text(mut self) -> Self {
        self.encoding = Some(ResponseEncoding::text());
        self
    }

    /// Mark as binary (`asUint8Array`).
    pub fn as_uint8array(mut self) -> Self {
        self.encoding = Some(ResponseEncoding::uint8array());
        self
    }

    /// Success status: the annotation, else 200.
    pub fn status_success(&self) -> u16 {
        self.status.unwrap_or(200)
    }

    /// Error status: the annotation, else 500.
    pub fn status_error(&self) -> u16 {
        self.status.unwrap_or(500)
    }

    /// Response encoding: the annotation, else JSON.
    pub fn response_encoding(&self) -> ResponseEncoding {
        self.encoding.clone().unwrap_or_else(ResponseEncoding::json)
    }
}

/// SSE payload mode: a full event codec or a JSON-string data schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseMode {
    /// Full event codec.
    Events,
    /// JSON-string data schema.
    Data,
}

/// `StreamSse` metadata: mode, content type, and error schema name.
///
/// `error: None` is the default `Never` error schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSseMeta {
    /// Event codec mode.
    pub sse_mode: SseMode,
    /// Declared content type.
    pub content_type: String,
    /// Error schema name, if one was supplied.
    pub error: Option<String>,
    description: Option<String>,
}

impl StreamSseMeta {
    /// An events stream with the default content type and optional error.
    pub fn events(error: Option<&str>) -> Self {
        Self::events_ct("text/event-stream", error)
    }

    /// An events stream with this content type and optional error.
    pub fn events_ct(content_type: &str, error: Option<&str>) -> Self {
        Self {
            sse_mode: SseMode::Events,
            content_type: content_type.to_owned(),
            error: error.map(str::to_owned),
            description: None,
        }
    }

    /// A data stream: the data field is a JSON-string codec over `data_schema`.
    pub fn data(data_schema: &str, error: Option<&str>) -> Self {
        let _ = data_schema;
        Self {
            sse_mode: SseMode::Data,
            content_type: "text/event-stream".to_owned(),
            error: error.map(str::to_owned),
            description: None,
        }
    }

    /// Stream mode: always `"sse"`.
    pub fn mode(&self) -> &'static str {
        "sse"
    }

    /// Error schema name: the annotation, else `"Never"`.
    pub fn error_or_never(&self) -> &str {
        self.error.as_deref().unwrap_or("Never")
    }

    /// Encode one data event: the payload becomes its JSON string.
    pub fn encode_data<T: serde::Serialize>(event: &str, data: &T) -> Result<SseDataEvent, String> {
        serde_json::to_string(data)
            .map(|data_json| SseDataEvent {
                id: None,
                event: event.to_owned(),
                data_json,
            })
            .map_err(|err| err.to_string())
    }

    /// Decode one data event back to its payload.
    pub fn decode_data<T: serde::de::DeserializeOwned>(event: &SseDataEvent) -> Result<T, String> {
        serde_json::from_str(&event.data_json).map_err(|err| err.to_string())
    }
}

/// One SSE data event: the data field carries a JSON string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseDataEvent {
    /// Event id, if set.
    pub id: Option<String>,
    /// Event name.
    pub event: String,
    /// JSON-encoded payload.
    pub data_json: String,
}

/// `StreamUint8Array` metadata: just the content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamUint8ArrayMeta {
    /// Declared content type.
    pub content_type: String,
    description: Option<String>,
}

impl StreamUint8ArrayMeta {
    /// A byte stream with the default content type.
    pub fn new() -> Self {
        Self::with_content_type("application/octet-stream")
    }

    /// A byte stream with this content type.
    pub fn with_content_type(content_type: &str) -> Self {
        Self {
            content_type: content_type.to_owned(),
            description: None,
        }
    }

    /// Stream mode: always `"uint8array"`.
    pub fn mode(&self) -> &'static str {
        "uint8array"
    }
}

impl Default for StreamUint8ArrayMeta {
    fn default() -> Self {
        Self::new()
    }
}

/// A `WithHeaders` response schema: inner schema plus headers record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithHeadersSchema {
    /// The wrapped schema.
    pub inner: Box<SchemaDecl>,
    /// Headers field names.
    pub header_names: Vec<String>,
    wrapper_status: Option<u16>,
    wrapper_encoding: Option<ResponseEncoding>,
    description: Option<String>,
}

/// One annotated schema: plain metadata, a stream, or a headers wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaDecl {
    /// Buffered schema metadata.
    Plain(SchemaMeta),
    /// SSE stream metadata.
    StreamSse(StreamSseMeta),
    /// Byte stream metadata.
    StreamBytes(StreamUint8ArrayMeta),
    /// Headers wrapper.
    WithHeaders(Box<WithHeadersSchema>),
}

/// Whether this schema is a stream schema.
pub fn is_stream_schema(schema: &SchemaDecl) -> bool {
    matches!(
        schema,
        SchemaDecl::StreamSse(_) | SchemaDecl::StreamBytes(_)
    )
}

/// Whether this schema is an SSE stream.
pub fn is_stream_sse(schema: &SchemaDecl) -> bool {
    matches!(schema, SchemaDecl::StreamSse(_))
}

/// Whether this schema is a byte stream.
pub fn is_stream_uint8array(schema: &SchemaDecl) -> bool {
    matches!(schema, SchemaDecl::StreamBytes(_))
}

/// Whether this schema is a `WithHeaders` wrapper.
pub fn is_with_headers(schema: &SchemaDecl) -> bool {
    matches!(schema, SchemaDecl::WithHeaders(_))
}

/// Wrap a schema with headers; nested wrappers are refused.
pub fn with_headers_schema(schema: &SchemaDecl, headers: &[&str]) -> Result<SchemaDecl, String> {
    if is_with_headers(schema) {
        return Err("WithHeaders schemas cannot be nested".to_owned());
    }
    Ok(SchemaDecl::WithHeaders(Box::new(WithHeadersSchema {
        inner: Box::new(schema.clone()),
        header_names: headers.iter().map(|name| (*name).to_owned()).collect(),
        wrapper_status: None,
        wrapper_encoding: None,
        description: None,
    })))
}

/// Annotate a schema, keeping a `WithHeaders` wrapper intact.
pub fn annotate_schema(schema: &SchemaDecl, description: &str) -> SchemaDecl {
    let mut next = schema.clone();
    match &mut next {
        SchemaDecl::Plain(meta) => meta.description = Some(description.to_owned()),
        SchemaDecl::StreamSse(meta) => meta.description = Some(description.to_owned()),
        SchemaDecl::StreamBytes(meta) => meta.description = Some(description.to_owned()),
        SchemaDecl::WithHeaders(wrapped) => {
            wrapped.description = Some(description.to_owned());
        }
    }
    next
}

/// Set the wrapper (or plain) status, mirroring `.pipe(status(code))`.
pub fn with_status_schema(schema: &SchemaDecl, status: u16) -> SchemaDecl {
    let mut next = schema.clone();
    match &mut next {
        SchemaDecl::Plain(meta) => meta.status = Some(status),
        SchemaDecl::WithHeaders(wrapped) => wrapped.wrapper_status = Some(status),
        SchemaDecl::StreamSse(_) | SchemaDecl::StreamBytes(_) => {
            panic!("status is not supported on stream schemas")
        }
    }
    next
}

/// Set the wrapper (or plain) encoding, mirroring `.pipe(asJson(...))`.
pub fn with_encoding_schema(schema: &SchemaDecl, encoding: ResponseEncoding) -> SchemaDecl {
    let mut next = schema.clone();
    match &mut next {
        SchemaDecl::Plain(meta) => meta.encoding = Some(encoding),
        SchemaDecl::WithHeaders(wrapped) => wrapped.wrapper_encoding = Some(encoding),
        SchemaDecl::StreamSse(_) | SchemaDecl::StreamBytes(_) => {
            panic!("encoding is not supported on stream schemas")
        }
    }
    next
}

/// Success status: the wrapper annotation first, then the inner schema.
pub fn status_success_schema(schema: &SchemaDecl) -> u16 {
    match schema {
        SchemaDecl::Plain(meta) => meta.status_success(),
        SchemaDecl::StreamSse(_) | SchemaDecl::StreamBytes(_) => 200,
        SchemaDecl::WithHeaders(wrapped) => wrapped
            .wrapper_status
            .unwrap_or_else(|| status_success_schema(&wrapped.inner)),
    }
}

/// Error status: the wrapper annotation first, then the inner schema.
pub fn status_error_schema(schema: &SchemaDecl) -> u16 {
    match schema {
        SchemaDecl::Plain(meta) => meta.status_error(),
        SchemaDecl::StreamSse(_) | SchemaDecl::StreamBytes(_) => 500,
        SchemaDecl::WithHeaders(wrapped) => wrapped
            .wrapper_status
            .unwrap_or_else(|| status_error_schema(&wrapped.inner)),
    }
}

/// Response encoding: the wrapper annotation first, then the inner schema.
pub fn response_encoding_schema(schema: &SchemaDecl) -> ResponseEncoding {
    match schema {
        SchemaDecl::Plain(meta) => meta.response_encoding(),
        SchemaDecl::StreamSse(_) | SchemaDecl::StreamBytes(_) => ResponseEncoding::json(),
        SchemaDecl::WithHeaders(wrapped) => wrapped
            .wrapper_encoding
            .clone()
            .unwrap_or_else(|| response_encoding_schema(&wrapped.inner)),
    }
}

/// A branded `WithHeaders` response value: only the constructor mints the brand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithHeadersValue<B, H> {
    branded: bool,
    /// Response body.
    pub body: B,
    /// Response headers.
    pub headers: H,
}

/// An unbranded-or-branded candidate for `WithHeaders` decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WithHeadersInput<B, H> {
    /// A value built by the constructor.
    Branded(WithHeadersValue<B, H>),
    /// A bare body/headers pair with no brand.
    Plain {
        /// Response body.
        body: B,
        /// Response headers.
        headers: H,
    },
}

impl<B, H> WithHeadersValue<B, H> {
    /// Build a branded response value.
    pub fn new(body: B, headers: H) -> Self {
        Self {
            branded: true,
            body,
            headers,
        }
    }

    /// Accept only branded values, mirroring the pin's branded decode.
    pub fn decode(input: WithHeadersInput<B, H>) -> Result<Self, String> {
        match input {
            WithHeadersInput::Branded(value) if value.branded => Ok(value),
            _ => Err("WithHeaders value is not branded".to_owned()),
        }
    }
}

/// An `encodeToWithHeaders` pair codec: the body schema is authoritative for
/// status and encoding, source response metadata never carries over.
pub struct EncodeToWithHeaders<S, B, H> {
    body: SchemaMeta,
    /// Headers field names.
    pub header_names: Vec<String>,
    encode: Box<dyn Fn(&S) -> (B, H)>,
    decode: Box<dyn Fn((B, H)) -> S>,
}

impl<S, B, H> EncodeToWithHeaders<S, B, H> {
    /// Build the pair codec from the body schema and the two mappings.
    pub fn new(
        body: SchemaMeta,
        headers: &[&str],
        encode: impl Fn(&S) -> (B, H) + 'static,
        decode: impl Fn((B, H)) -> S + 'static,
    ) -> Self {
        Self {
            body,
            header_names: headers.iter().map(|name| (*name).to_owned()).collect(),
            encode: Box::new(encode),
            decode: Box::new(decode),
        }
    }

    /// Encode a source value to its body/headers pair.
    pub fn encode(&self, value: &S) -> (B, H) {
        (self.encode)(value)
    }

    /// Decode a body/headers pair back to the source value.
    pub fn decode(&self, body: B, headers: H) -> S {
        (self.decode)((body, headers))
    }

    /// Error status: the body annotation, else 500.
    pub fn status_error(&self) -> u16 {
        self.body.status_error()
    }

    /// Response encoding: the body annotation, else JSON.
    pub fn response_encoding(&self) -> ResponseEncoding {
        self.body.response_encoding()
    }
}
