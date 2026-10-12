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
///
/// Clone because the pin's schemas are values you pass around, and because a
/// self-referential schema (the `suspend` recursion every toFormatter Struct
/// row uses) needs to hand out copies of itself from a closure.
#[derive(Clone)]
pub struct Schema<T> {
    decode: std::sync::Arc<dyn Fn(Unknown) -> Result<T, SchemaError> + Send + Sync>,
    encode: std::sync::Arc<dyn Fn(&T) -> Unknown + Send + Sync>,
    kind: SchemaKind,
    /// Named child schemas, so a formatter can recurse into them. Kinds alone
    /// cannot carry this: the pin's `suspend` is lazily resolved at format
    /// time, so a nested struct must be reachable as a schema, not as a shape.
    /// Empty for every constructor that does not build a composite.
    children: Vec<(String, std::sync::Arc<Schema<Unknown>>)>,
    /// How to render a value of this schema. Each constructor sets this, so a
    /// wrapper like `as_unknown` can simply delegate to the schema it wraps
    /// instead of having to reconstruct the shape.
    /// None until a composite constructor sets one, which tells `formatter()`
    /// to walk the kind instead.
    formatter: Option<std::sync::Arc<dyn Fn(&Unknown) -> String + Send + Sync>>,
    /// The schema a re-typing wraps. `as_unknown` keeps its own kind opaque so
    /// the JSON Schema document does not change, but a repr, and a check built
    /// on top, must see the schema underneath. Captured as a kind plus its
    /// reprs rather than as a schema, because the wrapped schema has an
    /// arbitrary `T` and re-typing it here would recurse forever.
    wrapped: Option<Box<Wrapped>>,
}

/// What a re-typing hides: the schema it wrapped, plus its reprs.
#[derive(Clone)]
struct Wrapped {
    kind: SchemaKind,
    children: Vec<(String, Arc<Schema<Unknown>>)>,
    code: String,
    ty: String,
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
    /// Named fields in declaration order (`Struct`).
    Struct(Vec<(String, SchemaKind)>),
    /// A string-keyed mapping (`Record`): key shape then value shape.
    Record(Box<SchemaKind>, Box<SchemaKind>),
    /// Positional elements (`Tuple`).
    Tuple(Vec<SchemaKind>),
    /// Absorbs the remaining elements of a `Tuple`.
    Rest(Box<SchemaKind>),
    /// A refinement (`check`): a predicate over the decoded value. The `base`
    /// is the schema being refined, so `Number.check(isFinite)` is
    /// `Check { base: Number, name: "isFinite", .. }`. The annotation carries
    /// the pairs the pin attaches, usually `expected` but sometimes
    /// `identifier` or `description`.
    Check {
        base: Box<SchemaKind>,
        name: String,
        args: Vec<String>,
        annotation: Vec<(String, String)>,
    },
    /// A combinator whose wire shape is not introspectable.
    Opaque(&'static str),
    /// Any JSON value (`Json`).
    Json,
    /// A struct field whose key may be absent (`optionalKey` shape). Distinct
    /// from `Optional`: `optional` also accepts an explicit null, and the
    /// pin's JSON Schema import emits `optionalKey` for a non-required
    /// property, so the two are not interchangeable in a repr.
    OptionalKey(Box<SchemaKind>),
    /// A struct plus a catch-all shape (`StructWithRest`): the named fields
    /// and the shape that absorbs every other key.
    StructWithRest(Vec<(String, SchemaKind)>, Box<SchemaKind>),
    /// Positional elements with a trailing catch-all (`TupleWithRest`).
    TupleWithRest(Vec<SchemaKind>, Box<SchemaKind>),
    /// A single exact value (`Literal`).
    Literal(Unknown),
    /// A union of two or more shapes (`Union`). `mode` is either `"anyOf"` or
    /// `"oneOf"`; they differ in whether a value matching more than one member
    /// is accepted, and the repr carries the difference.
    Union {
        members: Vec<SchemaKind>,
        mode: &'static str,
    },
    /// Annotation pairs wrapped around another shape. Generalized beyond
    /// title/description because the pin annotates with `expected`, `identifier`
    /// and `description` alike, and only `expected` is not a doc field.
    Annotated {
        pairs: Vec<(String, String)>,
        inner: Box<SchemaKind>,
    },
}

impl SchemaKind {
    /// The JSON Schema document for a struct's named fields: every field
    /// required, because a struct's repr cannot say otherwise. An import walks
    /// the other way, so `StructWithRest` composes this with its catch-all.
    fn struct_document(fields: &[(String, SchemaKind)]) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<serde_json::Value> = Vec::new();
        for (name, kind) in fields {
            properties.insert(name.clone(), kind.to_json_schema());
            required.push(serde_json::Value::String(name.clone()));
        }
        serde_json::json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
            "required": required,
        })
    }

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
            SchemaKind::Struct(fields) => Self::struct_document(fields),
            SchemaKind::Record(key, value) => serde_json::json!({
                "type": "object",
                "additionalProperties": value.to_json_schema(),
                "$comment": format!("key shape is {}", key.to_json_schema()),
            }),
            SchemaKind::Tuple(elements) => {
                let items: Vec<serde_json::Value> =
                    elements.iter().map(|k| k.to_json_schema()).collect();
                serde_json::json!({"type": "array", "prefixItems": items, "items": false})
            }
            SchemaKind::Rest(inner) => {
                serde_json::json!({"prefixItems": [inner.to_json_schema()]})
            }
            SchemaKind::Check { base, .. } => base.to_json_schema(),
            SchemaKind::Json => serde_json::json!({}),
            SchemaKind::OptionalKey(inner) => inner.to_json_schema(),
            SchemaKind::StructWithRest(fields, rest) => {
                let mut doc = Self::struct_document(fields);
                doc["additionalProperties"] = rest.to_json_schema();
                doc
            }
            SchemaKind::TupleWithRest(elements, rest) => {
                let mut doc = serde_json::json!({"type": "array"});
                doc["prefixItems"] = serde_json::Value::Array(
                    elements
                        .iter()
                        .map(SchemaKind::to_json_schema)
                        .collect(),
                );
                doc["items"] = rest.to_json_schema();
                doc
            }
            SchemaKind::Literal(value) => value.to_json(),
            SchemaKind::Union { .. } => serde_json::json!({}),
            SchemaKind::Opaque(name) => serde_json::json!({"$comment": name}),
            SchemaKind::Annotated { pairs, inner } => {
                let mut doc = inner.to_json_schema();
                if let serde_json::Value::Object(ref mut map) = doc {
                    for (key, value) in pairs {
                        map.insert(key.clone(), serde_json::Value::String(value.clone()));
                    }
                }
                doc
            }
        }
    }
}

