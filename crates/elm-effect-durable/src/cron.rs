//! Pure 5-field cron timing (ADR-0005).
//!
//! UTC, minute resolution, no clock and no I/O. The host passes `now_ms`,
//! turns a due tick into [`DurableEngine::start`](crate::DurableEngine::start)
//! under [`fire_key`], and keeps registration, pause/resume, catch-up policy,
//! and the tick loop. This module only answers "which minute is due".

/// Why a cron expression did not parse.
#[derive(Clone, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum CronError {
    /// The expression did not have exactly five whitespace-separated fields.
    #[error("cron expression must have 5 fields, got {0}")]
    FieldCount(usize),
    /// One field was not valid.
    #[error("invalid {field} field {value:?}: {reason}")]
    Field {
        /// Field name.
        field: &'static str,
        /// Whole field text that failed.
        value: String,
        /// Why it failed.
        reason: String,
    },
}

/// A parsed 5-field vixie cron schedule in UTC.
///
/// Fields are `minute hour day-of-month month day-of-week`. Lists, ranges,
/// steps (`*/15`, `1-5/2`, `5/10`), and month / weekday names are accepted.
/// Numeric day-of-week `0` and `7` are both Sunday. When both day-of-month and
/// day-of-week are restricted, either may match (vixie OR rule).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CronSchedule {
    minutes: u64,
    hours: u64,
    days_of_month: u64,
    months: u64,
    days_of_week: u64,
    dom_restricted: bool,
    dow_restricted: bool,
}

/// One field's bounds, names, and Sunday mapping.
#[derive(Clone, Copy)]
struct FieldSpec {
    name: &'static str,
    min: u64,
    max: u64,
    names: &'static [(&'static str, u64)],
    sunday_wrap: bool,
}

const MINUTE: FieldSpec = FieldSpec {
    name: "minute",
    min: 0,
    max: 59,
    names: &[],
    sunday_wrap: false,
};
const HOUR: FieldSpec = FieldSpec {
    name: "hour",
    min: 0,
    max: 23,
    names: &[],
    sunday_wrap: false,
};
const DAY_OF_MONTH: FieldSpec = FieldSpec {
    name: "day-of-month",
    min: 1,
    max: 31,
    names: &[],
    sunday_wrap: false,
};
const MONTH: FieldSpec = FieldSpec {
    name: "month",
    min: 1,
    max: 12,
    names: &[
        ("JAN", 1),
        ("FEB", 2),
        ("MAR", 3),
        ("APR", 4),
        ("MAY", 5),
        ("JUN", 6),
        ("JUL", 7),
        ("AUG", 8),
        ("SEP", 9),
        ("OCT", 10),
        ("NOV", 11),
        ("DEC", 12),
    ],
    sunday_wrap: false,
};
const DAY_OF_WEEK: FieldSpec = FieldSpec {
    name: "day-of-week",
    min: 0,
    max: 7,
    names: &[
        ("SUN", 0),
        ("MON", 1),
        ("TUE", 2),
        ("WED", 3),
        ("THU", 4),
        ("FRI", 5),
        ("SAT", 6),
    ],
    sunday_wrap: true,
};

const MINUTE_MS: u64 = 60_000;
const MINUTES_PER_DAY: u64 = 24 * 60;
/// The longest Gregorian gap between two February 29ths is 2096-02-29 to
/// 2104-02-29 (2100 is not a leap year): 2,922 days. One extra week plus an
/// 8-year span covers every valid schedule.
const SEARCH_HORIZON_MINUTES: u64 = (8 * 366 + 7) * MINUTES_PER_DAY;

