//! `Schema.Date`: the schema of a [`DateTime`] instant.
//!
//! The pin's `Schema.Date` is a `declare` over a JS `Date` whose JSON codec is
//! an ISO-8601 string: `toCodecJson` writes `toJSON` and reads back through
//! `dateFromString`. This port's instant is [`DateTime`], so the wire is the
//! same text and [`DateTime::to_json`] is the same spelling.
//!
//! A JS `Date` is a millisecond count, so decoding normalizes to milliseconds:
//! a text carrying more precision would otherwise decode to a value that
//! encodes back differently, which is a codec that does not round trip.

use elm_effect_schema::{Schema, SchemaError};

use crate::datetime::DateTime;

/// `Schema.Date`: a valid instant, as the ISO-8601 text its JSON codec takes.
pub fn date() -> Schema<DateTime> {
    Schema::convert(
        Schema::<String>::string(),
        |text: String| {
            let parsed = DateTime::make(&text).map_err(|_| SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "a string that will be decoded as a Date",
                got: "unparseable",
            })?;
            Ok(DateTime::from_epoch_millis(parsed.epoch_millis()))
        },
        |date: &DateTime| date.to_json(),
    )
    // `toFormatter(Schema.Date)` prints the instant's ISO text with no quotes
    // around it, which the String kind's own rendering would add.
    .with_formatter(|wire| match wire {
        elm_effect_schema::Unknown::Text(text) => text.clone(),
        other => elm_effect_schema::format_value(other),
    })
    .with_declaration(
        "effect/schema/Date",
        elm_effect_schema::serde_json::Value::Null,
        Vec::new(),
    )
    .with_code("Schema.Date", "DateTime")
}
