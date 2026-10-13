//! Conversions fence: the string/number and date transformation rows of
//! packages/effect/test/schema/Schema.test.ts (NumberFromString 2094,
//! DateFromString 2120, DateFromMillis 2136, FiniteFromString 2159).
//!
//! Encode-side refusals are not reproduced: this crate's `encode` is total,
//! the same convention the earlier cells use. A JavaScript `Invalid Date`
//! cannot exist here either, because the port models `Schema.Date` as whole
//! epoch millis.

use elm_effect_schema::transform::{date_from_millis_schema, date_schema};
use elm_effect_schema::{Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// Schema.test.ts:2094 `NumberFromString`.
#[test]
fn number_from_string_round_trips_non_finite_values() {
    let schema = Schema::number_from_string();

    assert_eq!(schema.decode(text("1")).unwrap(), 1.0);
    assert!(schema.decode(text("NaN")).unwrap().is_nan());
    assert_eq!(schema.decode(text("Infinity")).unwrap(), f64::INFINITY);
    assert_eq!(schema.decode(text("+Infinity")).unwrap(), f64::INFINITY);
    assert_eq!(schema.decode(text("-Infinity")).unwrap(), f64::NEG_INFINITY);

    assert_eq!(schema.encode(&1.0), text("1"));
    assert_eq!(schema.encode(&f64::NAN), text("NaN"));
    assert_eq!(schema.encode(&f64::INFINITY), text("Infinity"));
    assert_eq!(schema.encode(&f64::NEG_INFINITY), text("-Infinity"));
}

/// Schema.test.ts:2120 `DateFromString`.
#[test]
fn date_from_string_parses_and_renders_the_iso_wire() {
    const ISO: &str = "2021-01-01T00:00:00.000Z";
    let schema = date_schema();

    assert_eq!(schema.decode(text(ISO)).unwrap(), 1_609_459_200_000);
    assert!(schema.decode(text("invalid")).is_err());
    assert_eq!(schema.encode(&1_609_459_200_000), text(ISO));
}

/// Schema.test.ts:2136 `DateFromMillis`.
#[test]
fn date_from_millis_validates_whole_milliseconds_in_range() {
    let schema = date_from_millis_schema();

    assert_eq!(schema.decode(Unknown::Number(0.0)).unwrap(), 0);
    assert!(schema.decode(Unknown::Number(f64::NAN)).is_err());
    assert!(schema.decode(Unknown::Number(f64::INFINITY)).is_err());
    assert!(schema.decode(Unknown::Number(f64::NEG_INFINITY)).is_err());
    assert!(schema.decode(Unknown::Null).is_err());
    // One past the JS Date range (±8.64e15 ms).
    assert!(
        schema
            .decode(Unknown::Number(8_640_000_000_000_001.0))
            .is_err()
    );
    assert_eq!(schema.encode(&0), Unknown::Number(0.0));
}

/// Schema.test.ts:2159 `FiniteFromString`.
#[test]
fn finite_from_string_dedicated_decode_and_encode_table() {
    let schema = Schema::finite_from_string();

    assert_eq!(schema.decode(text("1")).unwrap(), 1.0);
    // The pin's own spellings, typos and all: none of them may decode.
    for spelling in ["a", "NaN", "Infintiy", "+Infintiy", "-Infintiy"] {
        assert!(
            schema.decode(text(spelling)).is_err(),
            "{spelling:?} must not decode"
        );
    }
    assert_eq!(schema.encode(&1.0), text("1"));
}
