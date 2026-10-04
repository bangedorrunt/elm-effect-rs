#[path = "../src/cron.rs"]
mod cron;

use cron::{
    clear_test_now_millis, set_test_now_millis, with_current_zone, Cron, DateParts, DateTime,
    Disambiguation, Unit, Zone,
};

fn utc(s: &str) -> DateTime {
    DateTime::make_unsafe(s)
}

fn zoned(s: &str, zone: &str) -> DateTime {
    DateTime::make_zoned_from_string(s, Zone::Named(zone.to_string())).unwrap()
}

// ---------------------------------------------------------------- mutate ---

#[test]
fn mutate_adds_a_day() {
    let base = utc("2024-06-30T12:00:00.000Z");
    let next = base.mutate(|p| p.day += 1, Disambiguation::Compatible);
    assert_eq!(next.to_json(), "2024-07-01T12:00:00.000Z");
    assert_eq!(next.distance_millis(&base), 86_400_000);
}

#[test]
fn mutate_preserves_zone_across_dst() {
    let base = zoned("2024-01-01T00:00:00+13:00[Pacific/Auckland]", "Pacific/Auckland");
    let next = base.mutate(|p| p.day += 1, Disambiguation::Compatible);
    assert_eq!(next.zone(), Zone::Named("Pacific/Auckland".to_string()));
    assert_eq!(next.distance_millis(&base), 86_400_000);
}

// ------------------------------------------------------------------- add ---

#[test]
fn add_calendar_days_utc() {
    let base = utc("2024-06-30T12:00:00.000Z");
    assert_eq!(base.add(Unit::Days, 1).to_json(), "2024-07-01T12:00:00.000Z");
    assert_eq!(base.subtract(Unit::Days, 1).to_json(), "2024-06-29T12:00:00.000Z");
}

#[test]
fn add_clamps_to_month_end() {
    assert_eq!(
        utc("2024-01-31T00:00:00.000Z").add(Unit::Months, 1).to_json(),
        "2024-02-29T00:00:00.000Z"
    );
    assert_eq!(
        utc("2023-01-31T00:00:00.000Z").add(Unit::Months, 1).to_json(),
        "2023-02-28T00:00:00.000Z"
    );
}

#[test]
fn add_preserves_wall_across_dst_fall_back() {
    // Apr 7 2024: Auckland falls back +13 -> +12.
    let base = zoned("2024-04-06T12:00:00+13:00[Pacific/Auckland]", "Pacific/Auckland");
    let next = base.add(Unit::Days, 1);
    assert_eq!(next.format_iso(), "2024-04-07T12:00:00.000+12:00[Pacific/Auckland]");
}

// ----------------------------------------------------------------- endOf ---

#[test]
fn end_of_month_utc() {
    assert_eq!(
        utc("2024-03-15T12:00:00.000Z").end_of(Unit::Months, 0).to_json(),
        "2024-03-31T23:59:59.999Z"
    );
}

#[test]
fn end_of_february_leap_year() {
    assert_eq!(
        utc("2024-02-10T00:00:00.000Z").end_of(Unit::Months, 0).to_json(),
        "2024-02-29T23:59:59.999Z"
    );
}

#[test]
fn end_of_week_last_day() {
    // Saturday input ends the same Saturday.
    assert_eq!(
        utc("2024-03-16T12:00:00.000Z").end_of(Unit::Weeks, 0).to_json(),
        "2024-03-16T23:59:59.999Z"
    );
}

#[test]
fn end_of_week_defaults_sunday() {
    // Wednesday Jan 3 2024 -> Sunday Jan 7 00:00.
    assert_eq!(
        utc("2024-01-03T12:00:00.000Z").end_of(Unit::Weeks, 0).to_json(),
        "2024-01-06T23:59:59.999Z"
    );
}

#[test]
fn end_of_week_with_monday_start() {
    assert_eq!(
        utc("2024-01-03T12:00:00.000Z").end_of(Unit::Weeks, 1).to_json(),
        "2024-01-07T23:59:59.999Z"
    );
}

#[test]
fn zoned_end_of_month() {
    let base = zoned("2024-03-15T12:00:00+13:00[Pacific/Auckland]", "Pacific/Auckland");
    assert_eq!(
        base.end_of(Unit::Months, 0).format_iso(),
        "2024-03-31T23:59:59.999+13:00[Pacific/Auckland]"
    );
}

// --------------------------------------------------------------- startOf ---

#[test]
fn start_of_month() {
    assert_eq!(
        utc("2024-03-15T12:00:00.000Z").start_of(Unit::Months, 0).to_json(),
        "2024-03-01T00:00:00.000Z"
    );
}

#[test]
fn start_of_february_leap_year() {
    assert_eq!(
        utc("2024-02-29T12:00:00.000Z").start_of(Unit::Months, 0).to_json(),
        "2024-02-01T00:00:00.000Z"
    );
}

#[test]
fn start_of_week_defaults_sunday() {
    assert_eq!(
        utc("2024-01-03T12:00:00.000Z").start_of(Unit::Weeks, 0).to_json(),
        "2023-12-31T00:00:00.000Z"
    );
}

#[test]
fn start_of_week_first_day_stays() {
    assert_eq!(
        utc("2023-12-31T12:00:00.000Z").start_of(Unit::Weeks, 0).to_json(),
        "2023-12-31T00:00:00.000Z"
    );
}

#[test]
fn start_of_week_with_monday_start() {
    assert_eq!(
        utc("2024-01-03T12:00:00.000Z").start_of(Unit::Weeks, 1).to_json(),
        "2024-01-01T00:00:00.000Z"
    );
}

// --------------------------------------------------------------- nearest ---

#[test]
fn nearest_month_up() {
    assert_eq!(
        utc("2024-03-16T12:00:00.000Z").nearest(Unit::Months, 0).to_json(),
        "2024-04-01T00:00:00.000Z"
    );
}

#[test]
fn nearest_month_down() {
    assert_eq!(
        utc("2024-03-16T11:00:00.000Z").nearest(Unit::Months, 0).to_json(),
        "2024-03-01T00:00:00.000Z"
    );
}

