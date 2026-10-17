//! MCP handler suites: tools, prompts, resources, logging, sampling, elicitation.
//!
//! In-memory handler layer for the MCP server runtime epic. JSON-RPC message
//! types are defined locally so this lane does not block on the transport
//! lane; they match `{jsonrpc:"2.0", id, method, params}` and
//! `{jsonrpc:"2.0", id, result|error}`. Tool argument validation checks
//! required-param presence and basic types only (no schema compiler).

use serde_json::{Map, Value};
use std::collections::HashMap;

// ── JSON-RPC ────────────────────────────────────────────────────────────────

/// A JSON-RPC request id (number or string).
#[derive(Debug, Clone, PartialEq)]
pub enum RpcId {
    /// Numeric id.
    Num(i64),
    /// String id.
    Text(String),
}

fn rpc_id(value: &Value) -> Option<RpcId> {
    match value {
        Value::Number(n) => n.as_i64().map(RpcId::Num),
        Value::String(s) => Some(RpcId::Text(s.clone())),
        _ => None,
    }
}

fn rpc_id_json(id: &RpcId) -> Value {
    match id {
        RpcId::Num(n) => Value::Number((*n).into()),
        RpcId::Text(s) => Value::String(s.clone()),
    }
}

/// A minimal JSON-RPC request.
#[derive(Debug, Clone)]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Absent for notifications.
    pub id: Option<RpcId>,
    /// Method name, e.g. `"tools/call"`.
    pub method: String,
    /// Method params.
    pub params: Option<Value>,
}

impl RpcRequest {
    /// Decode from wire JSON.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let root = value.as_object().ok_or_else(|| "request must be an object".to_owned())?;
        if root.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err("jsonrpc must be \"2.0\"".to_owned());
        }
        Ok(Self {
            jsonrpc: "2.0".to_owned(),
            id: root.get("id").and_then(rpc_id),
            method: root.get("method").and_then(Value::as_str).ok_or_else(|| "missing method".to_owned())?.to_owned(),
            params: root.get("params").cloned(),
        })
    }
}

/// A JSON-RPC error body.
#[derive(Debug, Clone)]
pub struct RpcError {
    /// Numeric code (e.g. -32602 for invalid params).
    pub code: i64,
    /// Human message.
    pub message: String,
}

impl RpcError {
    /// Invalid-params error (-32602).
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self { code: -32602, message: message.into() }
    }
    /// Method-not-found error (-32601).
    pub fn method_not_found(method: &str) -> Self {
        Self { code: -32601, message: format!("method not found: {method}") }
    }
}

/// A minimal JSON-RPC response.
#[derive(Debug, Clone)]
pub struct RpcResponse {
    /// The request id (absent only for notifications, which get no response).
    pub id: Option<RpcId>,
    /// Success payload.
    pub result: Option<Value>,
    /// Failure payload.
    pub error: Option<RpcError>,
}

impl RpcResponse {
    /// Encode to wire JSON.
    pub fn encode(&self) -> Value {
        let mut root = Map::new();
        root.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
        if let Some(id) = &self.id {
            root.insert("id".to_owned(), rpc_id_json(id));
        }
        if let Some(result) = &self.result {
            root.insert("result".to_owned(), result.clone());
        }
        if let Some(error) = &self.error {
            root.insert(
                "error".to_owned(),
                serde_json::json!({ "code": error.code, "message": error.message }),
            );
        }
        Value::Object(root)
    }
}

/// A server-to-client notification (no id).
#[derive(Debug, Clone)]
pub struct RpcNotification {
    /// Notification method, e.g. `"notifications/message"`.
    pub method: String,
    /// Notification params.
    pub params: Value,
    /// Always `None`: notifications carry no identifier.
    pub id: Option<RpcId>,
}

impl RpcNotification {
    /// Build a server-to-client notification (id is always `None`).
    pub fn new(method: &str, params: Value) -> Self {
        Self { method: method.to_owned(), params, id: None }
    }
}

// ── Content helpers ─────────────────────────────────────────────────────────

/// A `text` content block.
pub fn content_text(text: impl Into<Value>) -> Value {
    let text = text.into();
    match text {
        Value::String(s) => serde_json::json!({ "type": "text", "text": s }),
        other => serde_json::json!({ "type": "text", "text": other }),
    }
}