impl CronSchedule {
    /// Parse a 5-field vixie cron expression. Time is UTC.
    ///
    /// # Errors
    ///
    /// Returns [`CronError::FieldCount`] for anything other than five fields,
    /// or [`CronError::Field`] for an out-of-range value, reversed range, zero
    /// step, unknown name, or malformed list.
    pub fn parse(expression: &str) -> Result<Self, CronError> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronError::FieldCount(fields.len()));
        }
        let (minutes, _) = parse_field(fields[0], &MINUTE)?;
        let (hours, _) = parse_field(fields[1], &HOUR)?;
        let (days_of_month, dom_restricted) = parse_field(fields[2], &DAY_OF_MONTH)?;
        let (months, _) = parse_field(fields[3], &MONTH)?;
        let (days_of_week, dow_restricted) = parse_field(fields[4], &DAY_OF_WEEK)?;
        Ok(Self {
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
            dom_restricted,
            dow_restricted,
        })
    }

    /// The five field bitmasks: minutes, hours, days-of-month, months,
    /// days-of-week. Pure inspection for tests and the host tick loop.
    #[must_use]
    pub fn fields(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.minutes,
            self.hours,
            self.days_of_month,
            self.months,
            self.days_of_week,
        )
    }

    /// Next tick strictly after `from_ms`, in Unix milliseconds.
    ///
    /// Returns `None` when the expression names no tick inside the search
    /// horizon (for example `0 0 30 2 *`).
    #[must_use]
    pub fn next_after_ms(&self, from_ms: u64) -> Option<u64> {
        let start = (from_ms / MINUTE_MS).checked_add(1)?;
        let end = start.checked_add(SEARCH_HORIZON_MINUTES)?;
        let mut minute = start;
        while minute <= end {
            let day = minute / MINUTES_PER_DAY;
            if !self.day_matches(day) {
                // Skip the rest of this day; the next loop checks the next day.
                minute = (day + 1).checked_mul(MINUTES_PER_DAY)?;
                continue;
            }
            let minute_of_day = minute % MINUTES_PER_DAY;
            let hour = minute_of_day / 60;
            let minute_in_hour = minute_of_day % 60;
            if !self.hour_matches(hour) {
                minute = minute.checked_add(60 - minute_in_hour)?;
                continue;
            }
            let allowed = self.minutes & (!0u64 << minute_in_hour);
            if allowed == 0 {
                minute = minute.checked_add(60 - minute_in_hour)?;
                continue;
            }
            let candidate = minute - minute_in_hour + u64::from(allowed.trailing_zeros());
            if candidate > end {
                return None;
            }
            return candidate.checked_mul(MINUTE_MS);
        }
        None
    }

    /// Latest tick strictly after `last_ms` and at or before `now_ms`.
    ///
    /// This is the one-tick catch-up after downtime: the host runs the latest
    /// missed tick once, never a burst. Returns `None` when no tick is due in
    /// that window, including `now_ms < last_ms`.
    #[must_use]
    pub fn latest_at_or_before_ms(&self, last_ms: u64, now_ms: u64) -> Option<u64> {
        if now_ms < last_ms {
            return None;
        }
        let start = (last_ms / MINUTE_MS).checked_add(1)?;
        let now_minute = now_ms / MINUTE_MS;
        if start > now_minute {
            return None;
        }
        let horizon_start = now_minute.saturating_sub(SEARCH_HORIZON_MINUTES);
        let start = start.max(horizon_start);
        let mut minute = now_minute;
        loop {
            if minute < start {
                return None;
            }
            let day = minute / MINUTES_PER_DAY;
            if !self.day_matches(day) {
                if day == 0 {
                    return None;
                }
                minute = day * MINUTES_PER_DAY - 1;
                continue;
            }
            let minute_of_day = minute % MINUTES_PER_DAY;
            let hour = minute_of_day / 60;
            let minute_in_hour = minute_of_day % 60;
            let hour_start = minute - minute_in_hour;
            if !self.hour_matches(hour) {
                minute = previous_hour_end(day, hour_start)?;
                continue;
            }
            let allowed = self.minutes & ((1u64 << (minute_in_hour + 1)) - 1);
            if allowed == 0 {
                minute = previous_hour_end(day, hour_start)?;
                continue;
            }
            let candidate = hour_start + u64::from(63 - allowed.leading_zeros());
            if candidate < start {
                return None;
            }
            return candidate.checked_mul(MINUTE_MS);
        }
    }

    fn day_matches(&self, day_index: u64) -> bool {
        let (_, month, day) = civil_from_days(day_index);
        if self.months & (1u64 << month) == 0 {
            return false;
        }
        let dom_match = self.days_of_month & (1u64 << day) != 0;
        let dow_match = self.days_of_week & (1u64 << weekday_from_days(day_index)) != 0;
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom_match || dow_match,
            _ => dom_match && dow_match,
        }
    }

    fn hour_matches(&self, hour: u64) -> bool {
        self.hours & (1u64 << hour) != 0
    }

    /// Latest-tick policy: the one tick to run after downtime.
    ///
    /// A new schedule (`None`) anchors at `now - 60s`, so it only fires
    /// when the current minute matches and never replays hours of history
    /// it was never registered for. Returns the [`fire_key`] with the tick.
    #[must_use]
    pub fn plan_latest_tick(
        &self,
        name: &str,
        last_fired_ms: Option<u64>,
        now_ms: u64,
    ) -> Option<(String, u64)> {
        let anchor = last_fired_ms.unwrap_or_else(|| now_ms.saturating_sub(MINUTE_MS));
        self.latest_at_or_before_ms(anchor, now_ms)
            .map(|tick_ms| (fire_key(name, tick_ms), tick_ms))
    }

    /// Skip policy: run the next tick only while it is still fresh.
    ///
    /// Finds the first tick after the anchor and requires it to be due
    /// (`tick <= now`) and no older than `grace_ms`. Stale ticks return
    /// `None` instead of firing late.
    #[must_use]
    pub fn plan_skip_tick(
        &self,
        name: &str,
        grace_ms: u64,
        last_fired_ms: Option<u64>,
        now_ms: u64,
    ) -> Option<(String, u64)> {
        let anchor = last_fired_ms.unwrap_or_else(|| now_ms.saturating_sub(MINUTE_MS));
        let tick_ms = self.next_after_ms(anchor)?;
        if tick_ms > now_ms || now_ms.saturating_sub(tick_ms) > grace_ms {
            return None;
        }
        Some((fire_key(name, tick_ms), tick_ms))
    }

    /// Deterministic child-run key: parent run, step name, attempt number.
    #[must_use]
    pub fn child_key(parent: &str, step: &str, attempt: u32) -> String {
        child_key(parent, step, attempt)
    }

    /// Deterministic step key inside one run.
    #[must_use]
    pub fn step_key(run_key: &str, step: &str) -> String {
        step_key(run_key, step)
    }
}

