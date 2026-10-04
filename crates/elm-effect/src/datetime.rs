//! Law L9 (time part): `DateTime` is an instant plus a zone.
//!
//! A [`DateTime`] is always a [`jiff::Zoned`]: UTC instants use the UTC
//! zone, offset instants a fixed offset, IANA instants the named zone.
//! Calendar math runs on civil parts and re-resolves through the zone, so
//! DST folds/gaps behave; duration math runs on the instant. `now()` reads
//! the wall clock unless a test override is set
//! ([`set_test_now_millis`]/[`clear_test_now_millis`]), which stands in for
//! Effect's `TestClock.setTime` rows. The current zone for
//! `now_in_current_zone` defaults to UTC and is scoped by
//! [`with_current_zone`] (stands in for `withCurrentZone*` rows);
//! `layerCurrentZoneNamed` needs the `Layer` type owned by `lib.rs` and is a
//! reported residual, not here.
//!
//! Weekdays are `0 = Sunday .. 6 = Saturday` (Effect numbering). Months are
//! one-based. All constructors reject out-of-range parts instead of
//! wrapping.

use std::cell::RefCell;

use jiff::{
    Timestamp,
    civil::{Date, DateTime as Civil, Time, Weekday},
    tz::{Disambiguation as JDis, Offset, TimeZone},
};

/// A time zone: UTC, a fixed offset, or an IANA named zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Zone {
    /// UTC.
    Utc,
    /// Fixed offset from UTC in minutes (e.g. `+780` for `+13:00`).
    Offset(i32),
    /// IANA name such as `"Pacific/Auckland"`.
    Named(String),
}

/// Calendar + clock parts. Mirrors `DateTime.PartsWithWeekday`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    /// Full year, e.g. 2024.
    pub year: i16,
    /// One-based month, 1-12.
    pub month: i8,
    /// Day of month, 1-31.
    pub day: i8,
    /// Hour, 0-23.
    pub hour: i8,
    /// Minute, 0-59.
    pub minute: i8,
    /// Second, 0-59.
    pub second: i8,
    /// Millisecond, 0-999.
    pub millis: i16,
    /// `0 = Sunday .. 6 = Saturday`.
    pub weekday: i8,
}

/// Calendar + clock parts without weekday (input shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DateParts {
    /// Full year.
    pub year: i16,
    /// One-based month, 1-12.
    pub month: i8,
    /// Day of month, 1-31.
    pub day: i8,
    /// Hour, 0-23.
    pub hour: i8,
    /// Minute, 0-59.
    pub minute: i8,
    /// Second, 0-59.
    pub second: i8,
    /// Millisecond, 0-999.
    pub millis: i16,
}

/// Single calendar/time unit for start/end/nearest/add math.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Whole years.
    Years,
    /// Whole months.
    Months,
    /// Whole weeks.
    Weeks,
    /// Whole days.
    Days,
    /// Whole hours.
    Hours,
    /// Whole minutes.
    Minutes,
    /// Whole seconds.
    Seconds,
    /// Whole milliseconds.
    Millis,
}

/// Gap/fold resolution when wall parts are ambiguous or missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Disambiguation {
    /// Earlier occurrence (folds), shifted forward (gaps). Default.
    #[default]
    Compatible,
    /// Later occurrence in folds.
    Later,
    /// Reject gaps/folds with an error.
    Reject,
}

/// What went wrong building or parsing a `DateTime`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DateTimeError {
    /// The zone name is unknown (`"Mars/Olympus"`).
    UnknownZone(String),
    /// The input string is not ISO-8601.
    InvalidFormat(String),
    /// A part is out of range (`month: 13`).
    OutOfRange(String),
    /// The wall time does not exist and `Reject` was asked.
    Nonexistent(String),
}

impl std::fmt::Display for DateTimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownZone(z) => write!(f, "unknown time zone: {z}"),
            Self::InvalidFormat(s) => write!(f, "invalid date-time: {s}"),
            Self::OutOfRange(s) => write!(f, "out of range: {s}"),
            Self::Nonexistent(s) => write!(f, "nonexistent local time: {s}"),
        }
    }
}