// ── Tools ───────────────────────────────────────────────────────────────────

/// A registered tool: name, description, JSON-Schema input shape, handler.
pub struct ToolDef {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// Input JSON Schema (required presence + basic types are enforced).
    pub input_schema: Value,
    handler: Box<dyn Fn(&Value) -> Result<Vec<Value>, String>>,
}

/// A tool call failure: protocol errors reject the request, handler errors
/// ride inside the result with `isError`.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallError {
    /// Unknown tool, malformed params, or validation failure.
    Protocol(String),
}

impl ToolCallError {
    /// True for protocol errors (never for handler failures).
    pub fn is_protocol(&self) -> bool {
        matches!(self, Self::Protocol(_))
    }
}

/// A tool call outcome: content blocks plus the `isError` flag.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Content blocks in order.
    pub content: Vec<Value>,
    /// True when the handler failed (not a protocol error).
    pub is_error: bool,
}

/// In-memory tool registry with list/call and basic input validation.
#[derive(Default)]
pub struct ToolRegistry {
 tools: Vec<ToolDef>,
}

impl ToolRegistry {
    /// Empty registry (advertises no tools capability).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool; registration order is the list order.
    pub fn register(
        &mut self,
        name: &str,
        description: &str,
        input_schema: Value,
        handler: impl Fn(&Value) -> Result<Vec<Value>, String> + 'static,
    ) {
        self.tools.push(ToolDef {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema,
            handler: Box::new(handler),
        });
    }

    /// Whether the tools capability is advertised.
    pub fn capability(&self) -> bool {
        !self.tools.is_empty()
    }

    /// List tools in registration order.
    pub fn list(&self) -> &[ToolDef] {
        &self.tools
    }

    /// Call a tool: unknown names and validation failures are protocol
    /// errors; handler failures return `is_error` content.
    pub fn call(&self, name: &str, args: &Value) -> Result<ToolResult, ToolCallError> {
        let tool = self.tools.iter().find(|t| t.name == name).ok_or_else(|| ToolCallError::Protocol(format!("unknown tool: {name}")))?;
        validate_args(&tool.input_schema, args).map_err(ToolCallError::Protocol)?;
        match (tool.handler)(args) {
            Ok(content) => Ok(ToolResult { content, is_error: false }),
            Err(message) => Ok(ToolResult { content: vec![content_text(message)], is_error: true }),
        }
    }
}

/// Validate `args` against a JSON-Schema subset: object shape, `required`
/// presence, and basic `type` checks per property.
pub fn validate_args(schema: &Value, args: &Value) -> Result<(), String> {
    let args = args.as_object().ok_or_else(|| "tool arguments must be an object".to_owned())?;
    let schema = schema.as_object().ok_or_else(|| "tool schema must be an object".to_owned())?;
    if let Some(Value::Array(required)) = schema.get("required") {
        for name in required.iter().filter_map(Value::as_str) {
            if !args.contains_key(name) {
                return Err(format!("missing required parameter: {name}"));
            }
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, value) in args {
            if let Some(expected) = properties.get(name).and_then(Value::as_object).and_then(|p| p.get("type")).and_then(Value::as_str) {
                check_type(name, expected, value)?;
            }
        }
    }
    Ok(())
}

fn check_type(name: &str, expected: &str, value: &Value) -> Result<(), String> {
    let ok = match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    };
    if ok { Ok(()) } else { Err(format!("parameter {name} must be {expected}")) }
}

// ── Prompts ─────────────────────────────────────────────────────────────────

/// One prompt argument declaration.
#[derive(Debug, Clone)]
pub struct PromptArg {
    /// Argument name.
    pub name: String,
    /// Whether the argument is required.
    pub required: bool,
    /// Allowed values (empty means any value).
    pub choices: Vec<String>,
}

/// A registered prompt with message templates using `{{name}}` substitution.
pub struct PromptDef {
    /// Prompt name.
    pub name: String,
    /// Optional title.
    pub title: Option<String>,
    /// Prompt description.
    pub description: String,
    /// Argument declarations.
    pub arguments: Vec<PromptArg>,
    /// (role, template) message pairs.
    pub templates: Vec<(String, String)>,
}

