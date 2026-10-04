//! `Number.ts` port: `f64` is the substrate.
//!
//! Arithmetic operators are Rust's. What's here: parsing, rounding with the
//! tag's half-up semantics (Rust's `round` rounds half away from zero, which
//! differs for negatives), clamping, ranges, and aggregates. Out of scope:
//! `Number.Order`-dependent rows (.17 owns `Order`; `f64::clamp` propagates
//! NaN instead of ordering it least).
use std::num::ParseFloatError;

/// Always true for `f64` (the type already narrows; mirrors `isNumber`).
pub fn is_number(_: f64) -> bool {
    true
}

/// Parse with float semantics (`"NaN"`/`"inf"` parse; garbage is `None`).
pub fn parse(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

/// Strict parse errors for callers that need them.
pub fn parse_strict(s: &str) -> Result<f64, ParseFloatError> {
    s.trim().parse::<f64>()
}

pub fn add(a: f64, b: f64) -> f64 {
    a + b
}
pub fn sub(a: f64, b: f64) -> f64 {
    a - b
}
pub fn mul(a: f64, b: f64) -> f64 {
    a * b
}
pub fn div(a: f64, b: f64) -> f64 {
    a / b
}

/// Integer and decimal remainders (`remainder`).
pub fn remainder(a: f64, b: f64) -> f64 {
    a % b
}

/// Round half up (the tag's `round`): `round(2.5) == 3`, `round(-2.5) == -2`.
/// This differs from `f64::round` (half away from zero) on negatives.
pub fn round(x: f64) -> f64 {
    (x + 0.5).floor()
}

pub fn floor(x: f64) -> f64 {
    x.floor()
}
pub fn ceil(x: f64) -> f64 {
    x.ceil()
}

/// Clamp to `[min, max]`. NaN propagates (Rust semantics); the tag orders
/// NaN least via `Number.Order`, owned by .17.
pub fn clamp(x: f64, min: f64, max: f64) -> f64 {
    x.clamp(min, max)
}

pub fn min(a: f64, b: f64) -> f64 {
    a.min(b)
}
pub fn max(a: f64, b: f64) -> f64 {
    a.max(b)
}

/// Sign: -1, 0, or 1 (NaN yields NaN, like the tag).
pub fn sign(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// Inclusive range check (`between`).
pub fn between(x: f64, min: f64, max: f64) -> bool {
    min <= x && x <= max
}

pub fn increment(x: f64) -> f64 {
    x + 1.0
}
pub fn decrement(x: f64) -> f64 {
    x - 1.0
}

/// Sum of a slice (`sum`); empty sums to 0.
pub fn sum(values: &[f64]) -> f64 {
    values.iter().sum()
}

/// Mean of a slice (`avg`); empty is `None`.
pub fn avg(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(sum(values) / values.len() as f64)
    }
}

pub fn abs(x: f64) -> f64 {
    x.abs()
}
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}
