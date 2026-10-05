//! Effect Schema core: decode `unknown` into typed values, encode back.
//!
//! The unknown side is JSON-shaped (`Unknown`); the typed side is plain
//! Rust. `serde_json` serves the boring byte cases; the spec is Effect
//! Schema (parsers, issues with paths, strict never-lossy decoding).

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;

pub mod transform;

pub mod tagged;

pub use tagged::{Tagged, TaggedError, catch_tag};

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
    kind: SchemaKind,
}

/// The wire shape of a schema: drives JSON Schema documents.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaKind {
    String,
    Number,
    Boolean,
    Object,
    Array(Box<SchemaKind>),
    Optional(Box<SchemaKind>),
    /// A combinator whose wire shape is not introspectable.
    Opaque(&'static str),
    /// Title/description wrapped around another shape.
    Annotated {
        title: Option<String>,
        description: Option<String>,
        inner: Box<SchemaKind>,
    },
}

impl SchemaKind {
    fn to_json_schema(&self) -> serde_json::Value {
        match self {
            SchemaKind::String => serde_json::json!({"type": "string"}),
            SchemaKind::Number => serde_json::json!({"type": "number"}),
            SchemaKind::Boolean => serde_json::json!({"type": "boolean"}),
            SchemaKind::Object => serde_json::json!({"type": "object"}),
            SchemaKind::Array(items) => {
                serde_json::json!({"type": "array", "items": items.to_json_schema()})
            }
            SchemaKind::Optional(inner) => {
                serde_json::json!({"anyOf": [inner.to_json_schema(), {"type": "null"}]})
            }
            SchemaKind::Opaque(name) => serde_json::json!({"$comment": name}),
            SchemaKind::Annotated {
                title,
                description,
                inner,
            } => {
                let mut doc = inner.to_json_schema();
                if let serde_json::Value::Object(ref mut map) = doc {
                    if let Some(title) = title {
                        map.insert(
                            "title".to_string(),
                            serde_json::Value::String(title.clone()),
                        );
                    }
                    if let Some(description) = description {
                        map.insert(
                            "description".to_string(),
                            serde_json::Value::String(description.clone()),
                        );
                    }
                }
                doc
            }
        }
    }
}

impl<T> Schema<T> {
    pub(crate) fn new(
        decode: impl Fn(Unknown) -> Result<T, SchemaError> + Send + Sync + 'static,
        encode: impl Fn(&T) -> Unknown + Send + Sync + 'static,
        kind: SchemaKind,
    ) -> Self {
        Self {
            decode: Box::new(decode),
            encode: Box::new(encode),
            kind,
        }
    }

