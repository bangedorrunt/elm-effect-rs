//! Law L8: time enters through the Clock service, never the wall.
//!
//! [`Duration`] is millisecond time. [`ClockImpl`] is the service behind the
//! [`Clock`](crate::Clock) tag: [`SystemClock`] parks threads on real
//! deadlines, [`TestClock`] parks fibers on virtual time advanced only by
//! [`TestClock::adjust`]. A sleep on a fiber subscribes to that fiber's
//! interrupt notification, so the request wakes it directly; the `interrupted`
//! closure is the predicate, and the cooperative quantum is only the fallback
//! for a raw thread that has no fiber to subscribe to.
//!
//! Completions happen exclusively via `adjust` (virtual) or deadlines (real),
//! so test outcomes are virtual-time-deterministic.

use std::sync::{Arc, Condvar, Mutex};

/// Effect 4's `DurationValue`: a duration is stored in milliseconds (an IEEE
/// double), in nanoseconds (an exact integer), or as one of the infinite
/// sentinels.
#[derive(Clone, Copy, Debug)]
enum DurationValue {
    Millis(f64),
    Nanos(i128),
    Infinity,
    NegativeInfinity,
}

/// A span of time with nanosecond precision (`effect/Duration`).
///
/// The value model is Effect 4's: `Millis` for values built from
/// millisecond-scale constructors, `Nanos` for exact sub-millisecond
/// values, and the two infinite sentinels. Equality is Effect 4's
/// equivalence, so `millis(1)` and `nanos(1_000_000)` are equal and hash
/// alike; structural equality (`==`) still distinguishes the
/// representations, which is whin Effect 4's `deepStrictEqual` rows observe.
#[derive(Clone, Copy, Debug)]
pub struct Duration {
    value: DurationValue,
}

impl Duration {
    pub const ZERO: Self = Self {
        value: DurationValue::Millis(0.0),
    };

    pub const fn from_millis(millis: u64) -> Self {
        Self {
            value: DurationValue::Millis(millis as f64),
        }
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self {
            value: DurationValue::Millis(secs as f64 * 1000.0),
        }
    }

    pub const fn from_minutes(minutes: u64) -> Self {
        Self {
            value: DurationValue::Millis(minutes as f64 * 60.0 * 1000.0),
        }
    }

    /// Effect 4's `make`: a millis value that is not an integer becomes an
    /// exact nanos value, and NaN and both zeros are the zero duration.
    pub(crate) fn make_millis(millis: f64) -> Self {
        if millis.is_nan() || millis == 0.0 {
            return Self::zero();
        }
        if !millis.is_finite() {
            return if millis > 0.0 {
                Self::infinity()
            } else {
                Self::negative_infinity()
            };
        }
        if millis.fract() != 0.0 {
            return Self::nanos(Self::round_ties_away(millis * 1_000_000.0));
        }
        Self {
            value: DurationValue::Millis(millis),
        }
    }

    /// `Duration.millis`.
    pub fn millis(millis: f64) -> Self {
        Self::make_millis(millis)
    }

    /// `Duration.nanos`. Effect 4's `make` maps a zero bigint to the zero
    /// duration rather than to `Nanos(0)`, so this does too.
    pub fn nanos(nanos: i128) -> Self {
        if nanos == 0 {
            return Self::zero();
        }
        Self {
            value: DurationValue::Nanos(nanos),
        }
    }

    /// `Duration.micros`: microseconds are exact nanoseconds.
    pub fn micros(micros: i128) -> Self {
        Self {
            value: DurationValue::Nanos(micros * 1_000),
        }
    }

    /// `Duration.seconds`.
    pub fn seconds(seconds: f64) -> Self {
        Self::make_millis(seconds * 1000.0)
    }

    /// `Duration.minutes`.
    pub fn minutes(minutes: f64) -> Self {
        Self::make_millis(minutes * 60_000.0)
    }

    /// `Duration.hours`.
    pub fn hours(hours: f64) -> Self {
        Self::make_millis(hours * 3_600_000.0)
    }

    /// `Duration.days`.
    pub fn days(days: f64) -> Self {
        Self::make_millis(days * 86_400_000.0)
    }

    /// `Duration.weeks`.
    pub fn weeks(weeks: f64) -> Self {
        Self::make_millis(weeks * 604_800_000.0)
    }

