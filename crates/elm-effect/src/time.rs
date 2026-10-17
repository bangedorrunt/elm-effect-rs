//! Law L8: time enters through the Clock service, never the wall.
//!
//! [`Duration`] is millisecond time. [`ClockImpl`] is the service behind the
//! [`Clock`](crate::Clock) tag: [`SystemClock`] parks threads on real
//! deadlines, [`TestClock`] parks fibers on virtual time advanced only by
//! [`TestClock::adjust`]. A sleep observes interruption through a poll
//! closure, so bodies pass `&|| ctx.is_interrupted()`.
//!
//! The 1ms poll quantum inside sleeps bounds wall latency only: completions
//! happen exclusively via `adjust` (virtual) or deadlines (real), so test
//! outcomes are virtual-time-deterministic.

use std::sync::{Arc, Condvar, Mutex};

/// The pin's `DurationValue`: a duration is stored in milliseconds (an IEEE
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
/// The value model is the pin's: `Millis` for values built from
/// millisecond-scale constructors, `Nanos` for exact sub-millisecond
/// values, and the two infinite sentinels. Equality is the pin's
/// equivalence, so `millis(1)` and `nanos(1_000_000)` are equal and hash
/// alike; structural equality (`==`) still distinguishes the
/// representations, which is what the pin's `deepStrictEqual` rows observe.
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

    /// The pin's `make`: a millis value that is not an integer becomes an
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

    /// `Duration.nanos`. The pin's `make` maps a zero bigint to the zero
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

    /// The pin's `roundTiesAwayFromZero`, on integers.
    fn round_ties_away(value: f64) -> i128 {
        if value < 0.0 {
            (value - 0.5).ceil() as i128
        } else {
            (value + 0.5).floor() as i128
        }
    }

    /// The value's exact nanoseconds, or `None` when the millis value does
    /// not fit (`Millis` above ~1.7e21) or is infinite. The pin's BigInt is
    /// unbounded; the port's i128 is not, and the carried rows stay inside.
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
    /// schedule paths sleep on. The pin's row-facing conversion is
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

    /// Whether this is exactly zero (the pin's `isZero`).
    pub fn is_zero(self) -> bool {
        match self.value {
            DurationValue::Millis(millis) => millis == 0.0,
            DurationValue::Nanos(nanos) => nanos == 0,
            _ => false,
        }
    }

    /// The pin's `isFinite`.
    pub fn is_finite(self) -> bool {
        matches!(
            self.value,
            DurationValue::Millis(_) | DurationValue::Nanos(_)
        )
    }

    /// The pin's `isNegative`.
    pub fn is_negative(self) -> bool {
        match self.value {
            DurationValue::Millis(millis) => millis < 0.0,
            DurationValue::Nanos(nanos) => nanos < 0,
            DurationValue::NegativeInfinity => true,
            DurationValue::Infinity => false,
        }
    }

    /// The pin's `isPositive`.
    pub fn is_positive(self) -> bool {
        !self.is_negative() && !self.is_zero()
    }

    /// The pin's `abs`.
    pub fn abs(self) -> Self {
        match self.value {
            DurationValue::NegativeInfinity => Self::infinity(),
            DurationValue::Millis(millis) => Self::millis(millis.abs()),
            DurationValue::Nanos(nanos) => Self::nanos(nanos.abs()),
            other => Self { value: other },
        }
    }

    /// The pin's `negate`.
    pub fn negate(self) -> Self {
        match self.value {
            DurationValue::Infinity => Self::negative_infinity(),
            DurationValue::NegativeInfinity => Self::infinity(),
            DurationValue::Millis(millis) => Self::millis(-millis),
            DurationValue::Nanos(nanos) => Self::nanos(-nanos),
        }
    }

    /// The pin's `toMillis`.
    pub fn to_millis(self) -> f64 {
        match self.value {
            DurationValue::Millis(millis) => millis,
            DurationValue::Nanos(nanos) => nanos as f64 / 1_000_000.0,
            DurationValue::Infinity => f64::INFINITY,
            DurationValue::NegativeInfinity => f64::NEG_INFINITY,
        }
    }

    /// The pin's `toSeconds`: from the exact representation, so a single
    /// nanosecond is exactly `1e-9`.
    pub fn to_seconds(self) -> f64 {
        match self.value {
            DurationValue::Millis(millis) => millis / 1000.0,
            DurationValue::Nanos(nanos) => nanos as f64 / 1_000_000_000.0,
            DurationValue::Infinity => f64::INFINITY,
            DurationValue::NegativeInfinity => f64::NEG_INFINITY,
        }
    }

    /// The pin's `toMinutes`.
    pub fn to_minutes(self) -> f64 {
        self.to_millis() / 60_000.0
    }

    /// The pin's `toHours`.
    pub fn to_hours(self) -> f64 {
        self.to_millis() / 3_600_000.0
    }

    /// The pin's `toDays`.
    pub fn to_days(self) -> f64 {
        self.to_millis() / 86_400_000.0
    }

    /// The pin's `toWeeks`.
    pub fn to_weeks(self) -> f64 {
        self.to_millis() / 604_800_000.0
    }

    /// The pin's `toNanos`: `None` for the infinite sentinels.
    pub fn to_nanos(self) -> Option<i128> {
        self.exact_nanos()
    }

    /// The pin's `toNanosUnsafe`: it throws on the infinite sentinels, so
    /// this panics.
    pub fn to_nanos_unsafe(self) -> i128 {
        self.exact_nanos()
            .expect("an infinite duration has no nanoseconds")
    }

    /// The pin's `toHrTime`: `[seconds, nanos]` with the sign carried by
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
                // A millis value too large for i128 still renders: the pin's
                // hr-time of such a value overflows its BigInt too, so this
                // arm is unreachable for the carried rows.
                _ => (self.to_millis() / 1000.0, 0.0),
            },
        }
    }

    /// The pin's `sum`: same-sign infinities keep the sign, opposite
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

    /// The pin's `subtract`.
    pub fn subtract(self, that: Self) -> Self {
        self.sum(that.negate())
    }

    /// The pin's `times`: millis scales in millis space, nanos scales by the
    /// integer multiplier (a fractional multiplier throws in the pin, and
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
                    "times on a nanos duration needs an integer multiplier, like the pin's BigInt(times)"
                );
                Self::nanos(nanos * times as i128)
            }
        }
    }

    /// The pin's `divide`: `None` for a zero, NaN or infinite divisor, and
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
                // The pin divides the bigint by `BigInt(by)`, which throws
                // for a non-integer divisor: the quotient truncates toward
                // zero and the divisor must be a whole number.
                if by.fract() != 0.0 {
                    return None;
                }
                Some(Self::nanos(nanos / by as i128))
            }
        }
    }

    /// The pin's `divideUnsafe`: the IEEE 754 fallbacks instead of `None`.
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

    /// The pin's `compare`: total order across both representations, with
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
            // Mixed: the pin converts the millis side to exact nanos. When
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

    /// The pin's `Duration.Order`.
    pub fn order() -> crate::Order<Duration> {
        crate::Order::make(|a: &Duration, b: &Duration| a.compare(b))
    }

    /// The pin's `Duration.Equivalence`.
    pub fn equivalence() -> crate::Equivalence<Duration> {
        crate::Equivalence::make(|a: &Duration, b: &Duration| a.compare(b).is_eq())
    }

    /// The pin's `parts`: the signed, normalized decomposition.
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

    /// The pin's `format`.
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

    /// The pin's `toString`.
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
                    // pin's toString.
                    format!("{} nanos", self.to_nanos_unsafe())
                }
            }
        }
    }

    /// The pin's `toJSON`: the tagged representation, with nanos rendered
    /// as a decimal string like the pin's bigint.
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

    /// The pin's `inspect` (node's util.inspect renders the same shape).
    pub fn inspect(self) -> DurationJson {
        self.to_json()
    }

    /// `Duration.match`: one arm per representation; the pin folds a missing
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

    /// The pin's `parseNanos`: exact decimal-string arithmetic (no float
    /// rounding for the integer part) with ties away from zero.
    fn parse_nanos(input: &str, scale: i128) -> Option<i128> {
        let decimal_index = input.find('.');
        let Some(decimal_index) = decimal_index else {
            return input.parse::<i128>().ok().map(|value| value * scale);
        };
        let is_negative = input.starts_with('-');
        let fractional = &input[decimal_index + 1..];
        let fractional_scale = 10i128.checked_pow(fractional.len() as u32)?;
        let whole = input
            .get(if is_negative { 1 } else { 0 }..decimal_index)?
            .parse::<i128>()
            .ok()?;
        let fraction = if fractional.is_empty() {
            0
        } else {
            fractional.parse::<i128>().ok()?
        };
        let scaled = (whole * fractional_scale + fraction).checked_mul(scale)?;
        let rounded = scaled / fractional_scale
            + if scaled.rem_euclid(fractional_scale) * 2 >= fractional_scale {
                1
            } else {
                0
            };
        Some(if is_negative { -rounded } else { rounded })
    }

    /// `Duration.fromInput`: the shapes the pin's `Input` accepts, over the
    /// port's dynamic value domain. `None` for an unrecognized input.
    pub fn from_input(input: &crate::predicate::Value) -> Option<Self> {
        match input {
            crate::predicate::Value::Number(value) => {
                if value.is_nan() {
                    Some(Self::zero())
                } else if value.is_infinite() {
                    Some(if *value > 0.0 {
                        Self::infinity()
                    } else {
                        Self::negative_infinity()
                    })
                } else {
                    Some(Self::millis(*value))
                }
            }
            crate::predicate::Value::BigInt(value) => Some(Self::nanos(*value)),
            crate::predicate::Value::String(text) => Self::from_input_str(text),
            crate::predicate::Value::Array(parts) => {
                let (seconds, nanos) = match parts.as_slice() {
                    [a, b] => (a, b),
                    _ => return Some(Self::zero()),
                };
                let seconds = match seconds {
                    crate::predicate::Value::Number(value) => *value,
                    _ => return Some(Self::zero()),
                };
                let nanos = match nanos {
                    crate::predicate::Value::Number(value) => *value,
                    _ => return Some(Self::zero()),
                };
                if seconds.is_nan() || nanos.is_nan() {
                    return Some(Self::zero());
                }
                if seconds.is_infinite() || nanos.is_infinite() {
                    let positive = (seconds.is_infinite() && seconds > 0.0)
                        || (nanos.is_infinite() && nanos > 0.0);
                    return Some(if positive {
                        Self::infinity()
                    } else {
                        Self::negative_infinity()
                    });
                }
                let total = Self::round_ties_away(seconds * 1_000_000_000.0 + nanos);
                Some(Self::nanos(total))
            }
            crate::predicate::Value::Object(fields) => {
                let scale = |name: &str, factor: f64| -> Option<Self> {
                    match fields.get(name)? {
                        crate::predicate::Value::Number(value) => {
                            Some(Self::millis(value * factor))
                        }
                        _ => None,
                    }
                };
                if let Some(found) = scale("weeks", 604_800_000.0)
                    .or_else(|| scale("days", 86_400_000.0))
                    .or_else(|| scale("hours", 3_600_000.0))
                    .or_else(|| scale("minutes", 60_000.0))
                    .or_else(|| scale("seconds", 1000.0))
                    .or_else(|| scale("milliseconds", 1.0))
                {
                    return Some(found);
                }
                Some(Self::zero())
            }
            _ => Some(Self::zero()),
        }
    }

    /// The string form: `<count> <unit>` or `Infinity`. The pin's units are
    /// nano, micro, milli, second, minute, hour, day and week, singular or
    /// plural. Nano and micro parse as exact decimals; the millis-scale and
    /// larger units go through the constructors, so a fractional count
    /// becomes nanos exactly as `make` does.
    fn from_input_str(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed == "Infinity" {
            return Some(Self::infinity());
        }
        if trimmed == "-Infinity" {
            return Some(Self::negative_infinity());
        }
        let (count, unit) = trimmed.split_once(' ')?;
        let unit = unit.trim();
        match unit {
            "nano" | "nanos" => Some(Self::nanos(Self::parse_nanos(count, 1)?)),
            "micro" | "micros" => Some(Self::nanos(Self::parse_nanos(count, 1_000)?)),
            "milli" | "millis" => Some(Self::make_millis(count.parse::<f64>().ok()?)),
            "second" | "seconds" => Some(Self::seconds(count.parse::<f64>().ok()?)),
            "minute" | "minutes" => Some(Self::minutes(count.parse::<f64>().ok()?)),
            "hour" | "hours" => Some(Self::hours(count.parse::<f64>().ok()?)),
            "day" | "days" => Some(Self::days(count.parse::<f64>().ok()?)),
            "week" | "weeks" => Some(Self::weeks(count.parse::<f64>().ok()?)),
            _ => None,
        }
    }

    /// `Duration.fromInputUnsafe`: an unrecognized input becomes zero
    /// instead of `None`, which is what the pin's `fromInput` rows observe
    /// for `{}` and the invalid string.
    pub fn from_input_unsafe(input: &crate::predicate::Value) -> Self {
        Self::from_input(input).unwrap_or_else(Self::zero)
    }
}

