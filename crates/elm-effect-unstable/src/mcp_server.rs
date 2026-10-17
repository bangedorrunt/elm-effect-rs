//! MCP runtime foundation: JSON-RPC 2.0 framing over in-memory channels,
//! protocol-version negotiation, the initialize handshake, capability
//! advertisement, and ping.
//!
//! This is phase 1 of the MCP server runtime epic: transport plus handshake
//! plumbing only. There are no method handlers here; later lanes register
//! tools, prompts, resources, and the rest on top of [`InMemoryTransport`].
//!
//! Wire rules follow the pin
//! (`packages/effect/test/ai/McpServer/McpConformance/`): one compact JSON
//! object per line, batches rejected, notifications answered with silence,
//! and an unsupported `protocolVersion` negotiated down to [`LATEST_VERSION`].

use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

/// MCP protocol versions the server speaks, oldest to newest.
pub const SUPPORTED_VERSIONS: [&str; 5] =
    ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"];

/// Newest supported version; also the negotiate-down fallback.
pub const LATEST_VERSION: &str = "2026-07-28";

/// JSON-RPC 2.0 error codes used on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonRpcError(pub i32);

impl JsonRpcError {
    /// Malformed JSON.
    pub const PARSE_ERROR: Self = Self(-32700);
    /// Bad envelope: wrong version, missing method, bad id, or a batch.
    pub const INVALID_REQUEST: Self = Self(-32600);
    /// Unknown method.
    pub const METHOD_NOT_FOUND: Self = Self(-32601);
    /// Params do not match the method schema.
    pub const INVALID_PARAMS: Self = Self(-32602);
}

/// A request identifier: a string or an integer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestId {
    /// String identifier.
    Text(String),
    /// Integer identifier.
    Number(i64),
}

impl RequestId {
    /// Decode an identifier; anything else is an invalid request.
    pub fn decode(value: &Value) -> Result<Self, JsonRpcError> {
        match value {
            Value::String(text) => Ok(Self::Text(text.clone())),
            Value::Number(number) => {
                number.as_i64().map(Self::Number).ok_or(JsonRpcError::INVALID_REQUEST)
            }
            _ => Err(JsonRpcError::INVALID_REQUEST),
        }
    }
}

/// A decoded single JSON-RPC message: exactly one of result or error.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedMessage {
    /// Successful response.
    Result {
        /// Request id, `None` when the envelope carried `null`.
        id: Option<RequestId>,
        /// The `result` payload.
        result: Value,
    },
    /// Error response.
    Error {
        /// Request id, `None` for unparseable or id-less envelopes.
        id: Option<RequestId>,
        /// Numeric error code.
        code: JsonRpcError,
        /// Human-readable message.
        message: String,
    },
}

impl DecodedMessage {
    /// The response id, if any.
    pub fn id(&self) -> Option<&RequestId> {
        match self {
            Self::Result { id, .. } | Self::Error { id, .. } => id.as_ref(),
        }
    }

    /// True for result responses.
    pub fn is_result(&self) -> bool {
        matches!(self, Self::Result { .. })
    }

    /// True for error responses.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// The error code, if this is an error response.
    pub fn error_code(&self) -> Option<JsonRpcError> {
        match self {
            Self::Error { code, .. } => Some(*code),
            Self::Result { .. } => None,
        }
    }
}

/// Encode one message as a single compact JSON line (no newline).
pub fn encode_message(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

/// Decode one received line into a result or error response.
pub fn decode_message(line: &str) -> Result<DecodedMessage, String> {
    let value: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    let root = value.as_object().ok_or_else(|| "response must be an object".to_owned())?;
    let id = match root.get("id") {
        None | Some(Value::Null) => None,
        Some(raw) => RequestId::decode(raw).ok(),
    };
    if let Some(result) = root.get("result") {
        return Ok(DecodedMessage::Result { id, result: result.clone() });
    }
    if let Some(error) = root.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| "error must carry a numeric code".to_owned())?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("error")
            .to_owned();
        return Ok(DecodedMessage::Error { id, code: JsonRpcError(code as i32), message });
    }
    Err("response must carry exactly one of result or error".to_owned())
}