    /// The string arm of Effect 4's `fromInput`: `"1000 millis"`,
    /// `"1.5 hours"`, `"Infinity"`, `"-Infinity"`. A unit is required and may
    /// be singular or plural, the count may be signed and fractional, and no
    /// padding whitespace is allowed — Effect 4's own regex.
    ///
    /// `None` for anything else, which is what `Schema.DurationFromString`
    /// turns into a decoding failure. Effect 4 reads nanos through a BigInt, so
    /// `"1.0000000001 nanos"` stays exact; the count here is an f64, so a
    /// fraction of a nanosecond rounds to the nearest whole one.
    pub fn from_input_string(text: &str) -> Option<Self> {
        if text == "Infinity" {
            return Some(Self::infinity());
        }
        if text == "-Infinity" {
            return Some(Self::negative_infinity());
        }
        // The count and the unit, split on the whitespace run between them: a
        // leading or trailing space would leave an empty side, which the
        // shape checks below refuse.
        let split = text.find(char::is_whitespace)?;
        let count_text = &text[..split];
        let unit = text[split..].trim_start_matches(char::is_whitespace);
        if unit.is_empty() || unit.contains(char::is_whitespace) {
            return None;
        }
        if !is_decimal_number(count_text) {
            return None;
        }
        let count: f64 = count_text.parse().ok()?;
        match unit {
            "nano" | "nanos" => Some(Self::nanos(Self::round_ties_away(count))),
            "micro" | "micros" => Some(Self::nanos(Self::round_ties_away(count * 1_000.0))),
            "milli" | "millis" => Some(Self::millis(count)),
            "second" | "seconds" => Some(Self::seconds(count)),
            "minute" | "minutes" => Some(Self::minutes(count)),
            "hour" | "hours" => Some(Self::hours(count)),
            "day" | "days" => Some(Self::days(count)),
            "week" | "weeks" => Some(Self::weeks(count)),
            _ => None,
        }
    }

    /// `Duration.infinity`.
    pub fn infinity() -> Self {
        Self {
            value: DurationValue::Infinity,
        }
    }

    /// `Duration.negativeInfinity`.
    pub fn negative_infinity() -> Self {
        Self {
            value: DurationValue::NegativeInfinity,
        }
    }

    /// `Duration.zero`.
    pub fn zero() -> Self {
        Self::ZERO
    }

    /// Effect 4's `roundTiesAwayFromZero`, on integers.
    fn round_ties_away(value: f64) -> i128 {
        if value < 0.0 {
            (value - 0.5).ceil() as i128
        } else {
            (value + 0.5).floor() as i128
        }
    }

    /// The value's exact nanoseconds, or `None` when the millis value does
    /// not fit (`Millis` above ~1.7e21) or is infinite. Effect 4's BigInt is
    /// unbounded; the crate's i128 is not, and the carried rows stay inside.
    fn exact_nanos(&self) -> Option<i128> {
        match self.value {
            DurationValue::Nanos(nanos) => Some(nanos),
            DurationValue::Millis(millis) => {
                if !millis.is_finite() {
                    return None;
                }
                let scaled = millis * 1_000_000.0;
                if !scaled.is_finite() || scaled.abs() >= 1.7e38 {
                    return None;
                }
                Some(Self::round_ties_away(scaled))
            }
            DurationValue::Infinity | DurationValue::NegativeInfinity => None,
        }
    }

    /// Millis rounded to a `u64`, saturating: the accessor the clock and
    /// schedule paths sleep on. Effect 4's row-facing conversion is
    /// [`Duration::to_millis`].
    pub fn as_millis(self) -> u64 {
        match self.value {
            DurationValue::Millis(millis) => {
                if !millis.is_finite() {
                    return if millis > 0.0 { u64::MAX } else { 0 };
                }
                Self::round_ties_away(millis).clamp(0, u64::MAX as i128) as u64
            }
            DurationValue::Nanos(nanos) => {
                let rounded = if nanos >= 0 {
                    (nanos + 500_000) / 1_000_000
                } else {
                    -((-nanos + 500_000) / 1_000_000)
                };
                rounded.clamp(0, u64::MAX as i128) as u64
            }
            DurationValue::Infinity => u64::MAX,
            DurationValue::NegativeInfinity => 0,
        }
    }

    /// Whether this is exactly zero (Effect 4's `isZero`).
    pub fn is_zero(self) -> bool {
        match self.value {
            DurationValue::Millis(millis) => millis == 0.0,
            DurationValue::Nanos(nanos) => nanos == 0,
            _ => false,
        }
    }

    /// Effect 4's `isFinite`.
    pub fn is_finite(self) -> bool {
        matches!(
            self.value,
            DurationValue::Millis(_) | DurationValue::Nanos(_)
        )
    }

    /// Effect 4's `isNegative`.
    pub fn is_negative(self) -> bool {
        match self.value {
            DurationValue::Millis(millis) => millis < 0.0,
            DurationValue::Nanos(nanos) => nanos < 0,
            DurationValue::NegativeInfinity => true,
            DurationValue::Infinity => false,
        }
    }