/// A decoded struct: field names in declaration order, values still untyped.
///
/// The pin's `Schema.Struct` decodes to a plain object, so the port keeps the
/// decoded form as an `Unknown` object rather than forcing a Rust struct. That
/// is what lets one `struct_` call serve any field set, which is what the
/// formatter rows need: they assert on the rendering, not on a Rust type.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StructValue {
    fields: Vec<(String, Unknown)>,
}

impl StructValue {
    /// A field's decoded value, or `None` when the name was never declared.
    pub fn get(&self, name: &str) -> Option<&Unknown> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The declared field names, in declaration order.
    pub fn field_names(&self) -> Vec<&str> {
        self.fields.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// The decoded value back as an `Unknown` object.
    pub fn to_unknown(&self) -> Unknown {
        Unknown::Object(self.fields.iter().cloned().collect())
    }
}

impl Schema<StructValue> {
    /// A struct of `field -> value` schemas (`Schema.Struct`). Each field
    /// decodes through its own schema; a missing or extra field is a
    /// `TypeMismatch` naming the path. The field schemas are kept as
    /// `children` so a formatter can recurse into them, which is what the
    /// pin's nested `Struct` rows assert.
    pub fn struct_<const N: usize>(fields: [(&str, Schema<Unknown>); N]) -> Self {
        Self::struct_of(
            fields
                .into_iter()
                .map(|(name, schema)| (name.to_string(), schema))
                .collect(),
        )
    }