/// Capability flags the server advertises at initialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityFlags {
    /// Serve `tools/*`.
    pub tools: bool,
    /// Serve `prompts/*`.
    pub prompts: bool,
    /// Serve `resources/*`.
    pub resources: bool,
    /// Serve `logging/*`.
    pub logging: bool,
    /// Sampling flag, kept for later lanes to interpret per version.
    pub sampling: bool,
}

/// Server capabilities: the set of enabled [`CapabilityFlags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerCapabilities {
    /// Enabled capability flags.
    pub flags: CapabilityFlags,
}

impl ServerCapabilities {
    /// No capabilities; enough for framing/handshake/ping witnesses.
    pub fn minimal() -> Self {
        Self {
            flags: CapabilityFlags {
                tools: false,
                prompts: false,
                resources: false,
                logging: false,
                sampling: false,
            },
        }
    }

    /// The `capabilities` object sent in the initialize result: only
    /// registered entries appear, with the pin's shapes.
    pub fn advertise(&self) -> Map<String, Value> {
        let mut out = Map::new();
        if self.flags.prompts {
            out.insert("prompts".to_owned(), serde_json::json!({"listChanged": true}));
        }
        if self.flags.resources {
            out.insert(
                "resources".to_owned(),
                serde_json::json!({"listChanged": true, "subscribe": false}),
            );
        }
        if self.flags.tools {
            out.insert("tools".to_owned(), serde_json::json!({"listChanged": true}));
        }
        if self.flags.logging {
            out.insert("logging".to_owned(), serde_json::json!({}));
        }
        if self.flags.sampling {
            out.insert("sampling".to_owned(), serde_json::json!({}));
        }
        out
    }
}

/// Server implementation info returned by initialize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,
    /// Server version.
    pub version: String,
}

impl ServerInfo {
    /// Name and version for the initialize result.
    pub fn new(name: &str, version: &str) -> Self {
        Self { name: name.to_owned(), version: version.to_owned() }
    }

    /// Placeholder identity for transport-only witnesses.
    pub fn test() -> Self {
        Self::new("elm-effect-test-server", "0.0.0")
    }
}

/// Client implementation info required by initialize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// Client name.
    pub name: String,
    /// Client version.
    pub version: String,
}

impl ClientInfo {
    /// Client name and version.
    pub fn new(name: &str, version: &str) -> Self {
        Self { name: name.to_owned(), version: version.to_owned() }
    }
}

fn error_reply(id: &Value, code: JsonRpcError, message: &str) -> Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code.0, "message": message}})
}

fn result_reply(id: &Value, result: Value) -> Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Handshake state for one server connection.
#[derive(Debug)]
struct Handshake {
    capabilities: ServerCapabilities,
    server_info: ServerInfo,
    negotiated: Option<String>,
    initialized: bool,
}

