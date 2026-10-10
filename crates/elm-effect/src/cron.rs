//! Cron schedules (ADR-0002 Phase 3): parse, match, step, format.
//!
//! A [`Cron`] is six sorted value sets (seconds default `{0}`) plus an
//! optional zone and the day-match mode. `match` follows Effect exactly:
//! seconds/minutes/hours/months must hit (empty set is wildcard), then
//! days/weekdays combine by `and` (`&&`), single-set membership, or `||`
//! by default. `next`/`prev` are strict and step in the cron's zone with
//! fold/gap resolution (`Later` going back, `Compatible` going forward);
//! exhaustion is [`CronError::NoOccurrence`] (Effect throws).
//!
//! Self-contained on purpose: `lib.rs` is owned by the `.0c6.11` bead, so
//! this includes [`datetime`] by path instead of `crate::datetime`. The
//! follow-up is one `mod`/`pub use` pair plus dropping this shim.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
#[path = "datetime.rs"]
mod datetime;

// A test that includes this module by `#[path]` re-exports a wider set through
// it, so a few names are unused by this file itself. Allowing the single line
// keeps the parked module's surface intact for its includers.
#[allow(unused_imports)]
pub use datetime::{
    DateParts, DateTime, Disambiguation, Unit, Zone, clear_test_now_millis, current_zone,
    set_test_now_millis, with_current_zone, zone_from_string, zone_to_string,
};
use std::collections::BTreeSet;

/// Parse or schedule failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CronError {
    /// The expression or segment does not parse.
    Parse { message: String, input: String },
    /// No occurrence exists in the search window (Effect throws).
    NoOccurrence,
}

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse { message, input } => write!(f, "cron parse: {message} ({input})"),
            Self::NoOccurrence => write!(f, "cron: no occurrence in search window"),
        }
    }
}

impl std::error::Error for CronError {}

/// A parsed cron schedule.
#[derive(Clone, Debug)]
pub struct Cron {
    seconds: BTreeSet<i32>,
    minutes: BTreeSet<i32>,
    hours: BTreeSet<i32>,
    days: BTreeSet<i32>,
    months: BTreeSet<i32>,
    weekdays: BTreeSet<i32>,
    and: bool,
    zone: Option<Zone>,
}

impl PartialEq for Cron {
    fn eq(&self, other: &Self) -> bool {
        self.seconds == other.seconds
            && self.minutes == other.minutes
            && self.hours == other.hours
            && self.days == other.days
            && self.months == other.months
            && self.weekdays == other.weekdays
            && self.and == other.and
            && self.zone == other.zone
    }
}

impl Eq for Cron {}

const SEC: (i32, i32) = (0, 59);
const MIN: (i32, i32) = (0, 59);
const HOUR: (i32, i32) = (0, 23);
const DAY: (i32, i32) = (1, 31);
const MONTH: (i32, i32) = (1, 12);
const WDAY: (i32, i32) = (0, 7);