impl PartialEq for Duration {
    /// Structural equality: the same representation and payload, which is
    /// what the pin's `deepStrictEqual` rows observe.
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
    /// The pin's equivalence: equal across representations.
    fn equal(&self, other: &Self) -> bool {
        self.compare(other).is_eq()
    }
}

impl crate::Hash for Duration {
    /// The pin's `Hash.symbol`: the canonical nanoseconds, so equal values
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

/// The pin's `parts` result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub days: f64,
    pub hours: f64,
    pub minutes: f64,
    pub seconds: f64,
    pub millis: f64,
    pub nanos: f64,
}

/// The pin's `toJSON` result: the tagged shape with nanos as a string.
#[derive(Clone, Debug, PartialEq)]
pub enum DurationJson {
    Millis(f64),
    Nanos(String),
    Infinity,
    NegativeInfinity,
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
        loop {
            if interrupted() {
                return SleepOutcome::Interrupted;
            }
            if std::time::Instant::now() >= deadline {
                return SleepOutcome::Elapsed;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
    }

    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

struct Sleeper {
    deadline_ms: u64,
    woken: Arc<(Mutex<bool>, Condvar)>,
}

/// Virtual clock: time advances only through [`TestClock::adjust`]. Sleepers
/// park on per-sleeper latches woken by `adjust`; interruption is observed
/// through a 1ms poll quantum (latency only, never completion).
pub struct TestClock {
    now_ms: Mutex<u64>,
    sleepers: Mutex<Vec<Sleeper>>,
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            now_ms: Mutex::new(0),
            sleepers: Mutex::new(Vec::new()),
        }
    }

