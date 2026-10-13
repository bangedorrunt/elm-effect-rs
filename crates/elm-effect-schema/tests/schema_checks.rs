//! Check-tails fence: `isMultipleOf` divisor edges and the `isBetween` bounds.
//! Derives from the checks region of packages/effect/test/schema/Schema.test.ts
//! (isMultipleOf tails at 1659/1665, the isBetween quartet at 1687-1762).
//!
//! Encode-side rejections are not reproduced: this crate's `encode` is total,
//! the same convention the declaration cells use.

use elm_effect_schema::{Schema, Unknown};

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

/// The pin's `Schema.Int`: a number that is whole.
fn int_base() -> Schema<Unknown> {
    Schema::checked(Schema::as_unknown(Schema::number()), "isInt", vec![], vec![])
}

fn multiple_of(divisor: &str) -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isMultipleOf",
        vec![divisor],
        vec![],
    )
}

/// Schema.test.ts:1659 `isMultipleOf rejects nonzero subnormal remainders`.
#[test]
fn is_multiple_of_rejects_subnormal_remainders() {
    let schema = multiple_of("1e-323");

    // 1.042e-321 / 1e-323 is 104.2: a remainder, not a multiple.
    assert!(schema.decode(number(1.042e-321)).is_err());
    // The control: twice the divisor is a multiple.
    assert_eq!(
        schema.decode(number(2e-323)).unwrap(),
        number(2e-323)
    );
}

/// Schema.test.ts:1665 `isMultipleOf normalizes negative divisors`.
#[test]
fn is_multiple_of_normalizes_negative_divisors() {
    let schema = multiple_of("-2");

    assert_eq!(schema.decode(number(4.0)).unwrap(), number(4.0));
    assert!(schema.decode(number(3.0)).is_err());
}

/// Schema.test.ts:1687 `included & included`.
#[test]
fn is_between_includes_both_bounds() {
    let schema = Schema::checked(
        int_base(),
        "isBetween",
        vec!["1", "3"],
        vec![],
    );

    assert_eq!(schema.decode(number(1.0)).unwrap(), number(1.0));
    assert_eq!(schema.decode(number(3.0)).unwrap(), number(3.0));
    assert!(schema.decode(number(0.0)).is_err());
    assert!(schema.decode(number(4.0)).is_err());
    // The base's own check still runs: 2.5 is in range but not whole.
    assert!(schema.decode(number(2.5)).is_err());
}

/// Schema.test.ts:1712 `included & excluded`.
#[test]
fn is_between_excludes_the_maximum() {
    let schema = Schema::checked(
        int_base(),
        "isBetween",
        vec!["1", "3"],
        vec![("exclusiveMaximum", "true")],
    );

    assert_eq!(schema.decode(number(1.0)).unwrap(), number(1.0));
    assert!(schema.decode(number(3.0)).is_err());
    assert!(schema.decode(number(0.0)).is_err());
    assert!(schema.decode(number(4.0)).is_err());
}

/// Schema.test.ts:1737 `excluded & included`.
#[test]
fn is_between_excludes_the_minimum() {
    let schema = Schema::checked(
        int_base(),
        "isBetween",
        vec!["1", "3"],
        vec![("exclusiveMinimum", "true")],
    );

    assert!(schema.decode(number(1.0)).is_err());
    assert_eq!(schema.decode(number(3.0)).unwrap(), number(3.0));
    assert!(schema.decode(number(0.0)).is_err());
    assert!(schema.decode(number(4.0)).is_err());
}

/// Schema.test.ts:1762 `excluded & excluded`.
#[test]
fn is_between_excludes_both_bounds() {
    let schema = Schema::checked(
        int_base(),
        "isBetween",
        vec!["1", "3"],
        vec![("exclusiveMinimum", "true"), ("exclusiveMaximum", "true")],
    );

    assert_eq!(schema.decode(number(2.0)).unwrap(), number(2.0));
    assert!(schema.decode(number(1.0)).is_err());
    assert!(schema.decode(number(3.0)).is_err());
    assert!(schema.decode(number(0.0)).is_err());
    assert!(schema.decode(number(4.0)).is_err());
}