    /// The JSON Schema document describing this schema's wire shape.
    pub fn json_schema(&self) -> serde_json::Value {
        self.kind.to_json_schema()
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

    /// Wrap a schema with a title and/or description (`annotation`
    /// shapes): the wire shape is unchanged; the JSON Schema document
    /// carries the metadata.
    pub fn annotated(schema: Schema<T>, title: Option<&str>, description: Option<&str>) -> Schema<T>
    where
        T: Send + Sync + 'static,
    {
        let kind = SchemaKind::Annotated {
            title: title.map(str::to_string),
            description: description.map(str::to_string),
            inner: Box::new(schema.kind),
        };
        Schema {
            decode: schema.decode,
            encode: schema.encode,
            kind,
        }
    }

    /// Re-type a schema to `Unknown` through itself: decode validates
    /// and normalizes, then the value returns to unknown form. This is
    /// how heterogeneous struct fields share one `Field` type.
    pub fn as_unknown(schema: Schema<T>) -> Schema<Unknown>
    where
        T: Send + Sync + 'static,
    {
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        Schema::new(
            move |unknown| {
                decode_schema
                    .decode(unknown)
                    .map(|value| encode_schema.encode(&value))
            },
            |value| value.clone(),
            SchemaKind::Opaque("unknown"),
        )
    }

    /// Fill a missing input with a default before decoding
    /// (`withDecodingDefault` shape): `None` input decodes the default
    /// through the schema; encoding passes values through unchanged.
    pub fn with_decoding_default<U: Clone + Send + Sync + 'static>(
        schema: Schema<U>,
        default: impl Fn() -> U + Send + Sync + 'static,
    ) -> Schema<U> {
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Null => Ok(default()),
                value => decode_schema.decode(value),
            },
            move |value| schema.encode(value),
            SchemaKind::Opaque("withDecodingDefault"),
        )
    }

    /// A lazily-built schema for recursive shapes (`suspend` shape): the
    /// factory runs once per decode/encode call.
    pub fn suspend<U: Clone + Send + Sync + 'static>(
        factory: impl Fn() -> Schema<U> + Send + Sync + 'static,
    ) -> Schema<U> {
        let factory = Arc::new(factory);
        let decode_factory = factory.clone();
        let encode_factory = factory.clone();
        Schema::new(
            move |unknown| decode_factory().decode(unknown),
            move |value| encode_factory().encode(value),
            SchemaKind::Opaque("suspend"),
        )
    }

    /// Check a decoded value with a predicate (`refinement` shape): a
    /// failing predicate is a typed mismatch naming the check.
    pub fn refine(
        self,
        check: &'static str,
        predicate: impl Fn(&T) -> bool + Send + Sync + 'static,
    ) -> Self
    where
        T: Send + Sync + 'static,
    {
        let inner = Arc::new(self);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| {
                let value = decode_inner.decode(unknown)?;
                if predicate(&value) {
                    Ok(value)
                } else {
                    Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: check,
                        got: "a value failing the check",
                    })
                }
            },
            move |value| encode_inner.encode(value),
            SchemaKind::Opaque("refinement"),
        )
    }

    /// Adapt a schema to a new type through a total isomorphism pair.
    pub fn convert<A, D, E>(schema: Schema<A>, decode: D, encode: E) -> Schema<T>
    where
        A: Send + Sync + 'static,
        T: Send + Sync + 'static,
        D: Fn(A) -> Result<T, SchemaError> + Send + Sync + 'static,
        E: Fn(&T) -> A + Send + Sync + 'static,
    {
        let kind = schema.kind.clone();
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        Schema::new(
            move |unknown| decode_schema.decode(unknown).and_then(&decode),
            move |value| encode_schema.encode(&encode(value)),
            kind,
        )
    }
}

impl SchemaError {
    /// Re-root an error path under a prefix (`$[0]` + `[1]` = `$[0][1]`).
    fn at(self, prefix: &str) -> Self {
        match self {
            SchemaError::TypeMismatch {
                path,
                expected,
                got,
            } => {
                let path = match path.strip_prefix('$') {
                    Some(rest) => format!("{prefix}{rest}"),
                    None => format!("{prefix}{path}"),
                };
                SchemaError::TypeMismatch {
                    path,
                    expected,
                    got,
                }
            }
            other => other,
        }
    }
}

fn mismatch<T>(path: &str, expected: &'static str, got: &Unknown) -> Result<T, SchemaError> {
    Err(SchemaError::TypeMismatch {
        path: path.to_string(),
        expected,
        got: got.kind(),
    })
}

impl Schema<Unknown> {
    /// Any value at all (`Any` / `Unknown` shapes): decode always succeeds.
    pub fn any() -> Self {
        Self::new(Ok, |value| value.clone(), SchemaKind::Opaque("any"))
    }

    /// Anything decodes (`Unknown` shape): identity, like `any`.
    pub fn unknown() -> Self {
        Self::new(Ok, |value| value.clone(), SchemaKind::Opaque("unknown"))
    }

    /// No value decodes (`Never` shape): decode always fails.
    pub fn never() -> Self {
        Self::new(
            |unknown| mismatch("$", "never", &unknown),
            |value| value.clone(),
            SchemaKind::Opaque("never"),
        )
    }

    /// One exact value (`Literal` shape): equality decides.
    pub fn literal(expected: Unknown) -> Self {
        Self::new(
            move |unknown| {
                if unknown == expected {
                    Ok(unknown)
                } else {
                    mismatch("$", "that literal", &unknown)
                }
            },
            |value| value.clone(),
            SchemaKind::Opaque("literal"),
        )
    }

    /// Try the first schema, fall back to the second (`Union` shape).
    /// Encode always uses the first schema.
    pub fn union(first: Schema<Unknown>, second: Schema<Unknown>) -> Self {
        let kind = SchemaKind::Opaque("union");
        let first = Arc::new(first);
        let second = Arc::new(second);
        let decode_first = first.clone();
        let decode_second = second.clone();
        let encode_first = first.clone();
        Schema::new(
            move |unknown| match decode_first.decode(unknown.clone()) {
                ok @ Ok(_) => ok,
                Err(_) => decode_second.decode(unknown),
            },
            move |value| encode_first.encode(value),
            kind,
        )
    }
}

