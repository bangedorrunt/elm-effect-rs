//! `Schema.test.ts:2237` `TimeZoneNamedFromString` and `:2256`
//! `TimeZoneFromString`.
//!
//! `Zone` is the parked `datetime` module's type, reached here through the same
//! `cron` include the datetime tests use, so the two schema adapters live
//! beside their rows rather than in the crate's schema tree.

#[path = "../src/cron.rs"]
mod cron;

use cron::{DateTime, zone_from_string, zone_to_string, Zone};
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

/// The zone a zoned text already carries: `Z`, or a `±hh:mm` tail.
fn trailing_zone(text: &str) -> Option<Zone> {
    if text.ends_with('Z') {
        return Some(Zone::Utc);
    }
    let len = text.len();
    let tail = text.get(len.checked_sub(6)?..)?;
    zone_from_string(tail)
}

/// `Schema.DateTimeZonedFromString`: the ISO text decodes to an instant in the
/// zone it names (the RFC 9557 bracket, or the offset it carries), and encoding
/// writes that same text back.
fn date_time_zoned_from_string() -> Schema<DateTime> {
    Schema::<DateTime>::try_convert(
        Schema::as_unknown(Schema::string()),
        |wire: Unknown| {
            let Unknown::Text(whole) = wire else {
                return Err(refused("DateTime.Zoned", wire.kind()));
            };
            let (instant, bracketed) = match whole.rsplit_once('[') {
                Some((instant, rest)) => match rest.strip_suffix(']') {
                    Some(name) => (instant.to_string(), Some(name.to_string())),
                    None => (whole.clone(), None),
                },
                None => (whole.clone(), None),
            };
            let zone = match bracketed {
                Some(name) => zone_from_string(&name),
                None => trailing_zone(&instant),
            };
            let zone = zone.ok_or_else(|| {
                refused("a valid Zoned DateTime string", "text with no zone")
            })?;
            DateTime::make_zoned_from_string(&instant, zone).map_err(|_| {
                refused("a valid Zoned DateTime string", "unparseable instant")
            })
        },
        |value: &DateTime| Ok(text(&value.format_iso())),
    )
}

#[test]
fn date_time_zoned_from_string_reads_and_writes_the_zoned_text() {
    let schema = date_time_zoned_from_string();
    let zoned = DateTime::make_zoned_from_string(
        "2021-01-01T00:00:00.000Z",
        Zone::Named("Europe/London".to_string()),
    )
    .expect("a valid instant");

    // The zoned text is the value's own rendering, bracket included.
    assert!(zoned.format_iso().ends_with("[Europe/London]"), "{}", zoned.format_iso());
    assert_eq!(schema.decode(text(&zoned.format_iso())).unwrap(), zoned);
    assert_eq!(schema.try_encode(&zoned).unwrap(), text(&zoned.format_iso()));

    let refused = schema
        .decode(text("invalid"))
        .expect_err("`invalid` is not a zoned date-time");
    assert!(
        refused.to_string().contains("Zoned DateTime string"),
        "{refused}"
    );
}
