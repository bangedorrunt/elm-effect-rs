//! `Schema.ByteSize`, its three wire spellings, and `Schema.BigDecimal`.
//!
//! These live here rather than in the schema crate because they speak for this
//! crate's own `ByteSize` and `Decimal`, which that crate cannot name. `BigInt`
//! spellings are absent for a reason of their own: `Unknown` has no
//! arbitrary-precision integer variant, so `ByteSizeFromBigInt` has no wire
//! form in this port (the ledger excuses the row beside `DurationFromNanos`,
//! which is blocked by the same gap).
//!
//! A BigDecimal check runs over the value's printed spelling. That is exact:
//! [`Decimal::format`] normalizes and [`Decimal::from_string`] reads back what
//! it prints, so comparing the text and comparing the value answer the same.

use elm_effect_schema::{
    FilterIssue, FilterOutcome, FilterSpec, Schema, SchemaError, Unknown, transform::Transformation,
};

use crate::bigdecimal::Decimal;
use crate::byte_size::ByteSize;

/// The pin's `Number.MAX_SAFE_INTEGER`, which is both the largest byte count a
/// number wire can carry and the largest count `ByteSize::bytes_float` reads.
const MAX_SAFE: i128 = 9_007_199_254_740_991;

/// The port's refusal for a wire value that is not the shape the schema takes.
fn refused(expected: &'static str, got: &'static str) -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected,
        got,
    }
}

/// `Schema.ByteSize`: the wire is the exact byte count in decimal text, which
/// is what the pin's `toCodecJson` prints for the bigint it carries.
///
/// The count is read at `i128` precision, not through a double, because the
/// row that proves it uses 2^53 + 1, a byte count no double can hold.
pub fn byte_size() -> Schema<ByteSize> {
    Schema::convert(
        Schema::<String>::string(),
        |text: String| {
            let count: i128 = text
                .parse()
                .map_err(|_| refused("a decimal byte count", "string"))?;
            ByteSize::bytes(count).map_err(|_| refused("a non-negative byte count", "string"))
        },
        |size: &ByteSize| size.0.to_string(),
    )
}

/// `Schema.ByteSizeFromString`: a byte-size string on the wire (`"1.5 kB"`),
/// and the pin's own `<count> byte(s)` spelling on the way out.
pub fn byte_size_from_string() -> Schema<ByteSize> {
    Schema::convert(
        Schema::<String>::string(),
        |text: String| {
            ByteSize::from_input(&text).ok_or_else(|| refused("a valid ByteSize string", "string"))
        },
        |size: &ByteSize| byte_size_spelling(size),
    )
}

/// `Schema.ByteSizeFromNumber`: a count on the wire, safe-integer range only.
/// Encoding refuses a larger count rather than printing a rounded one.
pub fn byte_size_from_number() -> Schema<ByteSize> {
    Schema::decode_to(
        Schema::<f64>::number(),
        Transformation::new(
            |count: f64| {
                if !count.is_finite()
                    || count.fract() != 0.0
                    || !(0.0..=MAX_SAFE as f64).contains(&count)
                {
                    return Err(refused("a non-negative safe-integer byte count", "number"));
                }
                Ok(ByteSize(count as i128))
            },
            |size: ByteSize| {
                if size.0 > MAX_SAFE {
                    return Err(SchemaError::EncodingFailed {
                        path: "$".to_string(),
                        expected: "a ByteSize representable as a safe integer".to_string(),
                    });
                }
                Ok(size.0 as f64)
            },
        ),
    )
}

/// The pin's `ByteSize.toString`: plural except at exactly one byte.
fn byte_size_spelling(size: &ByteSize) -> String {
    if size.0 == 1 {
        "1 byte".to_string()
    } else {
        format!("{} bytes", size.0)
    }
}

/// `Schema.BigDecimal`: the wire is the decimal's printed spelling, which is
/// the pin's own JSON serialization for the type.
pub fn bigdecimal() -> Schema<Decimal> {
    Schema::convert(
        Schema::<String>::string(),
        |text: String| {
            Decimal::from_string(&text)
                .ok_or_else(|| refused("a valid BigDecimal string", "string"))
        },
        |value: &Decimal| value.format(),
    )
}

/// `Schema.BigDecimalFromString`: the pin's parse-a-string form.
///
/// The pin keeps two schemas here: `BigDecimal` admits a decimal value and
/// serializes it as text, `BigDecimalFromString` parses text into one. In this
/// port `Unknown` cannot hold a decimal, so `BigDecimal`'s wire already is the
/// string and one codec answers for both. The name stays so a ported call site
/// reads like the pin's.
pub fn bigdecimal_from_string() -> Schema<Decimal> {
    bigdecimal()
}

/// A check over a decimal's printed spelling, answering the pin's own message.
///
/// A spelling the base schema cannot read is passed through: the transform
/// already refused it, and reporting it twice would name the wrong refusal.
fn decimal_check(
    message: String,
    predicate: impl Fn(&Decimal) -> bool + Send + Sync + 'static,
) -> FilterSpec {
    FilterSpec::new(move |wire| match wire {
        Unknown::Text(text) => match Decimal::from_string(text) {
            Some(value) if !predicate(&value) => {
                FilterOutcome::Fail(FilterIssue::new(message.clone()))
            }
            _ => FilterOutcome::Pass,
        },
        _ => FilterOutcome::Pass,
    })
}

/// `Schema.isGreaterThanBigDecimal`.
pub fn is_greater_than_bigdecimal(exclusive_minimum: Decimal) -> FilterSpec {
    decimal_check(
        format!(
            "Expected a value greater than {}",
            exclusive_minimum.format()
        ),
        move |value| *value > exclusive_minimum,
    )
}

/// `Schema.isGreaterThanOrEqualToBigDecimal`.
pub fn is_greater_than_or_equal_to_bigdecimal(minimum: Decimal) -> FilterSpec {
    decimal_check(
        format!(
            "Expected a value greater than or equal to {}",
            minimum.format()
        ),
        move |value| *value >= minimum,
    )
}

/// `Schema.isLessThanBigDecimal`.
pub fn is_less_than_bigdecimal(exclusive_maximum: Decimal) -> FilterSpec {
    decimal_check(
        format!("Expected a value less than {}", exclusive_maximum.format()),
        move |value| *value < exclusive_maximum,
    )
}

/// `Schema.isLessThanOrEqualToBigDecimal`.
pub fn is_less_than_or_equal_to_bigdecimal(maximum: Decimal) -> FilterSpec {
    decimal_check(
        format!(
            "Expected a value less than or equal to {}",
            maximum.format()
        ),
        move |value| *value <= maximum,
    )
}

/// `Schema.isBetweenBigDecimal`, both ends inclusive.
pub fn is_between_bigdecimal(minimum: Decimal, maximum: Decimal) -> FilterSpec {
    decimal_check(
        format!(
            "Expected a value between {} and {}",
            minimum.format(),
            maximum.format()
        ),
        move |value| *value >= minimum && *value <= maximum,
    )
}