/// A prompt get outcome: ordered messages.
#[derive(Debug, Clone)]
pub struct PromptResult {
    /// Description echoed back.
    pub description: String,
    /// Ordered messages with text content.
    pub messages: Vec<Value>,
}

/// In-memory prompt registry with list/get and argument substitution.
#[derive(Default)]
pub struct PromptRegistry {
    prompts: Vec<PromptDef>,
}

impl PromptRegistry {
    /// Empty registry (advertises no prompts capability).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a prompt.
    pub fn register(
        &mut self,
        name: &str,
        title: Option<&str>,
        description: &str,
        arguments: Vec<PromptArg>,
        templates: Vec<(String, String)>,
    ) {
        self.prompts.push(PromptDef {
            name: name.to_owned(),
            title: title.map(str::to_owned),
            description: description.to_owned(),
            arguments,
            templates,
        });
    }

    /// Whether the prompts capability is advertised.
    pub fn capability(&self) -> bool {
        !self.prompts.is_empty()
    }

    /// List prompts in registration order.
    pub fn list(&self) -> &[PromptDef] {
        &self.prompts
    }

    /// Get a prompt: validates required args and choices before substituting.
    pub fn get(&self, name: &str, args: &Value) -> Result<PromptResult, String> {
        let prompt = self.prompts.iter().find(|p| p.name == name).ok_or_else(|| format!("unknown prompt: {name}"))?;
        let args = args.as_object().ok_or_else(|| "prompt arguments must be an object".to_owned())?;
        for arg in &prompt.arguments {
            match args.get(&arg.name).and_then(Value::as_str) {
                None if arg.required => return Err(format!("missing required argument: {}", arg.name)),
                Some(value) if !arg.choices.is_empty() && !arg.choices.iter().any(|c| c == value) => {
                    return Err(format!("invalid value for argument: {}", arg.name));
                }
                _ => {}
            }
        }
        let messages = prompt
            .templates
            .iter()
            .map(|(role, template)| {
                let mut text = template.clone();
                for arg in &prompt.arguments {
                    let placeholder = format!("{{{{{}}}}}", arg.name);
                    let value = args.get(&arg.name).and_then(Value::as_str).unwrap_or("");
                    text = text.replace(&placeholder, value);
                }
                serde_json::json!({ "role": role, "content": { "type": "text", "text": text } })
            })
            .collect();
        Ok(PromptResult { description: prompt.description.clone(), messages })
    }
}

// ── Resources ───────────────────────────────────────────────────────────────

/// One resource content entry.
#[derive(Debug, Clone)]
pub struct ResourceContent {
    /// Resource URI.
    pub uri: String,
    /// MIME type, when known.
    pub mime_type: Option<String>,
    /// Text payload.
    pub text: Option<String>,
    /// Base64 binary payload.
    pub blob: Option<String>,
}

impl ResourceContent {
    fn encode(&self) -> Value {
        let mut root = Map::new();
        root.insert("uri".to_owned(), Value::String(self.uri.clone()));
        if let Some(mime) = &self.mime_type {
            root.insert("mimeType".to_owned(), Value::String(mime.clone()));
        }
        if let Some(text) = &self.text {
            root.insert("text".to_owned(), Value::String(text.clone()));
        }
        if let Some(blob) = &self.blob {
            root.insert("blob".to_owned(), Value::String(blob.clone()));
        }
        Value::Object(root)
    }
}

/// A concrete resource entry.
#[derive(Debug, Clone)]
pub struct ResourceDef {
    /// Resource URI.
    pub uri: String,
    /// Resource name.
    pub name: String,
    /// Resource description.
    pub description: String,
    /// MIME type, when known.
    pub mime_type: Option<String>,
    /// Stored content.
    pub content: ResourceContent,
}

/// A resource template with `{param}` URI expansion.
pub struct ResourceTemplate {
    /// URI template, e.g. `"file:///docs/{doc}"`.
    pub uri_template: String,
    /// Template name.
    pub name: String,
    /// Template description.
    pub description: String,
    /// MIME type for expanded reads.
    pub mime_type: String,
    /// Handler receiving decoded params; not invoked when decoding fails.
    pub handler: Box<dyn Fn(&HashMap<String, String>) -> Result<Vec<ResourceContent>, String>>,
}

