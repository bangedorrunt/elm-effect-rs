//! `Schema.test.ts:2237` `TimeZoneNamedFromString` and `:2256`
//! `TimeZoneFromString`.
//!
//! `Zone` is the parked `datetime` module's type, reached here through the same
//! `cron` include the datetime tests use, so the two schema adapters live
//! beside their rows rather than in the crate's schema tree.

#[path = "../src/cron.rs"]
mod cron;

use cron::{zone_from_string, zone_to_string, Zone};
use elm_effect_schema::{Schema, SchemaError, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn refused(expected: &'static str, got: &'static str) -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected,
        got,
    }
}

/// `Schema.TimeZoneFromString`: the text names a zone, which is what decodes;
/// encoding writes the name, the offset or `Z` back.
fn time_zone_from_string() -> Schema<Zone> {
    Schema::<Zone>::try_convert(
        Schema::as_unknown(Schema::string()),
        |wire: Unknown| match wire {
            Unknown::Text(name) => zone_from_string(&name)
                .ok_or_else(|| refused("DateTime.TimeZone", "a string no zone names")),
            other => Err(refused("DateTime.TimeZone", other.kind())),
        },
        |zone: &Zone| Ok(text(&zone_to_string(zone))),
    )
}

/// `Schema.TimeZoneNamedFromString`: only an IANA name decides here; an offset
/// is a zone but not a named one.
fn time_zone_named_from_string() -> Schema<Zone> {
    Schema::<Zone>::try_convert(
        Schema::as_unknown(Schema::string()),
        |wire: Unknown| match wire {
            Unknown::Text(name) => match zone_from_string(&name) {
                Some(zone @ Zone::Named(_)) => Ok(zone),
                _ => Err(refused(
                    "DateTime.TimeZone.Named",
                    "a string that is not a named zone",
                )),
            },
            other => Err(refused("DateTime.TimeZone.Named", other.kind())),
        },
        |zone: &Zone| Ok(text(&zone_to_string(zone))),
    )
}

#[test]
fn time_zone_named_from_string_decodes_and_encodes() {
    let schema = time_zone_named_from_string();
    let london = Zone::Named("Europe/London".to_string());

    assert_eq!(schema.decode(text("Europe/London")).unwrap(), london);
    assert_eq!(schema.try_encode(&london).unwrap(), text("Europe/London"));

    // The pin's encode failure is a non-zone value, which cannot be a `Zone`
    // here; what the port can show is that an offset is refused on the way in.
    let offset = schema
        .decode(text("+03:00"))
        .expect_err("an offset is not a named zone");
    assert!(offset.to_string().contains("DateTime.TimeZone.Named"), "{offset}");
}

#[test]
fn time_zone_from_string_decodes_named_and_offset() {
    let schema = time_zone_from_string();
    let london = Zone::Named("Europe/London".to_string());
    let offset = Zone::Offset(3 * 60);

    assert_eq!(schema.decode(text("Europe/London")).unwrap(), london);
    assert_eq!(schema.decode(text("+03:00")).unwrap(), offset);

    assert_eq!(schema.try_encode(&london).unwrap(), text("Europe/London"));
    assert_eq!(schema.try_encode(&offset).unwrap(), text("+03:00"));

    let refused = schema
        .decode(text("a"))
        .expect_err("`a` names no zone");
    assert!(refused.to_string().contains("DateTime.TimeZone"), "{refused}");
}