impl Schema<()> {
    /// Only null (`Null` shape).
    pub fn null() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Null => Ok(()),
                other => mismatch("$", "null", &other),
            },
            |_| Unknown::Null,
            SchemaKind::Opaque("null"),
        )
    }
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
            SchemaKind::String,
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
            SchemaKind::Number,
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
            SchemaKind::Boolean,
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
            SchemaKind::Object,
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
            SchemaKind::Opaque("field"),
        )
    }

    /// An optional text field (`optionalKey` shape): missing or null
    /// decodes to `None`; encoding `None` omits the field.
    pub fn optional_field(name: &str, inner: Schema<String>) -> Schema<Option<String>> {
        let decode_name = name.to_string();
        let encode_name = name.to_string();
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Object(fields) => match fields.get(&decode_name) {
                    None | Some(Unknown::Null) => Ok(None),
                    Some(value) => decode_inner.decode(value.clone()).map(Some),
                },
                other => mismatch("$", "object", &other),
            },
            move |value| match value {
                None => Unknown::Object(BTreeMap::new()),
                Some(value) => {
                    let mut fields = BTreeMap::new();
                    fields.insert(encode_name.clone(), encode_inner.encode(value));
                    Unknown::Object(fields)
                }
            },
            SchemaKind::Opaque("optional field"),
        )
    }
}

impl<T: Send + Sync + 'static> Schema<Vec<T>> {
    /// Homogeneous arrays: every element decoded, failures name `[i]`.
    pub fn array(inner: Schema<T>) -> Schema<Vec<T>> {
        let kind = SchemaKind::Array(Box::new(inner.kind.clone()));
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
            kind,
        )
    }
}

impl<A: Send + Sync + 'static, B: Send + Sync + 'static> Schema<(A, B)> {
    /// Fixed-length two-element arrays (`Tuple` shape): length and order
    /// are both checked; element failures name `[0]` / `[1]`.
    pub fn tuple2(first: Schema<A>, second: Schema<B>) -> Schema<(A, B)> {
        let first = Arc::new(first);
        let second = Arc::new(second);
        let decode_first = first.clone();
        let decode_second = second.clone();
        let encode_first = first.clone();
        let encode_second = second.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Array(items) => {
                    let mut items = items.into_iter();
                    let a = decode_first
                        .decode(items.next().unwrap_or(Unknown::Null))
                        .map_err(|error| error.at("$[0]"))?;
                    let b = decode_second
                        .decode(items.next().unwrap_or(Unknown::Null))
                        .map_err(|error| error.at("$[1]"))?;
                    Ok((a, b))
                }
                other => mismatch("$", "tuple", &other),
            },
            move |(a, b)| Unknown::Array(vec![encode_first.encode(a), encode_second.encode(b)]),
            SchemaKind::Opaque("tuple"),
        )
    }
}

/// One field of a struct shape: required or optional with a decoder.
///
/// Two optional flavors: [`OptionalKey`](Optionality::OptionalKey) (exact:
/// the key may be absent, but an explicit null still decodes the inner
/// schema) and [`Optional`](Optionality::Optional) (absent or null both
/// skip the inner schema).
pub struct Field {
    name: String,
    schema: Arc<Schema<Unknown>>,
    optionality: Optionality,
    default: Option<Unknown>,
    omit_on_encode: bool,
}

/// How optional a struct field is on the decoding axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optionality {
    /// The key must be present.
    Required,
    /// Exact optional: the key may be absent, but an explicit null still
    /// decodes the inner schema.
    OptionalKey,
    /// The key may be absent or null; both skip the inner schema.
    Optional,
}

impl Field {
    /// A required field decoded with `schema`.
    pub fn required(name: &str, schema: Schema<Unknown>) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::Required,
            default: None,
            omit_on_encode: false,
        }
    }

    /// An optional field: missing or null decodes without error.
    pub fn optional(name: &str, schema: Schema<Unknown>) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::Optional,
            default: None,
            omit_on_encode: false,
        }
    }

    /// An exact-optional field: the key may be absent, but an explicit
    /// null still decodes the inner schema.
    pub fn optional_key(name: &str, schema: Schema<Unknown>) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::OptionalKey,
            default: None,
            omit_on_encode: false,
        }
    }

    /// Reverse `optional`/`optional_key`: the field becomes required.
    pub fn require(mut self) -> Self {
        self.optionality = Optionality::Required;
        self
    }

    /// Make every field required (`required` over a struct shape).
    pub fn require_all(fields: Vec<Field>) -> Vec<Field> {
        fields.into_iter().map(Field::require).collect()
    }

    /// A tag field (`tag`/`tagDefaultOmit`): absent keys fill the default
    /// on decode (and construction), and the key is omitted on encode.
    pub fn tag(name: &str, value: Unknown) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(Schema::literal(value.clone())),
            optionality: Optionality::Required,
            default: Some(value),
            omit_on_encode: true,
        }
    }
}