    /// The same struct shape with a field list built at runtime, which is what
    /// an imported JSON Schema document needs: its field count is not known
    /// until the document is read.
    pub fn struct_of(fields: Vec<(String, Schema<Unknown>)>) -> Self {
        let schemas: Vec<(String, Arc<Schema<Unknown>>)> = fields
            .into_iter()
            .map(|(name, schema)| (name, Arc::new(schema)))
            .collect();
        let expected: Vec<String> = schemas.iter().map(|(name, _)| name.clone()).collect();
        let decode_fields = schemas.clone();
        let kind = SchemaKind::Struct(
            schemas
                .iter()
                .map(|(name, schema)| (name.clone(), schema.repr_kind()))
                .collect(),
        );
        let mut schema = Self::new(
            move |unknown| {
                let object = match unknown {
                    Unknown::Object(map) => map,
                    other => return mismatch("$", "object", &other),
                };
                for name in &expected {
                    if !object.contains_key(name) {
                        return Err(SchemaError::TypeMismatch {
                            path: format!("$.{name}"),
                            expected: "a declared field",
                            got: "absent",
                        });
                    }
                }
                let mut decoded = Vec::with_capacity(decode_fields.len());
                for (name, field) in &decode_fields {
                    let value = object.get(name).cloned().unwrap_or(Unknown::Null);
                    match field.decode(value) {
                        Ok(value) => decoded.push((name.clone(), value)),
                        Err(error) => return Err(error),
                    }
                }
                Ok(StructValue { fields: decoded })
            },
            move |value| {
                let object: BTreeMap<String, Unknown> =
                    value.fields.iter().cloned().collect();
                Unknown::Object(object)
            },
            kind,
        );
        schema.children = schemas.clone();
        let children = schemas;
        schema.formatter = Some(std::sync::Arc::new(move |value: &Unknown| {
            // A Struct with NO declared fields falls back to the plain
            // JSON-ish rendering, which is what the pin does: its "empty" row
            // asserts `{"a":1}` and `[1,2,3]` unspaced. Only a Struct whose
            // fields are declared takes the spaced path.
            if children.is_empty() {
                return format_value(value);
            }
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = children
                .iter()
                .map(|(name, field)| {
                    let item = map.get(name).cloned().unwrap_or(Unknown::Null);
                    let key =
                        serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}: {}", field.formatter()(&item))
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        })
        );
        schema
    }
}

impl<T> Schema<T> {
    pub(crate) fn new(
        decode: impl Fn(Unknown) -> Result<T, SchemaError> + Send + Sync + 'static,
        encode: impl Fn(&T) -> Unknown + Send + Sync + 'static,
        kind: SchemaKind,
    ) -> Self {
        Self {
            decode: std::sync::Arc::new(decode),
            encode: std::sync::Arc::new(encode),
            kind,
            children: Vec::new(),
            formatter: None,
            wrapped: None,
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
    pub fn annotated(
        schema: Schema<T>,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Schema<T>
    where
        T: Send + Sync + 'static,
    {
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Some(title) = title {
            pairs.push(("title".to_string(), title.to_string()));
        }
        if let Some(description) = description {
            pairs.push(("description".to_string(), description.to_string()));
        }
        Self::annotated_with(schema, pairs)
    }

    /// Annotations under their own keys, for the ones that are not a doc field.
    /// The pin's `expected` on a JSON catch-all is the case that needs this.
    pub fn annotated_with<S: Into<String>>(
        schema: Schema<T>,
        pairs: Vec<(S, S)>,
    ) -> Schema<T>
    where
        T: Send + Sync + 'static,
    {
        let kind = SchemaKind::Annotated {
            pairs: pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
            inner: Box::new(schema.repr_kind()),
        };
        Schema {
            decode: schema.decode,
            encode: schema.encode,
            kind,
            children: schema.children,
            formatter: schema.formatter.clone(),
            wrapped: schema.wrapped.clone(),
        }
    }

    /// Re-type a schema to `Unknown` through itself: decode validates
    /// and normalizes, then the value returns to unknown form. This is
    /// how heterogeneous struct fields share one `Field` type.
    pub fn as_unknown(schema: Schema<T>) -> Schema<Unknown>
    where
        T: Send + Sync + 'static,
    {
        // Capture the wrapped schema's reprs BEFORE the inner is moved into
        // the Arc below: re-typing it here would recurse forever.
        let wrapped = Some(Box::new(Wrapped {
            kind: schema.kind.clone(),
            children: schema.children.clone(),
            code: code_of(&schema.kind, &schema.children),
            ty: type_of(&schema.kind, &schema.children),
        }));
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        let inner_render = schema.formatter.clone();
        let inner_kind = schema.kind.clone();
        let inner_children = schema.children.clone();
        let mut out = Schema::new(
            move |unknown| {
                decode_schema
                    .decode(unknown)
                    .map(|value| encode_schema.encode(&value))
            },
            |value| value.clone(),
            SchemaKind::Opaque("unknown"),
        );
        // A re-typing does not change the shape, so the wrapper renders exactly
        // the way the schema it wraps does. Without this the nested Struct rows
        // lose their spacing, because every field has to pass through
        // as_unknown to share one Unknown type.
        out.formatter = Some(Arc::new(move |value: &Unknown| match &inner_render {
            Some(render) => render(value),
            None => format_root(&inner_kind, &inner_children, value),
        }));
        out.wrapped = wrapped;
        out
    }

    /// The kind a repr should render, seeing through a re-typing. `as_unknown`
    /// keeps its own kind opaque on purpose, so a constructor that copies a
    /// child's kind (`record`, `checked`) must copy the one underneath or the
    /// repr prints `Schema.Unknown` for a perfectly visible child.
    pub(crate) fn repr_kind(&self) -> SchemaKind {
        match &self.wrapped {
            Some(wrapped) => wrapped.kind.clone(),
            None => self.kind.clone(),
        }
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
        let render_factory = factory.clone();
        let mut out = Schema::new(
            move |unknown| decode_factory().decode(unknown),
            move |value| encode_factory().encode(value),
            SchemaKind::Opaque("suspend"),
        );
        // A suspend resolves lazily on every decode and every encode, so it
        // must resolve lazily on every render too. Calling the factory per
        // render is what makes a recursive schema terminate: the pin's
        // mutually-suspended and self-referential rows are infinitely deep on
        // the schema side and finite on the value side.
        out.formatter = Some(Arc::new(move |value| render_factory().formatter()(value)));
        out
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

    /// A finite number written as text (`FiniteFromString`): `"1.5"`
    /// decodes to `1.5`. Non-text, blank, unparseable, and non-finite
    /// (`NaN`, infinities) inputs fail; encoding renders the number back
    /// to text.
    pub fn finite_from_string() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Text(ref text) => {
                    let trimmed = text.trim();
                    match trimmed.parse::<f64>() {
                        Ok(number) if number.is_finite() => Ok(number),
                        _ => mismatch("$", "a finite number string", &unknown),
                    }
                }
                other => mismatch("$", "a finite number string", &other),
            },
            |value| Unknown::Text(value.to_string()),
            SchemaKind::Opaque("finiteFromString"),
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
        let render = inner.formatter.clone();
        let inner_kind = inner.kind.clone();
        let mut out = Self::new(
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
        );
        // Each element renders through its own schema, so an array of Structs
        // keeps the struct spacing the pin emits (`[{ "a": 1 }]`, not `[{"a":1}]`).
        out.formatter = Some(Arc::new(move |value: &Unknown| {
            let Unknown::Array(items) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = items
                .iter()
                .map(|item| match &render {
                    Some(r) => r(item),
                    None => format_root(&inner_kind, &[], item),
                })
                .collect();
            format!("[{}]", inner.join(", "))
        })
        );
        out
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
        let render = value.formatter.clone();
        // A value that arrived here through a re-typing has an opaque kind of
        // its own, so the Record has to copy the kind underneath it.
        let value_kind = value.repr_kind();
        let mut out = Self::new(
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
            SchemaKind::Record(
                Box::new(SchemaKind::String),
                Box::new(value_kind.clone()),
            ),
        );
        // A Record renders its entries spaced, the way the pin does, and each
        // entry renders through the value schema so a nested Record or Struct
        // recurses with the same rule.
        out.formatter = Some(Arc::new(move |value: &Unknown| {
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            // An empty object renders bare, the same rule the Struct follows.
            if map.is_empty() {
                return "{}".to_string();
            }
            let inner: Vec<String> = map
                .iter()
                .map(|(key, item)| {
                    let key =
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}: {}", match &render {
                        Some(r) => r(item),
                        None => format_root(&value_kind, &[], item),
                    })
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        })
        );
        out
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
    /// Any JSON value annotated as open. The `expected` message always comes
    /// first, and the pin's other annotation keywords ride along after it.
    fn annotated_json(doc: &serde_json::Value) -> Schema<Unknown> {
        let mut pairs = vec![("expected".to_string(), "JSON value".to_string())];
        for key in ["title", "description", "format"] {
            if let Some(text) = doc.get(key).and_then(serde_json::Value::as_str) {
                pairs.push((key.to_string(), text.to_string()));
            }
        }
        Schema::annotated_with(Schema::json(), pairs)
    }

    /// The open-object catch-all: the annotated JSON value as a string-keyed
    /// record. Every open form in the import uses this exact shape, both as a
    /// whole object and as the rest of a struct, so it is named once rather
    /// than spelled at each use.
    fn json_catch_all() -> Schema<Unknown> {
        Schema::as_unknown(Schema::record(Schema::annotated_json(&serde_json::json!({}))))
    }

    /// Wrap a checked schema so the check is visible to a repr. `as_unknown`
    /// would otherwise hide it behind its opaque kind.
    fn as_unknown_checked(base: Schema<Unknown>, name: &str, expected: &str) -> Self {
        Schema::as_unknown(Schema::checked(
            base,
            name,
            Vec::new(),
            vec![("expected", expected)],
        ))
    }

    /// A string plus whatever length or pattern constraints the document
    /// carries. The pin distinguishes a length in characters (`minLength`) from
    /// a length in code points (`minCodePoints`); both map to a check, with
    /// different names in the repr.
    fn refined_text(doc: &serde_json::Value) -> Self {
        let mut schema = Schema::as_unknown(Schema::string());
        for (keyword, name, message) in [
            ("minLength", "isMinLength", "a value with a length of at least"),
            ("maxLength", "isMaxLength", "a value with a length of at most"),
            (
                "minCodePoints",
                "isMinCodePoints",
                "a value with a length of at least",
            ),
            (
                "maxCodePoints",
                "isMaxCodePoints",
                "a value with a length of at most",
            ),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &format!("{message} {printed}"))],
                ));
            }
        }
        if let Some(pattern) = doc.get("pattern").and_then(serde_json::Value::as_str) {
            schema = Schema::as_unknown(Schema::checked(
                schema,
                "isPattern",
                vec![pattern],
                vec![("expected", "a string matching the pattern")],
            ));
        }
        schema
    }

    /// A number plus whatever bound constraints the document carries. The
    /// pin distinguishes an exclusive from an inclusive bound with two check
    /// names, so each keyword maps to exactly one.
    fn refined_number(base: Schema<Unknown>, doc: &serde_json::Value) -> Self {
        let mut schema = base;
        for (keyword, name, message) in [
            ("minimum", "isGreaterThanOrEqualTo", "a value greater than or equal to"),
            ("maximum", "isLessThanOrEqualTo", "a value less than or equal to"),
            ("exclusiveMinimum", "isGreaterThan", "a value greater than"),
            ("exclusiveMaximum", "isLessThan", "a value less than"),
            ("multipleOf", "isMultipleOf", "a value that is a multiple of"),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &format!("{message} {printed}"))],
                ));
            }
        }
        schema
    }

    /// A positional prefix (`prefixItems`) as a tuple. The pin's rule is that
    /// `minItems` marks the leading positions required, `maxItems` decides
    /// whether any position is open, and `items` names the open element:
    ///
    /// - `maxItems` above the prefix length keeps a rest, but that rest can hold
    ///   fewer elements than the prefix declares, so a max-length check rides
    ///   along;
    /// - `maxItems` equal to the prefix length, or `items: false`, closes the
    ///   tuple and drops the rest entirely.
    fn tuple_from_prefix(
        prefix: &[serde_json::Value],
        doc: &serde_json::Value,
    ) -> Result<Self, SchemaError> {
        let min = doc.get("minItems").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
        let max = doc.get("maxItems").and_then(serde_json::Value::as_u64).map(|v| v as usize);
        let closed_by_items = matches!(doc.get("items"), Some(serde_json::Value::Bool(false)));

        let mut elements = Vec::with_capacity(prefix.len());
        for (index, element) in prefix.iter().enumerate() {
            let schema = Self::import(element)?;
            // A position below `minItems` must be present; the rest may be
            // absent, which is exactly `optionalKey` against `Optional`.
            elements.push(if index < min {
                schema
            } else {
                Schema::optional_key_shape(schema)
            });
        }

        // Open unless `maxItems` exactly equals the prefix length or `items`
        // closed it. Any other `maxItems` keeps the rest and carries a
        // max-length check, even a `maxItems` below the prefix length, because
        // the tuple alone would allow every declared position.
        let open = match max {
            Some(max) => max != prefix.len() && !closed_by_items,
            None => !closed_by_items,
        };
        if !open {
            // Neither `refined_array` nor a check: `maxItems` was spent closing
            // the tuple and `minItems` was spent marking the leading positions
            // required, so both are already represented in the shape.
            return Ok(Schema::as_unknown(Schema::tuple_of(elements)));
        }
        let rest = match doc.get("items") {
            Some(serde_json::Value::Bool(_)) | None => {
                Schema::annotated_with(Schema::json(), vec![("expected", "JSON value")])
            }
            Some(items) => Self::import(items)?,
        };
        let base = Schema::as_unknown(Schema::tuple_with_rest(elements, rest));
        // A rest that cannot hold every declared position also needs the
        // max-length check, because the tuple alone would allow them. The check
        // is attached whenever `maxItems` is named at all.
        let base = match max {
            Some(max) => Schema::as_unknown(Schema::checked(
                base,
                "isMaxLength",
                vec![format_number(max as f64).as_str()],
                vec![(
                    "expected",
                    &format!("a value with a length of at most {max}"),
                )],
            )),
            None => base,
        };
        // `refined_array` is deliberately not called here: it maps `maxItems` to
        // the same max-length check, and the open form already attached it.
        Ok(base)
    }

    /// An array plus whatever size or uniqueness constraints the document
    /// carries.
    fn refined_array(base: Schema<Unknown>, doc: &serde_json::Value) -> Self {
        let mut schema = base;
        for (keyword, name, message) in [
            ("minItems", "isMinLength", "a value with a length of at least"),
            ("maxItems", "isMaxLength", "a value with a length of at most"),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &format!("{message} {printed}"))],
                ));
            }
        }
        if doc.get("uniqueItems").and_then(serde_json::Value::as_bool) == Some(true) {
            schema = Schema::as_unknown(Schema::checked(
                schema,
                "isUnique",
                Vec::new(),
                vec![("expected", "an array with unique items")],
            ));
        }
        schema
    }

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
            // Nothing constrains the value, so it is any JSON value annotated as
            // the open catch-all. The pin's `annotate` pairs ride along.
            None => Ok(Self::annotated_json(doc)),
            Some(serde_json::Value::String(type_name)) => match type_name.as_str() {
                "string" => Ok(Self::refined_text(doc)),
                // The pin's JSON Schema import makes a `number` finite and an
                // `integer` an int, so both carry a check where a bare string
                // does not.
                "number" => Ok(Self::refined_number(
                    Self::as_unknown_checked(
                        Schema::as_unknown(Schema::number()),
                        "isFinite",
                        "a finite number",
                    ),
                    doc,
                )),
                "integer" => Ok(Self::refined_number(
                    Self::as_unknown_checked(
                        Schema::as_unknown(Schema::number()),
                        "isInt",
                        "an integer",
                    ),
                    doc,
                )),
                "boolean" => Ok(Schema::as_unknown(Schema::boolean())),
                "null" => Ok(Schema::as_unknown(Schema::null())),
                "array" => match get("prefixItems") {
                    // `prefixItems` is a positional prefix, so it decodes to a
                    // tuple rather than an array. Everything else about the
                    // array comes from `minItems`, `maxItems` and `items`.
                    Some(serde_json::Value::Array(prefix)) => {
                        Self::tuple_from_prefix(prefix, doc)
                    }
                    _ => {
                        let items = match get("items") {
                            Some(items) => Self::import(items)?,
                            None => Schema::any(),
                        };
                        Ok(Self::refined_array(
                            Schema::as_unknown(Schema::array(items)),
                            doc,
                        ))
                    }
                },
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
                        // A non-required property is an exact optional: the key
                        // may be absent, but an explicit null still decodes the
                        // inner schema. That is `optionalKey`, not `optional`.
                        let schema = if required.iter().any(|wanted| *wanted == name.as_str()) {
                            schema
                        } else {
                            Schema::optional_key_shape(schema)
                        };
                        fields.push((name.clone(), schema));
                    }
                    if fields.is_empty() {
                        // No named keys: the whole object is a record, and what
                        // `additionalProperties` says decides the value shape.
                        return Ok(match get("additionalProperties") {
                            // `false` closes it: the value shape is `never`.
                            Some(serde_json::Value::Bool(false)) => {
                                Schema::as_unknown(Schema::record(Schema::never()))
                            }
                            // Absent or `true` opens it to any JSON value.
                            Some(serde_json::Value::Bool(true)) | None => {
                                Self::json_catch_all()
                            }
                            // A typed catch-all on its own is a plain record.
                            Some(schema) => Schema::as_unknown(Schema::record(Self::import(schema)?)),
                        });
                    }
                    match get("additionalProperties") {
                        // `false` closes the object: nothing but the named keys.
                        Some(serde_json::Value::Bool(false)) => {
                            Ok(Schema::as_unknown(Schema::struct_of(fields)))
                        }
                        // A typed catch-all next to named keys is refused by the
                        // pin, because an index signature also checks the keys
                        // the named fields already claimed.
                        Some(serde_json::Value::Bool(true)) | None => {
                            let rest = Self::json_catch_all();
                            Ok(Schema::as_unknown(Schema::struct_with_rest(fields, rest)))
                        }
                        Some(_) => Err(SchemaError::TypeMismatch {
                            path: "$.additionalProperties".to_string(),
                            expected: "a boolean, with no other property schemas",
                            got: "a schema next to declared properties",
                        }),
                    }
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