impl Handshake {
    fn handle(&mut self, line: &str) -> Option<String> {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                return Some(encode_message(&error_reply(
                    &Value::Null,
                    JsonRpcError::PARSE_ERROR,
                    "malformed JSON",
                )));
            }
        };
        if value.is_array() {
            return Some(encode_message(&error_reply(
                &Value::Null,
                JsonRpcError::INVALID_REQUEST,
                "JSON-RPC batches are not supported",
            )));
        }
        let root = match value.as_object() {
            Some(root) => root,
            None => {
                return Some(encode_message(&error_reply(
                    &Value::Null,
                    JsonRpcError::INVALID_REQUEST,
                    "message must be an object",
                )));
            }
        };
        if root.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            let id = root.get("id").cloned().unwrap_or(Value::Null);
            let id = RequestId::decode(&id).ok();
            return Some(encode_message(&error_reply(
                &id.map(|id| match id {
                    RequestId::Text(text) => Value::String(text),
                    RequestId::Number(number) => Value::Number(number.into()),
                })
                .unwrap_or(Value::Null),
                JsonRpcError::INVALID_REQUEST,
                "jsonrpc must be 2.0",
            )));
        }
        let raw_id = root.get("id").cloned().unwrap_or(Value::Null);
        let is_notification = root.get("id").is_none();
        let id = if is_notification {
            None
        } else {
            match RequestId::decode(&raw_id) {
                Ok(id) => Some(id),
                Err(code) => {
                    return Some(encode_message(&error_reply(
                        &Value::Null,
                        code,
                        "identifier must be a string or integer",
                    )));
                }
            }
        };
        let wire_id = |id: &Option<RequestId>| match id {
            Some(RequestId::Text(text)) => Value::String(text.clone()),
            Some(RequestId::Number(number)) => Value::Number((*number).into()),
            None => Value::Null,
        };
        let method = match root.get("method").and_then(Value::as_str) {
            Some(method) => method,
            None => {
                return Some(encode_message(&error_reply(
                    &wire_id(&id),
                    JsonRpcError::INVALID_REQUEST,
                    "message must carry a method",
                )));
            }
        };
        let params = root.get("params").cloned().unwrap_or(Value::Null);
        let params_valid = params.is_null()
            || params.as_object().map(|object| {
                method == "initialize" || object.is_empty() || method == "ping"
            })
            .unwrap_or(false)
            || (params.is_array() && method != "initialize" && method != "ping");
        if !params_valid {
            if is_notification {
                return None;
            }
            return Some(encode_message(&error_reply(
                &wire_id(&id),
                JsonRpcError::INVALID_PARAMS,
                "params do not match the method schema",
            )));
        }

        match method {
            "initialize" => {
                let result = self.initialize(&params);
                Some(encode_message(&match result {
                    Ok(result) => result_reply(&wire_id(&id), result),
                    Err(message) => error_reply(&wire_id(&id), JsonRpcError::INVALID_PARAMS, &message),
                }))
            }
            "notifications/initialized" => {
                if self.negotiated.is_none() {
                    return Some(encode_message(&error_reply(
                        &wire_id(&id),
                        JsonRpcError::INVALID_REQUEST,
                        "initialized before initialize",
                    )));
                }
                self.initialized = true;
                None
            }
            "ping" => {
                if !params.is_null() && params.as_object().is_none() {
                    if is_notification {
                        return None;
                    }
                    return Some(encode_message(&error_reply(
                        &wire_id(&id),
                        JsonRpcError::INVALID_PARAMS,
                        "params do not match the method schema",
                    )));
                }
                if is_notification {
                    return None;
                }
                Some(encode_message(&result_reply(&wire_id(&id), serde_json::json!({}))))
            }
            _ => {
                if is_notification {
                    return None;
                }
                if self.negotiated.is_none() {
                    return Some(encode_message(&error_reply(
                        &wire_id(&id),
                        JsonRpcError::INVALID_REQUEST,
                        "server is not initialized",
                    )));
                }
                Some(encode_message(&error_reply(
                    &wire_id(&id),
                    JsonRpcError::METHOD_NOT_FOUND,
                    "unknown method",
                )))
            }
        }
    }

    fn initialize(&mut self, params: &Value) -> Result<Value, String> {
        let params = params
            .as_object()
            .ok_or_else(|| "initialize params must be an object".to_owned())?;
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| "initialize requires protocolVersion".to_owned())?;
        let capabilities = params
            .get("capabilities")
            .and_then(Value::as_object)
            .ok_or_else(|| "initialize requires capabilities".to_owned())?;
        let _ = capabilities;
        let client = params
            .get("clientInfo")
            .and_then(Value::as_object)
            .ok_or_else(|| "initialize requires clientInfo".to_owned())?;
        if client.get("name").and_then(Value::as_str).is_none()
            || client.get("version").and_then(Value::as_str).is_none()
        {
            return Err("initialize requires clientInfo name and version".to_owned());
        }
        let negotiated = if SUPPORTED_VERSIONS.contains(&requested) {
            requested.to_owned()
        } else {
            LATEST_VERSION.to_owned()
        };
        self.negotiated = Some(negotiated.clone());
        Ok(serde_json::json!({
            "protocolVersion": negotiated,
            "capabilities": self.capabilities.advertise(),
            "serverInfo": {"name": self.server_info.name, "version": self.server_info.version},
        }))
    }
}

