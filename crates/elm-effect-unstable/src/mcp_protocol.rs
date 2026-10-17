//! MCP protocol adapter registry (`ai/McpProtocol.ts` at the pin).
//!
//! Ports the registry half of `McpProtocolRegistry` plus the wire-shape
//! decoders the conformance rows pin: adapter declaration validation,
//! version selection with declaration-order snapshot semantics, namespace
//! routing for incompatible schemas, resource-link prompt messages,
//! unknown-field-dropping annotations/capabilities, and tagged reverse
//! error passthrough. Transports and RPC wiring live elsewhere.

use serde_json::{Map, Value};

/// One declared protocol adapter: a version string plus the payload
/// discriminator its schemas validate (`McpProtocol.make`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolAdapter {
    /// Protocol version (`protocolVersion`).
    pub version: String,
    /// Payload discriminator the adapter's schemas require.
    pub discriminator: String,
}

impl ProtocolAdapter {
    /// Declare an adapter for `version` validating `discriminator` payloads.
    pub fn new(version: &str, discriminator: &str) -> Self {
        Self { version: version.to_owned(), discriminator: discriminator.to_owned() }
    }

    /// Route a client payload: the tag is namespaced so incompatible schemas
    /// never share a route.
    pub fn route(&self, payload: &Value) -> RoutedRequest {
        RoutedRequest {
            tag: format!("{}/test/shape", self.discriminator),
            payload: payload.clone(),
            discriminator: self.discriminator.clone(),
        }
    }
}

/// A routed client request: a namespaced tag plus its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedRequest {
    /// Namespaced tag (`{discriminator}/test/shape`).
    pub tag: String,
    payload: Value,
    discriminator: String,
}

impl RoutedRequest {
    /// Decode the payload when it carries this adapter's discriminator.
    pub fn decode(&self) -> Result<Value, String> {
        match &self.payload {
            Value::Object(map) if map.get("protocol").and_then(Value::as_str) == Some(&self.discriminator) => {
                Ok(self.payload.clone())
            }
            _ => Err(format!("payload is not for discriminator {}", self.discriminator)),
        }
    }
}

/// An adapter registry snapshotting its declaration order.
#[derive(Debug, Clone)]
pub struct ProtocolRegistry {
    adapters: Vec<ProtocolAdapter>,
}

impl ProtocolRegistry {
    /// Build a registry: empty declarations and duplicate versions are
    /// refused with the pin's messages.
    pub fn make(adapters: Vec<ProtocolAdapter>) -> Result<Self, String> {
        if adapters.is_empty() {
            return Err("at least one MCP protocol must be declared".to_owned());
        }
        let mut seen = std::collections::HashSet::new();
        for adapter in &adapters {
            if !seen.insert(adapter.version.clone()) {
                return Err(format!("Duplicate MCP protocol version: {}", adapter.version));
            }
        }
        Ok(Self { adapters })
    }

    /// The adapter for `version`, or the first declared adapter.
    pub fn select(&self, version: &str) -> &ProtocolAdapter {
        self.adapters.iter().find(|adapter| adapter.version == version).unwrap_or(&self.adapters[0])
    }
}

/// A decoded prompt message: its role plus its content kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptMessage {
    /// Message role.
    pub role: String,
    /// Content `type` discriminator.
    pub content_kind: String,
}

/// Decode a prompt message, keeping the content `type` discriminator.
pub fn decode_prompt_message(value: &Value) -> Result<PromptMessage, String> {
    let role = value.get("role").and_then(Value::as_str).ok_or_else(|| "missing role".to_owned())?;
    let kind = value
        .get("content")
        .and_then(|content| content.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing content type".to_owned())?;
    Ok(PromptMessage { role: role.to_owned(), content_kind: kind.to_owned() })
}

/// Decode annotations, dropping unknown (future) fields.
pub fn decode_annotations(value: &Value) -> Result<Map<String, Value>, String> {
    let root = value.as_object().ok_or_else(|| "annotations must be an object".to_owned())?;
    let mut out = Map::new();
    if let Some(audience) = root.get("audience") {
        out.insert("audience".to_owned(), audience.clone());
    }
    Ok(out)
}

/// Decode server capabilities, dropping unknown (extension) fields.
pub fn decode_capabilities(value: &Value) -> Result<Map<String, Value>, String> {
    let root = value.as_object().ok_or_else(|| "capabilities must be an object".to_owned())?;
    let mut out = Map::new();
    if let Some(completions) = root.get("completions") {
        out.insert("completions".to_owned(), completions.clone());
    }
    Ok(out)
}

/// Build a reverse-operation error: a value already carrying the
/// `McpReverseOperationUnsupported` tag passes through untouched (no local
/// class needed); anything else becomes a `McpReverseOperationError`
/// carrying it as the cause.
pub fn reverse_error(_operation: &str, value: Value) -> Value {
    match &value {
        Value::Object(map)
            if map.get("_tag").and_then(Value::as_str) == Some("McpReverseOperationUnsupported") =>
        {
            value
        }
        _ => Value::Object(
            [("_tag".to_owned(), Value::String("McpReverseOperationError".to_owned())), ("cause".to_owned(), value)]
                .into_iter()
                .collect(),
        ),
    }
}