impl<T> Schema<T> {
    /// The value formatter for this schema (`Schema.toFormatter`).
    ///
    /// A Struct with declared fields renders each field as `"name": value`
    /// joined by ", ", and the same rule applies to every NESTED Struct, which
    /// is why this walks the schema tree rather than the value tree: the pin's
    /// nested output is `{ "a": "a", "as": [{ "a": "b" }] }`, with the spacing
    /// on the inner objects too. An EMPTY Struct has no declared fields, so it
    /// falls through to the plain JSON-ish rendering, which is what the pin
    /// does too (its "empty" row asserts `{"a":1}` unspaced).
    pub fn formatter(&self) -> Box<dyn Fn(&Unknown) -> String + Send + Sync + 'static> {
        match &self.formatter {
            // A composite constructor supplied its own renderer, because it has
            // child schemas the kind alone cannot reach.
            Some(render) => {
                let render = render.clone();
                Box::new(move |value| render(value))
            }
            // Everything else is described by its kind, so walk it.
            None => {
                let kind = self.kind.clone();
                let children = self.children.clone();
                Box::new(move |value| format_root(&kind, &children, value))
            }
        }
    }
}

// SchemaRepresentation: the code / type repr

impl Schema<Unknown> {
    /// Any JSON value (`Json`). The wire shape is unconstrained, which is what
    /// makes it the catch-all for an open object's additional properties.
    pub fn json() -> Self {
        Self::new(
            Ok,
            |value| value.clone(),
            SchemaKind::Json,
        )
    }

