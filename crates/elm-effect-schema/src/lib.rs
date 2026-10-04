//! Effect Schema core: decode `unknown` into typed values, encode back.
//!
//! The unknown side is JSON-shaped (`Unknown`); the typed side is plain
//! Rust. `serde_json` serves the boring byte cases; the spec is Effect
//! Schema (parsers, issues with paths, strict never-lossy decoding).

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;

pub mod transform;

/// An untyped value: the decode source and encode target.
#[derive(Debug, Clone, PartialEq)]
pub enum Unknown {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Unknown>),
    Object(BTreeMap<String, Unknown>),
}

impl Unknown {
    pub fn kind(&self) -> &'static str {
        match self {
            Unknown::Null => "null",
            Unknown::Bool(_) => "boolean",
            Unknown::Number(_) => "number",
            Unknown::Text(_) => "string",
            Unknown::Array(_) => "array",
            Unknown::Object(_) => "object",
        }
    }
}

/// One placement-aware failure: what was expected, what arrived, where.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SchemaError {
    /// A value of the wrong shape arrived.
    #[error("expected {expected} at {path}, got {got}")]
    TypeMismatch {
        path: String,
        expected: &'static str,
        got: &'static str,
    },
    /// Invalid JSON bytes or JSON that is not UTF-8 text.
    #[error("invalid JSON at {path}: {message}")]
    InvalidJson { path: String, message: String },
}

/// A two-way codec between `Unknown` and `T`.
pub struct Schema<T> {
    decode: Box<dyn Fn(Unknown) -> Result<T, SchemaError> + Send + Sync>,
    encode: Box<dyn Fn(&T) -> Unknown + Send + Sync>,
}

impl<T> Schema<T> {
    fn new(
        decode: impl Fn(Unknown) -> Result<T, SchemaError> + Send + Sync + 'static,
        encode: impl Fn(&T) -> Unknown + Send + Sync + 'static,
    ) -> Self {
        Self {
            decode: Box::new(decode),
            encode: Box::new(encode),
        }
    }

    /// Decode an unknown value, or a typed error naming the path.
    pub fn decode(&self, unknown: Unknown) -> Result<T, SchemaError> {
        (self.decode)(unknown)
    }

    /// Encode a typed value back to unknown (total: never fails).
    pub fn encode(&self, value: &T) -> Unknown {
        (self.encode)(value)
    }

    /// Decode JSON text through the unknown representation.
    pub fn decode_json(&self, json: &str) -> Result<T, SchemaError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| SchemaError::InvalidJson {
                path: "$".to_string(),
                message: error.to_string(),
            })?;
        self.decode(Unknown::from_json(value))
    }

    /// Encode to canonical JSON text (total when decode is total).
    pub fn encode_json(&self, value: &T) -> Result<String, SchemaError> {
        serde_json::to_string(&self.encode(value).to_json()).map_err(|error| {
            SchemaError::InvalidJson {
                path: "$".to_string(),
                message: error.to_string(),
            }
        })
    }

    /// Adapt a schema to a new type through a total isomorphism pair.
    pub fn convert<A, D, E>(schema: Schema<A>, decode: D, encode: E) -> Schema<T>
    where
        A: Send + Sync + 'static,
        T: Send + Sync + 'static,
        D: Fn(A) -> Result<T, SchemaError> + Send + Sync + 'static,
        E: Fn(&T) -> A + Send + Sync + 'static,
    {
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        Schema::new(
            move |unknown| decode_schema.decode(unknown).and_then(&decode),
            move |value| encode_schema.encode(&encode(value)),
        )
    }
}

fn mismatch<T>(path: &str, expected: &'static str, got: &Unknown) -> Result<T, SchemaError> {
    Err(SchemaError::TypeMismatch {
        path: path.to_string(),
        expected,
        got: got.kind(),
    })
}

impl Schema<String> {
    /// Any text.
    pub fn string() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Text(text) => Ok(text),
                other => mismatch("$", "string", &other),
            },
            |value| Unknown::Text(value.clone()),
        )
    }
}

