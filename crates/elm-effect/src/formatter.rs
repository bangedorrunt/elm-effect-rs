//! `Formatter.ts` behavior port: human-readable [`format`] and valid-JSON
//! [`format_json`] over the [`Json`] value model.
//!
//! The spec formats arbitrary JavaScript values (circular refs, `BigInt`,
//! class instances, `Error`s, `Set`/`Map`, `Redactable`); this port covers
//! the JSON value model, where `format` renders compact JSON and
//! `format_json` renders valid JSON with optional indentation. JS-only rows
//! (circular refs, class instances, `toString` hooks, `SchemaIssue`
//! reports) are skipped-upstream in the ledger.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use super::json::Json;

/// A formatting function from `Value` to `Format` (default `String`).
/// Ports the `Formatter` callable interface as a plain function pointer.
pub type FormatterFn<Value, Format = String> = fn(Value) -> Format;

/// Render a value compactly for humans. On the JSON model this is compact
/// JSON (`{"a":1,"b":[2,3]}`); `indent` spaces per level when nonzero.
pub fn format(value: &Json, indent: usize) -> String {
    value.to_json_pretty(indent)
}

/// Render a value as valid JSON, silently dropping nothing (the model has
/// no circular refs, `BigInt`, or foreign objects). `indent` spaces per
/// level; `0` is compact.
pub fn format_json(value: &Json, indent: usize) -> String {
    value.to_json_pretty(indent)
}