/// Deterministic child-run key: parent run, step name, attempt number.
#[must_use]
pub fn child_key(parent: &str, step: &str, attempt: u32) -> String {
    format!("{parent}:{step}:{attempt}")
}

/// Deterministic step key inside one run.
#[must_use]
pub fn step_key(run_key: &str, step: &str) -> String {
    format!("{run_key}:{step}")
}

/// The run key for one cron firing. `name` is the schedule identity.
///
/// Pass the key to [`DurableEngine::start`](crate::DurableEngine::start).
/// Starting the same key again re-attaches (`Already`) instead of dispatching
/// the effect twice.
#[must_use]
pub fn fire_key(name: &str, tick_ms: u64) -> String {
    format!("cron:{name}:{tick_ms}")
}

fn parse_field(text: &str, spec: &FieldSpec) -> Result<(u64, bool), CronError> {
    let mut bits = 0u64;
    // Vixie rule: only a bare `*` or a step over the whole range (`*/n`)
    // leaves the field unrestricted. Explicit values, ranges, and lists
    // (`1`, `1-5`, `1,15`) restrict day-of-month / day-of-week matching.
    let restricted = text != "*" && !text.starts_with("*/");
    for item in text.split(',') {
        if item.is_empty() {
            return Err(field_error(spec, text, "empty list item"));
        }
        let mut parts = item.split('/');
        let base = parts.next().unwrap_or_default();
        let step = match parts.next() {
            Some(step) => parse_step(step, spec, text)?,
            None => 1,
        };
        if parts.next().is_some() {
            return Err(field_error(spec, text, "more than one '/'"));
        }
        let (start, end) = if base == "*" {
            (spec.min, spec.max)
        } else if let Some((start, end)) = base.split_once('-') {
            let start = parse_value(start, spec, text)?;
            let end = parse_value(end, spec, text)?;
            (start, end)
        } else {
            let start = parse_value(base, spec, text)?;
            (start, if step > 1 { spec.max } else { start })
        };
        if start > end {
            return Err(field_error(spec, text, "range start is after range end"));
        }
        let mut value = start;
        loop {
            let bit = if spec.sunday_wrap && value == 7 {
                0
            } else {
                value
            };
            bits |= 1u64 << bit;
            value = match value.checked_add(step) {
                Some(next) => next,
                None => break,
            };
            if value > end {
                break;
            }
        }
    }
    Ok((bits, restricted))
}