struct Core {
    handshake: Mutex<Handshake>,
    queue: Mutex<VecDeque<String>>,
    inbound: Mutex<Vec<u8>>,
    closed: AtomicBool,
}

/// In-memory JSON-RPC channel: the local transport plus handshake state.
///
/// One handle drives the server directly with [`InMemoryTransport::handle_line`];
/// a [`InMemoryTransport::pair`] shares one server between two handles so
/// tests can speak the line protocol (`send_line`, `send_chunk`, `take_line`)
/// exactly like the pin's stdio harness, minus the process boundary.
#[derive(Clone)]
pub struct InMemoryTransport {
    core: Arc<Core>,
}

impl InMemoryTransport {
    fn build(capabilities: ServerCapabilities, server_info: ServerInfo) -> Self {
        Self {
            core: Arc::new(Core {
                handshake: Mutex::new(Handshake {
                    capabilities,
                    server_info,
                    negotiated: None,
                    initialized: false,
                }),
                queue: Mutex::new(VecDeque::new()),
                inbound: Mutex::new(Vec::new()),
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// A fresh uninitialized server with the given capabilities.
    pub fn new(capabilities: ServerCapabilities, server_info: ServerInfo) -> Self {
        Self::build(capabilities, server_info)
    }

    /// A server that already completed the handshake (framing witnesses).
    pub fn initialized(server_info: ServerInfo) -> Self {
        let transport =
            Self::build(ServerCapabilities::minimal(), server_info);
        let mut handshake = transport.core.handshake.lock().unwrap();
        handshake.negotiated = Some(LATEST_VERSION.to_owned());
        handshake.initialized = true;
        drop(handshake);
        transport
    }

    /// Two handles sharing one server: the pin's local-transport equivalent.
    pub fn pair(server_info: ServerInfo) -> (Self, Self) {
        let transport = Self::build(ServerCapabilities::minimal(), server_info);
        (transport.clone(), transport)
    }

    /// Drive one line through the server; `None` means no reply
    /// (a notification, or the closing `initialized` handshake line).
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line.is_empty() {
            return None;
        }
        self.core.handshake.lock().unwrap().handle(line)
    }

    /// Send one protocol line; any reply is queued for [`InMemoryTransport::take_line`].
    pub fn send_line(&self, line: &str) -> Result<(), String> {
        if self.core.closed.load(Ordering::SeqCst) {
            return Err("transport is closed".to_owned());
        }
        let line = line.strip_suffix('\n').unwrap_or(line);
        if let Some(reply) = self.core.handshake.lock().unwrap().handle(line) {
            self.core.queue.lock().unwrap().push_back(reply);
        }
        Ok(())
    }

    /// Feed raw bytes; complete newline-delimited lines are processed,
    /// so a UTF-8 record split across chunks still reassembles.
    pub fn send_chunk(&self, chunk: &[u8]) -> Result<(), String> {
        if self.core.closed.load(Ordering::SeqCst) {
            return Err("transport is closed".to_owned());
        }
        let mut inbound = self.core.inbound.lock().unwrap();
        inbound.extend_from_slice(chunk);
        let mut replies = Vec::new();
        while let Some(end) = inbound.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = inbound.drain(..=end).collect();
            let text = String::from_utf8(line[..line.len() - 1].to_vec())
                .map_err(|_| "input is not valid UTF-8".to_owned())?;
            if text.is_empty() {
                continue;
            }
            if let Some(reply) = self.core.handshake.lock().unwrap().handle(&text) {
                replies.push(reply);
            }
        }
        if !replies.is_empty() {
            self.core.queue.lock().unwrap().extend(replies);
        }
        Ok(())
    }

    /// Take the next queued reply line, or `None` when empty or closed.
    pub fn take_line(&self) -> Option<String> {
        self.core.queue.lock().unwrap().pop_front()
    }

    /// Close the client end; queued replies drain, then reads return `None`.
    pub fn close(&self) {
        self.core.closed.store(true, Ordering::SeqCst);
    }

    /// True once `notifications/initialized` completed the handshake.
    pub fn is_initialized(&self) -> bool {
        self.core.handshake.lock().unwrap().initialized
    }
}
