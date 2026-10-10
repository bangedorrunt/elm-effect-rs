//! `Data.ts` port: structural data with tags and variants.
//!
//! [`Struct`] is a field bag with structural equality (Effect `Equal`),
//! field enumeration, and pipe-style chaining. [`Tagged`] adds a `_tag`.
//! [`Variant`] is one tagged case of a `taggedEnum`: [`is`] checks the tag,
//! [`match_variant`] dispatches on it (data-first and data-last forms).
//! [`TaggedError`] is a `std::error::Error` with a tag, message, and
//! captured backtrace; `name()` returns the tag like JS `error.name`.
//!
//! Deliberate divergence: no classes (Rust structs + constructors), no
//! prototype chain (`instanceof Error` is the `Error` trait), `toJSON`
//! is [`Struct::to_json_fields`] (key/value pairs; full JSON needs
//! `serde_json`, owned by the schema wave).
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::collections::BTreeMap;

/// A field value: enough shapes for the ported rows, no JSON dependency.
/// `NaN == NaN` here, like Effect `Equal` (not IEEE).
#[derive(Clone, Debug)]
pub enum Field {
    /// String field.
    Str(String),
    /// Integer field.
    Int(i64),
    /// Float field (`NaN == NaN` here, like Effect `Equal`).
    Float(f64),
    /// Boolean field.
    Bool(bool),
    /// Nested struct field.
    Struct(Struct),
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b || (a.is_nan() && b.is_nan()),
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Struct(a), Self::Struct(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Field {}

impl PartialEq for Struct {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.fields == other.fields
    }
}

impl Eq for Struct {}

/// Structural data: optional `_tag` plus an ordered field bag.
#[derive(Clone, Debug)]
pub struct Struct {
    tag: Option<String>,
    fields: BTreeMap<String, Field>,
}

impl Struct {
    /// Build from field pairs (`new Person({ name })`).
    pub fn new(fields: impl IntoIterator<Item = (impl Into<String>, Field)>) -> Self {
        Self {
            tag: None,
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// Build with a `_tag` (`new TaggedPerson({ _tag, ... })`).
    pub fn tagged(
        tag: impl Into<String>,
        fields: impl IntoIterator<Item = (impl Into<String>, Field)>,
    ) -> Self {
        Self {
            tag: Some(tag.into()),
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// The `_tag`, if any.
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// Field names in order (fields are enumerable).
    pub fn field_names(&self) -> Vec<&str> {
        self.fields.keys().map(String::as_str).collect()
    }

    /// Read a field.
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }

    /// Pipe-style chaining (`data.pipe(f)` is `f(data)`).
    pub fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }

    /// `key=value` pairs for JSON-ish output (`toJSON` shape).
    pub fn to_json_fields(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(tag) = &self.tag {
            out.push(("_tag".to_string(), format!("{tag:?}")));
        }
        for (k, v) in &self.fields {
            out.push((k.clone(), field_json(v)));
        }
        out
    }
}

fn field_json(f: &Field) -> String {
    match f {
        Field::Str(s) => format!("{s:?}"),
        Field::Int(n) => n.to_string(),
        Field::Float(x) => {
            if x.is_nan() {
                "\"NaN\"".to_string()
            } else {
                x.to_string()
            }
        }
        Field::Bool(b) => b.to_string(),
        Field::Struct(s) => {
            let inner: Vec<String> = s
                .to_json_fields()
                .into_iter()
                .map(|(k, v)| format!("{k:?}:{v}"))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// One case of a tagged enum (`taggedEnum` variant).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    tag: String,
    fields: BTreeMap<String, Field>,
}

impl Variant {
    /// Build one variant (`Circle({ radius })` with `_tag: "Circle"`).
    pub fn new(
        tag: impl Into<String>,
        fields: impl IntoIterator<Item = (impl Into<String>, Field)>,
    ) -> Self {
        Self {
            tag: tag.into(),
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// Build a fieldless variant (`None({})`).
    pub fn empty(tag: impl Into<String>) -> Self {
        Self::new(tag, [] as [(String, Field); 0])
    }

    /// The variant tag.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Read a field.
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }
}

/// Tag check (`$is("Circle")`).
pub fn is(tag: &str, variant: &Variant) -> bool {
    variant.tag() == tag
}

/// Dispatch on the tag, data-first (`$match`).
pub fn match_variant<R>(
    variant: &Variant,
    cases: &[(&str, Box<dyn Fn(&Variant) -> R>)],
    otherwise: impl Fn(&Variant) -> R,
) -> R {
    for (tag, f) in cases {
        if variant.tag() == *tag {
            return f(variant);
        }
    }
    otherwise(variant)
}

/// Dispatch on the tag, data-last (cases first, variant last).
pub fn match_variant_last<R>(
    cases: &[(&str, Box<dyn Fn(&Variant) -> R>)],
    otherwise: impl Fn(&Variant) -> R,
    variant: &Variant,
) -> R {
    match_variant(variant, cases, otherwise)
}

/// A tagged error with message, tag-as-name, and backtrace.
#[derive(Debug)]
pub struct TaggedError {
    tag: String,
    message: String,
    backtrace: std::backtrace::Backtrace,
    fields: BTreeMap<String, Field>,
}

impl TaggedError {
    /// Build (`new MyError({ message, ... })`).
    pub fn new(
        tag: impl Into<String>,
        message: impl Into<String>,
        fields: impl IntoIterator<Item = (impl Into<String>, Field)>,
    ) -> Self {
        Self {
            tag: tag.into(),
            message: message.into(),
            backtrace: std::backtrace::Backtrace::capture(),
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// The tag; JS `error.name`.
    pub fn name(&self) -> &str {
        &self.tag
    }

    /// The tag.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Captured creation backtrace (JS `stack`).
    pub fn stack(&self) -> String {
        self.backtrace.to_string()
    }

    /// Read an extra field.
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }
}

impl PartialEq for TaggedError {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.message == other.message && self.fields == other.fields
    }
}

impl Eq for TaggedError {}

impl std::fmt::Display for TaggedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.tag, self.message)
    }
}

impl std::error::Error for TaggedError {}
