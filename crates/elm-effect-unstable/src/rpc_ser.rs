//! RPC serialization slice: NDJSON framing, JSON-RPC messages, causes.
//!
//! Pure data, no transport: [`NdjsonDecoder`] splits byte chunks into JSON
//! frames, [`RpcMessage`] encodes/decodes JSON-RPC shapes, and [`RpcCause`]
//! roundtrips failures. Binary fingerprint framing and codec holes belong to
//! later slices.

use serde_json::Value;

// NDJSON

/// NDJSON framing errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FramerError {
    /// Buffered bytes passed the cap with no newline in sight.
    BufferFull,
}

/// A streaming NDJSON decoder: feed arbitrary byte chunks, pull complete
/// JSON frames. Incomplete tails wait for more bytes. Multibyte characters
/// split across chunks reassemble (frames validate as UTF-8 only when
/// complete). Lines that are not JSON are skipped, decoding continues.
#[derive(Debug, Default)]
pub struct NdjsonDecoder {
    buf: Vec<u8>,
    /// Cap on buffered-but-incomplete bytes; `None` is unbounded.
    pub max_buffer_size: Option<usize>,
}

impl NdjsonDecoder {
    /// Empty decoder with no cap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cap the incomplete buffer at `max` bytes.
    pub fn with_max_buffer_size(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_buffer_size: Some(max),
        }
    }

    /// Push a chunk, returning the newly completed frames.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, FramerError> {
        self.buf.extend_from_slice(chunk);
        if let Some(max) = self.max_buffer_size {
            if self.buf.len() > max && !self.buf.contains(&b'\n') {
                return Err(FramerError::BufferFull);
            }
        }
        let mut frames = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = &line[..line.len() - 1];
            if line.is_empty() {
                continue;
            }
            // Only complete lines face UTF-8/JSON validation, so multibyte
            // characters split across chunks reassemble first.
            let Ok(text) = std::str::from_utf8(line) else {
                continue;
            };
            if let Ok(value) = serde_json::from_str::<Value>(text) {
                frames.push(value);
            }
        }
        Ok(frames)
    }
}

// JSON-RPC messages

/// One JSON-RPC message.
#[derive(Clone, Debug, PartialEq)]
pub enum RpcMessage {
    /// A call with an id (string ids survive, including `""`).
    Request {
        id: String,
        method: String,
        params: Value,
    },
    /// A call without an id.
    Notification { method: String, params: Value },
    /// A success for `id`.
    Response { id: String, result: Value },
    /// A typed failure for `id`. A missing (`null`) id decodes as an
    /// interrupt exit with no fiber.
    Failure {
        id: Option<String>,
        code: i64,
        message: String,
    },
}

impl RpcMessage {
    /// Decode one value. Non-objects are skipped by the caller (batch
    /// rule); this returns `None` for them without losing the rest.
    pub fn decode(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let id = match obj.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.clone()),
            Some(Value::Number(id)) => Some(id.to_string()),
            _ => None,
        };
        if let Some(result) = obj.get("result") {
            return Some(RpcMessage::Response {
                id: id.unwrap_or_default(),
                result: result.clone(),
            });
        }
        if let Some(error) = obj.get("error").and_then(Value::as_object) {
            return Some(RpcMessage::Failure {
                id,
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        let method = obj.get("method")?.as_str()?.to_string();
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        match id {
            Some(id) => Some(RpcMessage::Request { id, method, params }),
            None => Some(RpcMessage::Notification { method, params }),
        }
    }

    /// Encode one message. Notifications carry no id.
    pub fn encode(&self) -> Value {
        match self {
            RpcMessage::Request { id, method, params } => serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params,
            }),
            RpcMessage::Notification { method, params } => serde_json::json!({
                "jsonrpc": "2.0", "method": method, "params": params,
            }),
            RpcMessage::Response { id, result } => serde_json::json!({
                "jsonrpc": "2.0", "id": id, "result": result,
            }),
            RpcMessage::Failure { id, code, message } => {
                let error = serde_json::json!({"code": code, "message": message});
                let mut out = serde_json::json!({"jsonrpc": "2.0", "error": error});
                match id {
                    Some(id) => out["id"] = Value::String(id.clone()),
                    None => out["id"] = Value::Null,
                }
                out
            }
        }
    }

    /// Decode a batch: non-objects are skipped without losing requests.
    pub fn decode_batch(values: &[Value]) -> Vec<Self> {
        values.iter().filter_map(Self::decode).collect()
    }

    /// Encode responses: a single response is an object, batches are arrays.
    pub fn encode_responses(responses: &[Value]) -> Value {
        if responses.len() == 1 {
            responses[0].clone()
        } else {
            Value::Array(responses.to_vec())
        }
    }
}

// Causes

/// A serializable cause: typed failure, defect, interruption, or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcCause {
    /// An expected, typed failure.
    Failure(String),
    /// An unexpected defect.
    Defect(String),
    /// An interruption.
    Interrupt,
    /// No cause.
    Empty,
}

impl RpcCause {
    /// Encode (every variant survives, including empty).
    pub fn encode(&self) -> Value {
        match self {
            RpcCause::Failure(message) => serde_json::json!({"_tag": "Failure", "message": message}),
            RpcCause::Defect(message) => serde_json::json!({"_tag": "Defect", "message": message}),
            RpcCause::Interrupt => serde_json::json!({"_tag": "Interrupt"}),
            RpcCause::Empty => serde_json::json!({"_tag": "Empty"}),
        }
    }

    /// Decode.
    pub fn decode(value: &Value) -> Option<Self> {
        let tag = value.get("_tag")?.as_str()?;
        match tag {
            "Failure" => Some(RpcCause::Failure(
                value.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            )),
            "Defect" => Some(RpcCause::Defect(
                value.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            )),
            "Interrupt" => Some(RpcCause::Interrupt),
            "Empty" => Some(RpcCause::Empty),
            _ => None,
        }
    }
}