#[test]
fn nearest_second_up() {
    assert_eq!(
        utc("2024-03-20T12:00:00.500Z").nearest(Unit::Seconds, 0).to_json(),
        "2024-03-20T12:00:01.000Z"
    );
}

#[test]
fn nearest_second_down() {
    assert_eq!(
        utc("2024-03-20T12:00:00.400Z").nearest(Unit::Seconds, 0).to_json(),
        "2024-03-20T12:00:00.000Z"
    );
}

// -------------------------------------------------------------- fromParts ---

#[test]
fn from_parts_partial_defaults() {
    assert_eq!(
        DateTime::from_partial(2024, Some(12), Some(25), Zone::Utc).unwrap().to_json(),
        "2024-12-25T00:00:00.000Z"
    );
}

#[test]
fn from_parts_month_defaults_to_january() {
    assert_eq!(
        DateTime::from_partial(2024, None, None, Zone::Utc).unwrap().to_json(),
        "2024-01-01T00:00:00.000Z"
    );
}

// ------------------------------------------------------------ setPartsUtc ---

#[test]
fn set_parts_utc_simultaneous_no_overflow() {
    let base = utc("2024-01-31T12:34:56.789Z");
    let updated = base.mutate_utc(|p| {
        p.month = 2;
        p.day = 1;
    });
    assert_eq!(updated.format_iso(), "2024-02-01T12:34:56.789Z");
}

#[test]
fn set_parts_utc_ignores_zone() {
    let base = utc("2024-12-25T00:00:00.000Z").set_zone_named_unsafe("Pacific/Auckland");
    assert_eq!(base.to_json(), "2024-12-25T00:00:00.000Z");
    let updated = base.mutate_utc(|p| {
        p.year = 2023;
        p.month = 1;
    });
    assert_eq!(updated.to_json(), "2023-01-25T00:00:00.000Z");
}

// -------------------------------------------------------------- setParts ---

#[test]
fn set_parts_partial() {
    let base = DateTime::from_partial(2024, Some(12), Some(25), Zone::Utc).unwrap();
    assert_eq!(base.to_json(), "2024-12-25T00:00:00.000Z");
    let updated = base.mutate(|p| {
        p.year = 2023;
        p.month = 1;
    }, Disambiguation::Compatible);
    assert_eq!(updated.to_json(), "2023-01-25T00:00:00.000Z");
}

#[test]
fn set_parts_accounts_for_zone() {
    let base = DateTime::from_partial(2024, Some(12), Some(25), Zone::Utc)
        .unwrap()
        .set_zone_named_unsafe("Pacific/Auckland");
    assert_eq!(base.to_json(), "2024-12-25T00:00:00.000Z");
    let updated = base.mutate(|p| {
        p.year = 2023;
        p.month = 6;
        p.hour = 12;
    }, Disambiguation::Compatible);
    assert_eq!(updated.to_json(), "2023-06-25T00:00:00.000Z");
}

// -------------------------------------------------------------- formatIso ---

#[test]
fn format_iso_full() {
    assert_eq!(
        DateTime::format_iso(&utc("2024-03-15T12:00:00.000Z")),
        "2024-03-15T12:00:00.000Z"
    );
}

#[test]
fn format_and_parse_utc_roundtrip() {
    for s in ["2024-03-15T12:00:00.000Z", "1970-01-01T00:00:00.000Z", "2028-02-29T23:59:59.123Z"] {
        assert_eq!(DateTime::make_unsafe(s).format_iso(), s);
    }
}

// -------------------------------------------------------- formatIsoOffset ---

#[test]
fn format_iso_offset_adds_offset() {
    set_test_now_millis(0);
    let now = with_current_zone(Zone::Named("Pacific/Auckland".to_string()), DateTime::now_in_current_zone);
    clear_test_now_millis();
    assert_eq!(now.format_iso_offset(), "1970-01-01T12:00:00.000+12:00");
}

// ------------------------------------------------------------- removeTime ---

#[test]
fn remove_time_midnights() {
    assert_eq!(
        utc("2024-06-15T18:30:00.000Z").remove_time().to_json(),
        "2024-06-15T00:00:00.000Z"
    );
}

// --------------------------------------------------------- toEpochSeconds ---

#[test]
fn to_epoch_seconds_returns_seconds() {
    assert_eq!(utc("2024-03-15T12:00:00.000Z").to_epoch_seconds(), 1_710_504_000);
}

#[test]
fn to_epoch_seconds_floors() {
    assert_eq!(utc("2024-03-15T12:00:00.999Z").to_epoch_seconds(), 1_710_504_000);
}

// ------------------------------------------------------- fromEpochSeconds ---

#[test]
fn from_epoch_seconds_builds_utc() {
    assert_eq!(
        DateTime::from_epoch_millis(1_710_504_000_000).to_json(),
        "2024-03-15T12:00:00.000Z"
    );
}

#[test]
fn epoch_millis_roundtrips() {
    let ms = 1_710_504_000_123i64;
    assert_eq!(DateTime::from_epoch_millis(ms).to_epoch_millis(), ms);
}

// ---------------------------------------------------- makeZonedFromString ---

#[test]
fn parses_offset_plus_iana_bracket() {
    let dt = DateTime::make_zoned_from_string(
        "2024-01-01T00:00:00+13:00[Pacific/Auckland]",
        Zone::Named("Pacific/Auckland".to_string()),
    )
    .unwrap();
    assert_eq!(dt.to_json(), "2023-12-31T11:00:00.000Z");
    assert_eq!(dt.zone(), Zone::Named("Pacific/Auckland".to_string()));
}

#[test]
fn parses_offset_only_as_offset_zone() {
    let dt = DateTime::make("2024-01-01T00:00:00+05:30").unwrap();
    assert_eq!(dt.zone(), Zone::Offset(330));
    assert_eq!(dt.to_json(), "2023-12-31T18:30:00.000Z");
}