    /// Effect 4's `isPositive`.
    pub fn is_positive(self) -> bool {
        !self.is_negative() && !self.is_zero()
    }

    /// Effect 4's `abs`.
    pub fn abs(self) -> Self {
        match self.value {
            DurationValue::NegativeInfinity => Self::infinity(),
            DurationValue::Millis(millis) => Self::millis(millis.abs()),
            DurationValue::Nanos(nanos) => Self::nanos(nanos.abs()),
            other => Self { value: other },
        }
    }

    /// Effect 4's `negate`.
    pub fn negate(self) -> Self {
        match self.value {
            DurationValue::Infinity => Self::negative_infinity(),
            DurationValue::NegativeInfinity => Self::infinity(),
            DurationValue::Millis(millis) => Self::millis(-millis),
            DurationValue::Nanos(nanos) => Self::nanos(-nanos),
        }
    }

    /// Effect 4's `toMillis`.
    pub fn to_millis(self) -> f64 {
        match self.value {
            DurationValue::Millis(millis) => millis,
            DurationValue::Nanos(nanos) => nanos as f64 / 1_000_000.0,
            DurationValue::Infinity => f64::INFINITY,
            DurationValue::NegativeInfinity => f64::NEG_INFINITY,
        }
    }

    /// Effect 4's `toSeconds`: from the exact representation, so a single
    /// nanosecond is exactly `1e-9`.
    pub fn to_seconds(self) -> f64 {
        match self.value {
            DurationValue::Millis(millis) => millis / 1000.0,
            DurationValue::Nanos(nanos) => nanos as f64 / 1_000_000_000.0,
            DurationValue::Infinity => f64::INFINITY,
            DurationValue::NegativeInfinity => f64::NEG_INFINITY,
        }
    }

    /// Effect 4's `toMinutes`.
    pub fn to_minutes(self) -> f64 {
        self.to_millis() / 60_000.0
    }

    /// Effect 4's `toHours`.
    pub fn to_hours(self) -> f64 {
        self.to_millis() / 3_600_000.0
    }

    /// Effect 4's `toDays`.
    pub fn to_days(self) -> f64 {
        self.to_millis() / 86_400_000.0
    }

    /// Effect 4's `toWeeks`.
    pub fn to_weeks(self) -> f64 {
        self.to_millis() / 604_800_000.0
    }

    /// Effect 4's `toNanos`: `None` for the infinite sentinels.
    pub fn to_nanos(self) -> Option<i128> {
        self.exact_nanos()
    }

    /// Effect 4's `toNanosUnsafe`: it throws on the infinite sentinels, so
    /// this panics.
    pub fn to_nanos_unsafe(self) -> i128 {
        self.exact_nanos()
            .expect("an infinite duration has no nanoseconds")
    }

    /// Effect 4's `toHrTime`: `[seconds, nanos]` with the sign carried by
    /// both parts.
    pub fn to_hr_time(self) -> (f64, f64) {
        match self.exact_nanos() {
            Some(nanos) => {
                let sign = if nanos < 0 { -1.0 } else { 1.0 };
                let absolute = nanos.unsigned_abs();
                (
                    sign * (absolute / 1_000_000_000) as f64,
                    sign * (absolute % 1_000_000_000) as f64,
                )
            }
            None => match self.value {
                DurationValue::Infinity => (f64::INFINITY, 0.0),
                DurationValue::NegativeInfinity => (f64::NEG_INFINITY, 0.0),
                // A millis value too large for i128 still renders: Effect 4's
                // hr-time of such a value overflows its BigInt too, so this
                // arm is unreachable for the carried rows.
                _ => (self.to_millis() / 1000.0, 0.0),
            },
        }
    }

    /// Effect 4's `sum`: same-sign infinities keep the sign, opposite
    /// infinities cancel to zero, and finite values add in the wider
    /// representation (the millis side converts to exact nanos).
    pub fn sum(self, that: Self) -> Self {
        match (self.value, that.value) {
            (DurationValue::Infinity, DurationValue::NegativeInfinity)
            | (DurationValue::NegativeInfinity, DurationValue::Infinity) => Self::zero(),
            (DurationValue::Infinity, _) | (_, DurationValue::Infinity) => Self::infinity(),
            (DurationValue::NegativeInfinity, _) | (_, DurationValue::NegativeInfinity) => {
                Self::negative_infinity()
            }
            (DurationValue::Millis(a), DurationValue::Millis(b)) => Self::millis(a + b),
            (a, b) => match (
                Self { value: a }.exact_nanos(),
                Self { value: b }.exact_nanos(),
            ) {
                (Some(a), Some(b)) => Self::nanos(a + b),
                // Mixed values that do not fit i128 fall back to millis.
                _ => Self::millis(self.to_millis() + that.to_millis()),
            },
        }
    }

