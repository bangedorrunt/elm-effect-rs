//! MCP wire schemas behind the `McpSchema` rows.
//!
//! Ports the decode/encode core of `packages/effect/src/ai/McpSchema.ts`
//! for request metadata, the sampling `CreateMessage` payload, and the
//! November elicitation enum forms: unknown JSON in, typed value out, and
//! the same JSON back, so custom metadata and every enum shape survive.

use serde_json::{Map, Value};

/// A progress token: an opaque string or number (`ProgressToken`).
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressToken {
    /// String token.
    Text(String),
    /// Numeric token, always finite (`Schema.Finite`); the original JSON
    /// number is kept so encode preserves `1` versus `1.0`.
    Num(serde_json::Number),
}

/// Optional MCP request metadata (`RequestMeta`): `_meta` carries an
/// optional progress token plus preserved extension fields.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestMeta {
    progress_token: Option<ProgressToken>,
    rest: Map<String, Value>,
    present: bool,
}

impl RequestMeta {
    /// Decode `_meta` with its progress token and extension fields.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let root = value
            .as_object()
            .ok_or_else(|| "expected an object".to_owned())?;
        let Some(meta) = root.get("_meta") else {
            return Ok(Self {
                progress_token: None,
                rest: Map::new(),
                present: false,
            });
        };
        let meta = meta
            .as_object()
            .ok_or_else(|| "_meta must be an object".to_owned())?;
        let mut rest = meta.clone();
        let progress_token = match rest.remove("progressToken") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(ProgressToken::Text(text)),
            Some(Value::Number(number)) => {
                if !number.as_f64().is_some_and(f64::is_finite) {
                    return Err("progressToken must be a string or a finite number".to_owned());
                }
                Some(ProgressToken::Num(number.clone()))
            }
            Some(_) => {
                return Err("progressToken must be a string or a finite number".to_owned());
            }
        };
        Ok(Self {
            progress_token,
            rest,
            present: true,
        })
    }

    /// Encode back to the wire form, preserving extension fields.
    pub fn encode(&self) -> Value {
        if !self.present {
            return Value::Object(Map::new());
        }
        let mut meta = self.rest.clone();
        match &self.progress_token {
            Some(ProgressToken::Text(text)) => {
                meta.insert("progressToken".to_owned(), Value::String(text.clone()));
            }
            Some(ProgressToken::Num(number)) => {
                meta.insert("progressToken".to_owned(), Value::Number(number.clone()));
            }
            None => {}
        }
        Value::Object(
            [("_meta".to_owned(), Value::Object(meta))]
                .into_iter()
                .collect(),
        )
    }
}

/// Sampling `CreateMessage` payload: required messages and token budget,
/// optional metadata that must be an object when present.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateMessagePayload {
    /// The conversation messages to sample from.
    pub messages: Vec<Value>,
    /// Maximum tokens to sample.
    pub max_tokens: i64,
    /// Optional provider metadata.
    pub metadata: Option<Map<String, Value>>,
}

impl CreateMessagePayload {
    /// Decode the payload, rejecting non-object metadata.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let root = value
            .as_object()
            .ok_or_else(|| "expected an object".to_owned())?;
        let messages = root
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| "messages must be an array".to_owned())?
            .clone();
        let max_tokens = root
            .get("maxTokens")
            .and_then(Value::as_i64)
            .ok_or_else(|| "maxTokens must be an integer".to_owned())?;
        let metadata = match root.get("metadata") {
            None | Some(Value::Null) => None,
            Some(Value::Object(meta)) => Some(meta.clone()),
            Some(_) => return Err("metadata must be an object".to_owned()),
        };
        Ok(Self {
            messages,
            max_tokens,
            metadata,
        })
    }
}

/// One elicitation form field, classified by its enum shape (untitled,
/// titled `oneOf`, or legacy `enumNames`); the original JSON is kept so
/// decode preserves every November form byte-identically.
#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationField {
    /// Plain `{ type: "string", enum: [...] }`.
    Untitled(Value),
    /// `{ type: "string", oneOf: [{ const, title }] }`.
    Titled(Value),
    /// `{ type: "string", enum: [...], enumNames: [...] }`.
    Legacy(Value),
    /// Any other primitive field (`ElicitationString`/`Number`/`Boolean`).
    Other(Value),
}

impl ElicitationField {
    fn classify(value: &Value) -> Result<Self, String> {
        let field = value
            .as_object()
            .ok_or_else(|| "field must be an object".to_owned())?;
        if field.get("oneOf").is_some_and(|one| one.is_array()) {
            for entry in field["oneOf"].as_array().unwrap_or(&vec![]) {
                let entry = entry
                    .as_object()
                    .ok_or_else(|| "oneOf entries must be objects".to_owned())?;
                if !entry.get("const").is_some_and(Value::is_string)
                    || !entry.get("title").is_some_and(Value::is_string)
                {
                    return Err("oneOf entries need a string const and title".to_owned());
                }
            }
            return Ok(Self::Titled(value.clone()));
        }
        if let Some(names) = field.get("enumNames") {
            if !names.is_array() {
                return Err("enumNames must be an array".to_owned());
            }
            return Ok(Self::Legacy(value.clone()));
        }
        if field.get("enum").is_some() {
            return Ok(Self::Untitled(value.clone()));
        }
        Ok(Self::Other(value.clone()))
    }

    /// The original field JSON.
    pub fn original(&self) -> &Value {
        match self {
            Self::Untitled(value)
            | Self::Titled(value)
            | Self::Legacy(value)
            | Self::Other(value) => value,
        }
    }
}

/// A form-mode elicitation request (`ElicitRequestFormParams`).
#[derive(Debug, Clone, PartialEq)]
pub struct ElicitFormRequest {
    /// `mode` is optional and always `"form"` when present.
    pub mode: Option<String>,
    /// The prompt shown to the user.
    pub message: String,
    /// Classified fields, kept in request order.
    pub fields: Vec<(String, ElicitationField)>,
    /// The properties exactly as received, for byte-identical roundtrips.
    pub properties: Map<String, Value>,
}

impl ElicitFormRequest {
    /// Decode a form request, classifying every property's enum shape.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let root = value
            .as_object()
            .ok_or_else(|| "expected an object".to_owned())?;
        if let Some(mode) = root.get("mode") {
            if mode != "form" {
                return Err("mode must be \"form\"".to_owned());
            }
        }
        let message = root
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| "message must be a string".to_owned())?
            .to_owned();
        let schema = root
            .get("requestedSchema")
            .and_then(Value::as_object)
            .ok_or_else(|| "requestedSchema must be an object".to_owned())?;
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| "requestedSchema.properties must be an object".to_owned())?
            .clone();
        let mut fields = Vec::with_capacity(properties.len());
        for (name, field) in &properties {
            fields.push((name.clone(), ElicitationField::classify(field)?));
        }
        Ok(Self {
            mode: root.get("mode").and_then(Value::as_str).map(str::to_owned),
            message,
            fields,
            properties,
        })
    }
}