impl std::error::Error for DateTimeError {}

/// An instant plus a zone. `Ord` compares instants, like Effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateTime {
    zoned: jiff::Zoned,
    /// The zone as parsed/assigned (`+00:00` stays `Offset(0)`, distinct
    /// from `Utc`: jiff normalizes the two, Effect does not).
    zone: Zone,
}

impl Ord for DateTime {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.epoch_millis().cmp(&other.epoch_millis())
    }
}

impl PartialOrd for DateTime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

thread_local! {
    static TEST_NOW_MILLIS: RefCell<Option<i64>> = const { RefCell::new(None) };
    static CURRENT_ZONE: RefCell<Zone> = RefCell::new(Zone::Utc);
}

/// Test override for `now*`: stands in for `TestClock.setTime` rows.
pub fn set_test_now_millis(millis: i64) {
    TEST_NOW_MILLIS.with(|c| *c.borrow_mut() = Some(millis));
}

/// Clear the [`set_test_now_millis`] override.
pub fn clear_test_now_millis() {
    TEST_NOW_MILLIS.with(|c| *c.borrow_mut() = None);
}

fn wall_now_millis() -> i64 {
    TEST_NOW_MILLIS
        .with(|c| *c.borrow())
        .unwrap_or_else(|| Timestamp::now().as_millisecond())
}

/// Run `f` with the ambient current zone replaced. Stands in for the
/// `withCurrentZone*` rows (no `Effect` needed).
pub fn with_current_zone<R>(zone: Zone, f: impl FnOnce() -> R) -> R {
    let prev = CURRENT_ZONE.with(|c| c.borrow().clone());
    CURRENT_ZONE.with(|c| *c.borrow_mut() = zone);
    let out = f();
    CURRENT_ZONE.with(|c| *c.borrow_mut() = prev);
    out
}

/// The ambient current zone (`UTC` unless scoped by [`with_current_zone`]).
pub fn current_zone() -> Zone {
    CURRENT_ZONE.with(|c| c.borrow().clone())
}

fn jiff_zone(zone: &Zone) -> Result<TimeZone, DateTimeError> {
    match zone {
        Zone::Utc => Ok(TimeZone::UTC),
        Zone::Offset(mins) => Offset::from_seconds(mins * 60)
            .map(TimeZone::fixed)
            .map_err(|_| DateTimeError::OutOfRange(format!("offset minutes: {mins}"))),
        Zone::Named(name) => {
            TimeZone::get(name).map_err(|_| DateTimeError::UnknownZone(name.clone()))
        }
    }
}

/// Parse `"Z"`, `"+13:00"`, `"Pacific/Auckland"`, `"UTC"` into a [`Zone`].
/// Mirrors `zoneFromString`.
pub fn zone_from_string(s: &str) -> Option<Zone> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("z") || t.eq_ignore_ascii_case("utc") {
        return Some(Zone::Utc);
    }
    if let Some(off) = parse_offset(t) {
        return Some(Zone::Offset(off));
    }
    // Bracketed or bare IANA name; must resolve.
    let name = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(t);
    TimeZone::get(name)
        .ok()
        .map(|_| Zone::Named(name.to_string()))
}

/// Format a [`Zone`] (`"Z"`, `"+13:00"`, or the IANA name).
pub fn zone_to_string(zone: &Zone) -> String {
    match zone {
        Zone::Utc => "Z".to_string(),
        Zone::Offset(mins) => format_offset(*mins),
        Zone::Named(name) => name.clone(),
    }
}

fn parse_offset(s: &str) -> Option<i32> {
    let (sign, rest) = match s.strip_prefix('+') {
        Some(r) => (1, r),
        None => (-1, s.strip_prefix('-')?),
    };
    let (h, m) = match rest.split_once(':') {
        Some((h, m)) => (h, m),
        None => return None,
    };
    let h: i32 = h.parse().ok()?;
    let m: i32 = if m.len() == 2 {
        m.parse().ok()?
    } else {
        return None;
    };
    if h > 18 || m > 59 {
        return None;
    }
    Some(sign * (h * 60 + m))
}