    /// Effect 4's `subtract`.
    pub fn subtract(self, that: Self) -> Self {
        self.sum(that.negate())
    }

    /// Effect 4's `times`: millis scales in millis space, nanos scales by the
    /// integer multiplier (a fractional multiplier throws in Effect 4, and
    /// panics here), and the infinities scale by sign, with zero yielding
    /// the zero duration.
    pub fn times(self, times: f64) -> Self {
        match self.value {
            DurationValue::Infinity | DurationValue::NegativeInfinity => {
                if times > 0.0 {
                    self
                } else if times < 0.0 {
                    self.negate()
                } else {
                    Self::zero()
                }
            }
            DurationValue::Millis(millis) => Self::make_millis(millis * times),
            DurationValue::Nanos(nanos) => {
                assert!(
                    times.is_finite() && times.fract() == 0.0,
                    "times on a nanos duration needs an integer multiplier, like Effect 4's BigInt(times)"
                );
                Self::nanos(nanos * times as i128)
            }
        }
    }

    /// Effect 4's `divide`: `None` for a zero, NaN or infinite divisor, and
    /// for a nanos value the division does not land exactly on.
    pub fn divide(self, by: f64) -> Option<Self> {
        if by == 0.0 || !by.is_finite() {
            return None;
        }
        match self.value {
            DurationValue::Infinity => Some(if by < 0.0 {
                Self::negative_infinity()
            } else {
                Self::infinity()
            }),
            DurationValue::NegativeInfinity => Some(if by < 0.0 {
                Self::infinity()
            } else {
                Self::negative_infinity()
            }),
            DurationValue::Millis(millis) => Some(Self::make_millis(millis / by)),
            DurationValue::Nanos(nanos) => {
                // Effect 4 divides the bigint by `BigInt(by)`, which throws
                // for a non-integer divisor: the quotient truncates toward
                // zero and the divisor must be a whole number.
                if by.fract() != 0.0 {
                    return None;
                }
                Some(Self::nanos(nanos / by as i128))
            }
        }
    }

    /// Effect 4's `divideUnsafe`: the IEEE 754 fallbacks instead of `None`.
    pub fn divide_unsafe(self, by: f64) -> Self {
        match self.divide(by) {
            Some(divided) => divided,
            None => {
                if self.is_zero() {
                    return Self::zero();
                }
                if by.is_nan() || by.is_infinite() {
                    return Self::zero();
                }
                if by == 0.0 {
                    return if by.is_sign_negative() {
                        self.negate().infinity_or_negative()
                    } else {
                        self.infinity_or_negative()
                    };
                }
                Self::zero()
            }
        }
    }

    /// The infinite sentinel matching this value's sign.
    fn infinity_or_negative(self) -> Self {
        if self.is_negative() {
            Self::negative_infinity()
        } else {
            Self::infinity()
        }
    }