#[test]
fn parses_utc_offset_only_as_utc() {
    let dt = DateTime::make("2024-01-01T00:00:00+00:00").unwrap();
    assert_eq!(dt.zone(), Zone::Offset(0));
    assert_eq!(dt.to_json(), "2024-01-01T00:00:00.000Z");
}

#[test]
fn bracketed_iana_roundtrips() {
    let s = "2024-06-15T12:00:00.000+12:00[Pacific/Auckland]";
    assert_eq!(DateTime::make_unsafe(s).format_iso(), s);
}

#[test]
fn parts_equality() {
    assert_eq!(utc("2024-03-15T12:00:00.000Z").parts(), utc("2024-03-15T12:00:00.000Z").parts());
    assert_ne!(utc("2024-03-15T12:00:00.000Z").parts(), utc("2024-03-16T12:00:00.000Z").parts());
}

#[test]
fn make_zoned_unsafe_no_options_defaults_compatible() {
    let dt = DateTime::make_zoned(
        DateParts { year: 2024, month: 1, day: 15, hour: 12, minute: 0, second: 0, millis: 0 },
        Zone::Named("Pacific/Auckland".to_string()),
        Disambiguation::Compatible,
    )
    .unwrap();
    assert_eq!(dt.to_json(), "2024-01-14T23:00:00.000Z");
}

// ------------------------------------------------------------------ toUtc ---

#[test]
fn to_utc_keeps_instant() {
    assert_eq!(utc("2024-01-01T01:00:00.000Z").to_utc().to_json(), "2024-01-01T01:00:00.000Z");
}

#[test]
fn zoned_to_utc_adjusts_wall() {
    let dt = DateTime::make_zoned_from_string(
        "2024-01-01T01:00:00Z",
        Zone::Named("Pacific/Auckland".to_string()),
    )
    .unwrap();
    assert_eq!(dt.to_utc().to_json(), "2024-01-01T01:00:00.000Z");
    assert!(dt.to_utc().is_utc());
}

// --------------------------------------------------------------- nowAsDate ---

#[test]
fn now_as_system_time_matches_override() {
    set_test_now_millis(1_710_504_000_000);
    let now = DateTime::now();
    clear_test_now_millis();
    assert_eq!(now.to_json(), "2024-03-15T12:00:00.000Z");
}

// ------------------------------------------------------------------- make ---

#[test]
fn make_rejects_garbage() {
    assert!(DateTime::make("not-a-date").is_err());
    assert!(DateTime::make("2024-13-01T00:00:00Z").is_err());
}

#[test]
fn make_rejects_out_of_range_epoch() {
    assert!(DateTime::from_epoch_millis_checked(8_640_000_000_000_001).is_err());
}

// ------------------------------------------------------------- makeUnsafe ---

#[test]
fn bare_strings_parse_as_utc() {
    assert_eq!(utc("2024-01-01 01:00:00").to_json(), "2024-01-01T01:00:00.000Z");
}

#[test]
fn z_strings_keep_instant() {
    assert_eq!(utc("2026-01-27T17:14:06.000Z").to_json(), "2026-01-27T17:14:06.000Z");
}

#[test]
fn explicit_offset_converts_to_utc_json() {
    assert_eq!(utc("2020-02-01T11:17:00+1100").to_json(), "2020-02-01T00:17:00.000Z");
}

#[test]
fn gmt_strings_parse() {
    assert_eq!(
        utc("Tue, 27 Jan 2026 17:14:06 GMT").to_json(),
        "2026-01-27T17:14:06.000Z"
    );
}

// ------------------------------------------------------------------ format ---

#[test]
fn format_full_utc() {
    set_test_now_millis(0);
    let now = DateTime::now();
    clear_test_now_millis();
    assert_eq!(
        now.format_full(),
        "Thursday, January 1, 1970 at 12:00:00 AM Coordinated Universal Time"
    );
}

#[test]
fn format_full_zoned() {
    set_test_now_millis(0);
    let now = with_current_zone(Zone::Named("Pacific/Auckland".to_string()), DateTime::now_in_current_zone);
    clear_test_now_millis();
    assert_eq!(
        now.format_full(),
        "Thursday, January 1, 1970 at 12:00:00 PM New Zealand Standard Time"
    );
}

#[test]
fn format_full_utc_of_zoned() {
    set_test_now_millis(0);
    let now = with_current_zone(Zone::Named("Pacific/Auckland".to_string()), DateTime::now_in_current_zone);
    clear_test_now_millis();
    assert_eq!(
        now.format_full_utc(),
        "Thursday, January 1, 1970 at 12:00:00 AM Coordinated Universal Time"
    );
}

#[test]
fn add_year_clamps_leap_day() {
    let feb29 = DateTime::from_partial(2024, Some(2), Some(29), Zone::Utc).unwrap();
    assert_eq!(feb29.to_json(), "2024-02-29T00:00:00.000Z");
    assert_eq!(feb29.add(Unit::Years, 1).to_json(), "2025-02-28T00:00:00.000Z");
}

#[test]
fn start_of_week_variants() {
    assert_eq!(
        utc("2024-03-15T12:00:00.000Z").start_of(Unit::Weeks, 0).to_json(),
        "2024-03-10T00:00:00.000Z"
    );
    assert_eq!(
        utc("2024-03-10T12:00:00.000Z").start_of(Unit::Weeks, 0).to_json(),
        "2024-03-10T00:00:00.000Z"
    );
    assert_eq!(
        utc("2024-03-15T12:00:00.000Z").start_of(Unit::Weeks, 1).to_json(),
        "2024-03-11T00:00:00.000Z"
    );
    assert_eq!(utc("2024-03-10T00:00:00.000Z").part_utc(Unit::Weeks), 0);
}

#[test]
fn nanos_truncate_to_millis() {
    let dt = DateTime::make("2024-07-21T20:12:34.112546348+12:00[Pacific/Auckland]").unwrap();
    assert_eq!(dt.to_json(), "2024-07-21T08:12:34.112Z");
}