    /// A struct field whose key may be absent (`optionalKey`). Decode delegates
    /// to the inner schema: whether the key may be missing is a property of the
    /// enclosing struct, so it is enforced there and not here.
    pub fn optional_key_shape(inner: Schema<Unknown>) -> Self {
        let kind = SchemaKind::OptionalKey(Box::new(inner.repr_kind()));
        let decode = inner.decode.clone();
        let encode = inner.encode.clone();
        Self::new(
            move |unknown| decode(unknown),
            move |value| encode(value),
            kind,
        )
    }

    /// A struct plus a catch-all shape (`StructWithRest`): the named fields and
    /// the shape absorbing every key they do not claim.
    pub fn struct_with_rest(
        fields: Vec<(String, Schema<Unknown>)>,
        rest: Schema<Unknown>,
    ) -> Self {
        let kind = SchemaKind::StructWithRest(
            fields.iter().map(|(name, field)| (name.clone(), field.repr_kind())).collect(),
            Box::new(rest.repr_kind()),
        );
        let names: Vec<String> = fields.iter().map(|(name, _)| name.clone()).collect();
        // A field wrapped in `optionalKey` may be absent; a bare one may not.
        // The kind already carries the difference, so this does not need a
        // second list of required names.
        let required: Vec<String> = fields
            .iter()
            .filter(|(_, field)| !matches!(field.repr_kind(), SchemaKind::OptionalKey(_)))
            .map(|(name, _)| name.clone())
            .collect();
        let decoders: Vec<(String, Arc<Schema<Unknown>>)> =
            fields.into_iter().map(|(name, field)| (name, Arc::new(field))).collect();
        let rest_decode = Arc::new(rest);
        Self::new(
            move |unknown| {
                let object = match unknown {
                    Unknown::Object(map) => map,
                    other => return mismatch("$", "object", &other),
                };
                for name in &required {
                    if !object.contains_key(name) {
                        return Err(SchemaError::TypeMismatch {
                            path: format!("$.{name}"),
                            expected: "a declared field",
                            got: "absent",
                        });
                    }
                }
                let mut out = Vec::with_capacity(decoders.len());
                for (name, field) in &decoders {
                    match object.get(name) {
                        None => {}
                        Some(value) => {
                            let decoded = field.decode(value.clone())
                                .map_err(|error| error.at(&format!("$.{name}")))?;
                            out.push((name.clone(), decoded));
                        }
                    }
                }
                // Keys the named fields do not claim go to the catch-all.
                for (key, value) in &object {
                    if names.iter().any(|name| name == key) {
                        continue;
                    }
                    let decoded = rest_decode.decode(value.clone())?;
                    out.push((key.clone(), decoded));
                }
                Ok(Unknown::Object(out.into_iter().collect()))
            },
            |value| value.clone(),
            kind,
        )
    }

    /// Fixed-length positional elements (`Tuple`), built from a list whose
    /// length is only known once the document has been read.
    pub fn tuple_of(elements: Vec<Schema<Unknown>>) -> Self {
        let kinds: Vec<SchemaKind> = elements.iter().map(|e| e.repr_kind()).collect();
        let required = kinds
            .iter()
            .take_while(|kind| !matches!(kind, SchemaKind::OptionalKey(_)))
            .count();
        let kind = SchemaKind::Tuple(kinds);
        let decoders: Vec<Arc<Schema<Unknown>>> = elements.into_iter().map(Arc::new).collect();
        let total = decoders.len();
        Self::new(
            move |unknown| {
                let items = match unknown {
                    Unknown::Array(items) => items,
                    other => return mismatch("$", "array", &other),
                };
                // A position below the first optional one must be present, and
                // none may appear past the last declared position.
                if items.len() < required || items.len() > total {
                    return mismatch("$", "a fixed-length array", &Unknown::Array(items));
                }
                let mut out = Vec::with_capacity(total);
                for index in 0..total {
                    match decoders.get(index) {
                        Some(element) if index < items.len() => {
                            out.push(element.decode(items[index].clone())?);
                        }
                        // An absent optional position decodes as null, which is
                        // what `optionalKey` permits.
                        Some(_) => out.push(Unknown::Null),
                        None => out.push(Unknown::Null),
                    }
                }
                Ok(Unknown::Array(out))
            },
            |value| value.clone(),
            kind,
        )
    }