impl Schema<BTreeMap<String, Unknown>> {
    /// A struct shape (`Struct`/`pick`/`Partial` shapes): every required
    /// field must decode; optional fields may be missing or null. Extra
    /// input fields pass through untouched.
    pub fn struct_shape(fields: Vec<Field>) -> Self {
        let omitted: Vec<String> = fields
            .iter()
            .filter(|field| field.omit_on_encode)
            .map(|field| field.name.clone())
            .collect();
        Self::new(
            move |unknown| match unknown {
                Unknown::Object(mut input) => {
                    let mut out = BTreeMap::new();
                    for field in &fields {
                        match input.remove(&field.name) {
                            None if field.default.is_some() => {
                                out.insert(
                                    field.name.clone(),
                                    field.default.clone().unwrap_or(Unknown::Null),
                                );
                            }
                            None if field.optionality != Optionality::Required => {}
                            Some(Unknown::Null) if field.optionality == Optionality::Optional => {}
                            None => {
                                return Err(SchemaError::TypeMismatch {
                                    path: format!("$.{}", field.name),
                                    expected: "required field",
                                    got: "missing",
                                });
                            }
                            Some(value) => {
                                let decoded = field
                                    .schema
                                    .decode(value)
                                    .map_err(|error| error.at(&format!("$.{}", field.name)))?;
                                out.insert(field.name.clone(), decoded);
                            }
                        }
                    }
                    for (key, value) in input {
                        out.insert(key, value);
                    }
                    Ok(out)
                }
                other => mismatch("$", "object", &other),
            },
            move |output| {
                let mut filtered = output.clone();
                for name in &omitted {
                    filtered.remove(name);
                }
                Unknown::Object(filtered)
            },
            SchemaKind::Object,
        )
    }

    /// A record shape (`Record` shape): every value decodes with the
    /// value schema; keys pass through. Empty objects decode fine.
    pub fn record(value: Schema<Unknown>) -> Self {
        let value = Arc::new(value);
        Self::new(
            move |unknown| match unknown {
                Unknown::Object(fields) => fields
                    .into_iter()
                    .map(|(key, item)| {
                        value
                            .decode(item)
                            .map(|decoded| (key.clone(), decoded))
                            .map_err(|error| error.at(&format!("$.{key}")))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>(),
                other => mismatch("$", "object", &other),
            },
            |fields| Unknown::Object(fields.clone()),
            SchemaKind::Object,
        )
    }

    /// A subset of fields (`pick` shape): only the named fields decode.
    pub fn pick(fields: Vec<Field>, names: &[&str]) -> Self {
        let wanted: Vec<Field> = fields
            .into_iter()
            .filter(|field| names.contains(&field.name.as_str()))
            .collect();
        Self::struct_shape(wanted)
    }
}

impl<T: Send + Sync + 'static> Schema<Option<T>> {
    /// `null` (or missing, by the caller's hand) decodes to `None`.
    pub fn optional(inner: Schema<T>) -> Schema<Option<T>> {
        let kind = SchemaKind::Optional(Box::new(inner.kind.clone()));
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
            kind,
        )
    }
}

impl Schema<Unknown> {
    /// Import a JSON Schema document (`fromJsonSchemaDocument` shapes):
    /// objects with `type`, `properties`/`required`, `items`, `enum`,
    /// `const` and `anyOf` import; `$ref` and anything else is a typed
    /// error naming the gap.
    pub fn from_json_schema(doc: &serde_json::Value) -> Result<Self, SchemaError> {
        Self::import(doc)
    }