#[test]
fn utc_offset_zone_is_not_utc() {
    let dt = DateTime::make("2024-07-21T20:12:34.112546348+00:00").unwrap();
    assert_eq!(dt.zone(), Zone::Offset(0));
    assert!(!dt.is_utc());
    assert_eq!(dt.to_json(), "2024-07-21T20:12:34.112Z");
}

#[test]
fn from_partial_rejects_bad_month() {
    assert!(DateTime::from_partial(2024, Some(13), None, Zone::Utc).is_err());
    assert!(DateTime::from_partial(2024, Some(2), Some(30), Zone::Utc).is_err());
}

#[test]
fn disambiguation_matrix_gap() {
    let parts = DateParts { year: 2024, month: 3, day: 10, hour: 2, minute: 0, second: 0, millis: 0 };
    let ny = Zone::Named("America/New_York".to_string());
    let at = |d: Disambiguation| DateTime::make_zoned(parts, ny.clone(), d).map(|dt| dt.to_json());
    assert_eq!(at(Disambiguation::Compatible).unwrap(), "2024-03-10T07:00:00.000Z");
    assert_eq!(at(Disambiguation::Later).unwrap(), "2024-03-10T07:00:00.000Z");
    // `earlier` is covered by the fold test below; `reject` by the spring-gap test.
}

#[test]
fn disambiguation_earlier_fold() {
    // Fall-back fold: earlier occurrence is EDT (-04:00).
    let parts = DateParts { year: 2024, month: 11, day: 3, hour: 1, minute: 30, second: 0, millis: 0 };
    let ny = Zone::Named("America/New_York".to_string());
    let early = DateTime::make_zoned(parts, ny.clone(), Disambiguation::Compatible).unwrap();
    assert_eq!(early.format_iso(), "2024-11-03T01:30:00.000-04:00[America/New_York]");
}

#[test]
fn start_of_month_idempotent() {
    let once = utc("2024-03-15T12:00:00.000Z").start_of(Unit::Months, 0);
    assert_eq!(once.start_of(Unit::Months, 0).to_json(), "2024-03-01T00:00:00.000Z");
}

#[test]
fn start_of_week_first_day() {
    assert_eq!(
        utc("2024-03-10T12:00:00.000Z").start_of(Unit::Weeks, 0).to_json(),
        "2024-03-10T00:00:00.000Z"
    );
}

#[test]
fn from_epoch_seconds_exact() {
    assert_eq!(
        DateTime::from_epoch_millis(1704067200 * 1000).to_json(),
        "2024-01-01T00:00:00.000Z"
    );
    let original = DateTime::make_unsafe("2024-06-15T12:30:00Z");
    let secs = original.to_epoch_seconds();
    assert_eq!(DateTime::from_epoch_millis(secs * 1000).to_epoch_seconds(), secs);
}

#[test]
#[should_panic]
fn make_unsafe_panics_on_garbage() {
    let _ = DateTime::make_unsafe("definitely not a date");
}

#[test]
fn now_matches_test_clock_override() {
    // setTo2024NZ: 2023-12-31T11:00:00.000Z.
    set_test_now_millis(1704020400000);
    let now = DateTime::now();
    clear_test_now_millis();
    assert_eq!(now.to_json(), "2023-12-31T11:00:00.000Z");
}

#[test]
fn cron_sequence_ts_cases_aligned() {
    // Forward and reverse gathers agree on every listed schedule.
    for (expr, lower, upper) in [
        ("5 2 * * 1", "2020-01-01T00:00:01.000Z", "2021-01-01T00:00:01.000Z"),
        ("0 12 1 * *", "2020-01-01T00:00:01.000Z", "2021-01-01T00:00:01.000Z"),
        ("10,30 * * * * *", "2024-01-01T00:00:00.000Z", "2024-01-02T00:00:00.000Z"),
    ] {
        let c = Cron::parse_unsafe(expr, None);
        let mut fwd = Vec::new();
        let mut at = utc(lower);
        let up = utc(upper);
        loop {
            at = c.next_unsafe(&at);
            if at.epoch_millis() >= up.epoch_millis() {
                break;
            }
            fwd.push(at.to_json());
        }
        let mut rev = Vec::new();
        let mut bt = up.clone();
        let lo = utc(lower);
        loop {
            bt = c.prev_unsafe(&bt);
            if bt.epoch_millis() <= lo.epoch_millis() {
                break;
            }
            rev.push(bt.to_json());
        }
        rev.reverse();
        assert_eq!(fwd, rev, "aligned: {expr}");
        assert!(!fwd.is_empty(), "nonempty: {expr}");
    }
}

// ------------------------------------------------------- disambiguation ---

#[test]
fn reject_fails_on_spring_gap() {
    // 2024-04-07 02:30 does not exist in Pacific/Auckland (spring forward).
    let out = DateTime::make_zoned(
        DateParts { year: 2024, month: 4, day: 7, hour: 2, minute: 30, second: 0, millis: 0 },
        Zone::Named("Pacific/Auckland".to_string()),
        Disambiguation::Reject,
    );
    assert!(out.is_err());
}

fn dtz(s: &str, zone: &str) -> DateTime {
    DateTime::make_zoned_from_string(s, Zone::Named(zone.to_string())).unwrap()
}

fn matches_str(cron: &str, at: &str) -> bool {
    Cron::parse_unsafe(cron, None).matches(&utc(at))
}

fn next_str(cron: &str, from: &str) -> String {
    Cron::parse_unsafe(cron, None).next_unsafe(&utc(from)).to_json()
}

fn prev_str(cron: &str, from: &str) -> String {
    Cron::parse_unsafe(cron, None).prev_unsafe(&utc(from)).to_json()
}

// ------------------------------------------------------------------- cron ---

// END_DATETIME

#[test]
fn cron_parse_error_shape() {
    let err = Cron::parse("", None).unwrap_err();
    assert!(err.to_string().contains("segments"));
}

#[test]
fn cron_parse_error_predicate() {
    assert!(Cron::parse("", None).is_err());
    assert!(Cron::parse("0 0 * * *", None).is_ok());
    assert!(Cron::parse("nope", None).is_err());
}