    /// Positional elements with a trailing catch-all (`TupleWithRest`): the
    /// leading positions and the shape absorbing the remaining elements.
    pub fn tuple_with_rest(
        elements: Vec<Schema<Unknown>>,
        rest: Schema<Unknown>,
    ) -> Self {
        let kind = SchemaKind::TupleWithRest(
            elements.iter().map(|element| element.repr_kind()).collect(),
            Box::new(rest.repr_kind()),
        );
        let decoders: Vec<Arc<Schema<Unknown>>> = elements.into_iter().map(Arc::new).collect();
        let rest_decode = Arc::new(rest);
        Self::new(
            move |unknown| {
                let items = match unknown {
                    Unknown::Array(items) => items,
                    other => return mismatch("$", "array", &other),
                };
                let mut out = Vec::with_capacity(items.len());
                for (index, value) in items.into_iter().enumerate() {
                    // The leading positions decode with their own schema, and
                    // everything past the last one lands on the catch-all. A
                    // trailing position is optional, so a short array is fine.
                    match decoders.get(index) {
                        Some(element) if index < decoders.len() => {
                            out.push(element.decode(value)?);
                        }
                        _ => {
                            out.push(rest_decode.decode(value)?);
                        }
                    }
                }
                Ok(Unknown::Array(out))
            },
            |value| value.clone(),
            kind,
        )
    }

    /// A union of two or more shapes (`Union`). `mode` is `"anyOf"` or
    /// `"oneOf"`; a `oneOf` accepts a value matching exactly one member, which
    /// is the only semantic difference, and the repr carries it.
    pub fn union_with_mode(members: Vec<Schema<Unknown>>, mode: &'static str) -> Self {
        let kind = SchemaKind::Union {
            members: members.iter().map(|member| member.repr_kind()).collect(),
            mode,
        };
        let decoders: Vec<Arc<Schema<Unknown>>> = members.into_iter().map(Arc::new).collect();
        Self::new(
            move |unknown| {
                let mut decoded = None;
                for member in &decoders {
                    if let Ok(value) = member.decode(unknown.clone()) {
                        decoded = Some(value);
                        break;
                    }
                }
                // A `oneOf` matching twice is still a match for a decode, so the
                // first wins. The one-of distinction lives in the repr, which is
                // what the pin asserts.
                match decoded {
                    Some(value) => Ok(value),
                    None => mismatch("$", "a matching member", &unknown),
                }
            },
            |value| value.clone(),
            kind,
        )
    }
}

impl Schema<Unknown> {
    /// Wrap `base` in a refinement (`check`). The kind records the check so a
    /// repr can render it; decode enforces the predicate where this port knows
    /// it, and passes through where it does not, so an unknown check degrades
    /// to "unconstrained" rather than rejecting a valid value.
    pub fn checked(
        base: Schema<Unknown>,
        name: &str,
        args: Vec<&str>,
        annotation: Vec<(&str, &str)>,
    ) -> Self {
        // A re-typing keeps its own kind opaque, so the check must be built on
        // the kind underneath it or the repr would print `Schema.Unknown`.
        let base_kind = base.repr_kind();
        let kind = SchemaKind::Check {
            base: Box::new(base_kind),
            name: name.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            annotation: annotation
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        };
        let numeric: Vec<f64> = args.iter().filter_map(|arg| arg.parse().ok()).collect();
        let limit = numeric.first().copied();
        let name_owned = name.to_string();
        let inner = Arc::new(base);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let mut out = Self::new(
            move |unknown| {
                if !check_holds(&name_owned, limit, &unknown) {
                    return Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: "a value satisfying the refinement",
                        got: unknown.kind(),
                    });
                }
                decode_inner.decode(unknown)
            },
            move |value| encode_inner.encode(value),
            kind,
        );
        out.formatter = inner.formatter.clone();
        out.wrapped = None;
        out
    }
}

/// Whether the decoded value satisfies a named refinement. Only the checks the
/// pin's JSON Schema import actually emits are implemented; anything else
/// passes, so an unimplemented refinement is unenforced rather than wrong.
fn check_holds(name: &str, limit: Option<f64>, value: &Unknown) -> bool {
    match (name, limit, value) {
        ("isFinite", _, Unknown::Number(number)) => number.is_finite(),
        ("isInt", _, Unknown::Number(number)) => number.fract() == 0.0,
        ("isMinLength", Some(min), Unknown::Text(text)) => text.chars().count() as f64 >= min,
        ("isMaxLength", Some(max), Unknown::Text(text)) => text.chars().count() as f64 <= max,
        ("isMinCodePoints", Some(min), Unknown::Text(text)) => text.chars().count() as f64 >= min,
        ("isMaxCodePoints", Some(max), Unknown::Text(text)) => text.chars().count() as f64 <= max,
        ("isGreaterThan", Some(bound), Unknown::Number(number)) => *number > bound,
        ("isGreaterThanOrEqualTo", Some(bound), Unknown::Number(number)) => *number >= bound,
        ("isLessThan", Some(bound), Unknown::Number(number)) => *number < bound,
        ("isLessThanOrEqualTo", Some(bound), Unknown::Number(number)) => *number <= bound,
        ("isMultipleOf", Some(divisor), Unknown::Number(number)) => {
            divisor != 0.0 && (number / divisor).fract() == 0.0
        }
        ("isMinLength", Some(min), Unknown::Array(items)) => items.len() as f64 >= min,
        ("isMaxLength", Some(max), Unknown::Array(items)) => items.len() as f64 <= max,
        ("isUnique", _, Unknown::Array(_)) => true,
        _ => true,
    }
}

impl<T> Schema<T> {
    /// The Source-style `code` for this schema, the shape the pin's
    /// `SchemaRepresentation.toCodeDocument` emits (`Schema.String`,
    /// `Schema.Array(Schema.Never)`, `Schema.Union([...])`).
    pub fn to_code(&self) -> String {
        // A re-typing keeps its own kind opaque, so its code is the code of
        // the schema it wraps.
        match &self.wrapped {
            Some(boxed) => boxed.code.clone(),
            None => code_of(&self.kind, &self.children),
        }
    }