    /// Effect 4's `compare`: total order across both representations, with
    /// `NegativeInfinity` below every finite value below `Infinity`.
    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (&self.value, &other.value) {
            (DurationValue::Infinity, DurationValue::Infinity) => Ordering::Equal,
            (DurationValue::NegativeInfinity, DurationValue::NegativeInfinity) => Ordering::Equal,
            (DurationValue::Infinity, _) => Ordering::Greater,
            (_, DurationValue::Infinity) => Ordering::Less,
            (DurationValue::NegativeInfinity, _) => Ordering::Less,
            (_, DurationValue::NegativeInfinity) => Ordering::Greater,
            (DurationValue::Millis(a), DurationValue::Millis(b)) => {
                a.partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (DurationValue::Nanos(a), DurationValue::Nanos(b)) => a.cmp(b),
            // Mixed: Effect 4 converts the millis side to exact nanos. When
            // that overflows i128, compare in millis space instead.
            (_, _) => match (self.exact_nanos(), other.exact_nanos()) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => self
                    .to_millis()
                    .partial_cmp(&other.to_millis())
                    .unwrap_or(Ordering::Equal),
            },
        }
    }

    /// Effect 4's `Duration.Order`.
    pub fn order() -> crate::Order<Duration> {
        crate::Order::make(|a: &Duration, b: &Duration| a.compare(b))
    }

    /// Effect 4's `Duration.Equivalence`.
    pub fn equivalence() -> crate::Equivalence<Duration> {
        crate::Equivalence::make(|a: &Duration, b: &Duration| a.compare(b).is_eq())
    }

    /// Effect 4's `Duration.max` (ties keep the first value).
    pub fn max(self, other: Self) -> Self {
        if self.compare(&other).is_lt() {
            other
        } else {
            self
        }
    }

    /// Effect 4's `Duration.min` (ties keep the first value).
    pub fn min(self, other: Self) -> Self {
        if self.compare(&other).is_gt() {
            other
        } else {
            self
        }
    }

    /// Effect 4's `Duration.clamp`, inclusive at both bounds.
    pub fn clamp(self, minimum: Self, maximum: Self) -> Self {
        self.max(minimum).min(maximum)
    }

    /// Effect 4's `Duration.equals`: equivalence, not representation.
    pub fn equals(self, other: Self) -> bool {
        self.compare(&other).is_eq()
    }

    /// Effect 4's `Duration.between`, inclusive at both bounds.
    pub fn between(self, minimum: Self, maximum: Self) -> bool {
        !self.compare(&minimum).is_lt() && !self.compare(&maximum).is_gt()
    }

    /// Effect 4's `Duration.isGreaterThan`.
    pub fn is_greater_than(self, other: Self) -> bool {
        self.compare(&other).is_gt()
    }

    /// Effect 4's `Duration.isGreaterThanOrEqualTo`.
    pub fn is_greater_than_or_equal(self, other: Self) -> bool {
        !self.compare(&other).is_lt()
    }

    /// Effect 4's `Duration.isLessThan`.
    pub fn is_less_than(self, other: Self) -> bool {
        self.compare(&other).is_lt()
    }

    /// Effect 4's `Duration.isLessThanOrEqualTo`.
    pub fn is_less_than_or_equal(self, other: Self) -> bool {
        !self.compare(&other).is_gt()
    }

    /// Effect 4's `Duration.CombinerMax`.
    pub fn combiner_max() -> crate::Combiner<Duration> {
        crate::Combiner::max(Self::order())
    }

    /// Effect 4's `Duration.CombinerMin`.
    pub fn combiner_min() -> crate::Combiner<Duration> {
        crate::Combiner::min(Self::order())
    }

    /// Effect 4's `Duration.ReducerSum`.
    pub fn reducer_sum() -> crate::Reducer<Duration> {
        crate::Reducer::make(Self::zero(), |a: &Duration, b: &Duration| a.sum(*b))
    }

    /// Effect 4's `parts`: the signed, normalized decomposition.
    pub fn parts(self) -> Parts {
        match self.value {
            DurationValue::Infinity | DurationValue::NegativeInfinity => {
                let sign = if matches!(self.value, DurationValue::Infinity) {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                };
                return Parts {
                    days: sign,
                    hours: sign,
                    minutes: sign,
                    seconds: sign,
                    millis: sign,
                    nanos: sign,
                };
            }
            _ => {}
        }
        let nanos = self.to_nanos_unsafe();
        let negative = nanos < 0;
        let absolute = nanos.unsigned_abs();
        let millis_total = absolute / 1_000_000;
        let seconds_total = millis_total / 1000;
        let minutes_total = seconds_total / 60;
        let hours_total = minutes_total / 60;
        let sign = if negative { -1.0 } else { 1.0 };
        Parts {
            days: sign * (hours_total / 24) as f64,
            hours: sign * (hours_total % 24) as f64,
            minutes: sign * (minutes_total % 60) as f64,
            seconds: sign * (seconds_total % 60) as f64,
            millis: sign * (millis_total % 1000) as f64,
            nanos: sign * (absolute % 1_000_000) as f64,
        }
    }

    /// Effect 4's `format`.
    pub fn format(self) -> String {
        if matches!(self.value, DurationValue::Infinity) {
            return "Infinity".to_string();
        }
        if matches!(self.value, DurationValue::NegativeInfinity) {
            return "-Infinity".to_string();
        }
        if self.is_zero() {
            return "0".to_string();
        }
        if self.is_negative() {
            return format!("-{}", self.abs().format());
        }
        let parts = self.parts();
        let mut pieces = Vec::new();
        if parts.days != 0.0 {
            pieces.push(format!("{}d", parts.days));
        }
        if parts.hours != 0.0 {
            pieces.push(format!("{}h", parts.hours));
        }
        if parts.minutes != 0.0 {
            pieces.push(format!("{}m", parts.minutes));
        }
        if parts.seconds != 0.0 {
            pieces.push(format!("{}s", parts.seconds));
        }
        if parts.millis != 0.0 {
            pieces.push(format!("{}ms", parts.millis));
        }
        if parts.nanos != 0.0 {
            pieces.push(format!("{}ns", parts.nanos));
        }
        pieces.join(" ")
    }

    /// Effect 4's `toString`.
    pub fn to_pin_string(self) -> String {
        match self.value {
            DurationValue::Infinity => "Infinity".to_string(),
            DurationValue::NegativeInfinity => "-Infinity".to_string(),
            DurationValue::Nanos(nanos) => format!("{nanos} nanos"),
            DurationValue::Millis(millis) => {
                if millis.fract() == 0.0 {
                    format!("{} millis", millis as i128)
                } else {
                    // A fractional millis renders in nanoseconds, like the
                    // Effect 4's toString.
                    format!("{} nanos", self.to_nanos_unsafe())
                }
            }
        }
    }

    /// Effect 4's `toJSON`: the tagged representation, with nanos rendered
    /// as a decimal string like Effect 4's bigint.
    pub fn to_json(self) -> DurationJson {
        match self.value {
            DurationValue::Infinity => DurationJson::Infinity,
            DurationValue::NegativeInfinity => DurationJson::NegativeInfinity,
            DurationValue::Millis(millis) => {
                if millis.fract() == 0.0 {
                    DurationJson::Millis(millis)
                } else {
                    DurationJson::Nanos(self.to_nanos_unsafe().to_string())
                }
            }
            DurationValue::Nanos(nanos) => DurationJson::Nanos(nanos.to_string()),
        }
    }

    /// Effect 4's `inspect` (node's util.inspect renders the same shape).
    pub fn inspect(self) -> DurationJson {
        self.to_json()
    }

    /// `Duration.match`: one arm per representation; Effect 4 folds a missing
    /// `onNegativeInfinity` into `onInfinity`, and this does the same by
    /// taking both closures.
    pub fn match_duration<A>(
        self,
        on_millis: impl FnOnce(f64) -> A,
        on_nanos: impl FnOnce(i128) -> A,
        on_infinity: impl FnOnce() -> A,
        on_negative_infinity: impl FnOnce() -> A,
    ) -> A {
        match self.value {
            DurationValue::Millis(millis) => on_millis(millis),
            DurationValue::Nanos(nanos) => on_nanos(nanos),
            DurationValue::Infinity => on_infinity(),
            DurationValue::NegativeInfinity => on_negative_infinity(),
        }
    }
}