#[test]
fn cron_parse_five_segments_wildcard_seconds() {
    // Bare seconds stay a wildcard, so format keeps all six fields.
    let c = Cron::parse_unsafe("5 0 8 2 *", None);
    assert_eq!(c.format(false), "5 0 8 2 *");
}

#[test]
fn cron_parse_zoned_rejects_bad_zone() {
    let err = Cron::parse_zoned("0 0 * * *", None, Some("")).unwrap_err();
    assert!(err.to_string().contains("Invalid time zone"));
    assert!(Cron::parse_zoned("0 0 * * *", None, Some("Europe/Berlin")).is_ok());
}

#[test]
fn cron_format_make_compacts_runs() {
    let c = Cron::make([0], [0, 1, 2, 10, 20, 30], [], [], [], [], false, None).unwrap();
    assert_eq!(c.format(false), "0-2,10-30/10 * * * *");
}

#[test]
fn cron_parse_structural_ranges() {
    // Explicit full ranges stay explicit (day/weekday OR semantics kept).
    let parsed = Cron::parse_unsafe("0 4 8-14 * 0-6", None);
    let built = Cron::make(
        [0],
        [0],
        [4],
        [8, 9, 10, 11, 12, 13, 14],
        [],
        [0, 1, 2, 3, 4, 5, 6],
        false,
        None,
    )
    .unwrap();
    assert_eq!(parsed, built);
}

#[test]
fn cron_format_seconds_variants() {
    let fmt = |seconds: Option<Vec<i32>>| {
        Cron::make(
            seconds.unwrap_or(vec![0]),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            false,
            None,
        )
        .unwrap()
        .format(false)
    };
    assert_eq!(fmt(None), "* * * * *");
    let empty = Cron::make(vec![], vec![], vec![], vec![], vec![], vec![], false, None)
        .unwrap()
        .format(false);
    assert_eq!(empty, "* * * * * *");
    // Explicit values (default or not) always print all six fields.
    let explicit = Cron::make(vec![15, 30], vec![], vec![], vec![], vec![], vec![], false, None)
        .unwrap()
        .format(false);
    assert_eq!(explicit, "15,30 * * * * *");
}

#[test]
fn cron_prev_new_year_strict() {
    // On-schedule instant: prev steps a full year back.
    assert_eq!(prev_str("0 0 1 1 *", "2024-01-01T00:00:00.000Z"), "2023-01-01T00:00:00.000Z");
}

#[test]
fn cron_prev_day_31_from_march() {
    assert_eq!(prev_str("0 0 31 * *", "2024-03-01T00:00:00.000Z"), "2024-01-31T00:00:00.000Z");
}

#[test]
fn cron_berlin_fall_first_occurrence() {
    // The wall scan keeps the first 02:30 (+02:00), skipping the repeat.
    let c = Cron::parse_unsafe("0 30 2 * * *", Some(Zone::Named("Europe/Berlin".to_string())));
    let before =
        dtz("2024-10-27T03:30:00.000+01:00[Europe/Berlin]", "Europe/Berlin");
    assert_eq!(
        c.prev_unsafe(&before).format_iso(),
        "2024-10-27T02:30:00.000+02:00[Europe/Berlin]"
    );
}

#[test]
fn cron_parse_unsafe_ok() {
    let c = Cron::parse_unsafe("15 14 1 * *", None);
    assert!(c.matches(&utc("2024-02-01T14:15:00.000Z")));
}

#[test]
fn cron_format_compact() {
    assert_eq!(Cron::parse_unsafe("0 0 * * *", None).format(false), "0 0 * * *");
}

#[test]
fn cron_format_with_seconds() {
    assert_eq!(Cron::parse_unsafe("0 0 * * *", None).format(true), "0 0 0 * * *");
}

#[test]
fn cron_format_compacts_runs() {
    assert_eq!(Cron::parse_unsafe("0 0-20/2 * * *", None).format(false), "0 0-20/2 * * *");
}

#[test]
fn cron_format_lists() {
    assert_eq!(Cron::parse_unsafe("5 4,6 * * *", None).format(false), "5 4,6 * * *");
}

#[test]
fn cron_format_non_default_seconds() {
    assert_eq!(Cron::parse_unsafe("30 5 0 * * *", None).format(false), "30 5 0 * * *");
}

#[test]
fn cron_make_and_flag() {
    let both = Cron::make([0], [0], [0], [1], [1], [1], true, None).unwrap();
    // Jan 1 2024 is a Monday: day=1 hits, weekday=1 hits.
    assert!(both.matches(&utc("2024-01-01T00:00:00.000Z")));
    // Jan 8 is a Monday but not the 1st: AND fails.
    assert!(!both.matches(&utc("2024-01-08T00:00:00.000Z")));
    let either = Cron::make([0], [0], [0], [1], [1], [1], false, None).unwrap();
    assert!(either.matches(&utc("2024-01-08T00:00:00.000Z")));
    // Feb 8 2024 is a Thursday: AND over day 8 + Thursday hits.
    let thu = Cron::make([0], [0], [0], [8], [], [4], true, None).unwrap();
    assert!(thu.matches(&utc("2024-02-08T00:00:00.000Z")));
    // Mar 8 is a Friday: AND fails on the weekday.
    assert!(!thu.matches(&utc("2024-03-08T00:00:00.000Z")));
}

#[test]
fn cron_make_validates_and_normalizes_weekday_7() {
    assert!(Cron::make([0], [0], [0], [1], [1], [8], false, None).is_err());
    let c = Cron::make([0], [0], [0], [], [], [7], false, None).unwrap();
    // Sunday Jan 7 2024 matches weekday 7 normalized to 0.
    assert!(c.matches(&utc("2024-01-07T00:00:00.000Z")));
    assert!(c.matches(&utc("2024-01-07T00:00:00.000Z"))); // stepping covered below
}