/// In-memory resource store: concrete map plus templates with URI expansion.
#[derive(Default)]
pub struct ResourceStore {
    resources: Vec<ResourceDef>,
    templates: Vec<ResourceTemplate>,
    subscriptions: Vec<String>,
}

impl ResourceStore {
    /// Empty store (advertises no resources capability).
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a text resource.
    pub fn insert_text(&mut self, uri: &str, name: &str, description: &str, mime: &str, text: &str) {
        self.resources.push(ResourceDef {
            uri: uri.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            mime_type: Some(mime.to_owned()),
            content: ResourceContent { uri: uri.to_owned(), mime_type: Some(mime.to_owned()), text: Some(text.to_owned()), blob: None },
        });
    }

    /// Insert a base64 binary resource.
    pub fn insert_binary(&mut self, uri: &str, name: &str, description: &str, mime: &str, blob: &str) {
        self.resources.push(ResourceDef {
            uri: uri.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            mime_type: Some(mime.to_owned()),
            content: ResourceContent { uri: uri.to_owned(), mime_type: Some(mime.to_owned()), text: None, blob: Some(blob.to_owned()) },
        });
    }

    /// Add a resource template.
    pub fn add_template(&mut self, template: ResourceTemplate) {
        self.templates.push(template);
    }

    /// Whether the resources capability is advertised.
    pub fn capability(&self) -> bool {
        !self.resources.is_empty() || !self.templates.is_empty()
    }

    /// List concrete resources in insertion order.
    pub fn list(&self) -> &[ResourceDef] {
        &self.resources
    }

    /// List registered templates.
    pub fn list_templates(&self) -> &[ResourceTemplate] {
        &self.templates
    }

    /// Read a URI: exact match first, then template expansion.
    pub fn read(&self, uri: &str) -> Result<Vec<ResourceContent>, String> {
        if let Some(found) = self.resources.iter().find(|r| r.uri == uri) {
            return Ok(vec![found.content.clone()]);
        }
        for template in &self.templates {
            if let Some(params) = match_template(&template.uri_template, uri) {
                return (template.handler)(&params).map_err(|e| format!("template handler failed: {e}"));
            }
        }
        Err(format!("unknown resource: {uri}"))
    }

    /// Subscribe to update notifications for a URI.
    pub fn subscribe(&mut self, uri: &str) {
        if !self.subscriptions.iter().any(|u| u == uri) {
            self.subscriptions.push(uri.to_owned());
        }
    }

    /// Stop update notifications for a URI.
    pub fn unsubscribe(&mut self, uri: &str) {
        self.subscriptions.retain(|u| u != uri);
    }

    /// Build an update notification when `uri` is subscribed, else `None`.
    pub fn notify_update(&self, uri: &str) -> Option<RpcNotification> {
        if self.subscriptions.iter().any(|u| u == uri) {
            Some(RpcNotification {
                method: "notifications/resources/updated".to_owned(),
                params: serde_json::json!({ "uri": uri }),
                id: None,
            })
        } else {
            None
        }
    }
}

/// Match `uri` against a `{param}` template; `None` means no match (the
/// handler is never invoked). Percent-encoded bytes that fail to decode
/// also yield `None`.
pub fn match_template(template: &str, uri: &str) -> Option<HashMap<String, String>> {
    let template_parts: Vec<&str> = template.split('/').collect();
    let uri_parts: Vec<&str> = uri.split('/').collect();
    if template_parts.len() != uri_parts.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (pattern, actual) in template_parts.iter().zip(uri_parts.iter()) {
        if let Some(name) = pattern.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            params.insert(name.to_owned(), percent_decode(actual)?);
        } else if pattern != actual {
            return None;
        }
    }
    Some(params)
}

fn percent_decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

// ── Logging ─────────────────────────────────────────────────────────────────

/// MCP log severity, ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Debug.
    Debug,
    /// Info.
    Info,
    /// Notice.
    Notice,
    /// Warning.
    Warning,
    /// Error.
    Error,
    /// Critical.
    Critical,
    /// Alert.
    Alert,
    /// Emergency.
    Emergency,
}

impl std::str::FromStr for LogLevel {
    type Err = String;
    fn from_str(level: &str) -> Result<Self, String> {
        match level {
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "notice" => Ok(Self::Notice),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            "critical" => Ok(Self::Critical),
            "alert" => Ok(Self::Alert),
            "emergency" => Ok(Self::Emergency),
            _ => Err(format!("unknown log level: {level}")),
        }
    }
}

