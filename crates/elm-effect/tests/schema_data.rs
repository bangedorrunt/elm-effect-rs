//! `Schema.test.ts` rows for the ByteSize and BigDecimal schema families.
//!
//! These witness the schemas in `crates/elm-effect/src/schema_data.rs`, which
//! live in this crate because they speak for `ByteSize` and `Decimal`.
//! `ByteSizeFromBigInt` (`Schema.test.ts:5777`) is excused in the ledger: its
//! wire is a bigint, and `Unknown` has no arbitrary-precision integer variant,
//! the same gap that excuses `DurationFromNanos`.
//!
//!     cargo test -p elm-effect --features schema --test schema_data

use elm_effect::{
    ByteSize, Decimal, Schema, Unknown, bigdecimal, byte_size, byte_size_from_number,
    byte_size_from_string, is_between_bigdecimal, is_greater_than_bigdecimal,
    is_greater_than_or_equal_to_bigdecimal, is_less_than_bigdecimal,
    is_less_than_or_equal_to_bigdecimal,
};
use elm_effect_schema::FilterSpec;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn decimal(value: &str) -> Decimal {
    Decimal::make_unsafe(value)
}

fn checked(spec: FilterSpec) -> Schema<Decimal> {
    Schema::filtered_with(bigdecimal(), vec![spec])
}

/// The message a refused decimal decoding reports.
fn refusal(schema: &Schema<Decimal>, wire: &str) -> String {
    schema
        .decode(text(wire))
        .expect_err("the check refuses this value")
        .to_string()
}

/// `Schema.test.ts:5761` — "ByteSize": the wire is the exact count in decimal
/// text, at a precision no double can hold.
#[test]
fn byte_size_speaks_a_decimal_text() {
    let schema = byte_size();
    // 2^53 + 1: the first integer a double cannot represent.
    let huge = ByteSize::bytes(9_007_199_254_740_993).expect("non-negative");

    assert_eq!(
        schema.decode(text("9007199254740993")).expect("decodes"),
        huge
    );
    assert_eq!(
        schema.try_encode(&huge).expect("encodes"),
        text("9007199254740993")
    );
    assert_eq!(schema.decode(text("0")).expect("decodes"), ByteSize::zero());
    assert!(
        schema.decode(text("-1")).is_err(),
        "a count is non-negative"
    );
    assert!(schema.decode(text("1.5")).is_err(), "a count is whole");
    assert!(schema.decode(Unknown::Number(1.0)).is_err(), "text only");
}

/// `Schema.test.ts:5770` — "ByteSizeFromString": a unit-bearing string in, the
/// pin's `<count> byte(s)` spelling out.
#[test]
fn byte_size_from_string_parses_and_spells_bytes() {
    let schema = byte_size_from_string();

    assert_eq!(
        schema.decode(text("1.5 kB")).expect("decodes"),
        ByteSize::bytes(1_500).expect("non-negative")
    );
    assert_eq!(
        schema.decode(text("2 mebibytes")).expect("decodes"),
        ByteSize::bytes(2_097_152).expect("non-negative")
    );
    // `KB` is not a spelling of the SI kilobyte in the pin's parser.
    assert!(schema.decode(text("1 KB")).is_err());

    assert_eq!(
        schema
            .try_encode(&ByteSize::bytes(1_500).expect("non-negative"))
            .expect("encodes"),
        text("1500 bytes")
    );
    assert_eq!(
        schema
            .try_encode(&ByteSize::bytes(1).expect("non-negative"))
            .expect("encodes"),
        text("1 byte"),
        "a single byte is the singular spelling"
    );
}