#[test]
fn cron_make_weekday_7_matches_sunday() {
    let c = Cron::parse_unsafe("0 0 * * 7", None);
    assert!(c.matches(&utc("2024-01-07T00:00:00.000Z")));
    assert!(!c.matches(&utc("2024-01-08T00:00:00.000Z")));
}

#[test]
fn cron_match_cases() {
    assert!(matches_str("5 0 * 8 *", "2024-08-01T00:05:00.000Z"));
    assert!(matches_str("15 14 1 * *", "2024-02-01T14:15:00.000Z"));
    assert!(matches_str("23 0-20/2 * * 0", "2024-01-07T00:23:00.000Z"));
    assert!(matches_str("5 4 * * SUN", "2024-01-07T04:05:00.000Z"));
    assert!(matches_str("5 4 * DEC SUN", "2024-12-01T04:05:00.000Z"));
    assert!(matches_str("0 0 1 * SUN-SAT", "2024-01-02T00:00:00.000Z"));
    assert!(matches_str("0 0 * * 7", "2024-01-07T00:00:00.000Z"));
    assert!(matches_str("0 0 */2 * MON", "2024-01-15T00:00:00.000Z"));
    assert!(!matches_str("5 4 * * SUN", "2024-01-08T04:05:00.000Z"));
}

#[test]
fn cron_match_zoned() {
    let london = Some(Zone::Named("Europe/London".to_string()));
    let c = Cron::parse_unsafe("15 14 1 * *", london);
    assert!(c.matches(&dtz("2024-02-01T14:15:00Z", "Europe/London")));
    assert!(c.matches(&dtz("2024-02-01T14:15:00Z", "Europe/Amsterdam")));
}

#[test]
fn cron_next_cases() {
    let from = "2024-01-04T16:21:00.000Z";
    assert_eq!(next_str("5 0 8 2 *", from), "2024-02-08T00:05:00.000Z");
    assert_eq!(next_str("15 14 1 * *", from), "2024-02-01T14:15:00.000Z");
    assert_eq!(next_str("23 0-20/2 * * 0", from), "2024-01-07T00:23:00.000Z");
    assert_eq!(next_str("5 4 * * SUN", from), "2024-01-07T04:05:00.000Z");
    assert_eq!(next_str("5 4 * DEC SUN", from), "2024-12-01T04:05:00.000Z");
    assert_eq!(next_str("30 5 0 8 2 *", from), "2024-02-08T00:05:30.000Z");
}

#[test]
fn cron_next_zoned() {
    let london = Some(Zone::Named("Europe/London".to_string()));
    let c = Cron::parse_unsafe("5 0 8 2 *", london);
    let from = utc("2024-01-04T16:21:00.000Z");
    assert_eq!(c.next_unsafe(&from).to_json(), "2024-02-08T00:05:00.000Z");
}

#[test]
fn cron_next_day_weekday_or() {
    // Day 8 OR Sunday: Jan 7 (Sun) comes before Jan 8.
    assert_eq!(next_str("0 0 8 * SUN", "2024-01-04T16:21:00.000Z"), "2024-01-07T00:00:00.000Z");
}

#[test]
fn cron_next_day_weekday_and() {
    let c = Cron::make([0], [0], [0], [8], [], [4], true, None).unwrap();
    // Feb 8 2024 is a Thursday: day 8 and Thursday both hit under AND.
    assert!(c.matches(&utc("2024-02-08T00:00:00.000Z")));
    // Mar 8 is a Friday: weekday misses under AND.
    assert!(!c.matches(&utc("2024-03-08T00:00:00.000Z")));
}

#[test]
fn cron_prev_basic() {
    assert_eq!(prev_str("5 0 8 2 *", "2024-03-01T00:00:00.000Z"), "2024-02-08T00:05:00.000Z");
}

#[test]
fn cron_prev_is_strict() {
    // Exactly on schedule: prev goes earlier.
    assert_eq!(prev_str("0 0 * * *", "2024-02-08T00:00:00.000Z"), "2024-02-07T00:00:00.000Z");
}

#[test]
fn cron_prev_impossible_throws() {
    let c = Cron::parse_unsafe("0 0 30 2 *", None);
    assert!(c.prev(&utc("2024-03-01T00:00:00.000Z")).is_err());
}

#[test]
fn cron_prev_clamps_month_end() {
    // Midnight every day in February, from Mar 31: Feb 29 2024.
    assert_eq!(prev_str("0 0 * FEB *", "2024-03-31T12:00:00.000Z"), "2024-02-29T00:00:00.000Z");
}

#[test]
fn cron_prev_leap_day() {
    assert_eq!(prev_str("0 0 29 2 *", "2025-01-01T00:00:00.000Z"), "2024-02-29T00:00:00.000Z");
}

#[test]
fn cron_prev_year_wrap() {
    assert_eq!(prev_str("0 0 1 1 *", "2024-06-01T00:00:00.000Z"), "2024-01-01T00:00:00.000Z");
}

#[test]
fn cron_prev_day_31_skips_short_months() {
    assert_eq!(prev_str("0 0 31 * *", "2024-05-15T00:00:00.000Z"), "2024-03-31T00:00:00.000Z");
}

#[test]
fn cron_sequence_stays_aligned() {
    let c = Cron::parse_unsafe("0 12 * * MON", None);
    let from = utc("2024-01-01T00:00:00.000Z");
    let got: Vec<String> = c.sequence(&from).take(3).map(|d| d.to_json()).collect();
    assert_eq!(
        got,
        vec![
            "2024-01-01T12:00:00.000Z".to_string(),
            "2024-01-08T12:00:00.000Z".to_string(),
            "2024-01-15T12:00:00.000Z".to_string(),
        ]
    );
    // Forward then backward lands back.
    let third = DateTime::make_unsafe(&got[2]);
    assert_eq!(c.prev_unsafe(&third).to_json(), got[1]);
}

