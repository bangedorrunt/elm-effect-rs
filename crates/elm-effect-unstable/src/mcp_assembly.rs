//! Server assembly: the `McpServer` wiring the transport/handshake
//! foundation (`mcp_server`) to the handler registries (`mcp_handlers`).
//!
//! Typed API only (no second JSON-RPC framing layer): initialize with
//! version negotiation, capability advertisement from registration state,
//! tool/prompt/resource dispatch, completion helpers, and a notification
//! outbox that works with no transport running.

use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::mcp_handlers::{
    Logger, PromptRegistry, PromptResult, ResourceContent, ResourceStore, RpcNotification,
    ToolRegistry, ToolResult, content_text, validate_args,
};
use crate::mcp_server::{LATEST_VERSION, SUPPORTED_VERSIONS, ServerInfo};

/// JSON-RPC invalid-params code the pin uses for dispatch failures.
pub const INVALID_PARAMS_CODE: i64 = -32602;

/// A dispatch failure: always a protocol error, never handler content.
#[derive(Debug, Clone, PartialEq)]
pub struct AssemblyError {
    /// Numeric code (`-32602` for invalid params / unknown names).
    pub code: i64,
    /// Human-readable message.
    pub message: String,
}

impl AssemblyError {
    fn invalid_params(message: String) -> Self {
        Self { code: INVALID_PARAMS_CODE, message }
    }
}

/// The neutral request identity handlers observe.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestContext {
    /// Client id.
    pub client_id: u64,
    /// Client name from `clientInfo`.
    pub client_name: String,
    /// Negotiated protocol version.
    pub protocol_version: String,
}

/// Stage a tool success value: objects (and scalars) ride both as text
/// content and as structured content; null and arrays omit structured
/// content (no full schema compiler in this lane).
pub fn encode_tool_success(value: &Value) -> (Vec<Value>, Option<Value>) {
    match value {
        Value::Null | Value::Array(_) => (vec![], None),
        Value::Object(_) => (vec![content_text(value.to_string())], Some(value.clone())),
        Value::String(text) => (vec![content_text(text.clone())], Some(value.clone())),
        Value::Number(_) | Value::Bool(_) => {
            (vec![content_text(value.to_string())], Some(value.clone()))
        }
    }
}

/// The pin's generic client message for internal failures; details stay
/// server-side (recorded only when error reporting is on).
pub fn internal_error_message() -> &'static str {
    "Tool execution failed due to an internal server error."
}

/// A context-aware tool handler: sees the invoking request.
type CtxHandler = Box<dyn Fn(&RequestContext, &Value) -> Result<Vec<Value>, String>>;

struct CtxTool {
    description: String,
    input_schema: Value,
    handler: CtxHandler,
}

/// The assembled server: registries plus handshake state plus outbox.
pub struct McpServer {
    tools: ToolRegistry,
    ctx_tools: HashMap<String, CtxTool>,
    prompts: PromptRegistry,
    resources: ResourceStore,
    logger: Logger,
    completions: HashMap<String, Box<dyn Fn(&str) -> Vec<String>>>,
    pending: Vec<RpcNotification>,
    last_list_changed: Option<String>,
    negotiated: Option<String>,
    initialized: bool,
    report_errors: bool,
    reported: Vec<String>,
    server_info: ServerInfo,
}

impl McpServer {
    /// Empty server with error reporting on.
    pub fn new(server_info: ServerInfo) -> Self {
        Self::with_report_errors(server_info, true)
    }

    /// Empty server with an explicit error-reporting flag.
    pub fn with_report_errors(server_info: ServerInfo, report_errors: bool) -> Self {
        Self {
            tools: ToolRegistry::new(),
            ctx_tools: HashMap::new(),
            prompts: PromptRegistry::new(),
            resources: ResourceStore::new(),
            logger: Logger::new(),
            completions: HashMap::new(),
            pending: Vec::new(),
            last_list_changed: None,
            negotiated: None,
            initialized: false,
            report_errors,
            reported: Vec::new(),
            server_info,
        }
    }