    /// Advance virtual time, waking every sleeper whose deadline passed.
    /// Never wakes the future: sleepers scheduled after this call see the new
    /// time (the adjust-first test pins this).
    pub fn adjust(&self, duration: Duration) {
        let now = {
            let mut guard = self.now_ms.lock().unwrap();
            *guard = guard.saturating_add(duration.as_millis());
            *guard
        };
        let sleepers = self.sleepers.lock().unwrap();
        for sleeper in sleepers.iter() {
            if sleeper.deadline_ms <= now {
                let (lock, cvar) = &*sleeper.woken;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
            }
        }
    }

    /// Sleepers currently parked. Test aid: asserts scheduling happened (or
    /// not) before driving time.
    pub fn pending_sleepers(&self) -> usize {
        self.sleepers.lock().unwrap().len()
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
        let now = self.now_millis();
        let deadline = now.saturating_add(duration.as_millis());
        if deadline <= now {
            return SleepOutcome::Elapsed;
        }
        let woken: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
        self.sleepers.lock().unwrap().push(Sleeper {
            deadline_ms: deadline,
            woken: Arc::clone(&woken),
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
            let _ = cvar
                .wait_timeout(guard, std::time::Duration::from_millis(1))
                .unwrap();
        }
    }

    fn now_millis(&self) -> u64 {
        *self.now_ms.lock().unwrap()
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