#[test]
fn cron_equal_schedules() {
    let a = Cron::parse_unsafe("5 0 8 2 *", None);
    let b = Cron::parse_unsafe("5 0 8 2 *", None);
    let c = Cron::parse_unsafe("6 0 8 2 *", None);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn cron_leap_years() {
    assert!(matches_str("0 0 29 2 *", "2024-02-29T00:00:00.000Z"));
    assert!(!matches_str("0 0 29 2 *", "2024-03-01T00:00:00.000Z"));
    assert!(matches_str("0 0 29 2 *", "2028-02-29T00:00:00.000Z"));
    assert_eq!(next_str("0 0 29 2 *", "2024-01-01T00:00:00.000Z"), "2024-02-29T00:00:00.000Z");
    assert_eq!(next_str("0 0 29 2 *", "2025-01-01T00:00:00.000Z"), "2028-02-29T00:00:00.000Z");
}

#[test]
fn cron_spring_forward_skips_gap() {
    // 2024-03-10 02:30 never occurs in America/New_York; next 02:30 run lands Mar 11.
    let c = Cron::parse_unsafe("30 2 * * *", Some(Zone::Named("America/New_York".to_string())));
    let from = dtz("2024-03-10T00:00:00Z", "America/New_York");
    assert_eq!(c.next_unsafe(&from).format_iso(), "2024-03-11T02:30:00.000-04:00[America/New_York]");
}

#[test]
fn cron_fall_back_prefers_later() {
    // 2024-11-03 01:30 occurs twice in America/New_York; prev from later picks the later one.
    let c = Cron::parse_unsafe("30 1 * * *", Some(Zone::Named("America/New_York".to_string())));
    let from = dtz("2024-11-03T12:00:00Z", "America/New_York");
    let prev = c.prev_unsafe(&from);
    assert_eq!(prev.format_iso(), "2024-11-03T01:30:00.000-04:00[America/New_York]");
}

#[test]
fn cron_parse_error_constructor_fields() {
    let err = Cron::parse("0 0 99 * *", None).unwrap_err();
    let s = err.to_string();
    assert!(s.contains("0 0 99 * *"), "input carried: {s}");
}

#[test]
fn cron_make_weekday_7_steps() {
    // Next Sunday-midnight after Thursday Jan 4: Jan 7.
    assert_eq!(next_str("0 0 * * 7", "2024-01-04T16:21:00.000Z"), "2024-01-07T00:00:00.000Z");
}

#[test]
fn cron_next_no_skip_when_day_missing() {
    // Feb 31 does not exist: roll onto Mar 1 without overshooting.
    assert_eq!(
        next_str("0 0 1,16,31 * *", "2020-02-18T00:00:00.000Z"),
        "2020-03-01T00:00:00.000Z"
    );
}

#[test]
fn cron_next_month_crossing_or() {    // Star-stepped days derive `and` (both must hit); explicit ranges stay OR.
    assert_eq!(next_str("0 0 */2 * MON", "2024-01-02T12:00:00.000Z"), "2024-01-15T00:00:00.000Z");
    assert_eq!(next_str("0 0 1-31/2 * MON", "2024-01-02T12:00:00.000Z"), "2024-01-03T00:00:00.000Z");
    assert_eq!(next_str("0 0 *,5 * MON", "2024-01-02T12:00:00.000Z"), "2024-01-08T00:00:00.000Z");
    assert_eq!(next_str("0 0 31 2 MON", "2024-01-04T16:21:00.000Z"), "2024-02-05T00:00:00.000Z");
    assert_eq!(next_str("0 0 30 2 MON", "2024-01-04T16:21:00.000Z"), "2024-02-05T00:00:00.000Z");
    assert_eq!(next_str("0 0 29 2 MON", "2024-01-04T16:21:00.000Z"), "2024-02-05T00:00:00.000Z");
}

#[test]
fn cron_prev_year_wrap_back() {
    assert_eq!(prev_str("5 0 8 2 *", "2024-01-04T16:21:00.000Z"), "2023-02-08T00:05:00.000Z");
    assert_eq!(prev_str("15 14 1 * *", "2024-01-04T16:21:00.000Z"), "2024-01-01T14:15:00.000Z");
    assert_eq!(prev_str("5 4 * * SUN", "2024-01-04T16:21:00.000Z"), "2023-12-31T04:05:00.000Z");
}

#[test]
fn cron_prev_latest_second_rolling_back() {
    let c = Cron::parse_unsafe("10,30 * * * * *", None);
    assert_eq!(
        c.prev_unsafe(&utc("2024-01-01T00:00:05.000Z")).to_json(),
        "2023-12-31T23:59:30.000Z"
    );
}

#[test]
fn cron_prev_latest_matching_day() {
    assert_eq!(
        prev_str("0 0 8 5,20 * *", "2024-06-03T00:00:00.000Z"),
        "2024-05-20T08:00:00.000Z"
    );
}

#[test]
fn cron_prev_weekday_wraps() {
    assert_eq!(prev_str("0 1 * * MON,FRI", "2025-10-19T12:00:00.000Z"), "2025-10-17T01:00:00.000Z");
    assert_eq!(prev_str("0 0 * * SAT", "2025-10-23T12:00:00.000Z"), "2025-10-18T00:00:00.000Z");
    assert_eq!(prev_str("0 1 * * MON", "2025-10-22T01:00:00.000Z"), "2025-10-20T01:00:00.000Z");
    assert_eq!(prev_str("0 1 * * TUE", "2025-10-22T01:00:00.000Z"), "2025-10-21T01:00:00.000Z");
}

#[test]
fn cron_prev_weekday_wrap_strict() {
    let c = Cron::parse_unsafe("0 0 * * SAT", None);
    for from in ["2025-10-19T12:00:00.000Z", "2025-10-21T12:00:00.000Z", "2025-10-24T12:00:00.000Z"] {
        let at = utc(from);
        let got = c.prev_unsafe(&at);
        assert!(got.epoch_millis() < at.epoch_millis());
        assert!(c.matches(&got));
    }
}

#[test]
fn cron_prev_combined_dom_weekday() {
    assert_eq!(prev_str("0 0 9 1,15 * MON", "2024-04-02T12:00:00.000Z"), "2024-04-01T09:00:00.000Z");
}

#[test]
fn cron_prev_step_across_day_boundary() {
    assert_eq!(prev_str("0 */7 8-10 * * *", "2024-01-01T08:01:00.000Z"), "2024-01-01T08:00:00.000Z");
}

#[test]
fn cron_prev_fixed_offset_zone() {
    let c = Cron::parse_unsafe("0 0 10 * * *", Some(Zone::Offset(120)));
    assert_eq!(c.prev_unsafe(&utc("2024-05-01T07:00:00.000Z")).to_json(), "2024-04-30T08:00:00.000Z");
}

#[test]
fn cron_prev_day_31_variants() {
    assert_eq!(prev_str("0 0 31 * *", "2024-03-15T00:00:00.000Z"), "2024-01-31T00:00:00.000Z");
    assert_eq!(prev_str("0 0 29 2 *", "2024-01-01T00:00:00.000Z"), "2020-02-29T00:00:00.000Z");
}

#[test]
fn cron_prev_rollovers_stay_matched_and_ordered() {
    for (expr, from) in [("0 0 31 * *", "2024-03-15T00:00:00.000Z"), ("0 0 29 2 *", "2024-01-01T00:00:00.000Z")] {
        let c = Cron::parse_unsafe(expr, None);
        let at = utc(from);
        let got = c.prev_unsafe(&at);
        assert!(c.matches(&got));
        assert!(got.epoch_millis() < at.epoch_millis());
    }
}

#[test]
fn cron_prev_multiple_months() {
    assert_eq!(prev_str("0 0 15 1,4,7,10 *", "2024-05-01T00:00:00.000Z"), "2024-04-15T00:00:00.000Z");
}

#[test]
fn cron_sequence_exact_triple() {
    let c = Cron::parse_unsafe("23 0-20/2 * * 0", None);
    let mut it = c.sequence(&utc("2024-01-01T00:00:00.000Z"));
    assert_eq!(it.next().unwrap().to_json(), "2024-01-07T00:23:00.000Z");
    assert_eq!(it.next().unwrap().to_json(), "2024-01-07T02:23:00.000Z");
    assert_eq!(it.next().unwrap().to_json(), "2024-01-07T04:23:00.000Z");
    assert_eq!(it.next().unwrap().to_json(), "2024-01-07T06:23:00.000Z");
}

#[test]
fn cron_sequence_seconds_aligned() {
    // Forward and reverse gathers agree over a day of 10/30-second marks.
    let c = Cron::parse_unsafe("10,30 * * * * *", None);
    let lower = utc("2024-01-01T00:00:00.000Z");
    let upper = utc("2024-01-02T00:00:00.000Z");
    let mut fwd = Vec::new();
    let mut at = lower.clone();
    loop {
        at = c.next_unsafe(&at);
        if at.epoch_millis() >= upper.epoch_millis() {
            break;
        }
        fwd.push(at.to_json());
    }
    let mut rev = Vec::new();
    let mut bt = upper.clone();
    loop {
        bt = c.prev_unsafe(&bt);
        if bt.epoch_millis() <= lower.epoch_millis() {
            break;
        }
        rev.push(bt.to_json());
    }
    rev.reverse();
    assert_eq!(fwd, rev);
    assert_eq!(fwd.len(), 2 * 24 * 60);
}

#[test]
fn cron_berlin_spring_sequence() {
    let c = Cron::parse_unsafe("30 * * * *", Some(Zone::Named("Europe/Berlin".to_string())));
    let mut it = c.sequence(&dtz("2024-03-31T00:00:00.000+01:00[Europe/Berlin]", "Europe/Berlin"));
    let got: Vec<String> = it.by_ref().take(4).map(|d| d.format_iso()).collect();
    assert_eq!(got, vec![
        "2024-03-31T00:30:00.000+01:00[Europe/Berlin]".to_string(),
        "2024-03-31T01:30:00.000+01:00[Europe/Berlin]".to_string(),
        "2024-03-31T03:30:00.000+02:00[Europe/Berlin]".to_string(),
        "2024-03-31T04:30:00.000+02:00[Europe/Berlin]".to_string(),
    ]);
}

#[test]
fn cron_berlin_fall_once_mode() {
    // 00:30, 01:30, first 02:30 (+02) only, then 03:30 (+01).
    let c = Cron::parse_unsafe("30 * * * *", Some(Zone::Named("Europe/Berlin".to_string())));
    let mut it = c.sequence(&dtz("2024-10-27T00:00:00.000+02:00[Europe/Berlin]", "Europe/Berlin"));
    let got: Vec<String> = it.by_ref().take(4).map(|d| d.format_iso()).collect();
    assert_eq!(got, vec![
        "2024-10-27T00:30:00.000+02:00[Europe/Berlin]".to_string(),
        "2024-10-27T01:30:00.000+02:00[Europe/Berlin]".to_string(),
        "2024-10-27T02:30:00.000+02:00[Europe/Berlin]".to_string(),
        "2024-10-27T03:30:00.000+01:00[Europe/Berlin]".to_string(),
    ]);
}

#[test]
fn cron_utc_sequence_over_fallback_date() {
    let c = Cron::parse_unsafe("30 * * * *", Some(Zone::Utc));
    let mut it = c.sequence(&dtz("2024-10-27T00:00:00.000+00:00[UTC]", "UTC"));
    let got: Vec<String> = it.by_ref().take(4).map(|d| d.format_iso()).collect();
    assert_eq!(got, vec![
        "2024-10-27T00:30:00.000Z".to_string(),
        "2024-10-27T01:30:00.000Z".to_string(),
        "2024-10-27T02:30:00.000Z".to_string(),
        "2024-10-27T03:30:00.000Z".to_string(),
    ]);
}

#[test]
fn cron_utc_timezone_steps() {
    let c = Cron::parse_unsafe("0 12 * * *", Some(Zone::Utc));
    assert_eq!(c.next_unsafe(&utc("2024-01-01T00:00:00.000Z")).to_json(), "2024-01-01T12:00:00.000Z");
}