fn month_alias(name: &str) -> Option<i32> {
    match name {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

fn weekday_alias(name: &str) -> Option<i32> {
    match name {
        "sun" => Some(0),
        "mon" => Some(1),
        "tue" => Some(2),
        "wed" => Some(3),
        "thu" => Some(4),
        "fri" => Some(5),
        "sat" => Some(6),
        _ => None,
    }
}

fn split_step(field: &str) -> (&str, Option<&str>) {
    match field.split_once('/') {
        Some((raw, step)) => (raw, Some(step)),
        None => (field, None),
    }
}

fn atom_value(
    raw: &str,
    min: i32,
    max: i32,
    alias: &dyn Fn(&str) -> Option<i32>,
) -> Result<i32, ()> {
    let lower = raw.to_ascii_lowercase();
    if let Some(v) = alias(&lower) {
        return Ok(v);
    }
    lower.parse::<i32>().map_err(|_| ()).and_then(|v| {
        if (min..=max).contains(&v) {
            Ok(v)
        } else {
            Err(())
        }
    })
}

/// Parsed value set plus whether the segment led with `*` (drives `and`).
struct Segment {
    values: BTreeSet<i32>,
    wildcard: bool,
}

fn parse_segment(
    input: &str,
    min: i32,
    max: i32,
    alias: &dyn Fn(&str) -> Option<i32>,
    normalize: &dyn Fn(i32) -> i32,
    seg_input: &str,
) -> Result<Segment, CronError> {
    let fail = |message: &str| CronError::Parse {
        message: message.to_string(),
        input: seg_input.to_string(),
    };
    let mut values = BTreeSet::new();
    let fields: Vec<&str> = input.split(',').collect();
    let (first_raw, first_step) = split_step(fields[0]);
    let wildcard = first_raw == "*";
    // A leading bare `*` (no step, or step 1) voids every add: the field
    // stays a wildcard no matter what follows the comma.
    let void_add = wildcard && (first_step.is_none() || first_step == Some("1"));
    let add = |v: i32, values: &mut BTreeSet<i32>| {
        if !void_add {
            values.insert(normalize(v));
        }
    };
    for (index, field) in fields.iter().enumerate() {
        let (raw, step_raw) = if index == 0 {
            (first_raw, first_step)
        } else {
            split_step(field)
        };
        let step = match step_raw {
            None => None,
            Some(st) => {
                let v: i32 = st
                    .parse()
                    .map_err(|_| fail("Expected a positive integer"))?;
                if v < 1 {
                    return Err(fail("Expected step value to be greater than 0"));
                }
                if v > max {
                    return Err(fail("Expected step value within field maximum"));
                }
                Some(v)
            }
        };
        if raw == "*" {
            if index == 0 && step.unwrap_or(1) == 1 {
                continue;
            }
            let st = step.unwrap_or(1);
            for v in (min..=max).step_by(st as usize) {
                add(v, &mut values);
            }
            continue;
        }
        let (left_raw, right_raw) = match raw.split_once('-') {
            Some((l, r)) => (l, Some(r)),
            None => (raw, None),
        };
        let left =
            atom_value(left_raw, min, max, alias).map_err(|_| fail("Expected a value in range"))?;
        match right_raw {
            None => {
                let end = if step.is_some() { max } else { left };
                for v in (left..=end).step_by(step.unwrap_or(1) as usize) {
                    add(v, &mut values);
                }
            }
            Some(r) => {
                let right = atom_value(r, min, max, alias)
                    .map_err(|_| fail("Expected a value in range"))?;
                if right < min || right > max {
                    return Err(fail("Expected a value in range"));
                }
                if right < left {
                    return Err(fail("Invalid value range"));
                }
                for v in (left..=right).step_by(step.unwrap_or(1) as usize) {
                    add(v, &mut values);
                }
            }
        }
    }
    Ok(Segment { values, wildcard })
}

impl Cron {
    /// Build from explicit value sets (`Cron.make`). Empty seconds means
    /// `{0}`; weekday `7` normalizes to Sunday.
    pub fn make(
        seconds: impl IntoIterator<Item = i32>,
        minutes: impl IntoIterator<Item = i32>,
        hours: impl IntoIterator<Item = i32>,
        days: impl IntoIterator<Item = i32>,
        months: impl IntoIterator<Item = i32>,
        weekdays: impl IntoIterator<Item = i32>,
        and: bool,
        zone: Option<Zone>,
    ) -> Result<Self, CronError> {
        let check = |field: &str, min: i32, max: i32, vs: Vec<i32>| {
            for v in &vs {
                if !(min..=max).contains(v) {
                    return Err(CronError::Parse {
                        message: format!("{field} must be between {min} and {max}"),
                        input: format!("{v}"),
                    });
                }
            }
            Ok(vs.into_iter().collect::<BTreeSet<_>>())
        };
        let seconds: Vec<i32> = seconds.into_iter().collect();
        Ok(Self {
            seconds: check("seconds", 0, 59, seconds)?,
            minutes: check("minutes", 0, 59, minutes.into_iter().collect())?,
            hours: check("hours", 0, 23, hours.into_iter().collect())?,
            days: check("days", 1, 31, days.into_iter().collect())?,
            months: check("months", 1, 12, months.into_iter().collect())?,
            weekdays: check("weekdays", 0, 7, weekdays.into_iter().collect())?
                .into_iter()
                .map(|v| if v == 7 { 0 } else { v })
                .collect(),
            and,
            zone,
        })
    }

    /// Parse `"sec min hour day month weekday"` (seconds optional).
    /// `and` is derived like Effect: a leading `*` on days or weekdays
    /// with both sets non-empty requires both to hit.
    pub fn parse(input: &str, zone: Option<Zone>) -> Result<Self, CronError> {
        Self::parse_zoned(input, zone, None)
    }

    /// Parse with a string time zone (`parse` second argument).
    pub fn parse_zoned(
        input: &str,
        zone: Option<Zone>,
        tz: Option<&str>,
    ) -> Result<Self, CronError> {
        let zone = match (zone, tz) {
            (Some(z), _) => Some(z),
            (None, Some(t)) => {
                Some(
                    datetime::zone_from_string(t).ok_or_else(|| CronError::Parse {
                        message: "Invalid time zone in cron expression".to_string(),
                        input: t.to_string(),
                    })?,
                )
            }
            (None, None) => None,
        };
        let mut segs: Vec<&str> = input.trim().split_whitespace().collect();
        if segs.len() != 5 && segs.len() != 6 {
            return Err(CronError::Parse {
                message: "Invalid number of segments in cron expression".to_string(),
                input: input.to_string(),
            });
        }
        if segs.len() == 5 {
            segs.insert(0, "0");
        }
        let no_alias = &|_: &str| None;
        let ident = &|v: i32| v;
        let seven = &|v: i32| if v == 7 { 0 } else { v };
        let m_alias = &|s: &str| month_alias(s);
        let w_alias = &|s: &str| weekday_alias(s);
        let seconds = parse_segment(segs[0], SEC.0, SEC.1, no_alias, ident, input)?;
        let minutes = parse_segment(segs[1], MIN.0, MIN.1, no_alias, ident, input)?;
        let hours = parse_segment(segs[2], HOUR.0, HOUR.1, no_alias, ident, input)?;
        let days = parse_segment(segs[3], DAY.0, DAY.1, no_alias, ident, input)?;
        let months = parse_segment(segs[4], MONTH.0, MONTH.1, m_alias, ident, input)?;
        let weekdays = parse_segment(segs[5], WDAY.0, WDAY.1, w_alias, seven, input)?;
        let and = (days.wildcard || weekdays.wildcard)
            && !days.values.is_empty()
            && !weekdays.values.is_empty();
        let mut out = Self::make(
            seconds.values,
            minutes.values,
            hours.values,
            days.values,
            months.values,
            weekdays.values,
            and,
            zone,
        )?;
        out.and = and;
        Ok(out)
    }

    /// Parse, panicking on error (`parseUnsafe`).
    pub fn parse_unsafe(input: &str, zone: Option<Zone>) -> Self {
        Self::parse(input, zone).unwrap_or_else(|e| panic!("Cron.parseUnsafe: {e}"))
    }

    /// Compact format (`format`). Default seconds `{0}` are omitted unless
    /// asked.
    pub fn format(&self, include_seconds: bool) -> String {
        let mut segs = vec![
            fmt_set(&self.seconds),
            fmt_set(&self.minutes),
            fmt_set(&self.hours),
            fmt_set(&self.days),
            fmt_set(&self.months),
            fmt_set(&self.weekdays),
        ];
        if !include_seconds && self.seconds.len() == 1 && self.seconds.contains(&0) {
            segs.remove(0);
        }
        segs.join(" ")
    }

    /// The zone this schedule steps in (UTC when absent).
    pub fn zone(&self) -> Zone {
        self.zone.clone().unwrap_or(Zone::Utc)
    }

    /// `true` when `instant` fires this schedule (`match`).
    pub fn matches(&self, instant: &DateTime) -> bool {
        let at = instant.set_zone(&self.zone());
        let p = at.parts();
        if !self.seconds.is_empty() && !self.seconds.contains(&(p.second as i32)) {
            return false;
        }
        if !self.minutes.is_empty() && !self.minutes.contains(&(p.minute as i32)) {
            return false;
        }
        if !self.hours.is_empty() && !self.hours.contains(&(p.hour as i32)) {
            return false;
        }
        if !self.months.is_empty() && !self.months.contains(&(p.month as i32)) {
            return false;
        }
        if self.days.is_empty() && self.weekdays.is_empty() {
            return true;
        }
        let day = self.days.is_empty() || self.days.contains(&(p.day as i32));
        let wday = self.weekdays.is_empty() || self.weekdays.contains(&(p.weekday as i32));
        if self.and {
            return day && wday;
        }
        if self.weekdays.is_empty() {
            return self.days.contains(&(p.day as i32));
        }
        if self.days.is_empty() {
            return self.weekdays.contains(&(p.weekday as i32));
        }
        self.days.contains(&(p.day as i32)) || self.weekdays.contains(&(p.weekday as i32))
    }

    /// First instant strictly after `from` (`next`).
    pub fn next(&self, from: &DateTime) -> Result<DateTime, CronError> {
        self.step(from, 1)
    }

    /// First instant strictly before `from` (`prev`).
    pub fn prev(&self, from: &DateTime) -> Result<DateTime, CronError> {
        self.step(from, -1)
    }

    /// `next`, panicking on exhaustion.
    pub fn next_unsafe(&self, from: &DateTime) -> DateTime {
        self.next(from).expect("Cron.next: no occurrence")
    }

    /// `prev`, panicking on exhaustion.
    pub fn prev_unsafe(&self, from: &DateTime) -> DateTime {
        self.prev(from).expect("Cron.prev: no occurrence")
    }

    /// Infinite forward iterator (`sequence`).
    pub fn sequence(&self, from: &DateTime) -> Sequence<'_> {
        Sequence {
            cron: self,
            at: from.clone(),
            forward: true,
        }
    }

    fn step(&self, from: &DateTime, tick: i64) -> Result<DateTime, CronError> {
        let zone = self.zone();
        // Folds resolve to the earlier occurrence both ways: the stepper
        // scans wall time and keeps the first hit.
        let dis = Disambiguation::Compatible;
        // Start strictly off the input, truncated to the second. The floor
        // is instant math: a wall rebuild would collapse fold occurrences.
        let mut at = from.set_zone(&zone);
        let floored = at.epoch_millis() - at.epoch_millis().rem_euclid(1_000);
        at = DateTime::from_epoch_millis(floored)
            .set_zone(&zone)
            .add_millis(1000 * tick);
        for _ in 0..20_000 {
            // Fold policy (mirrors TS `adjustDst`): a repeated wall is
            // visited once. Forward jumps over the duplicate by the drift;
            // backward snaps to the first occurrence.
            let first = fold_first(&at);
            if tick > 0 && first.epoch_millis() < at.epoch_millis() {
                at = DateTime::from_epoch_millis(
                    at.epoch_millis() + (at.epoch_millis() - first.epoch_millis()),
                )
                .set_zone(&zone);
                continue;
            }
            if tick < 0 && first.epoch_millis() != at.epoch_millis() {
                at = first;
            }
            let p = at.parts();
            if !month_ok(self, p.month) {
                at = shift_month(&at, &zone, tick, dis)?;
                continue;
            }
            if !day_ok(self, &at) {
                at = shift_day(&at, &zone, tick, dis)?;
                continue;
            }
            if !field_ok(&self.hours, p.hour) {
                at = shift_hour(&at, &zone, tick, dis)?;
                continue;
            }
            if !field_ok(&self.minutes, p.minute) {
                at = shift_minute(&at, &zone, tick, dis)?;
                continue;
            }
            if !field_ok(&self.seconds, p.second) {
                at = shift_second(&at, &zone, tick, dis)?;
                continue;
            }
            // All fields hit. No wall re-resolve: the instant maps to
            // exactly one wall+offset, and re-resolving would collapse a
            // legitimate second fold occurrence back to the first.
            if self.matches(&at) && tick_ok(&at, from, tick) {
                return Ok(at);
            }
            at = at.add_millis(1000 * tick);
        }
        Err(CronError::NoOccurrence)
    }
}

/// First occurrence of `at`'s wall in its zone (identity outside folds).
fn fold_first(at: &DateTime) -> DateTime {
    let p = at.parts();
    DateTime::make_zoned(
        DateParts {
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
            minute: p.minute,
            second: p.second,
            millis: p.millis,
        },
        at.zone(),
        Disambiguation::Compatible,
    )
    .unwrap_or_else(|_| at.clone())
}

fn tick_ok(at: &DateTime, from: &DateTime, tick: i64) -> bool {
    if tick > 0 {
        at.epoch_millis() > from.epoch_millis()
    } else {
        at.epoch_millis() < from.epoch_millis()
    }
}

fn field_ok(set: &BTreeSet<i32>, v: i8) -> bool {
    set.is_empty() || set.contains(&(v as i32))
}

fn month_ok(cron: &Cron, month: i8) -> bool {
    field_ok(&cron.months, month)
}

fn day_ok(cron: &Cron, at: &DateTime) -> bool {
    let p = at.parts();
    let has_day = !cron.days.is_empty();
    let has_wday = !cron.weekdays.is_empty();
    if !has_day && !has_wday {
        return true;
    }
    let day = cron.days.is_empty() || cron.days.contains(&(p.day as i32));
    let wday = cron.weekdays.is_empty() || cron.weekdays.contains(&(p.weekday as i32));
    if cron.and {
        return day && wday;
    }
    if !has_wday {
        return cron.days.contains(&(p.day as i32));
    }
    if !has_day {
        return cron.weekdays.contains(&(p.weekday as i32));
    }
    cron.days.contains(&(p.day as i32)) || cron.weekdays.contains(&(p.weekday as i32))
}

fn shift_month(
    at: &DateTime,
    zone: &Zone,
    tick: i64,
    dis: Disambiguation,
) -> Result<DateTime, CronError> {
    let p = at.parts();
    // Jump to the first/last day of the next/previous candidate month.
    let mut y = p.year as i64;
    let mut m = p.month as i64 + tick;
    for _ in 0..30 {
        if m < 1 {
            m = 12;
            y -= 1;
        }
        if m > 12 {
            m = 1;
            y += 1;
        }
        if y < 1900 || y > 2200 {
            return Err(CronError::NoOccurrence);
        }
        let cand = DateParts {
            year: y as i16,
            month: m as i8,
            day: if tick > 0 {
                1
            } else {
                month_end(y as i16, m as i8)
            },
            hour: if tick > 0 { 0 } else { 23 },
            minute: if tick > 0 { 0 } else { 59 },
            second: if tick > 0 { 0 } else { 59 },
            millis: 0,
        };
        if let Ok(z) = DateTime::make_zoned(cand, zone.clone(), dis) {
            if month_ok_of(&z, m as i8) {
                return Ok(z);
            }
        }
        m += tick;
    }
    Err(CronError::NoOccurrence)
}

fn month_ok_of(at: &DateTime, m: i8) -> bool {
    at.parts().month == m
}

fn shift_day(
    at: &DateTime,
    zone: &Zone,
    tick: i64,
    dis: Disambiguation,
) -> Result<DateTime, CronError> {
    let p = at.parts();
    let base = DateParts {
        year: p.year,
        month: p.month,
        day: p.day,
        hour: if tick > 0 { 0 } else { 23 },
        minute: if tick > 0 { 0 } else { 59 },
        second: if tick > 0 { 0 } else { 59 },
        millis: 0,
    };
    let anchored = DateTime::make_zoned(base, zone.clone(), dis).unwrap_or_else(|_| at.clone());
    Ok(anchored.add_millis(86_400_000 * tick))
}

/// Hour anchor as an instant: wall anchors re-resolve fold walls to
/// the first occurrence and loop the fold forever.
fn shift_hour(
    at: &DateTime,
    _zone: &Zone,
    tick: i64,
    _dis: Disambiguation,
) -> Result<DateTime, CronError> {
    let floored = at.epoch_millis() - at.epoch_millis().rem_euclid(3_600_000);
    let base = if tick > 0 {
        floored + 3_600_000
    } else {
        floored - 1_000
    };
    Ok(DateTime::from_epoch_millis(base).set_zone(&at.zone()))
}

/// Minute anchor as an instant (same fold-loop reason as `shift_hour`).
fn shift_minute(
    at: &DateTime,
    _zone: &Zone,
    tick: i64,
    _dis: Disambiguation,
) -> Result<DateTime, CronError> {
    let floored = at.epoch_millis() - at.epoch_millis().rem_euclid(60_000);
    let base = if tick > 0 {
        floored + 60_000
    } else {
        floored - 1_000
    };
    Ok(DateTime::from_epoch_millis(base).set_zone(&at.zone()))
}

fn shift_second(
    at: &DateTime,
    _zone: &Zone,
    tick: i64,
    _dis: Disambiguation,
) -> Result<DateTime, CronError> {
    Ok(at.add_millis(1000 * tick))
}

/// Last day of a month (Clamp target when stepping back). Duplicates
/// the calendar table: cron cannot see `datetime` internals.
fn month_end(year: i16, month: i8) -> i8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

fn fmt_set(set: &BTreeSet<i32>) -> String {
    if set.is_empty() {
        return "*".to_string();
    }
    let a: Vec<i32> = set.iter().copied().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.len() {
        let start = a[i];
        let step = if i + 1 < a.len() { a[i + 1] - start } else { 0 };
        if step != 0 && i + 2 < a.len() && a[i + 2] - a[i + 1] == step {
            let mut end = i + 2;
            while end + 1 < a.len() && a[end + 1] - a[end] == step {
                end += 1;
            }
            out.push(if step == 1 {
                format!("{start}-{}", a[end])
            } else {
                format!("{start}-{}", a[end]) + &format!("/{step}")
            });
            i = end + 1;
        } else {
            out.push(format!("{start}"));
            i += 1;
        }
    }
    out.join(",")
}

/// Forward occurrence iterator (`sequence`).
pub struct Sequence<'a> {
    cron: &'a Cron,
    at: DateTime,
    forward: bool,
}

impl Iterator for Sequence<'_> {
    type Item = DateTime;

    fn next(&mut self) -> Option<Self::Item> {
        let out = if self.forward {
            self.cron.next(&self.at).ok()?
        } else {
            self.cron.prev(&self.at).ok()?
        };
        self.at = out.clone();
        Some(out)
    }
}
