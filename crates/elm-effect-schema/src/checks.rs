//! The pin's named checks, as filters for `Schema::filtered` (`Schema.check`).
//!
//! A named check is a `makeFilter` in the pin: it answers a message rather than
//! a boolean, and several of them in one `check(...)` run in order with the
//! first failure winning unless the caller collects them all. That is exactly
//! what [`FilterSpec`] and `Schema::filtered` already do, so each constructor
//! here is a predicate plus the pin's own `expected` text, carried as a
//! `FilterIssue` rather than an annotation a later `.annotated(..)` would
//! replace.
//!
//! The predicates themselves are not re-implemented: they are the ones the
//! JSON Schema import path already enforces ([`check_holds_arg`]), so a check
//! built here and a check imported from a document mean the same thing.
//!
//! [`check_holds_arg`]: crate::check_holds_arg

use crate::filters::{FilterIssue, FilterOutcome, FilterSpec};

/// The value passes, or the message is the failure.
fn named(name: &'static str, argument: String, message: String) -> FilterSpec {
    FilterSpec::new(move |value| {
        if crate::check_holds_arg(name, Some(&argument), value) {
            FilterOutcome::Pass
        } else {
            FilterOutcome::Fail(FilterIssue::new(message.clone()))
        }
    })
}

/// `Schema.isMinLength`: text at least `length` characters long (code points,
/// like the pin's `length`).
pub fn is_min_length(length: usize) -> FilterSpec {
    named(
        "isMinLength",
        length.to_string(),
        format!("Expected a value with a length of at least {length}"),
    )
}

/// `Schema.isIncluding`: text that contains `included`. The message quotes it
/// the way the pin's `JSON.stringify` does.
pub fn is_including(included: &str) -> FilterSpec {
    let message = format!("Expected a string including {}", quote_text(included));
    let argument = included.to_string();
    FilterSpec::new(move |value| {
        if contains_text(value, &argument) {
            FilterOutcome::Pass
        } else {
            FilterOutcome::Fail(FilterIssue::new(message.clone()))
        }
    })
}

/// The pin's `String.prototype.includes`, which a check answers for text and
/// refuses for anything else.
fn contains_text(value: &crate::Unknown, needle: &str) -> bool {
    matches!(value, crate::Unknown::Text(text) if text.contains(needle))
}

/// `Schema.isTrimmed`: no leading or trailing whitespace.
pub fn is_trimmed() -> FilterSpec {
    named(
        "isTrimmed",
        String::new(),
        "Expected a string with no leading or trailing whitespace".to_string(),
    )
}

/// `Schema.isStringBigInt`: text that reads as a bigint, `-?\d+` in the pin
/// (SchemaAST.ts:4868). The value itself is never parsed, so a bigint longer
/// than any integer type still passes.
pub fn is_string_big_int() -> FilterSpec {
    named(
        "isStringBigInt",
        String::new(),
        "Expected a string representing a bigint".to_string(),
    )
}

/// `Schema.isStringSymbol`: the `Symbol(description)` spelling Effect uses for
/// its symbol string encoding (SchemaAST.ts:4924). It is a shape test, not a
/// question about whether `Symbol()` would accept the text.
pub fn is_string_symbol() -> FilterSpec {
    named(
        "isStringSymbol",
        String::new(),
        "Expected a string representing a symbol".to_string(),
    )
}

/// `Schema.isPattern`: text the pattern matches. The pattern is a JavaScript
/// regular expression source, as the pin's `RegExp.source` is, and it is read
/// back through the same literal form a repr prints (`new RegExp(source,
/// flags)`); a pattern outside the [supported subset](crate::pattern) is
/// unenforced rather than guessed at.
pub fn is_pattern(pattern: &str) -> FilterSpec {
    is_pattern_with(pattern, "u")
}

/// `Schema.isPattern` with the pattern's flags spelled out, which is what a
/// pattern whose meaning depends on `s` or `y` needs.
pub fn is_pattern_with(pattern: &str, flags: &str) -> FilterSpec {
    let message = format!("Expected a string matching the RegExp {pattern}");
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    named(
        "isPattern",
        format!("new RegExp({}, {})", quote(pattern), quote(flags)),
        message,
    )
}

/// `Schema.isMultipleOf`: a number that is a whole multiple of `divisor`, with
/// a negative divisor normalized the way the pin normalizes it.
///
/// Panics for a divisor that is zero, negative zero, NaN, or infinite, which is
/// the pin's `RangeError` (mirroring the pin's own `throw`).
pub fn is_multiple_of(divisor: f64) -> FilterSpec {
    if !divisor.is_finite() || divisor == 0.0 {
        panic!(
            "Expected a finite non-zero number, got {}",
            crate::js_number_text(divisor)
        );
    }
    let divisor = divisor.abs();
    let printed = crate::js_number_text(divisor);
    named(
        "isMultipleOf",
        printed.clone(),
        format!("Expected a value that is a multiple of {printed}"),
    )
}

/// `Schema.makeFilterGroup`: several checks behind one annotation, as the chain
/// the pin's group holds.
///
/// ponytail: the pin's `FilterGroup` is a node of its own; the port's filter
/// chain is flat, so the group's `title`/`description` ride on the first member
/// and the members enforce exactly what they would on their own. Grow a group
/// node if a repr or a JSON Schema export ever has to show the group itself.
pub fn make_filter_group(
    mut members: Vec<FilterSpec>,
    title: &str,
    description: &str,
) -> Vec<FilterSpec> {
    if let Some(first) = members.first_mut() {
        first
            .annotation
            .push(("title".to_string(), title.to_string()));
        first
            .annotation
            .push(("description".to_string(), description.to_string()));
    }
    members
}

/// The pin's `JSON.stringify` spelling for a string.
fn quote_text(text: &str) -> String {
    format!("{text:?}")
}