impl PartialEq for Duration {
    /// Structural equality: the same representation and payload, which is
    /// whin Effect 4's `deepStrictEqual` rows observe.
    fn eq(&self, other: &Self) -> bool {
        match (self.value, other.value) {
            (DurationValue::Millis(a), DurationValue::Millis(b)) => a == b,
            (DurationValue::Nanos(a), DurationValue::Nanos(b)) => a == b,
            (DurationValue::Infinity, DurationValue::Infinity) => true,
            (DurationValue::NegativeInfinity, DurationValue::NegativeInfinity) => true,
            _ => false,
        }
    }
}

impl Eq for Duration {}

impl PartialOrd for Duration {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Duration {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Duration::compare(self, other)
    }
}

impl crate::Equal for Duration {
    /// Effect 4's equivalence: equal across representations.
    fn equal(&self, other: &Self) -> bool {
        self.compare(other).is_eq()
    }
}

impl crate::Hash for Duration {
    /// Effect 4's `Hash.symbol`: the canonical nanoseconds, so equal values
    /// hash alike across representations.
    fn hash_value(&self) -> u32 {
        match self.value {
            DurationValue::Millis(millis) => {
                let scaled = millis * 1_000_000.0;
                if scaled.is_finite() && scaled.abs() < 1.7e38 {
                    crate::hash_f64(Duration::round_ties_away(scaled) as f64)
                } else {
                    crate::hash_f64(millis)
                }
            }
            DurationValue::Nanos(nanos) => crate::hash_f64(nanos as f64),
            DurationValue::Infinity | DurationValue::NegativeInfinity => {
                crate::hash_f64(self.to_millis())
            }
        }
    }
}

impl std::fmt::Display for Duration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_pin_string())
    }
}

/// `Duration.isDuration`: the runtime type guard.
pub fn is_duration(value: &dyn std::any::Any) -> bool {
    value.is::<Duration>()
}

/// Effect 4's `parts` result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub days: f64,
    pub hours: f64,
    pub minutes: f64,
    pub seconds: f64,
    pub millis: f64,
    pub nanos: f64,
}

/// Effect 4's `toJSON` result: the tagged shape with nanos as a string.
#[derive(Clone, Debug, PartialEq)]
pub enum DurationJson {
    Millis(f64),
    Nanos(String),
    Infinity,
    NegativeInfinity,
}