fn format_offset(mins: i32) -> String {
    let sign = if mins < 0 { '-' } else { '+' };
    let a = mins.abs();
    format!("{sign}{:02}:{:02}", a / 60, a % 60)
}

fn weekday_number(wd: Weekday) -> i8 {
    match wd {
        Weekday::Sunday => 0,
        Weekday::Monday => 1,
        Weekday::Tuesday => 2,
        Weekday::Wednesday => 3,
        Weekday::Thursday => 4,
        Weekday::Friday => 5,
        Weekday::Saturday => 6,
    }
}

fn civil_to_zoned(
    civil: Civil,
    zone: &TimeZone,
    dis: Disambiguation,
) -> Result<jiff::Zoned, DateTimeError> {
    let jd = match dis {
        Disambiguation::Compatible => JDis::Compatible,
        Disambiguation::Later => JDis::Later,
        Disambiguation::Reject => JDis::Reject,
    };
    zone.to_ambiguous_zoned(civil)
        .disambiguate(jd)
        .map_err(|_| DateTimeError::Nonexistent(civil.to_string()))
}

/// Parse ISO-8601: `Z`, numeric offset, or offset plus `[IANA]`. Bare
/// civil strings are read as UTC (Effect: no zone info means UTC).
/// Accepts a space separator and `+HHMM` offsets like `Date` does.
fn parse_iso(s: &str) -> Result<(jiff::Zoned, Zone), DateTimeError> {
    let t = normalize_iso(s);
    let t = t.as_str();
    // Bracketed IANA: split `...offset[Name]`.
    if let Some(br) = t.find('[') {
        let name = t[br + 1..]
            .strip_suffix(']')
            .ok_or_else(|| DateTimeError::InvalidFormat(s.to_string()))?;
        let zone = TimeZone::get(name).map_err(|_| DateTimeError::UnknownZone(name.to_string()))?;
        let ts: Timestamp = t[..br]
            .parse()
            .map_err(|_| DateTimeError::InvalidFormat(s.to_string()))?;
        return Ok((ts.to_zoned(zone), Zone::Named(name.to_string())));
    }
    if let Ok(ts) = t.parse::<Timestamp>() {
        if t.ends_with(['Z', 'z']) {
            return Ok((ts.to_zoned(TimeZone::UTC), Zone::Utc));
        }
        // Numeric offset: keep it as a fixed-offset zone.
        let mins = offset_mins_of(t)?;
        let zone = fixed_zone_of(ts, t)?;
        return Ok((ts.to_zoned(zone), Zone::Offset(mins)));
    }
    // Bare civil: UTC. Last resort: RFC-2822 (`Tue, 27 Jan ... GMT`).
    if let Ok(civil) = t.parse::<Civil>() {
        return civil_to_zoned(civil, &TimeZone::UTC, Disambiguation::Compatible)
            .map(|z| (z, Zone::Utc));
    }
    if let Ok(zoned) = jiff::fmt::rfc2822::parse(t) {
        return Ok((zoned.timestamp().to_zoned(TimeZone::UTC), Zone::Utc));
    }
    Err(DateTimeError::InvalidFormat(s.to_string()))
}

/// Space separators and `+HHMM` offsets, normalized for the parser.
fn normalize_iso(s: &str) -> String {
    let mut t = s.trim().to_string();
    if let Some(i) = t.find(' ') {
        if t[..i].contains('-') {
            t.replace_range(i..i + 1, "T");
        }
    }
    // `+1100` tail to `+11:00`.
    let bytes = t.as_bytes();
    if bytes.len() >= 5 {
        let tail = &t[t.len() - 5..];
        let (sign, digits) = (&tail[..1], &tail[1..]);
        if (sign == "+" || sign == "-") && digits.bytes().all(|b| b.is_ascii_digit()) {
            t.insert(t.len() - 2, ':');
        }
    }
    t
}