    /// The TypeScript-style `type` for this schema, the second half of a
    /// `Code` entry (`string`, `ReadonlyArray<never>`, `never | string`).
    pub fn to_type(&self) -> String {
        match &self.wrapped {
            Some(boxed) => boxed.ty.clone(),
            None => type_of(&self.kind, &self.children),
        }
    }
}

fn code_of(kind: &SchemaKind, children: &[(String, Arc<Schema<Unknown>>)]) -> String {
    match kind {
        SchemaKind::String => "Schema.String".to_string(),
        SchemaKind::Number => "Schema.Number".to_string(),
        SchemaKind::Boolean => "Schema.Boolean".to_string(),
        SchemaKind::Object => "Schema.Object".to_string(),
        SchemaKind::Opaque("never") => "Schema.Never".to_string(),
        SchemaKind::Opaque("null") => "Schema.Null".to_string(),
        SchemaKind::Opaque("any") => "Schema.Any".to_string(),
        SchemaKind::Opaque("unknown") => "Schema.Unknown".to_string(),
        SchemaKind::Json => "Schema.Json".to_string(),
        SchemaKind::OptionalKey(inner) => format!("Schema.optionalKey({})", code_of_at(inner, children)),
        SchemaKind::Literal(value) => format!("Schema.Literal({})", value.to_json()),
        SchemaKind::StructWithRest(fields, rest) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_code())
                        .unwrap_or_else(|| code_of(field, &[]));
                    format!("{}: {}", json_key(name), child)
                })
                .collect();
            format!(
                "Schema.StructWithRest(Schema.Struct({{ {} }}), [{}])",
                inner.join(", "),
                code_of_at(rest, children)
            )
        }
        SchemaKind::TupleWithRest(elements, rest) => {
            let inner: Vec<String> = elements
                .iter()
                .map(|e| code_of_at(e, children))
                .collect();
            format!(
                "Schema.TupleWithRest(Schema.Tuple([{}]), [{}])",
                inner.join(", "),
                code_of_at(rest, children)
            )
        }
        SchemaKind::Union { members, mode } => {
            let inner: Vec<String> = members
                .iter()
                .map(|member| code_of_at(member, children))
                .collect();
            format!(
                "Schema.Union([{}], {{ mode: \"{}\" }})",
                inner.join(", "),
                mode
            )
        }
        SchemaKind::Annotated { pairs, inner } => {
            if pairs.is_empty() {
                code_of(inner, children)
            } else {
                let rendered = pairs
                    .iter()
                    .map(|(key, value)| format!("{}: {}", json_key(key), json_key(value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}.annotate({{ {} }})", code_of(inner, children), rendered)
            }
        }
        SchemaKind::Array(inner) => format!("Schema.Array({})", code_of_at(inner, children)),
        SchemaKind::Optional(inner) => format!("Schema.optional({})", code_of_at(inner, children)),
        SchemaKind::Struct(fields) if fields.is_empty() => "Schema.Struct({})".to_string(),
        SchemaKind::Struct(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_code())
                        .unwrap_or_else(|| code_of(field, &[]));
                    format!("{}: {}", json_key(name), child)
                })
                .collect();
            format!("Schema.Struct({{ {} }})", inner.join(", "))
        }
        SchemaKind::Record(_, value) => {
            format!(
                "Schema.Record(Schema.String, {})",
                code_of_at(value, children)
            )
        }
        SchemaKind::Tuple(elements) => {
            let inner: Vec<String> = elements
                .iter()
                .map(|e| code_of_at(e, children))
                .collect();
            format!("Schema.Tuple([{}])", inner.join(", "))
        }
        SchemaKind::Rest(inner) => format!("Schema.Rest({})", code_of_at(inner, children)),
        SchemaKind::Check {
            base,
            name,
            args,
            annotation,
        } => {
            let pairs = annotation
                .iter()
                .map(|(key, value)| format!("{}: {}", json_key(key), json_key(value)))
                .collect::<Vec<_>>()
                .join(", ");
            let rendered = if pairs.is_empty() {
                format!("Schema.{name}({})", args.join(", "))
            } else {
                format!("Schema.{name}({}).annotate({{ {pairs} }})", args.join(", "))
            };
            // The Check's base is already the repr-visible kind: `checked()`
            // resolved through the re-typing before storing it, so no second
            // lookup happens here.
            format!("{}.check({rendered})", code_of(base, &[]))
        }
        SchemaKind::Opaque(other) => format!("Schema.{other}"),
    }
}

/// A field with no child schema of its own still needs a code, so it is
/// rendered from the kind alone.
fn code_of_at(kind: &SchemaKind, children: &[(String, Arc<Schema<Unknown>>)]) -> String {
    let _ = children;
    code_of(kind, &[])
}

