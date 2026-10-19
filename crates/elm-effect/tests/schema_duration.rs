//! `Schema.test.ts` rows for the two Duration schemas whose wire form this
//! port can speak: the duration string and the millisecond count.
//!
//! Row `:5711` (`DurationFromNanos`) is excused in the ledger rather than
//! witnessed here: its wire type is a `bigint`, and `Unknown` has no
//! arbitrary-precision integer variant, the same gap that excuses
//! `Formatter.test.ts:66`.
//!
//!     cargo test -p elm-effect --features schema --test schema_duration

use elm_effect::{Duration, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// `Schema.test.ts:5692` — "DurationFromString".
#[test]
fn duration_from_string_parses_and_prints_the_pin_spellings() {
    let schema = elm_effect::duration_from_string();

    assert_eq!(
        schema.decode(text("1000 millis")).expect("decodes"),
        Duration::millis(1000.0)
    );
    assert_eq!(
        schema.decode(text("1 second")).expect("decodes"),
        Duration::seconds(1.0)
    );
    assert_eq!(
        schema.decode(text("Infinity")).expect("decodes"),
        Duration::infinity()
    );
    assert_eq!(
        schema.decode(text("-Infinity")).expect("decodes"),
        Duration::negative_infinity()
    );
    assert!(
        schema.decode(text("value")).is_err(),
        "a bare word is not a duration"
    );

    assert_eq!(
        schema.try_encode(&Duration::zero()).expect("encodes"),
        text("0 millis")
    );
    assert_eq!(
        schema.try_encode(&Duration::seconds(5.0)).expect("encodes"),
        text("5000 millis")
    );
    assert_eq!(
        schema.try_encode(&Duration::nanos(5000)).expect("encodes"),
        text("5000 nanos")
    );
    assert_eq!(
        schema.try_encode(&Duration::infinity()).expect("encodes"),
        text("Infinity")
    );
    assert_eq!(
        schema
            .try_encode(&Duration::negative_infinity())
            .expect("encodes"),
        text("-Infinity")
    );
}

/// `Schema.test.ts:5732` — "DurationFromMillis".
#[test]
fn duration_from_millis_reads_and_writes_the_millisecond_count() {
    let schema = elm_effect::duration_from_millis();

    assert_eq!(
        schema
            .decode(Unknown::Number(f64::INFINITY))
            .expect("decodes"),
        Duration::infinity()
    );
    assert_eq!(
        schema
            .decode(Unknown::Number(f64::NEG_INFINITY))
            .expect("decodes"),
        Duration::negative_infinity()
    );
    assert_eq!(
        schema.decode(Unknown::Number(0.0)).expect("decodes"),
        Duration::millis(0.0)
    );
    assert_eq!(
        schema.decode(Unknown::Number(-1.0)).expect("decodes"),
        Duration::millis(-1.0)
    );
    assert_eq!(
        schema.decode(Unknown::Number(1000.0)).expect("decodes"),
        Duration::seconds(1.0)
    );
    assert_eq!(
        schema.decode(Unknown::Number(60_000.0)).expect("decodes"),
        Duration::minutes(1.0)
    );
    assert_eq!(
        schema.decode(Unknown::Number(0.1)).expect("decodes"),
        Duration::millis(0.1)
    );
    assert_eq!(
        schema.decode(Unknown::Number(f64::NAN)).expect("decodes"),
        Duration::zero(),
        "a NaN count is the zero duration, the pin's own make"
    );

    assert_eq!(
        schema.try_encode(&Duration::infinity()).expect("encodes"),
        Unknown::Number(f64::INFINITY)
    );
    assert_eq!(
        schema
            .try_encode(&Duration::negative_infinity())
            .expect("encodes"),
        Unknown::Number(f64::NEG_INFINITY)
    );
    assert_eq!(
        schema
            .try_encode(&Duration::millis(f64::NAN))
            .expect("encodes"),
        Unknown::Number(0.0)
    );
    assert_eq!(
        schema.try_encode(&Duration::millis(-1.0)).expect("encodes"),
        Unknown::Number(-1.0)
    );
    assert_eq!(
        schema.try_encode(&Duration::seconds(5.0)).expect("encodes"),
        Unknown::Number(5000.0)
    );
    assert_eq!(
        schema
            .try_encode(&Duration::millis(5000.0))
            .expect("encodes"),
        Unknown::Number(5000.0)
    );
    assert_eq!(
        schema.try_encode(&Duration::millis(0.1)).expect("encodes"),
        Unknown::Number(0.1)
    );
    assert_eq!(
        schema.try_encode(&Duration::nanos(5000)).expect("encodes"),
        Unknown::Number(0.005),
        "an exact nanos value reads back as a millis fraction"
    );
}