    fn import(doc: &serde_json::Value) -> Result<Self, SchemaError> {
        let get = |key: &str| doc.get(key);
        if let Some(r#ref) = get("$ref") {
            let _ = r#ref;
            return Err(SchemaError::TypeMismatch {
                path: "$.$ref".to_string(),
                expected: "an inlinable schema",
                got: "a reference",
            });
        }
        if let Some(constants) = get("const") {
            return Ok(Schema::literal(Unknown::from_json(constants.clone())));
        }
        if let Some(options) = get("enum") {
            let serde_json::Value::Array(members) = options else {
                return Err(SchemaError::TypeMismatch {
                    path: "$.enum".to_string(),
                    expected: "an array",
                    got: "a non-array",
                });
            };
            let mut union: Option<Schema<Unknown>> = None;
            for member in members {
                let literal = Schema::literal(Unknown::from_json(member.clone()));
                union = Some(match union {
                    None => literal,
                    Some(accumulated) => Schema::union(accumulated, literal),
                });
            }
            return union.ok_or_else(|| SchemaError::TypeMismatch {
                path: "$.enum".to_string(),
                expected: "a non-empty array",
                got: "an empty array",
            });
        }
        if let Some(any_of) = get("anyOf") {
            let serde_json::Value::Array(options) = any_of else {
                return Err(SchemaError::TypeMismatch {
                    path: "$.anyOf".to_string(),
                    expected: "an array",
                    got: "a non-array",
                });
            };
            let mut union: Option<Schema<Unknown>> = None;
            for option in options {
                let schema = Self::import(option)?;
                union = Some(match union {
                    None => schema,
                    Some(accumulated) => Schema::union(accumulated, schema),
                });
            }
            return union.ok_or_else(|| SchemaError::TypeMismatch {
                path: "$.anyOf".to_string(),
                expected: "a non-empty array",
                got: "an empty array",
            });
        }
        match get("type") {
            None => Ok(Schema::any()),
            Some(serde_json::Value::String(type_name)) => match type_name.as_str() {
                "string" => Ok(Schema::as_unknown(Schema::string())),
                "number" | "integer" => Ok(Schema::as_unknown(Schema::number())),
                "boolean" => Ok(Schema::as_unknown(Schema::boolean())),
                "null" => Ok(Schema::as_unknown(Schema::null())),
                "array" => {
                    let items = match get("items") {
                        Some(items) => Self::import(items)?,
                        None => Schema::any(),
                    };
                    Ok(Schema::as_unknown(Schema::array(items)))
                }
                "object" => {
                    let empty = serde_json::Map::new();
                    let properties = match get("properties") {
                        Some(serde_json::Value::Object(properties)) => properties,
                        _ => &empty,
                    };
                    let required: Vec<&str> = match get("required") {
                        Some(serde_json::Value::Array(names)) => {
                            names.iter().filter_map(|name| name.as_str()).collect()
                        }
                        _ => Vec::new(),
                    };
                    let mut fields = Vec::new();
                    for (name, property) in properties {
                        let schema = Self::import(property)?;
                        if required.iter().any(|wanted| wanted == name) {
                            fields.push(Field::required(name, schema));
                        } else {
                            fields.push(Field::optional(name, schema));
                        }
                    }
                    Ok(Schema::as_unknown(Schema::struct_shape(fields)))
                }
                _ => Err(SchemaError::TypeMismatch {
                    path: "$.type".to_string(),
                    expected: "a known type name",
                    got: "an unknown type name",
                }),
            },
            _ => Err(SchemaError::TypeMismatch {
                path: "$.type".to_string(),
                expected: "a type name",
                got: "a non-string",
            }),
        }
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

// Template literals

/// One part of a template-literal parser: a span (string-like capture) or a
/// literal anchor that must match verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplatePart {
    /// Any string, possibly empty.
    AnyString,
    /// A non-empty string.
    NonEmptyString,
    /// A leading integer (`-`? digits).
    Int,
    /// A literal anchor matched verbatim.
    Anchor(String),
}

/// A template-literal parser: a sequence of spans and anchors matched
/// greedily with backtracking. Decoding yields one capture per part (anchors
/// echo themselves); encoding concatenates the captures back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateParser {
    parts: Vec<TemplatePart>,
}

impl TemplateParser {
    /// Build from parts.
    pub fn new(parts: Vec<TemplatePart>) -> Self {
        Self { parts }
    }

    /// Parse the input, returning one capture per part, or `None`.
    pub fn parse(&self, input: &str) -> Option<Vec<Unknown>> {
        let mut captures = Vec::new();
        Self::match_parts(&self.parts, input, &mut captures).then_some(captures)
    }

    /// Render captures back to a string (encoding axis).
    pub fn render(&self, captures: &[Unknown]) -> Option<String> {
        if captures.len() != self.parts.len() {
            return None;
        }
        let mut out = String::new();
        for capture in captures {
            match capture {
                Unknown::Text(text) => out.push_str(text),
                Unknown::Number(number) => out.push_str(&trim_int(*number)),
                _ => return None,
            }
        }
        Some(out)
    }