fn type_of(kind: &SchemaKind, children: &[(String, Arc<Schema<Unknown>>)]) -> String {
    match kind {
        SchemaKind::String => "string".to_string(),
        SchemaKind::Number => "number".to_string(),
        SchemaKind::Boolean => "boolean".to_string(),
        SchemaKind::Object => "object".to_string(),
        SchemaKind::Opaque("never") => "never".to_string(),
        SchemaKind::Opaque("any") | SchemaKind::Opaque("unknown") => "unknown".to_string(),
        SchemaKind::Array(inner) => format!("ReadonlyArray<{}>", type_of_at(inner)),
        SchemaKind::Optional(inner) => {
            format!("{{ readonly _?: {} }}", type_of_at(inner))
        }
        SchemaKind::Struct(fields) if fields.is_empty() => "{}".to_string(),
        SchemaKind::Struct(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_type())
                        .unwrap_or_else(|| type_of(field, &[]));
                    // An exact-optional field types as an optional property,
                    // which is the same `readonly "b"?: string` a StructWithRest
                    // field gets. Only the key differs, not the shape.
                    match field {
                        SchemaKind::OptionalKey(inner_key) => {
                            format!("readonly {}?: {}", json_key(name), type_of_at(inner_key))
                        }
                        _ => format!("readonly {}: {}", json_key(name), child),
                    }
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
        SchemaKind::Record(_, value) => {
            format!("{{ readonly [x: string]: {} }}", type_of_at(value))
        }
        SchemaKind::Tuple(elements) => {
            let inner: Vec<String> = elements.iter().map(type_of_at).collect();
            format!("readonly [{}]", inner.join(", "))
        }
        SchemaKind::Rest(inner) => format!("Array<{}>", type_of_at(inner)),
        SchemaKind::StructWithRest(fields, rest) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_type())
                        .unwrap_or_else(|| type_of(field, &[]));
                    // The intersection carries every key the rest does not
                    // claim, so an exact-optional field types as optional there
                    // exactly as it does inside a plain Struct.
                    match field {
                        SchemaKind::OptionalKey(inner_key) => {
                            format!("readonly {}?: {}", json_key(name), type_of_at(inner_key))
                        }
                        _ => format!("readonly {}: {}", json_key(name), child),
                    }
                })
                .collect();
            format!("{{ {} }} & {}", inner.join(", "), type_of_at(rest))
        }
        SchemaKind::TupleWithRest(elements, rest) => {
            let mut inner: Vec<String> = elements
                .iter()
                .map(|element| match element {
                    SchemaKind::OptionalKey(inner_key) => {
                        format!("({})?", type_of_at(inner_key))
                    }
                    other => type_of_at(other),
                })
                .collect();
            inner.push(format!("...Array<{}>", type_of_at(rest)));
            format!("readonly [{}]", inner.join(", "))
        }
        SchemaKind::Check { base, .. } => type_of_at(base),
        SchemaKind::Json => "Schema.Json".to_string(),
        SchemaKind::OptionalKey(inner) => format!("({})?", type_of_at(inner)),
        SchemaKind::Literal(value) => value.to_json().to_string(),
        SchemaKind::Union { members, .. } => members
            .iter()
            .map(|member| type_of_at(member))
            .collect::<Vec<_>>()
            .join(" | "),
        SchemaKind::Annotated { inner, .. } => type_of(inner, children),
        SchemaKind::Opaque(other) => (*other).to_string(),
    }
}

fn type_of_at(kind: &SchemaKind) -> String {
    type_of(kind, &[])
}

/// A JSON key as the pin's repr quotes it.
fn json_key(name: &str) -> String {
    serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
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

/// Render a value at a composite position, recursing through `children` where
/// the owner is a Struct. `children` is empty for every other owner.
fn format_root(
    kind: &SchemaKind,
    children: &[(String, Arc<Schema<Unknown>>)],
    value: &Unknown,
) -> String {
    const NO_CHILDREN: &[(String, Arc<Schema<Unknown>>)] = &[];
    match kind {
        SchemaKind::Opaque("never") => "never".to_string(),
        SchemaKind::Struct(fields) if !fields.is_empty() => {
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, _)| {
                    let item = map.get(name).cloned().unwrap_or(Unknown::Null);
                    let key =
                        serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
                    let (child_kind, child_children) = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| (schema.kind.clone(), schema.children.clone()))
                        .unwrap_or_else(|| (SchemaKind::Object, Vec::new()));
                    format!("{key}: {}", format_root(&child_kind, &child_children, &item))
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        SchemaKind::Record(_, value_kind) => match value {
            Unknown::Object(map) => {
                let inner: Vec<String> = map
                    .iter()
                    .map(|(key, item)| {
                        let key =
                            serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                        format!("{key}: {}", format_root(value_kind, NO_CHILDREN, item))
                    })
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Tuple(elements) => match value {
            Unknown::Array(items) => {
                let mut inner: Vec<String> = Vec::new();
                for (index, element_kind) in elements.iter().enumerate() {
                    match element_kind {
                        SchemaKind::Rest(rest) => {
                            for item in items.iter().skip(index) {
                                inner.push(format_root(rest, NO_CHILDREN, item));
                            }
                            break;
                        }
                        _ => match items.get(index) {
                            Some(item) => inner.push(format_root(element_kind, NO_CHILDREN, item)),
                            None => break,
                        },
                    }
                }
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Array(item_kind) => match value {
            Unknown::Array(items) => {
                let inner: Vec<String> =
                    items.iter().map(|i| format_root(item_kind, NO_CHILDREN, i)).collect();
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Optional(inner) => match value {
            Unknown::Null => "null".to_string(),
            other => format_root(inner, NO_CHILDREN, other),
        },
        SchemaKind::Annotated { inner, .. } => format_root(inner, children, value),
        _ => format_value(value),
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

/// Render `value` through `kind`. A Struct with declared fields renders each
/// field as `"name": value` joined by ", ", and the same rule applies to every
/// nested Struct, which is why this walks the SCHEMA tree rather than the value
/// tree: the pin's nested output is `{ "a": "a", "as": [{ "a": "b" }] }`, with
/// the spacing on the inner objects too. An EMPTY Struct has no declared
/// fields, so it falls through to the plain JSON-ish rendering, which is what
/// the pin does too (its "empty" row asserts `{"a":1}` unspaced).
fn format_with(kind: &SchemaKind, value: &Unknown) -> String {
    match kind {
        SchemaKind::Opaque("never") => "never".to_string(),
        SchemaKind::Struct(fields) if !fields.is_empty() => {
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field_kind)| {
                    let item = map.get(name).cloned().unwrap_or(Unknown::Null);
                    let key = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}: {}", format_with(field_kind, &item))
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        SchemaKind::Record(_, value_kind) => match value {
            Unknown::Object(map) => {
                let inner: Vec<String> = map
                    .iter()
                    .map(|(key, item)| {
                        let key =
                            serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                        format!("{key}: {}", format_with(value_kind, item))
                    })
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Tuple(elements) => match value {
            Unknown::Array(items) => {
                let mut inner: Vec<String> = Vec::new();
                for (index, element_kind) in elements.iter().enumerate() {
                    match element_kind {
                        SchemaKind::Rest(inner_kind) => {
                            for item in items.iter().skip(index) {
                                inner.push(format_with(inner_kind, item));
                            }
                            break;
                        }
                        _ => match items.get(index) {
                            Some(item) => inner.push(format_with(element_kind, item)),
                            None => break,
                        },
                    }
                }
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Array(item_kind) => match value {
            Unknown::Array(items) => {
                let inner: Vec<String> =
                    items.iter().map(|i| format_with(item_kind, i)).collect();
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Optional(inner) => match value {
            Unknown::Null => "null".to_string(),
            other => format_with(inner, other),
        },
        SchemaKind::Annotated { inner, .. } => format_with(inner, value),
        _ => format_value(value),
    }
}