    /// Tool registry (plain handlers).
    pub fn tools_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tools
    }

    /// Prompt registry.
    pub fn prompts_mut(&mut self) -> &mut PromptRegistry {
        &mut self.prompts
    }

    /// Resource store.
    pub fn resources_mut(&mut self) -> &mut ResourceStore {
        &mut self.resources
    }

    /// Log router.
    pub fn logger_mut(&mut self) -> &mut Logger {
        &mut self.logger
    }

    /// Register a context-aware tool: invoked with the calling request.
    pub fn register_ctx_tool(
        &mut self,
        name: &str,
        description: &str,
        input_schema: Value,
        handler: impl Fn(&RequestContext, &Value) -> Result<Vec<Value>, String> + 'static,
    ) {
        self.ctx_tools.insert(
            name.to_owned(),
            CtxTool {
                description: description.to_owned(),
                input_schema,
                handler: Box::new(handler),
            },
        );
    }

    /// Register a completion function under a caller-chosen key
    /// (`"prompt:{name}:{arg}"` / `"resource:{template}:{param}"`).
    pub fn register_completion(
        &mut self,
        key: &str,
        complete: impl Fn(&str) -> Vec<String> + 'static,
    ) {
        self.completions.insert(key.to_owned(), Box::new(complete));
    }

    /// Negotiate the version (unknown falls back to latest) and validate
    /// the initialize params; returns the initialize result object.
    pub fn initialize(
        &mut self,
        version: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<Value, String> {
        if version.is_empty() {
            return Err("initialize requires protocolVersion".to_owned());
        }
        if client_name.is_empty() || client_version.is_empty() {
            return Err("initialize requires clientInfo name and version".to_owned());
        }
        let negotiated = if SUPPORTED_VERSIONS.contains(&version) {
            version.to_owned()
        } else {
            LATEST_VERSION.to_owned()
        };
        self.negotiated = Some(negotiated.clone());
        Ok(serde_json::json!({
            "protocolVersion": negotiated,
            "capabilities": self.advertise(),
            "serverInfo": {"name": self.server_info.name, "version": self.server_info.version},
        }))
    }

    /// Complete the handshake after `initialize`.
    pub fn notify_initialized(&mut self) {
        self.initialized = true;
    }

    /// Whether `notifications/initialized` completed the handshake.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Negotiated version, if `initialize` ran.
    pub fn negotiated_version(&self) -> Option<&str> {
        self.negotiated.as_deref()
    }

    /// Capability object: entries appear only when registered (logging is
    /// always available via the log router).
    pub fn advertise(&self) -> Map<String, Value> {
        let mut out = Map::new();
        if self.prompts.capability() {
            out.insert("prompts".to_owned(), serde_json::json!({"listChanged": true}));
        }
        if self.resources.capability() {
            out.insert(
                "resources".to_owned(),
                serde_json::json!({"listChanged": true, "subscribe": false}),
            );
        }
        if self.tools.capability() || !self.ctx_tools.is_empty() {
            out.insert("tools".to_owned(), serde_json::json!({"listChanged": true}));
        }
        out.insert("logging".to_owned(), serde_json::json!({}));
        out
    }

    /// The request identity for one caller: negotiated version, else latest.
    pub fn context(&self, client_id: u64, client_name: &str) -> RequestContext {
        RequestContext {
            client_id,
            client_name: client_name.to_owned(),
            protocol_version: self
                .negotiated
                .clone()
                .unwrap_or_else(|| LATEST_VERSION.to_owned()),
        }
    }

    /// Call a tool: unknown names are protocol errors; omitted arguments
    /// default to `{}`; handler failures ride inside the result.
    pub fn call_tool(
        &self,
        ctx: &RequestContext,
        name: &str,
        args: Option<&Value>,
    ) -> Result<ToolResult, AssemblyError> {
        let empty = Value::Object(Map::new());
        let args = args.unwrap_or(&empty);
        if let Some(tool) = self.ctx_tools.get(name) {
            validate_args(&tool.input_schema, args)
                .map_err(AssemblyError::invalid_params)?;
            return match (tool.handler)(ctx, args) {
                Ok(content) => Ok(ToolResult { content, is_error: false }),
                Err(message) => {
                    Ok(ToolResult { content: vec![content_text(message)], is_error: true })
                }
            };
        }
        self.tools.call(name, args).map_err(|error| {
            let message = match error {
                crate::mcp_handlers::ToolCallError::Protocol(message) => {
                    if message.starts_with("unknown tool: ") {
                        format!("Tool '{}' not found", name)
                    } else {
                        message
                    }
                }
            };
            AssemblyError::invalid_params(message)
        })
    }

    /// Read a resource: unknown URIs are invalid-params errors.
    pub fn find_resource(
        &self,
        _ctx: &RequestContext,
        uri: &str,
    ) -> Result<Vec<ResourceContent>, AssemblyError> {
        self.resources.read(uri).map_err(|_| {
            AssemblyError::invalid_params(format!("Resource '{uri}' not found"))
        })
    }

    /// Render a prompt with the given arguments.
    pub fn get_prompt(
        &self,
        _ctx: &RequestContext,
        name: &str,
        args: &Value,
    ) -> Result<PromptResult, AssemblyError> {
        self.prompts.get(name, args).map_err(AssemblyError::invalid_params)
    }

    /// Run a registered completion; empty when the key is unknown.
    pub fn complete(&self, _ctx: &RequestContext, key: &str, value: &str) -> Vec<String> {
        self.completions.get(key).map(|f| f(value)).unwrap_or_default()
    }

    /// Elicitation completion with no live peer is a silent no-op.
    pub fn notify_elicitation_complete(&self, _client_id: u64, _elicitation_id: &str) {}

    /// Queue a server-to-client notification (works with no transport).
    pub fn emit_notification(&mut self, method: &str, params: Value) {
        self.pending.push(RpcNotification::new(method, params));
    }

    /// Queue a `list_changed` notification, coalescing rapid repeats of one
    /// kind into a single pending notification.
    pub fn note_list_changed(&mut self, kind: &str) {
        if self.last_list_changed.as_deref() == Some(kind) {
            return;
        }
        self.last_list_changed = Some(kind.to_owned());
        self.pending.push(RpcNotification::new(
            &format!("notifications/{kind}/list_changed"),
            Value::Object(Map::new()),
        ));
    }

    /// Drain the outbox.
    pub fn drain_notifications(&mut self) -> Vec<RpcNotification> {
        self.last_list_changed = None;
        std::mem::take(&mut self.pending)
    }

    /// Build the update notification for a subscribed URI, if subscribed.
    pub fn resource_update(
        &self,
        _ctx: &RequestContext,
        uri: &str,
    ) -> Option<RpcNotification> {
        self.resources.notify_update(uri)
    }

    /// Record a handler defect server-side and return the scrubbed client
    /// message (recorded only when error reporting is on).
    pub fn record_defect(&mut self, detail: &str) -> String {
        if self.report_errors {
            self.reported.push(detail.to_owned());
        }
        internal_error_message().to_owned()
    }

    /// Server-side defect log.
    pub fn reported(&self) -> &[String] {
        &self.reported
    }

    /// Tool description, for capability listing rows.
    pub fn ctx_tool_description(&self, name: &str) -> Option<&str> {
        self.ctx_tools.get(name).map(|tool| tool.description.as_str())
    }
}
