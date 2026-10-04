//! Shared JSON value model for the `JsonPointer` / `JsonPatch` ports.
//!
//! [`Json`] is the Rust spelling of `Schema.Json`: null, boolean, number
//! (one `f64`, like JavaScript), string, array, and object. Objects use a
//! [`BTreeMap`] so key order is stable and patches are deterministic.
//! Values are ordinary immutable data: "does not mutate" rows hold because
//! every operation takes `&Json`, and "preserves references" rows hold as
//! clone-equality.

use std::collections::BTreeMap;

/// A JSON value. Ordinary immutable data.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// JSON null.
    Null,
    /// JSON true / false.
    Bool(bool),
    /// JSON number (one `f64`, like JavaScript).
    Num(f64),
    /// JSON string.
    Str(String),
    /// JSON array.
    Arr(Vec<Json>),
    /// JSON object with stable (sorted) key order.
    Obj(BTreeMap<String, Json>),
}

impl Json {
    /// True for [`Json::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Structural equality with `NaN` treated as equal to `NaN`, so
    /// "structurally equal" rows hold for every representable value.
    pub fn structurally_equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Num(a), Self::Num(b)) => a == b || (a.is_nan() && b.is_nan()),
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Arr(a), Self::Arr(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.structurally_equal(y))
            }
            (Self::Obj(a), Self::Obj(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|((ka, va), (kb, vb))| ka == kb && va.structurally_equal(vb))
            }
            _ => false,
        }
    }

    /// The JSON type tag: null, boolean, number, string, array, object.
    /// Drives "root replace for type changes".
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Num(_) => "number",
            Self::Str(_) => "string",
            Self::Arr(_) => "array",
            Self::Obj(_) => "object",
        }
    }

    /// Render as valid compact JSON.
    pub fn to_json(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(true) => "true".into(),
            Self::Bool(false) => "false".into(),
            Self::Num(n) => {
                if n.is_nan() || n.is_infinite() {
                    "null".into()
                } else if n.fract() == 0.0 && n.abs() < 1e21 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Self::Str(s) => quoted(s),
            Self::Arr(items) => {
                let mut out = String::from("[");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&item.to_json());
                }
                out.push(']');
                out
            }
            Self::Obj(map) => {
                let mut out = String::from("{");
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&quoted(k));
                    out.push(':');
                    out.push_str(&v.to_json());
                }
                out.push('}');
                out
            }
        }
    }

    /// Render as valid JSON with `indent` spaces per level (`0` is compact).
    pub fn to_json_pretty(&self, indent: usize) -> String {
        self.pretty(indent, 0)
    }

    fn pretty(&self, indent: usize, depth: usize) -> String {
        if indent == 0 {
            return self.to_json();
        }
        let pad = |d: usize| " ".repeat(indent * d);
        match self {
            Self::Arr(items) if items.is_empty() => "[]".into(),
            Self::Obj(map) if map.is_empty() => "{}".into(),
            Self::Arr(items) => {
                let mut out = String::from("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push_str(&item.pretty(indent, depth + 1));
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(depth));
                out.push(']');
                out
            }
            Self::Obj(map) => {
                let mut out = String::from("{\n");
                for (i, (k, v)) in map.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push_str(&quoted(k));
                    out.push_str(": ");
                    out.push_str(&v.pretty(indent, depth + 1));
                    if i + 1 < map.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(depth));
                out.push('}');
                out
            }
            _ => self.to_json(),
        }
    }
}

/// JSON-quote a string (`"` delimiters, minimal escapes).
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