/// `Schema.test.ts:5785` — "ByteSizeFromNumber": a count on the wire, inside
/// the safe-integer window in both directions.
#[test]
fn byte_size_from_number_holds_the_safe_integer_window() {
    let schema = byte_size_from_number();
    let max_safe = 9_007_199_254_740_991.0;

    assert_eq!(
        schema.decode(Unknown::Number(max_safe)).expect("decodes"),
        ByteSize::bytes(9_007_199_254_740_991).expect("non-negative")
    );
    assert!(schema.decode(Unknown::Number(-1.0)).is_err(), "negative");
    assert!(schema.decode(Unknown::Number(0.5)).is_err(), "fractional");
    assert!(
        schema.decode(Unknown::Number(max_safe + 1.0)).is_err(),
        "one past the safe window"
    );

    assert_eq!(
        schema
            .try_encode(&ByteSize::bytes(1_500).expect("non-negative"))
            .expect("encodes"),
        Unknown::Number(1_500.0)
    );
    // The encode direction refuses rather than printing a rounded count.
    assert!(
        schema
            .try_encode(&ByteSize::bytes(9_007_199_254_740_992).expect("non-negative"))
            .is_err()
    );
}

/// `Schema.test.ts:5798` — "BigDecimal": the wire is the printed decimal.
#[test]
fn bigdecimal_reads_and_prints_decimal_text() {
    let schema = bigdecimal();

    assert_eq!(
        schema.decode(text("123.45")).expect("decodes"),
        decimal("123.45")
    );
    assert_eq!(
        schema.try_encode(&decimal("123.45")).expect("encodes"),
        text("123.45")
    );
    assert!(schema.decode(Unknown::Null).is_err(), "a decimal is text");
    assert!(schema.decode(text("not a number")).is_err());
}

/// `Schema.test.ts:5815` — `isGreaterThanBigDecimal`.
#[test]
fn is_greater_than_bigdecimal_names_the_bound() {
    let schema = checked(is_greater_than_bigdecimal(decimal("1")));

    assert_eq!(schema.decode(text("2")).expect("decodes"), decimal("2"));
    assert!(refusal(&schema, "1").contains("Expected a value greater than 1"));
    assert!(refusal(&schema, "0.5").contains("Expected a value greater than 1"));
    // The encoding direction enforces the same rule.
    assert!(schema.try_encode(&decimal("1")).is_err());
}

/// `Schema.test.ts:5827` — `isGreaterThanOrEqualToBigDecimal`.
#[test]
fn is_greater_than_or_equal_to_bigdecimal_admits_the_bound() {
    let schema = checked(is_greater_than_or_equal_to_bigdecimal(decimal("1")));

    assert_eq!(schema.decode(text("1")).expect("decodes"), decimal("1"));
    assert_eq!(schema.decode(text("1.0")).expect("decodes"), decimal("1"));
    assert!(refusal(&schema, "0").contains("Expected a value greater than or equal to 1"));
}

/// `Schema.test.ts:5841` — `isLessThanBigDecimal`.
#[test]
fn is_less_than_bigdecimal_names_the_bound() {
    let schema = checked(is_less_than_bigdecimal(decimal("1")));

    assert_eq!(schema.decode(text("0")).expect("decodes"), decimal("0"));
    assert!(refusal(&schema, "1").contains("Expected a value less than 1"));
}

/// `Schema.test.ts:5853` — `isLessThanOrEqualToBigDecimal`.
#[test]
fn is_less_than_or_equal_to_bigdecimal_admits_the_bound() {
    let schema = checked(is_less_than_or_equal_to_bigdecimal(decimal("1")));

    assert_eq!(schema.decode(text("1")).expect("decodes"), decimal("1"));
    assert!(refusal(&schema, "2").contains("Expected a value less than or equal to 1"));
}

/// `Schema.test.ts:5865` — `isBetweenBigDecimal`, both ends inclusive.
#[test]
fn is_between_bigdecimal_admits_both_ends() {
    let schema = checked(is_between_bigdecimal(decimal("1"), decimal("5")));

    assert_eq!(schema.decode(text("3")).expect("decodes"), decimal("3"));
    assert_eq!(schema.decode(text("1")).expect("decodes"), decimal("1"));
    assert_eq!(schema.decode(text("5.00")).expect("decodes"), decimal("5"));
    assert!(refusal(&schema, "0").contains("Expected a value between 1 and 5"));
    assert!(refusal(&schema, "6").contains("Expected a value between 1 and 5"));
}