/// Effect 4's duration-string number: an optional sign, at least one digit, and
/// an optional `.`-prefixed fraction that must have digits of its own (so `1.`,
/// `.5` and `1e3` are all refused, exactly as Effect 4's regex refuses them).
fn is_decimal_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() {
        return false;
    }
    match digits.split_once('.') {
        None => digits.bytes().all(|byte| byte.is_ascii_digit()),
        Some((whole, fraction)) => {
            !whole.is_empty()
                && whole.bytes().all(|byte| byte.is_ascii_digit())
                && !fraction.is_empty()
                && fraction.bytes().all(|byte| byte.is_ascii_digit())
        }
    }
}

/// How a sleep ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepOutcome {
    /// The deadline passed.
    Elapsed,
    /// Interruption was requested first.
    Interrupted,
}

/// The clock service implementation. Object-safe so tests share one clock
/// behind `Arc` while bodies hold the trait object.
pub trait ClockImpl: Send + Sync + 'static {
    /// Park until `duration` elapses or `interrupted()` turns true, whichever
    /// comes first. Never advances virtual time itself.
    fn sleep(&self, duration: Duration, interrupted: &dyn Fn() -> bool) -> SleepOutcome;
    /// Current time in millis (virtual for tests, wall for system).
    fn now_millis(&self) -> u64;
    /// Monotonic time in millis: it only ever moves forward.
    ///
    /// A measured duration reads this, never `now_millis`, so a wall clock set
    /// backwards cannot make elapsed time negative (`Metric.test.ts:1034`). An
    /// implementation with no separate monotonic source reuses its wall
    /// reading.
    fn monotonic_millis(&self) -> u64 {
        self.now_millis()
    }
}

/// A service tag for the clock: provide any [`ClockImpl`].
pub struct Clock;

impl crate::ServiceTag for Clock {
    type Service = Arc<dyn ClockImpl>;
    const ID: &'static str = "elm-effect/clock";
}

/// Wall-clock implementation: real deadlines, interruptible.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock;

impl ClockImpl for SystemClock {
    fn sleep(&self, duration: Duration, interrupted: &dyn Fn() -> bool) -> SleepOutcome {
        if duration.is_zero() {
            return SleepOutcome::Elapsed;
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(duration.as_millis());
        // An interrupt request notifies this latch. A thread outside a fiber
        // has no source to subscribe to, so it keeps the bounded wait.
        let wake = Arc::new((Mutex::new(()), Condvar::new()));
        let hook = crate::fiber::subscribe_current_interrupt({
            let wake = Arc::clone(&wake);
            move || {
                let _guard = wake.0.lock().unwrap();
                wake.1.notify_all();
            }
        });
        loop {
            // Lock before re-checking: the interrupt waker takes this lock
            // too, so the request cannot land between the check and the wait.
            let (lock, cvar) = &*wake;
            let guard = lock.lock().unwrap();
            if interrupted() {
                return SleepOutcome::Interrupted;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return SleepOutcome::Elapsed;
            }
            let wait = if hook.is_some() {
                remaining
            } else {
                remaining.min(crate::COOPERATIVE_QUANTUM)
            };
            let _ = cvar.wait_timeout(guard, wait).unwrap();
        }
    }

    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Process-relative elapsed time: an `Instant` never runs backwards, so a
    /// wall-clock correction changes nothing here.
    fn monotonic_millis(&self) -> u64 {
        static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        ORIGIN
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
}

struct Sleeper {
    deadline_ms: u64,
    woken: Arc<(Mutex<bool>, Condvar)>,
}

/// Virtual clock: time advances only through [`TestClock::adjust`]. Sleepers
/// park on per-sleeper latches woken by `adjust`; interruption is observed
/// through the cooperative poll quantum (latency only, never completion).
pub struct TestClock {
    now_ms: Mutex<u64>,
    /// Time that passed, which only [`adjust`](TestClock::adjust) moves.
    /// `set_time` moves the wall reading and leaves this alone, exactly as the
    /// Effect 4's test clock does.
    monotonic_ms: Mutex<u64>,
    sleepers: Mutex<Vec<Sleeper>>,
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            now_ms: Mutex::new(0),
            monotonic_ms: Mutex::new(0),
            sleepers: Mutex::new(Vec::new()),
        }
    }

    /// Advance virtual time, waking every sleeper whose deadline passed.
    /// Never wakes the future: sleepers scheduled after this call see the new
    /// time (the adjust-first test pins this).
    pub fn adjust(&self, duration: Duration) {
        {
            let mut monotonic = self.monotonic_ms.lock().unwrap();
            *monotonic = monotonic.saturating_add(duration.as_millis());
        }
        let now = {
            let mut guard = self.now_ms.lock().unwrap();
            *guard = guard.saturating_add(duration.as_millis());
            *guard
        };
        let mut sleepers = self.sleepers.lock().unwrap();
        // Woken sleepers leave the list here, not when they next run: a woken
        // sleeper is no longer parked, and `pending_sleepers` is the test aid
        // that says so.
        sleepers.retain(|sleeper| {
            if sleeper.deadline_ms <= now {
                let (lock, cvar) = &*sleeper.woken;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
                false
            } else {
                true
            }
        });
    }

