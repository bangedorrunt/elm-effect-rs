//! `Boolean.ts` port: the full combinator set. Rust's operators are the
//! substrate; these names mirror the tag's data-first API (data-last pipe
//! forms are the same functions: argument order is fixed in Rust).

/// Whether the value is a boolean (mirrors `isBoolean`; always true: the
/// type already narrows, so this pins the row, not a check).
pub fn is_boolean(_: bool) -> bool {
    true
}

/// Coerce with Boolean semantics (mirrors the global `Boolean`
/// constructor; identity on `bool`).
pub fn coerce(value: bool) -> bool {
    value
}

/// Conjunction.
pub fn and_(a: bool, b: bool) -> bool {
    a && b
}

/// Disjunction.
pub fn or_(a: bool, b: bool) -> bool {
    a || b
}

/// Negation.
pub fn not(a: bool) -> bool {
    !a
}

/// Exclusive or.
pub fn xor(a: bool, b: bool) -> bool {
    a ^ b
}

/// Implication: `false` implies anything.
pub fn implies(a: bool, b: bool) -> bool {
    !a || b
}

/// Fold both cases.
pub fn match_bool<T>(value: bool, on_false: impl FnOnce() -> T, on_true: impl FnOnce() -> T) -> T {
    if value { on_true() } else { on_false() }
}

/// All true. Vacuously true on empty; short-circuits.
pub fn all<I: IntoIterator<Item = bool>>(iter: I) -> bool {
    iter.into_iter().all(|b| b)
}

/// Any true. Vacuously false on empty; short-circuits.
pub fn any<I: IntoIterator<Item = bool>>(iter: I) -> bool {
    iter.into_iter().any(|b| b)
}