    fn match_parts(parts: &[TemplatePart], input: &str, captures: &mut Vec<Unknown>) -> bool {
        let Some((first, rest)) = parts.split_first() else {
            return input.is_empty();
        };
        match first {
            TemplatePart::Anchor(anchor) => {
                let Some(tail) = input.strip_prefix(anchor.as_str()) else {
                    return false;
                };
                captures.push(Unknown::Text(anchor.clone()));
                if Self::match_parts(rest, tail, captures) {
                    return true;
                }
                captures.pop();
                false
            }
            span => {
                let min = match span {
                    &TemplatePart::NonEmptyString | &TemplatePart::Int => 1,
                    _ => 0,
                };
                let mut end = input.len();
                loop {
                    while end > 0 && !input.is_char_boundary(end) {
                        end -= 1;
                    }
                    if end < min {
                        return false;
                    }
                    let (head, tail) = input.split_at(end);
                    if let Some(capture) = check_span(span, head) {
                        captures.push(capture);
                        if Self::match_parts(rest, tail, captures) {
                            return true;
                        }
                        captures.pop();
                    }
                    if end == 0 {
                        return false;
                    }
                    end -= 1;
                }
            }
        }
    }
}

/// Format an integer-valued float without a trailing `.0`.
fn trim_int(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

/// Check one span candidate, returning its capture.
fn check_span(span: &TemplatePart, head: &str) -> Option<Unknown> {
    match *span {
        TemplatePart::AnyString => Some(Unknown::Text(head.to_string())),
        TemplatePart::NonEmptyString => {
            (!head.is_empty()).then(|| Unknown::Text(head.to_string()))
        }
        TemplatePart::Int => parse_int(head).map(Unknown::Number),
        TemplatePart::Anchor(_) => None,
    }
}

/// A leading integer: optional `-` plus digits, the longest valid prefix.
/// The caller enumerates lengths, so this only validates one candidate.
fn parse_int(head: &str) -> Option<f64> {
    let digits = head.strip_prefix('-').unwrap_or(head);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    head.parse::<f64>().ok()
}

impl Schema<Vec<Unknown>> {
    /// A template-literal parser schema: decodes text into per-part
    /// captures, encodes captures back into text.
    pub fn template(parts: Vec<TemplatePart>) -> Self {
        let parser = TemplateParser::new(parts);
        let encode_parser = parser.clone();
        Self::new(
            move |unknown| match unknown {
                Unknown::Text(input) => parser.parse(&input).ok_or_else(|| SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "a string matching template literal parts",
                    got: "no match",
                }),
                other => mismatch("$", "string", &other),
            },
            move |captures| {
                encode_parser
                    .render(captures)
                    .map(Unknown::Text)
                    .unwrap_or(Unknown::Null)
            },
            SchemaKind::Opaque("template"),
        )
    }
}

// Construction

impl Schema<std::collections::BTreeMap<String, Unknown>> {
    /// Build a value (`make`): absent fields with defaults fill in. For
    /// non-transformed shapes this is decode over an object input.
    pub fn make(
        &self,
        input: std::collections::BTreeMap<String, Unknown>,
    ) -> Result<std::collections::BTreeMap<String, Unknown>, SchemaError> {
        self.decode(Unknown::Object(input))
    }
}

// Value formatting

/// Render a value the way `toFormatter` does: JSON shapes with `never`
/// for impossible values. Numbers print without a trailing `.0`.
pub fn format_value(value: &Unknown) -> String {
    match value {
        Unknown::Null => "null".to_string(),
        Unknown::Bool(true) => "true".to_string(),
        Unknown::Bool(false) => "false".to_string(),
        Unknown::Number(number) => format_number(*number),
        Unknown::Text(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()),
        Unknown::Array(items) => {
            let inner: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", inner.join(","))
        }
        Unknown::Object(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                        format_value(item)
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// A JS-style number: integers print bare.
fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

impl<T> Schema<T> {
    /// The value formatter for this schema: `never` schemas always print
    /// `"never"`, everything else renders the value.
    pub fn formatter(&self) -> Box<dyn Fn(&Unknown) -> String + Send + Sync + 'static> {
        match &self.kind {
            SchemaKind::Opaque("never") => Box::new(|_| "never".to_string()),
            _ => Box::new(|value| format_value(value)),
        }
    }
}