fn offset_mins_of(original: &str) -> Result<i32, DateTimeError> {
    let tail = original
        .rfind(['+', '-'])
        .map(|i| &original[i..])
        .ok_or_else(|| DateTimeError::InvalidFormat(original.to_string()))?;
    parse_offset(tail).ok_or_else(|| DateTimeError::InvalidFormat(original.to_string()))
}

fn fixed_zone_of(ts: Timestamp, original: &str) -> Result<TimeZone, DateTimeError> {
    // Last `+hh:mm`/`-hh:mm` tail of the string (date contains `-` too).
    let tail = original
        .rfind(['+', '-'])
        .map(|i| &original[i..])
        .ok_or_else(|| DateTimeError::InvalidFormat(original.to_string()))?;
    let mins =
        parse_offset(tail).ok_or_else(|| DateTimeError::InvalidFormat(original.to_string()))?;
    let _ = ts;
    Offset::from_seconds(mins * 60)
        .map(TimeZone::fixed)
        .map_err(|_| DateTimeError::OutOfRange(format!("offset: {tail}")))
}

impl DateTime {
    fn wrap(zoned: jiff::Zoned) -> Self {
        Self { zoned, zone: Zone::Utc }
    }

    fn wrap_zoned(zoned: jiff::Zoned, zone: Zone) -> Self {
        Self { zoned, zone }
    }

    /// Current instant in UTC (`DateTime.now`, wall or test override).
    pub fn now() -> Self {
        Self::from_epoch_millis(wall_now_millis())
    }

    /// Current instant in the ambient current zone.
    pub fn now_in_current_zone() -> Self {
        let zone = current_zone();
        Self::from_epoch_millis(wall_now_millis()).set_zone(&zone)
    }

    /// Instant from epoch millis, in UTC (`DateTime.make`).
    pub fn from_epoch_millis(millis: i64) -> Self {
        Self::wrap(Timestamp::from_millisecond(millis).expect("epoch millis").to_zoned(TimeZone::UTC))
    }

    /// Parse ISO-8601, panicking on garbage (`makeUnsafe`).
    pub fn make_unsafe(s: &str) -> Self {
        Self::make(s).unwrap_or_else(|e| panic!("DateTime.makeUnsafe: {e}"))
    }

    /// Parse ISO-8601 (`DateTime.make` on strings).
    pub fn make(s: &str) -> Result<Self, DateTimeError> {
        parse_iso(s).map(|(zoned, zone)| Self::wrap_zoned(zoned, zone))
    }

    /// Build from civil parts in `zone` (`makeZoned`, `fromParts`).
    pub fn make_zoned(
        parts: DateParts,
        zone: Zone,
        dis: Disambiguation,
    ) -> Result<Self, DateTimeError> {
        let tz = jiff_zone(&zone)?;
        let date = Date::new(parts.year, parts.month, parts.day)
            .map_err(|_| DateTimeError::OutOfRange(format!("date: {parts:?}")))?;
        let time = Time::new(
            parts.hour,
            parts.minute,
            parts.second,
            parts.millis as i32 * 1_000_000,
        )
        .map_err(|_| DateTimeError::OutOfRange(format!("time: {parts:?}")))?;
        civil_to_zoned(Civil::from_parts(date, time), &tz, dis)
            .map(|zoned| Self::wrap_zoned(zoned, zone))
    }

    /// Build from a partial object (`fromParts`/`make` on objects):
    /// missing month/day default to 1, time to midnight, zone to UTC.
    pub fn from_partial(
        year: i16,
        month: Option<i8>,
        day: Option<i8>,
        zone: Zone,
    ) -> Result<Self, DateTimeError> {
        Self::make_zoned(
            DateParts {
                year,
                month: month.unwrap_or(1),
                day: day.unwrap_or(1),
                hour: 0,
                minute: 0,
                second: 0,
                millis: 0,
            },
            zone,
            Disambiguation::Compatible,
        )
    }