fn parse_value(token: &str, spec: &FieldSpec, text: &str) -> Result<u64, CronError> {
    if token.is_empty() {
        return Err(field_error(spec, text, "empty value"));
    }
    if token.bytes().all(|byte| byte.is_ascii_digit()) {
        let value = token
            .parse::<u64>()
            .map_err(|_| field_error(spec, text, "value is not a number"))?;
        if value < spec.min || value > spec.max {
            return Err(field_error(
                spec,
                text,
                &format!("value must be {}..={}", spec.min, spec.max),
            ));
        }
        return Ok(value);
    }
    let upper = token.to_ascii_uppercase();
    spec.names
        .iter()
        .find_map(|(name, value)| (*name == upper).then_some(*value))
        .ok_or_else(|| field_error(spec, text, &format!("unknown name {token:?}")))
}

fn parse_step(step: &str, spec: &FieldSpec, text: &str) -> Result<u64, CronError> {
    let value = step
        .parse::<u64>()
        .map_err(|_| field_error(spec, text, "step is not a number"))?;
    if value == 0 {
        return Err(field_error(spec, text, "step must be at least 1"));
    }
    Ok(value)
}

fn field_error(spec: &FieldSpec, text: &str, reason: &str) -> CronError {
    CronError::Field {
        field: spec.name,
        value: text.to_string(),
        reason: reason.to_string(),
    }
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Last minute of the hour before `hour_start`, or the previous day's last
/// minute when the hour is the first of the day.
fn previous_hour_end(day: u64, hour_start: u64) -> Option<u64> {
    if hour_start == 0 {
        if day == 0 {
            return None;
        }
        Some(day * MINUTES_PER_DAY - 1)
    } else {
        Some(hour_start - 1)
    }
}

/// 0 = Sunday. 1970-01-01 was Thursday.
fn weekday_from_days(days: u64) -> u64 {
    (days + 4) % 7
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let yoe = year - era * 400;
        let mp = if month > 2 { month - 3 } else { month + 9 };
        let doy = (153 * mp + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn ms(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> u64 {
        let days = days_from_civil(year, month, day);
        ((days * 86_400 + hour * 3_600 + minute * 60) * 1_000) as u64
    }

    fn schedule(expression: &str) -> CronSchedule {
        CronSchedule::parse(expression).expect("valid cron")
    }

    fn brute_matches(cron: &CronSchedule, minute_index: u64) -> bool {
        let day_index = minute_index / MINUTES_PER_DAY;
        let minute_of_day = minute_index % MINUTES_PER_DAY;
        let hour = minute_of_day / 60;
        let minute = minute_of_day % 60;
        let (_, month, day) = civil_from_days(day_index);
        if cron.months & (1u64 << month) == 0
            || cron.hours & (1u64 << hour) == 0
            || cron.minutes & (1u64 << minute) == 0
        {
            return false;
        }
        let dom = cron.days_of_month & (1u64 << day) != 0;
        let dow = cron.days_of_week & (1u64 << weekday_from_days(day_index)) != 0;
        match (cron.dom_restricted, cron.dow_restricted) {
            (true, true) => dom || dow,
            _ => dom && dow,
        }
    }

    fn brute_next(cron: &CronSchedule, from_ms: u64) -> Option<u64> {
        let start = from_ms / MINUTE_MS + 1;
        let end = start + SEARCH_HORIZON_MINUTES;
        (start..=end)
            .find(|minute| brute_matches(cron, *minute))
            .map(|minute| minute * MINUTE_MS)
    }

    fn brute_prev(cron: &CronSchedule, last_ms: u64, now_ms: u64) -> Option<u64> {
        if now_ms < last_ms {
            return None;
        }
        let start = last_ms / MINUTE_MS + 1;
        let now_minute = now_ms / MINUTE_MS;
        if start > now_minute {
            return None;
        }
        (start..=now_minute)
            .rev()
            .find(|minute| brute_matches(cron, *minute))
            .map(|minute| minute * MINUTE_MS)
    }

    #[test]
    fn jump_search_agrees_with_brute_force() {
        for (expression, base) in [
            ("* * * * *", ms(2024, 9, 6, 12, 0)),
            ("*/15 * * * *", ms(2024, 9, 6, 12, 7)),
            ("30 3 * * *", ms(2024, 9, 6, 23, 59)),
            ("0 0 13 * FRI", ms(2024, 10, 12, 0, 0)),
            ("59 23 * * *", ms(2024, 9, 6, 0, 0)),
            ("0 0 29 2 *", ms(2024, 2, 28, 0, 0)),
        ] {
            let cron = schedule(expression);
            assert_eq!(
                cron.next_after_ms(base),
                brute_next(&cron, base),
                "{expression} next from {base}"
            );
            for delta in [1, 60_000, 86_400_000, 3 * 86_400_000] {
                let now = base + delta;
                let last = base.saturating_sub(1);
                assert_eq!(
                    cron.latest_at_or_before_ms(last, now),
                    brute_prev(&cron, last, now),
                    "{expression} latest from {last} to {now}"
                );
            }
        }
    }

    #[test]
    fn every_minute_is_strictly_after_from() {
        let cron = schedule("* * * * *");
        assert_eq!(cron.next_after_ms(0), Some(60_000));
        assert_eq!(
            cron.next_after_ms(ms(2024, 9, 6, 12, 0) + 30_000),
            Some(ms(2024, 9, 6, 12, 1))
        );
    }

    #[test]
    fn steps_ranges_and_names() {
        let cron = schedule("*/15 * * * *");
        assert_eq!(
            cron.next_after_ms(ms(2024, 9, 6, 12, 1)),
            Some(ms(2024, 9, 6, 12, 15))
        );

        let weekdays = schedule("0 8 * * MON-FRI");
        assert_eq!(
            weekdays.next_after_ms(ms(2024, 9, 7, 0, 0)),
            Some(ms(2024, 9, 9, 8, 0))
        );

        let months = schedule("0 0 1 JAN *");
        assert_eq!(
            months.next_after_ms(ms(2024, 6, 1, 0, 0)),
            Some(ms(2025, 1, 1, 0, 0))
        );
    }

    #[test]
    fn plane_schedules_parse() {
        for expression in [
            "0 8 * * *",
            "0 9 * * *",
            "10 3 * * *",
            "30 * * * *",
            "40 * * * *",
            "20 4 * * *",
            "*/15 * * * *",
            "35 * * * *",
            "50 * * * *",
            "0 * * * *",
            "15 * * * *",
            "45 * * * *",
            "0 4 * * 0",
        ] {
            assert!(
                CronSchedule::parse(expression).is_ok(),
                "{expression:?} must parse"
            );
        }
    }

    #[test]
    fn sunday_is_zero_and_seven() {
        let zero = schedule("0 3 * * 0");
        let seven = schedule("0 3 * * 7");
        let want = ms(1970, 1, 4, 3, 0);
        assert_eq!(zero.next_after_ms(0), Some(want));
        assert_eq!(seven.next_after_ms(0), Some(want));
    }

    #[test]
    fn day_and_hour_rollovers() {
        let midnight = schedule("0 0 * * *");
        assert_eq!(
            midnight.next_after_ms(ms(2024, 9, 6, 23, 59)),
            Some(ms(2024, 9, 7, 0, 0))
        );
        assert_eq!(
            midnight.latest_at_or_before_ms(ms(2024, 9, 5, 12, 0), ms(2024, 9, 6, 23, 59)),
            Some(ms(2024, 9, 6, 0, 0))
        );
        assert_eq!(
            midnight.latest_at_or_before_ms(ms(2024, 9, 6, 0, 0), ms(2024, 9, 6, 23, 59)),
            None
        );

        let late = schedule("59 23 * * *");
        assert_eq!(
            late.next_after_ms(ms(2024, 9, 6, 0, 0)),
            Some(ms(2024, 9, 6, 23, 59))
        );
        assert_eq!(
            late.latest_at_or_before_ms(ms(2024, 9, 5, 23, 58), ms(2024, 9, 6, 0, 0)),
            Some(ms(2024, 9, 5, 23, 59))
        );
        assert_eq!(
            late.latest_at_or_before_ms(ms(2024, 9, 5, 23, 59), ms(2024, 9, 6, 0, 0)),
            None
        );
    }

    #[test]
    fn restricted_day_of_month_or_day_of_week() {
        let cron = schedule("0 0 13 * FRI");
        // 2024-10-13 is a Sunday, so only the day-of-month can match.
        assert_eq!(
            cron.next_after_ms(ms(2024, 10, 12, 0, 0)),
            Some(ms(2024, 10, 13, 0, 0))
        );
    }

    #[test]
    fn latest_due_is_one_tick_after_downtime() {
        let cron = schedule("30 3 * * *");
        let last = ms(2024, 1, 1, 0, 0);
        assert_eq!(
            cron.latest_at_or_before_ms(last, ms(2024, 1, 3, 4, 0)),
            Some(ms(2024, 1, 3, 3, 30))
        );
        assert_eq!(
            cron.latest_at_or_before_ms(last, ms(2024, 1, 1, 3, 30)),
            Some(ms(2024, 1, 1, 3, 30))
        );
        assert_eq!(
            cron.latest_at_or_before_ms(last, ms(2024, 1, 1, 3, 29)),
            None
        );
        assert_eq!(
            cron.latest_at_or_before_ms(ms(2024, 1, 3, 3, 30), ms(2024, 1, 3, 4, 0)),
            None
        );
    }

    #[test]
    fn leap_day_and_impossible_date() {
        let leap = schedule("0 0 29 2 *");
        assert_eq!(
            leap.next_after_ms(ms(2024, 3, 1, 0, 0)),
            Some(ms(2028, 2, 29, 0, 0))
        );
        assert_eq!(
            leap.latest_at_or_before_ms(ms(2024, 3, 1, 0, 0), ms(2028, 3, 1, 0, 0)),
            Some(ms(2028, 2, 29, 0, 0))
        );
        let impossible = schedule("0 0 30 2 *");
        assert_eq!(impossible.next_after_ms(ms(2024, 1, 1, 0, 0)), None);
        assert_eq!(
            impossible.latest_at_or_before_ms(ms(2024, 1, 1, 0, 0), ms(2025, 1, 1, 0, 0)),
            None
        );
    }

    #[test]
    fn malformed_expressions_are_typed_errors() {
        assert_eq!(
            CronSchedule::parse("* * * *"),
            Err(CronError::FieldCount(4))
        );
        assert_eq!(
            CronSchedule::parse("* * * * * *"),
            Err(CronError::FieldCount(6))
        );
        for expression in [
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "1,,2 * * * *",
            "JAN * * * *",
        ] {
            assert!(
                matches!(
                    CronSchedule::parse(expression),
                    Err(CronError::Field { .. })
                ),
                "{expression:?} must be a field error"
            );
        }
    }

    #[test]
    fn fire_key_is_stable_per_tick() {
        assert_eq!(fire_key("weekly-review", 1_234), "cron:weekly-review:1234");
        assert_eq!(
            fire_key("weekly-review", 1_234),
            fire_key("weekly-review", 1_234)
        );
        assert_ne!(
            fire_key("weekly-review", 1_234),
            fire_key("weekly-review", 1_235)
        );
    }
}