impl Schema<f64> {
    /// Any number.
    pub fn number() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Number(number) => Ok(number),
                other => mismatch("$", "number", &other),
            },
            |value| Unknown::Number(*value),
        )
    }
}

impl Schema<bool> {
    /// Either boolean.
    pub fn boolean() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Bool(boolean) => Ok(boolean),
                other => mismatch("$", "boolean", &other),
            },
            |value| Unknown::Bool(*value),
        )
    }
}

impl Schema<BTreeMap<String, Unknown>> {
    /// Any object, fields left untyped.
    pub fn object() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Object(fields) => Ok(fields),
                other => mismatch("$", "object", &other),
            },
            |fields| Unknown::Object(fields.clone()),
        )
    }

    /// One required text field of an object.
    pub fn field(name: &str, inner: Schema<String>) -> Schema<String> {
        let decode_name = name.to_string();
        let encode_name = name.to_string();
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Object(mut fields) => match fields.remove(&decode_name) {
                    Some(value) => decode_inner.decode(value).map_err(|error| match error {
                        SchemaError::TypeMismatch { expected, got, .. } => {
                            SchemaError::TypeMismatch {
                                path: format!("$.{decode_name}"),
                                expected,
                                got,
                            }
                        }
                        other => other,
                    }),
                    None => Err(SchemaError::TypeMismatch {
                        path: format!("$.{decode_name}"),
                        expected: "required field",
                        got: "missing",
                    }),
                },
                other => mismatch("$", "object", &other),
            },
            move |value| {
                let mut fields = BTreeMap::new();
                fields.insert(encode_name.clone(), encode_inner.encode(value));
                Unknown::Object(fields)
            },
        )
    }
}

impl<T: Send + Sync + 'static> Schema<Vec<T>> {
    /// Homogeneous arrays: every element decoded, failures name `[i]`.
    pub fn array(inner: Schema<T>) -> Schema<Vec<T>> {
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Array(items) => items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| {
                        decode_inner.decode(item).map_err(|error| match error {
                            SchemaError::TypeMismatch { expected, got, .. } => {
                                SchemaError::TypeMismatch {
                                    path: format!("$[{index}]"),
                                    expected,
                                    got,
                                }
                            }
                            other => other,
                        })
                    })
                    .collect(),
                other => mismatch("$", "array", &other),
            },
            move |values: &Vec<T>| {
                Unknown::Array(
                    values
                        .iter()
                        .map(|value| encode_inner.encode(value))
                        .collect(),
                )
            },
        )
    }
}

impl<T: Send + Sync + 'static> Schema<Option<T>> {
    /// `null` (or missing, by the caller's hand) decodes to `None`.
    pub fn optional(inner: Schema<T>) -> Schema<Option<T>> {
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Null => Ok(None),
                other => decode_inner.decode(other).map(Some),
            },
            move |value| match value {
                None => Unknown::Null,
                Some(value) => encode_inner.encode(value),
            },
        )
    }
}

impl Unknown {
    fn from_json(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Unknown::Null,
            serde_json::Value::Bool(boolean) => Unknown::Bool(boolean),
            serde_json::Value::Number(number) => Unknown::Number(number.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(text) => Unknown::Text(text),
            serde_json::Value::Array(items) => {
                Unknown::Array(items.into_iter().map(Unknown::from_json).collect())
            }
            serde_json::Value::Object(fields) => Unknown::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, Unknown::from_json(value)))
                    .collect(),
            ),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Unknown::Null => serde_json::Value::Null,
            Unknown::Bool(boolean) => serde_json::Value::Bool(*boolean),
            Unknown::Number(number) => serde_json::Number::from_f64(*number)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Unknown::Text(text) => serde_json::Value::String(text.clone()),
            Unknown::Array(items) => {
                serde_json::Value::Array(items.iter().map(Unknown::to_json).collect())
            }
            Unknown::Object(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_json()))
                    .collect(),
            ),
        }
    }
}