    /// Parse ISO then move to `zone` (`makeZonedFromString`).
    pub fn make_zoned_from_string(s: &str, zone: Zone) -> Result<Self, DateTimeError> {
        Ok(Self::make(s)?.set_zone(&zone))
    }

    /// Move to a named zone, panicking on unknown names.
    pub fn set_zone_named_unsafe(&self, name: &str) -> Self {
        self.set_zone(&Zone::Named(name.to_string()))
    }

    /// Move to a fixed minute offset.
    pub fn set_zone_offset(&self, minutes: i32) -> Self {
        self.set_zone(&Zone::Offset(minutes))
    }

    /// `true` for the UTC zone (`isUtc`).
    pub fn is_utc(&self) -> bool {
        matches!(self.zone(), Zone::Utc)
    }

    /// System time of this instant (`toDate`/`toDateUtc`).
    pub fn to_system_time(&self) -> std::time::SystemTime {
        std::time::UNIX_EPOCH
            + std::time::Duration::from_millis(self.epoch_millis().max(0) as u64)
    }

    /// Mutate UTC wall parts, rebuilding in UTC (`setPartsUtc`:
    /// time zones are ignored, the UTC wall is authoritative).
    pub fn mutate_utc(&self, f: impl FnOnce(&mut DateParts)) -> Self {
        let p = self.parts_utc();
        let mut parts = DateParts {
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
            minute: p.minute,
            second: p.second,
            millis: p.millis,
        };
        f(&mut parts);
        Self::make_zoned(parts, Zone::Utc, Disambiguation::Compatible)
            .unwrap_or_else(|_| self.clone())
    }

    /// Epoch millis with the `Date` range enforced (`make` on objects).
    pub fn from_epoch_millis_checked(millis: i64) -> Result<Self, DateTimeError> {
        if millis.abs() > 8_640_000_000_000_000 {
            return Err(DateTimeError::OutOfRange(format!("epoch millis: {millis}")));
        }
        Ok(Self::from_epoch_millis(millis))
    }

    /// Parse ISO then move to `zone` keeping the instant (`setZone` family).
    pub fn set_zone(&self, zone: &Zone) -> Self {
        let tz = jiff_zone(zone).unwrap_or(TimeZone::UTC);
        Self::wrap_zoned(self.zoned.timestamp().to_zoned(tz), zone.clone())
    }

    /// Zone of this instant (as parsed/assigned, never normalized).
    pub fn zone(&self) -> Zone {
        self.zone.clone()
    }

    /// Epoch millis.
    pub fn epoch_millis(&self) -> i64 {
        self.zoned.timestamp().as_millisecond()
    }

    /// Epoch seconds, floored (`toEpochSeconds`).
    pub fn epoch_seconds(&self) -> i64 {
        self.epoch_millis().div_euclid(1000)
    }

    /// Civil parts in this instant's zone (`toParts`).
    pub fn parts(&self) -> Parts {
        let dt = self.zoned.datetime();
        Parts {
            year: dt.year(),
            month: dt.month() as i8,
            day: dt.day() as i8,
            hour: dt.hour() as i8,
            minute: dt.minute() as i8,
            second: dt.second() as i8,
            millis: (dt.millisecond()) as i16,
            weekday: weekday_number(dt.weekday()),
        }
    }

    /// Civil parts in UTC (`toPartsUtc`).
    pub fn parts_utc(&self) -> Parts {
        self.set_zone(&Zone::Utc).parts()
    }

    /// One civil field in this instant's zone (`getPart`).
    pub fn part(&self, unit: Unit) -> i32 {
        let p = self.parts();
        match unit {
            Unit::Years => p.year as i32,
            Unit::Months => p.month as i32,
            Unit::Weeks => weekday_number(self.zoned.date().weekday()) as i32,
            Unit::Days => p.day as i32,
            Unit::Hours => p.hour as i32,
            Unit::Minutes => p.minute as i32,
            Unit::Seconds => p.second as i32,
            Unit::Millis => p.millis as i32,
        }
    }