impl LogLevel {
    fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Notice => "notice",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
            Self::Alert => "alert",
            Self::Emergency => "emergency",
        }
    }
}

/// Per-session log router: `setLevel` plus level-gated message routing.
#[derive(Debug, Clone)]
pub struct Logger {
    min_level: LogLevel,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    /// Logger accepting every level.
    pub fn new() -> Self {
        Self { min_level: LogLevel::Debug }
    }

    /// Current minimum level.
    pub fn level(&self) -> LogLevel {
        self.min_level
    }

    /// Apply `logging/setLevel`; unknown levels are rejected.
    pub fn set_level_name(&mut self, level: &str) -> Result<(), String> {
        self.min_level = level.parse()?;
        Ok(())
    }

    /// Route a log message: `None` when below the minimum or the level is
    /// unknown, else a `notifications/message` notification (no id).
    pub fn log(&self, level: &str, logger: Option<&str>, data: Value) -> Option<RpcNotification> {
        let parsed: LogLevel = level.parse().ok()?;
        if parsed < self.min_level {
            return None;
        }
        Some(RpcNotification {
            method: "notifications/message".to_owned(),
            params: serde_json::json!({
                "level": parsed.name(),
                "logger": logger.unwrap_or(""),
                "data": data,
            }),
            id: None,
        })
    }
}

// ── Sampling ────────────────────────────────────────────────────────────────

/// Sampling passthrough: echoes a canned assistant response preserving the
/// request messages and options.
#[derive(Debug, Clone)]
pub struct Sampler {
    /// Model name stamped on the canned response.
    pub model: String,
}

impl Sampler {
    /// Sampler stamping `model` on responses.
    pub fn new(model: &str) -> Self {
        Self { model: model.to_owned() }
    }

    /// Handle a `sampling/createMessage` request with a canned response.
    pub fn create_message(&self, request: &Value) -> Result<Value, String> {
        let root = request.as_object().ok_or_else(|| "sampling request must be an object".to_owned())?;
        let messages = root.get("messages").and_then(Value::as_array).ok_or_else(|| "messages must be an array".to_owned())?;
        if messages.is_empty() {
            return Err("messages must not be empty".to_owned());
        }
        for message in messages {
            Self::decode_content(message.get("content").unwrap_or(&Value::Null))?;
        }
        let summary = messages
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.get("text")).and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(serde_json::json!({
            "role": "assistant",
            "content": { "type": "text", "text": format!("echo: {summary}") },
            "model": self.model,
        }))
    }

    /// Decode one sampling content block to its payload text/data.
    pub fn decode_content(content: &Value) -> Result<String, String> {
        let kind = content.get("type").and_then(Value::as_str).ok_or_else(|| "sampling content needs a type".to_owned())?;
        match kind {
            "text" => content.get("text").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| "text content needs text".to_owned()),
            "image" => content.get("data").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| "image content needs data".to_owned()),
            _ => Err(format!("unsupported sampling content: {kind}")),
        }
    }
}

// ── Elicitation ─────────────────────────────────────────────────────────────

/// A form-mode elicitation request scaffold (`elicitation/create`, form).
#[derive(Debug, Clone)]
pub struct ElicitFormRequest {
    /// Always `"form"` when present.
    pub mode: Option<String>,
    /// The prompt shown to the user.
    pub message: String,
    /// Requested schema properties, in order.
    pub fields: Vec<String>,
    /// The raw requested schema.
    pub requested_schema: Value,
}

impl ElicitFormRequest {
    /// Decode and validate a form request scaffold.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let root = value.as_object().ok_or_else(|| "elicitation request must be an object".to_owned())?;
        if let Some(mode) = root.get("mode") {
            if mode != "form" {
                return Err("mode must be \"form\"".to_owned());
            }
        }
        let message = root.get("message").and_then(Value::as_str).ok_or_else(|| "message must be a string".to_owned())?.to_owned();
        let properties = root
            .get("requestedSchema")
            .and_then(Value::as_object)
            .and_then(|s| s.get("properties"))
            .and_then(Value::as_object)
            .ok_or_else(|| "requestedSchema.properties must be an object".to_owned())?;
        Ok(Self {
            mode: root.get("mode").and_then(Value::as_str).map(str::to_owned),
            message,
            fields: properties.keys().cloned().collect(),
            requested_schema: root["requestedSchema"].clone(),
        })
    }
}

