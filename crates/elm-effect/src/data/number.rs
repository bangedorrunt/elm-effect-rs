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

/// Divide, yielding `None` for zero divisors (including `-0.0`).
pub fn divide(a: f64, b: f64) -> Option<f64> {
    if b == 0.0 { None } else { Some(a / b) }
}

#[path = "bigint.rs"]
mod bigint;

use bigint::Big;

/// Integer and decimal remainders (`remainder`): integral operands use
/// the raw operator; anything else goes through exact decimal arithmetic
/// so `0.3 % 0.2` is `0.1`, with zero-sign and underflow rules intact.
/// Integer and decimal remainders (`remainder`): integral operands use
/// the raw operator; anything else goes through exact decimal arithmetic
/// so `0.3 % 0.2` is `0.1`, with zero-sign and underflow rules intact.
pub fn remainder(a: f64, b: f64) -> f64 {
    if !a.is_finite() || !b.is_finite() {
        return f64::NAN;
    }
    if a.fract() == 0.0 && b.fract() == 0.0 {
        return a % b;
    }
    let (mut self_coefficient, self_exponent) = match decimal_parts(a) {
        Some(pair) => pair,
        None => return f64::NAN,
    };
    let (mut divisor_coefficient, divisor_exponent) = match decimal_parts(b) {
        Some(pair) => pair,
        None => return f64::NAN,
    };
    if a.is_sign_negative() {
        self_coefficient = self_coefficient.negate();
    }
    if b.is_sign_negative() {
        divisor_coefficient = divisor_coefficient.negate();
    }
    let exponent = self_exponent.min(divisor_exponent);
    let scaled_self = self_coefficient.multiply(&pow10((self_exponent - exponent) as u32));
    let scaled_divisor = divisor_coefficient.multiply(&pow10((divisor_exponent - exponent) as u32));
    if scaled_divisor == Big::zero() {
        return f64::NAN;
    }
    let quotient = match scaled_self.divide(&scaled_divisor) {
        Some(quotient) => quotient,
        None => return f64::NAN,
    };
    let out = scaled_self.add(&quotient.negate().multiply(&scaled_divisor));
    if out == Big::zero() {
        return if a < 0.0 || (a == 0.0 && a.is_sign_negative()) {
            -0.0
        } else {
            0.0
        };
    }
    let text = format!("{out}e{exponent}");
    match text.parse::<f64>() {
        Ok(0.0) => {
            if a.is_sign_negative() {
                -f64::from_bits(1)
            } else {
                f64::from_bits(1)
            }
        }
        Ok(value) => value,
        Err(_) => f64::NAN,
    }
}

/// Split a finite float into unsigned decimal digits and a base-10
/// exponent: `value == digits * 10^exponent`. Returns `None` for
/// non-finite inputs.
fn decimal_parts(value: f64) -> Option<(Big, i32)> {
    if !value.is_finite() {
        return None;
    }
    if value.fract() == 0.0 {
        // BigInt(n): integers stay exact at any size.
        return Some((exact_integer(value.abs())?, 0));
    }
    // toExponential() digits for fractions (shortest exponential form).
    let text = format!("{:e}", value.abs());
    let (mantissa, exp) = text.split_once('e')?;
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let shift = exp.parse::<i32>().ok()?;
    let trimmed = digits.trim_start_matches('0');
    let coefficient = Big::from_string(if trimmed.is_empty() { "0" } else { trimmed })?;
    Some((coefficient, shift - digits.len() as i32 + 1))
}

/// Exact integer value of an integral float (`BigInt(n)` semantics).
fn exact_integer(value: f64) -> Option<Big> {
    let bits = value.to_bits();
    let raw_exponent = ((bits >> 52) & 0x7ff) as i32;
    let raw_mantissa = bits & 0xfffffffffffff;
    if raw_exponent == 0x7ff {
        return None;
    }
    let mantissa = if raw_exponent == 0 {
        raw_mantissa
    } else {
        raw_mantissa | 0x10000000000000
    };
    let exp2 = if raw_exponent == 0 {
        -1074
    } else {
        raw_exponent - 1075
    };
    let two = Big::from_int(2);
    let mut out = Big::from_int(mantissa as i64);
    if exp2 >= 0 {
        for _ in 0..exp2 {
            out = out.multiply(&two);
        }
    } else {
        for _ in 0..(-exp2) {
            out = out.divide(&two)?;
        }
    }
    Some(out)
}

/// `10^exp` as an exact big integer.
fn pow10(exp: u32) -> Big {
    Big::from_string(&format!("1{:0>e$}", "", e = exp as usize)).expect("decimal power")
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

/// Product of values, stopping at the first zero like the reference fold.
/// Empty multiplies to 1.
pub fn product(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut out = 1.0;
    for value in values {
        out *= value;
        if value == 0.0 {
            return out;
        }
    }
    out
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

// --- Wave-6 residuals (Order-dependent NaN rows; .17 owns `Order`) ---

use super::foundation::number_order;

/// Tag-order comparison: `NaN` equals `NaN`, `NaN` sorts below everything,
/// ties report `Greater` (`Number.Order`).
pub fn compare(a: f64, b: f64) -> std::cmp::Ordering {
    number_order().compare(&a, &b)
}

/// Tag equivalence: `NaN` equals `NaN`, `+0` equals `-0`.
pub fn equivalent(a: f64, b: f64) -> bool {
    super::foundation::number_equivalence().is_equivalent(&a, &b)
}

/// Order-aware clamp: `NaN` sorts below everything, so it clamps to `min`.
pub fn clamp_ordered(x: f64, min: f64, max: f64) -> f64 {
    if x.is_nan() { min } else { x.clamp(min, max) }
}

/// Order-based less-than (delegates to `Number.Order`, so `NaN < x`).
pub fn less_than(a: f64, b: f64) -> bool {
    compare(a, b) == std::cmp::Ordering::Less
}

/// `true` when `a` is strictly greater than `b`.
pub fn greater_than(a: f64, b: f64) -> bool {
    a > b
}

/// `true` when `a` is smaller than or equal to `b`.
pub fn less_than_or_equal(a: f64, b: f64) -> bool {
    a <= b
}

/// `true` when `a` is greater than or equal to `b`.
pub fn greater_than_or_equal(a: f64, b: f64) -> bool {
    a >= b
}
