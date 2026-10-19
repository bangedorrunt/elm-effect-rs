//! `Schema.DurationFromString` and `Schema.DurationFromMillis`.
//!
//! Both adapt this crate's [`Duration`] to the schema layer, and the schema
//! crate cannot name that type, so they live here as free functions beside the
//! type they speak for. `Schema.DurationFromNanos` is not here: its wire form is
//! a `bigint`, which `Unknown` cannot hold (see the ledger's excuse on
//! `Schema.test.ts:5711`).

use elm_effect_schema::{Schema, SchemaError};

use crate::time::Duration;

/// `Schema.DurationFromString`: the wire is a string `Duration::from_input_string`
/// parses, and encoding prints the pin's own `toString`.
pub fn duration_from_string() -> Schema<Duration> {
    Schema::convert(
        Schema::<String>::string(),
        |text: String| {
            Duration::from_input_string(&text).ok_or_else(|| SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "a valid Duration string",
                got: "unparseable",
            })
        },
        |duration: &Duration| duration.to_pin_string(),
    )
}

/// `Schema.DurationFromMillis`: the wire is a count of milliseconds, in the
/// pin's own `make` reading of one — a NaN count is the zero duration, and a
/// fraction of a millisecond becomes exact nanoseconds.
pub fn duration_from_millis() -> Schema<Duration> {
    Schema::convert(
        Schema::<f64>::number(),
        |millis: f64| Ok(Duration::millis(millis)),
        |duration: &Duration| duration.to_millis(),
    )
}