/// A URL-mode elicitation request scaffold (`elicitation/create`, url).
#[derive(Debug, Clone)]
pub struct ElicitUrlRequest {
    /// The prompt shown to the user.
    pub message: String,
    /// The URL the user should open (http/https only).
    pub url: String,
    /// Elicitation id for completing the flow.
    pub elicitation_id: String,
}

impl ElicitUrlRequest {
    /// Decode and validate a URL request scaffold.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let root = value.as_object().ok_or_else(|| "elicitation request must be an object".to_owned())?;
        let message = root.get("message").and_then(Value::as_str).ok_or_else(|| "message must be a string".to_owned())?.to_owned();
        let url = root.get("url").and_then(Value::as_str).ok_or_else(|| "url must be a string".to_owned())?.to_owned();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err("url must use http or https".to_owned());
        }
        let elicitation_id = root.get("elicitationId").and_then(Value::as_str).ok_or_else(|| "elicitationId must be a string".to_owned())?.to_owned();
        Ok(Self { message, url, elicitation_id })
    }
}

// ── Dispatch ────────────────────────────────────────────────────────────────

/// Route one client-initiated request to the tools/prompts/resources/logging
/// handlers and build the JSON-RPC response.
pub fn dispatch(
    request: &RpcRequest,
    tools: &ToolRegistry,
    prompts: &PromptRegistry,
    resources: &ResourceStore,
    logger: &mut Logger,
) -> RpcResponse {
    let respond = |result: Value| RpcResponse { id: request.id.clone(), result: Some(result), error: None };
    let fail = |error: RpcError| RpcResponse { id: request.id.clone(), result: None, error: Some(error) };
    let params = request.params.clone().unwrap_or(Value::Null);
    match request.method.as_str() {
        "tools/list" => respond(serde_json::json!({ "tools": tools.list().iter().map(|t| serde_json::json!({
            "name": t.name, "description": t.description, "inputSchema": t.input_schema,
        })).collect::<Vec<_>>() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() {
                return fail(RpcError::invalid_params("tools/call needs a name"));
            }
            let args = params.get("arguments").unwrap_or(&Value::Null);
            match tools.call(name, args) {
                Ok(out) => respond(serde_json::json!({ "content": out.content, "isError": out.is_error })),
                Err(ToolCallError::Protocol(message)) => fail(RpcError::invalid_params(message)),
            }
        }
        "prompts/list" => respond(serde_json::json!({ "prompts": prompts.list().iter().map(|p| serde_json::json!({
            "name": p.name, "title": p.title, "description": p.description,
            "arguments": p.arguments.iter().map(|a| serde_json::json!({
                "name": a.name, "required": a.required,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>() })),
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").unwrap_or(&Value::Null);
            match prompts.get(name, args) {
                Ok(out) => respond(serde_json::json!({ "description": out.description, "messages": out.messages })),
                Err(message) => fail(RpcError::invalid_params(message)),
            }
        }
        "resources/list" => respond(serde_json::json!({ "resources": resources.list().iter().map(|r| serde_json::json!({
            "uri": r.uri, "name": r.name, "description": r.description, "mimeType": r.mime_type,
        })).collect::<Vec<_>>() })),
        "resources/templates/list" => respond(serde_json::json!({ "resourceTemplates": resources.list_templates().iter().map(|t| serde_json::json!({
            "uriTemplate": t.uri_template, "name": t.name,
        })).collect::<Vec<_>>() })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            match resources.read(uri) {
                Ok(contents) => respond(serde_json::json!({ "contents": contents.iter().map(ResourceContent::encode).collect::<Vec<_>>() })),
                Err(message) => fail(RpcError::invalid_params(message)),
            }
        }
        "logging/setLevel" => {
            let level = params.get("level").and_then(Value::as_str).unwrap_or("");
            match logger.set_level_name(level) {
                Ok(()) => respond(Value::Object(Map::new())),
                Err(message) => fail(RpcError::invalid_params(message)),
            }
        }
        _ => fail(RpcError::method_not_found(&request.method)),
    }
}