    /// Sleepers currently parked. Test aid: asserts scheduling happened (or
    /// not) before driving time.
    pub fn pending_sleepers(&self) -> usize {
        self.sleepers.lock().unwrap().len()
    }

    /// Set the wall reading without advancing time (`TestClock.setTime`).
    /// Sleepers keep their deadlines; the monotonic reading is untouched, so a
    /// duration measured across this call is the time that really passed.
    pub fn set_time(&self, millis: u64) {
        *self.now_ms.lock().unwrap() = millis;
    }

    fn remove_sleeper(&self, woken: &Arc<(Mutex<bool>, Condvar)>) {
        self.sleepers
            .lock()
            .unwrap()
            .retain(|s| !Arc::ptr_eq(&s.woken, woken));
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockImpl for TestClock {
    fn sleep(&self, duration: Duration, interrupted: &dyn Fn() -> bool) -> SleepOutcome {
        let woken: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
        // Read the clock while holding the sleeper list. A concurrent
        // `adjust` then either lands before this read (its time is already in
        // `now`) or finds the sleeper registered and wakes it. Sampling before
        // taking the list left a window where neither held.
        let deadline = {
            let mut sleepers = self.sleepers.lock().unwrap();
            let now = self.now_millis();
            let deadline = now.saturating_add(duration.as_millis());
            if deadline <= now {
                return SleepOutcome::Elapsed;
            }
            sleepers.push(Sleeper {
                deadline_ms: deadline,
                woken: Arc::clone(&woken),
            });
            deadline
        };
        // A virtual sleeper wakes when `adjust` reaches its deadline; an
        // interrupt request notifies the same latch here. Outside a fiber the
        // predicate has no notification source, so the bounded wait remains.
        let hook = crate::fiber::subscribe_current_interrupt({
            let woken = Arc::clone(&woken);
            move || {
                let (lock, cvar) = &*woken;
                *lock.lock().unwrap() = true;
                cvar.notify_all();
            }
        });
        loop {
            if interrupted() {
                self.remove_sleeper(&woken);
                return SleepOutcome::Interrupted;
            }
            if self.now_millis() >= deadline {
                self.remove_sleeper(&woken);
                return SleepOutcome::Elapsed;
            }
            let (lock, cvar) = &*woken;
            let guard = lock.lock().unwrap();
            if *guard {
                continue;
            }
            if hook.is_some() {
                // `adjust` and the interrupt hook are the only wake sources;
                // the deadline itself is what `adjust` compares against.
                drop(cvar.wait(guard).unwrap());
            } else {
                let remaining =
                    std::time::Duration::from_millis(deadline.saturating_sub(self.now_millis()));
                let _ = cvar
                    .wait_timeout(guard, remaining.min(crate::COOPERATIVE_QUANTUM))
                    .unwrap();
            }
        }
    }

    fn now_millis(&self) -> u64 {
        *self.now_ms.lock().unwrap()
    }

    fn monotonic_millis(&self) -> u64 {
        *self.monotonic_ms.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn duration_orders_and_constructs() {
        assert!(Duration::from_minutes(1) > Duration::from_secs(59));
        assert_eq!(Duration::from_secs(2).as_millis(), 2000);
        assert!(Duration::ZERO.is_zero());
        assert!(!Duration::from_millis(1).is_zero());
    }

    #[test]
    fn zero_sleep_completes_without_parking() {
        let clock = TestClock::new();
        let out = clock.sleep(Duration::ZERO, &|| false);
        assert_eq!(out, SleepOutcome::Elapsed);
        assert_eq!(clock.pending_sleepers(), 0);
    }

    #[test]
    fn adjust_wakes_parked_sleeper() {
        let clock = Arc::new(TestClock::new());
        let worker = clock.clone();
        let done = Arc::new(AtomicBool::new(false));
        let probe = done.clone();
        let t = std::thread::spawn(move || {
            let out = worker.sleep(Duration::from_minutes(1), &|| false);
            probe.store(true, Ordering::SeqCst);
            out
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while clock.pending_sleepers() < 1 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        clock.adjust(Duration::from_minutes(1));
        assert_eq!(t.join().expect("sleeper"), SleepOutcome::Elapsed);
        assert!(done.load(Ordering::SeqCst));
        assert_eq!(clock.pending_sleepers(), 0);
    }
}
