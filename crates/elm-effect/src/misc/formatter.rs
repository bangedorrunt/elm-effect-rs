//! `Formatter` port (`Formatter.ts` at the pin).
//!
//! The tag's `Formatter<Value, Format>` is a function interface; the runtime
//! surface is `formatUnknown`: render an unknown value for display. The Rust
//! spelling formats [`Json`](crate::json::Json), the shared unknown-value
//! model, reusing its TS-compatible rendering (compact `{"a":1}` shapes,
//! pretty `space` variant).
//!
//! Residuals (JS-runtime values, no Rust spelling): `symbol`, `bigint`,
//! `undefined`, functions, `Error`/`Date`/`RegExp` instances, `Map`/`Set`,
//! `FormData`, `Uint8Array`, `Redacted`, `Schema.Class`, circular/hostile
//! inputs (impossible in safe Rust: values are owned trees).

use super::json::Json;

/// A formatter from `Value` to `Format`. Mirrors the tag's interface.
pub trait Formatter<Value, Format = String> {
    /// Render `value`.
    fn format(&self, value: &Value) -> Format;
}

/// Render an unknown value compactly (`{"a":1}`, `[1,2]`, `"a"`, `123`).
pub fn format_unknown(value: &Json) -> String {
    value.to_json()
}

/// Render with `spaces` per level (`0` is compact).
pub fn format_unknown_pretty(value: &Json, spaces: usize) -> String {
    value.to_json_pretty(spaces)
}

/// A [`Formatter`] over [`Json`] with configurable indentation.
#[derive(Clone, Debug, Default)]
pub struct JsonFormatter {
    /// Spaces per level; `0` is compact.
    pub spaces: usize,
}

impl JsonFormatter {
    /// Compact formatter.
    pub fn new() -> Self {
        Self { spaces: 0 }
    }

    /// Pretty formatter with `spaces` per level.
    pub fn pretty(spaces: usize) -> Self {
        Self { spaces }
    }
}

impl Formatter<Json> for JsonFormatter {
    fn format(&self, value: &Json) -> String {
        value.to_json_pretty(self.spaces)
    }
}
