//! Check-tails fence: `isMultipleOf` divisor edges and the `isBetween` bounds.
//! Derives from the checks region of packages/effect/test/schema/Schema.test.ts
//! (isMultipleOf tails at 1659/1665, the isBetween quartet at 1687-1762).
//!
//! Encode-side rejections are not reproduced: this crate's `encode` is total,
//! the same convention the declaration cells use.

use std::collections::BTreeMap;

use elm_effect_schema::{Field, Schema, Unknown};

fn number(value: f64) -> Unknown {
    Unknown::Number(value)
}

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    )
}

/// The pin's `Record(String, Finite)`: an open object of numbers.
fn record_of_numbers() -> Schema<Unknown> {
    Schema::as_unknown(Schema::record(Schema::as_unknown(Schema::number())))
}

/// The pin's `Schema.Int`: a number that is whole.
fn int_base() -> Schema<Unknown> {
    Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isInt",
        vec![],
        vec![],
    )
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
    assert_eq!(schema.decode(number(2e-323)).unwrap(), number(2e-323));
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
    let schema = Schema::checked(int_base(), "isBetween", vec!["1", "3"], vec![]);

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

fn checked_string(name: &str, args: Vec<&str>) -> Schema<Unknown> {
    Schema::checked(Schema::as_unknown(Schema::string()), name, args, vec![])
}

/// Schema.test.ts:1427 `isPattern`.
#[test]
fn is_pattern_checks_the_string() {
    let schema = checked_string("isPattern", vec![r#"new RegExp("^a", "u")"#]);

    assert_eq!(
        schema.decode(Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
    assert!(schema.decode(Unknown::Text("b".to_string())).is_err());
    assert_eq!(
        schema.try_encode(&Unknown::Text("a".to_string())).unwrap(),
        Unknown::Text("a".to_string())
    );
}

/// Schema.test.ts:1446 `isPattern with stateful RegExp flags`. The pin's point
/// is that a `g`/`y` flag must not make matching stateful: the same input
/// matches again, and a mismatch still fails.
#[test]
fn is_pattern_is_stateless_across_decodes() {
    let schema = checked_string("isPattern", vec![r#"new RegExp("^a", "u")"#]);

    assert!(schema.decode(Unknown::Text("a".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("a".to_string())).is_ok());
    assert!(schema.decode(Unknown::Text("b".to_string())).is_err());
}

/// Schema.test.ts:1592 `isGreaterThan`.
#[test]
fn is_greater_than_checks_the_bound() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isGreaterThan",
        vec!["1"],
        vec![],
    );

    assert_eq!(schema.decode(number(2.0)).unwrap(), number(2.0));
    assert!(schema.decode(number(1.0)).is_err());
}

/// Schema.test.ts:1611 `isGreaterThanOrEqualTo`.
#[test]
fn is_greater_than_or_equal_to_checks_the_bound() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isGreaterThanOrEqualTo",
        vec!["1"],
        vec![],
    );

    assert_eq!(schema.decode(number(1.0)).unwrap(), number(1.0));
    assert!(schema.decode(number(0.0)).is_err());
}

/// Schema.test.ts:1623 `isLessThan`.
#[test]
fn is_less_than_checks_the_bound() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isLessThan",
        vec!["1"],
        vec![],
    );

    assert_eq!(schema.decode(number(0.0)).unwrap(), number(0.0));
    assert!(schema.decode(number(1.0)).is_err());
}

/// Schema.test.ts:1635 `isLessThanOrEqualTo`.
#[test]
fn is_less_than_or_equal_to_checks_the_bound() {
    let schema = Schema::checked(
        Schema::as_unknown(Schema::number()),
        "isLessThanOrEqualTo",
        vec!["1"],
        vec![],
    );

    assert_eq!(schema.decode(number(1.0)).unwrap(), number(1.0));
    assert!(schema.decode(number(2.0)).is_err());
}

/// Schema.test.ts:1940 `isMinProperties`.
#[test]
fn is_min_properties_counts_entries() {
    let schema = Schema::checked(record_of_numbers(), "isMinProperties", vec!["1"], vec![]);

    assert!(
        schema
            .decode(obj(&[("a", number(1.0)), ("b", number(2.0))]))
            .is_ok()
    );
    assert!(schema.decode(Unknown::Object(BTreeMap::new())).is_err());
}

/// Schema.test.ts:1952 `isMaxProperties`.
#[test]
fn is_max_properties_counts_entries() {
    let schema = Schema::checked(record_of_numbers(), "isMaxProperties", vec!["2"], vec![]);

    assert!(
        schema
            .decode(obj(&[("a", number(1.0)), ("b", number(2.0))]))
            .is_ok()
    );
    assert!(
        schema
            .decode(obj(&[
                ("a", number(1.0)),
                ("b", number(2.0)),
                ("c", number(3.0))
            ]))
            .is_err()
    );
}

/// Schema.test.ts:2000 `isBetweenProperties`.
#[test]
fn is_between_properties_counts_entries() {
    let schema = Schema::checked(
        record_of_numbers(),
        "isBetweenProperties",
        vec!["2", "2"],
        vec![],
    );

    assert!(
        schema
            .decode(obj(&[("a", number(1.0)), ("b", number(2.0))]))
            .is_ok()
    );
    // The pin's second success case: its odd keys are ordinary string keys here.
    assert!(
        schema
            .decode(obj(&[("__proto__", number(0.0)), ("", number(0.0))]))
            .is_ok()
    );
    assert!(schema.decode(obj(&[("a", number(1.0))])).is_err());
    assert!(
        schema
            .decode(obj(&[
                ("a", number(1.0)),
                ("b", number(2.0)),
                ("c", number(3.0))
            ]))
            .is_err()
    );
}

/// Schema.test.ts:2068 `UniqueArray`.
#[test]
fn unique_array_rejects_structural_duplicates() {
    let item = Schema::as_unknown(Schema::struct_shape(vec![
        Field::required("a", Schema::as_unknown(Schema::string())),
        Field::required("b", Schema::as_unknown(Schema::string())),
    ]));
    let schema = Schema::checked(
        Schema::as_unknown(Schema::array(item)),
        "isUnique",
        vec![],
        vec![],
    );

    let row = |a: &str, b: &str| {
        obj(&[
            ("a", Unknown::Text(a.to_string())),
            ("b", Unknown::Text(b.to_string())),
        ])
    };
    assert!(
        schema
            .decode(Unknown::Array(vec![row("a", "b"), row("c", "d")]))
            .is_ok()
    );
    assert!(
        schema
            .decode(Unknown::Array(vec![row("a", "b"), row("a", "b")]))
            .is_err()
    );
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

/// Schema.test.ts:1306 `single check`: one refinement on a string accepts what
/// satisfies it and refuses what does not. The pin writes
/// `Schema.String.check(Schema.isMinLength(3))`; this port spells a check as
/// its name plus arguments on `Schema::checked`, the same pair `checked_string`
/// builds for every other refinement in this file.
#[test]
fn a_single_check_refuses_a_short_string() {
    let schema = checked_string("isMinLength", vec!["3"]);
    let long = Unknown::Text("abc".to_string());
    let short = Unknown::Text("ab".to_string());

    assert_eq!(schema.decode(long.clone()).unwrap(), long);
    assert!(schema.decode(short).is_err());
}