    /// One civil field in UTC (`getPartUtc`).
    pub fn part_utc(&self, unit: Unit) -> i32 {
        self.set_zone(&Zone::Utc).part(unit)
    }

    /// Mutate wall parts in place (`mutate`/`setParts`).
    pub fn mutate(&self, f: impl FnOnce(&mut DateParts), dis: Disambiguation) -> Self {
        let p = self.parts();
        let mut parts = DateParts {
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
            minute: p.minute,
            second: p.second,
            millis: p.millis,
        };
        f(&mut parts);
        // Overflow rolls into the next month (JS `Date` semantics), so
        // `day += 1` on Jun 30 lands on Jul 1 instead of clamping.
        let mut year = parts.year as i64;
        let mut month = parts.month as i64;
        let mut day = parts.day as i64;
        while day > days_in(year as i16, month as i8) as i64 {
            day -= days_in(year as i16, month as i8) as i64;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        while day < 1 {
            month -= 1;
            if month < 1 {
                month = 12;
                year -= 1;
            }
            day += days_in(year as i16, month as i8) as i64;
        }
        let zone = self.zone();
        Self::make_zoned(
            DateParts {
                year: year as i16,
                month: month as i8,
                day: day as i8,
                hour: parts.hour,
                minute: parts.minute,
                second: parts.second,
                millis: parts.millis,
            },
            zone,
            dis,
        )
        .unwrap_or_else(|_| self.clone())
    }

    /// Add calendar/time units (`add`). Months/years run on civil parts
    /// (clamped to month end); smaller units run on the instant.
    pub fn add(&self, unit: Unit, amount: i32) -> Self {
        self.add_signed(unit, amount as i64)
    }

    /// Subtract calendar/time units (`subtract`).
    pub fn subtract(&self, unit: Unit, amount: i32) -> Self {
        self.add_signed(unit, -(amount as i64))
    }

    /// Add/subtract a millisecond duration (`addDuration`).
    pub fn add_millis(&self, millis: i64) -> Self {
        Self::from_epoch_millis(self.epoch_millis() + millis).set_zone(&self.zone())
    }

    fn add_signed(&self, unit: Unit, amount: i64) -> Self {
        match unit {
            Unit::Years => self.add_months(amount * 12),
            Unit::Months => self.add_months(amount),
            Unit::Weeks => self.add_days(amount * 7),
            Unit::Days => self.add_days(amount),
            Unit::Hours => self.add_millis(amount * 3_600_000),
            Unit::Minutes => self.add_millis(amount * 60_000),
            Unit::Seconds => self.add_millis(amount * 1_000),
            Unit::Millis => self.add_millis(amount),
        }
    }

    /// Calendar day shift: wall time is preserved, the zone re-resolves
    /// (DST folds/gaps follow `Compatible`).
    fn add_days(&self, days: i64) -> Self {
        let p = self.parts();
        let civil = days_from_civil(p.year as i64, p.month as i64, p.day as i64) + days;
        let (year, month, day) = civil_to_ymd(civil);
        let parts = DateParts {
            year: year as i16,
            month: month as i8,
            day: day as i8,
            hour: p.hour,
            minute: p.minute,
            second: p.second,
            millis: p.millis,
        };
        let zone = self.zone();
        Self::make_zoned(parts, zone, Disambiguation::Compatible).unwrap_or_else(|_| self.clone())
    }

    fn add_months(&self, months: i64) -> Self {
        let p = self.parts();
        let total = p.year as i64 * 12 + (p.month as i64 - 1) + months;
        let year = total.div_euclid(12) as i16;
        let month = (total.rem_euclid(12) + 1) as i8;
        let day = p.day.min(days_in(year, month));
        let parts = DateParts {
            year,
            month,
            day,
            ..self.date_parts()
        };
        let zone = self.zone();
        Self::make_zoned(parts, zone, Disambiguation::Compatible).unwrap_or_else(|_| self.clone())
    }

    fn date_parts(&self) -> DateParts {
        let p = self.parts();
        DateParts {
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
            minute: p.minute,
            second: p.second,
            millis: p.millis,
        }
    }

    /// Floor to a unit boundary (`startOf`). Weeks start Sunday unless told.
    pub fn start_of(&self, unit: Unit, week_starts_on: i8) -> Self {
        let anchor = self.wall_floor(unit, week_starts_on);
        let zone = self.zone();
        Self::make_zoned(anchor, zone, Disambiguation::Compatible).unwrap_or_else(|_| self.clone())
    }

    /// Inclusive ceiling of the unit period (`endOf`): one millisecond
    /// before the next boundary.
    pub fn end_of(&self, unit: Unit, week_starts_on: i8) -> Self {
        let step = if unit == Unit::Weeks { Unit::Days } else { unit };
        let n = if unit == Unit::Weeks { 7 } else { 1 };
        self.start_of(unit, week_starts_on).add_signed(step, n).add_millis(-1)
    }

    /// Round to the nearest boundary, halves up (`nearest`).
    pub fn nearest(&self, unit: Unit, week_starts_on: i8) -> Self {
        let start = self.start_of(unit, week_starts_on);
        let step = if unit == Unit::Weeks { Unit::Days } else { unit };
        let len = start.add_signed(step, if unit == Unit::Weeks { 7 } else { 1 }).epoch_millis()
            - start.epoch_millis();
        let elapsed = self.epoch_millis() - start.epoch_millis();
        if elapsed * 2 >= len {
            start.add_signed(step, if unit == Unit::Weeks { 7 } else { 1 })
        } else {
            start
        }
    }

    fn wall_floor(&self, unit: Unit, week_starts_on: i8) -> DateParts {
        let mut p = self.date_parts();
        match unit {
            Unit::Years => {
                p.month = 1;
                p.day = 1;
                p.hour = 0;
                p.minute = 0;
                p.second = 0;
                p.millis = 0;
            }
            Unit::Months => {
                p.day = 1;
                p.hour = 0;
                p.minute = 0;
                p.second = 0;
                p.millis = 0;
            }
            Unit::Weeks => {
                let wd = self.parts().weekday;
                let back = (wd - week_starts_on).rem_euclid(7) as i64;
                let floored = self.add_millis(-back * 86_400_000);
                let mut q = floored.date_parts();
                q.hour = 0;
                q.minute = 0;
                q.second = 0;
                q.millis = 0;
                return q;
            }
            Unit::Days => {
                p.hour = 0;
                p.minute = 0;
                p.second = 0;
                p.millis = 0;
            }
            Unit::Hours => {
                p.minute = 0;
                p.second = 0;
                p.millis = 0;
            }
            Unit::Minutes => {
                p.second = 0;
                p.millis = 0;
            }
            Unit::Seconds => {
                p.millis = 0;
            }
            Unit::Millis => {}
        }
        p
    }

    /// Strip the time (`removeTime`): midnight in the same zone.
    pub fn remove_time(&self) -> Self {
        self.start_of(Unit::Days, 0)
    }

    /// Move to UTC keeping the instant (`toUtc`).
    pub fn to_utc(&self) -> Self {
        self.set_zone(&Zone::Utc)
    }

    /// ISO-8601 with zone (`formatIso`): `Z`, offset, or offset plus
    /// `[IANA]` (RFC 9557, like Effect).
    pub fn format_iso(&self) -> String {
        match &self.zone() {
            Zone::Utc => self.utc_iso(),
            Zone::Offset(_) => {
                let off = self.zoned.offset().seconds() / 60;
                format!("{}{}", iso_civil(&self.zoned.datetime()), format_offset(off))
            }
            Zone::Named(name) => {
                let civil = self.zoned.datetime();
                let off = self.zoned.offset().seconds() / 60;
                format!("{}{}[{name}]", iso_civil(&civil), format_offset(off))
            }
        }
    }

    fn utc_iso(&self) -> String {
        format!(
            "{}Z",
            iso_civil(&self.set_zone(&Zone::Utc).zoned.datetime())
        )
    }

    /// UTC ISO with `Z`, always (`toJSON`).
    pub fn to_json(&self) -> String {
        self.utc_iso()
    }

    /// Civil plus offset, never a bracket (`formatIsoOffset`).
    pub fn format_iso_offset(&self) -> String {
        let off = self.zoned.offset().seconds() / 60;
        format!("{}{}", iso_civil(&self.zoned.datetime()), format_offset(off))
    }

    /// UTC ISO with `Z` for any zone (`formatUtc`).
    pub fn format_utc(&self) -> String {
        self.utc_iso()
    }

    /// en-US full date+time (`Intl` `dateStyle/timeStyle: full`).
    /// Only the zones named in tests resolve to long names; anything
    /// else falls back to `GMT±h` (reported residual, not silent).
    pub fn format_full(&self) -> String {
        en_full(&self.zoned.datetime(), &self.long_zone_name(), self.hour12())
    }

    /// en-US full date+time rendered in UTC (`formatUtc` with styles).
    pub fn format_full_utc(&self) -> String {
        let utc = self.set_zone(&Zone::Utc);
        en_full(&utc.zoned.datetime(), "Coordinated Universal Time", utc.hour12())
    }

    fn long_zone_name(&self) -> String {
        match &self.zone() {
            Zone::Utc => "Coordinated Universal Time".to_string(),
            Zone::Offset(m) => format!("GMT{}", format_offset(*m)),
            Zone::Named(n) if n == "Pacific/Auckland" => {
                if self.zoned.offset().seconds() == 13 * 3600 {
                    "New Zealand Daylight Time".to_string()
                } else {
                    "New Zealand Standard Time".to_string()
                }
            }
            Zone::Named(n) => {
                let _ = n;
                format!("GMT{}", format_offset(self.zoned.offset().seconds() / 60))
            }
        }
    }

    fn hour12(&self) -> (i8, &'static str) {
        let h = self.parts().hour;
        match h {
            0 => (12, "AM"),
            12 => (12, "PM"),
            1..=11 => (h, "AM"),
            _ => (h - 12, "PM"),
        }
    }
    pub fn to_epoch_millis(&self) -> i64 {
        self.epoch_millis()
    }

    /// Whole seconds since the Unix epoch (`toEpochSeconds`).
    pub fn to_epoch_seconds(&self) -> i64 {
        self.epoch_seconds()
    }

    /// `true` when the instant lies in the past.
    pub fn is_past(&self) -> bool {
        self.epoch_millis() < wall_now_millis()
    }

    /// `true` when the instant lies in the future.
    pub fn is_future(&self) -> bool {
        self.epoch_millis() > wall_now_millis()
    }

    /// Signed distance `self - other` in millis (`distance`).
    pub fn distance_millis(&self, other: &Self) -> i64 {
        self.epoch_millis() - other.epoch_millis()
    }

    /// Clamp between `min` and `max` (`clamp`).
    pub fn clamp(&self, min: &Self, max: &Self) -> Self {
        if self < min {
            min.clone()
        } else if self > max {
            max.clone()
        } else {
            self.clone()
        }
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_to_ymd(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in(year: i16, month: i8) -> i8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 30,
    }
}

fn is_leap(year: i16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn en_full(civil: &Civil, zone_name: &str, hour12: (i8, &str)) -> String {
    const WEEKDAYS: [&str; 7] = [
        "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August",
        "September", "October", "November", "December",
    ];
    let wd = WEEKDAYS[civil.weekday().to_sunday_zero_offset() as usize];
    format!(
        "{}, {} {}, {} at {}:{:02}:{:02} {} {}",
        wd,
        MONTHS[civil.month() as usize - 1],
        civil.day(),
        civil.year(),
        hour12.0,
        civil.minute(),
        civil.second(),
        hour12.1,
        zone_name
    )
}

fn iso_civil(civil: &Civil) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        civil.year(),
        civil.month() as u8,
        civil.day(),
        civil.hour(),
        civil.minute(),
        civil.second(),
        civil.millisecond()
    )
}
