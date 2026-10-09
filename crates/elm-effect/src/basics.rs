//! Core basics (`Predicate`, `Duration` helpers, `Random` at the pin).
//!
//! Self-contained on purpose (test-included like the other port modules):
//! [`Predicate`] composes boolean checks (`and`/`or`/`not`/`xor`/`implies`
//! plus `every`/`some` over slices); duration helpers work in plain
//! milliseconds with `Order`-like compare, min/max/clamp/equals/between and
//! `from_input` parsing (`"500ms"`, `"30s"`, `"2m"`, `"1h"`, bare numbers);
//! [`SeedRandom`] is a seeded xorshift stream with deterministic output per
//! seed (`[0, 1)` doubles, booleans, half-open integer ranges).
//!
//! Residuals: struct-field refinement combinators, nanos representations,
//! `Date`-linked durations, and non-seeded entropy sources.

use std::cmp::Ordering;

/// Composable boolean check.
#[derive(Clone)]
pub struct Predicate<T> {
    test: std::sync::Arc<dyn Fn(&T) -> bool + Send + Sync>,
}

impl<T> Predicate<T> {
    /// Build from a test.
    pub fn new(test: impl Fn(&T) -> bool + Send + Sync + 'static) -> Self {
        Self {
            test: std::sync::Arc::new(test),
        }
    }

    /// Run the check.
    pub fn test(&self, value: &T) -> bool {
        (self.test)(value)
    }

    /// Both must hold.
    pub fn and(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| (a)(v) && (b)(v))
    }

    /// Either may hold (short-circuits).
    pub fn or(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| (a)(v) || (b)(v))
    }

    /// Negation.
    pub fn not(&self) -> Self
    where
        T: 'static,
    {
        let a = self.test.clone();
        Self::new(move |v| !(a)(v))
    }

    /// Exactly one holds.
    pub fn xor(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| (a)(v) != (b)(v))
    }

    /// Antecedent false or consequent true.
    pub fn implies(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| !(a)(v) || (b)(v))
    }

    /// Neither holds.
    pub fn nor(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| !((a)(v) || (b)(v)))
    }

    /// Not both hold.
    pub fn nand(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| !((a)(v) && (b)(v)))
    }

    /// Both agree (`Predicate.eqv` / xnor): true only when the two
    /// predicates return the same answer.
    pub fn eqv(&self, other: &Self) -> Self
    where
        T: 'static,
    {
        let (a, b) = (self.test.clone(), other.test.clone());
        Self::new(move |v| (a)(v) == (b)(v))
    }
}

/// Always true.
pub fn always<T>() -> Predicate<T> {
    Predicate::new(|_| true)
}

/// Always false.
pub fn never<T>() -> Predicate<T> {
    Predicate::new(|_| false)
}

/// `Predicate.every` (the COMBINING form): one predicate that holds when
/// every predicate in the list holds. An empty list is vacuously true.
///
/// Named `all_of` to keep it distinct from [`every`] above, which is the
/// slice form: one predicate quantified over a list of VALUES. The pin uses
/// the same spelling for both, which is a shape Rust cannot express in one
/// name without ambiguity, so the two forms carry different names here.
///
/// Built directly from the closures rather than by folding `and`: the fold
/// cannot prove `T: 'static` from the free-named function's bound, because
/// `Predicate<T>` itself carries no such bound on the struct.
pub fn all_of<T: 'static>(predicates: Vec<Predicate<T>>) -> Predicate<T> {
    let tests: Vec<std::sync::Arc<dyn Fn(&T) -> bool + Send + Sync>> =
        predicates.into_iter().map(|p| p.test).collect();
    Predicate::new(move |v: &T| tests.iter().all(|t| t(v)))
}

/// `Predicate.some` (the COMBINING form): one predicate that holds when any
/// predicate in the list holds. An empty list is false.
pub fn any_of<T: 'static>(predicates: Vec<Predicate<T>>) -> Predicate<T> {
    let tests: Vec<std::sync::Arc<dyn Fn(&T) -> bool + Send + Sync>> =
        predicates.into_iter().map(|p| p.test).collect();
    Predicate::new(move |v: &T| tests.iter().any(|t| t(v)))
}

/// True when every element passes (vacuous on empty).
pub fn every<T>(predicate: &Predicate<T>, values: &[T]) -> bool {
    values.iter().all(|v| predicate.test(v))
}

/// True when some element passes (stops at the first pass).
pub fn some<T>(predicate: &Predicate<T>, values: &[T]) -> bool {
    values.iter().any(|v| predicate.test(v))
}

/// Millisecond compare with total order.
pub fn compare_ms(a: u64, b: u64) -> Ordering {
    a.cmp(&b)
}

/// Millisecond equality.
pub fn equals_ms(a: u64, b: u64) -> bool {
    a == b
}

/// Smaller of two millisecond spans.
pub fn min_ms(a: u64, b: u64) -> u64 {
    a.min(b)
}

/// Larger of two millisecond spans.
pub fn max_ms(a: u64, b: u64) -> u64 {
    a.max(b)
}

/// Clamp into `[min, max]`.
pub fn clamp_ms(value: u64, min: u64, max: u64) -> u64 {
    value.clamp(min, max)
}

/// Inclusive range check.
pub fn between_ms(value: u64, min: u64, max: u64) -> bool {
    min <= value && value <= max
}

/// Integer division of a span (truncates, like the tag).
pub fn divide_ms(value: u64, by: u64) -> u64 {
    value / by.max(1)
}

/// Parse `"500ms"`, `"30s"`, `"2m"`, `"1h"` or bare millis.
/// `None` on malformed input (`fromInput` shape; panicking callers use
/// `from_input_unsafe`).
pub fn from_input(input: &str) -> Option<u64> {
    let text = input.trim();
    for (suffix, factor) in [("ms", 1u64), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)] {
        if let Some(number) = text.strip_suffix(suffix) {
            // `"ms"` strips last so `"500ms"` hits `"ms"` before `"s"`;
            // guard the `"m"`-vs-`"ms"` overlap explicitly.
            if suffix == "m" && text.ends_with("ms") {
                continue;
            }
            return number
                .trim()
                .parse::<u64>()
                .ok()
                .map(|n| n.saturating_mul(factor));
        }
    }
    text.parse::<u64>().ok()
}

/// Parse or panic (`fromInputUnsafe` shape).
pub fn from_input_unsafe(input: &str) -> u64 {
    from_input(input).expect("malformed duration input")
}

/// Seeded stream. Same seed gives the same sequence, every time.
#[derive(Clone, Debug)]
pub struct SeedRandom {
    state: u64,
}

impl SeedRandom {
    /// Seed the stream (zero maps to a fixed nonzero fallback).
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    fn step(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Double in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        const DIVISOR: f64 = u64::MAX as f64;
        (self.step() as f64) / (DIVISOR + 1.0)
    }

    /// Random boolean.
    pub fn next_bool(&mut self) -> bool {
        self.step() & 1 == 1
    }

    /// Integer in `[min, max)` (half-open; panics on empty ranges,
    /// saturates degenerate `min == max` to `min`).
    pub fn range(&mut self, min: i64, max: i64) -> i64 {
        assert!(min <= max, "empty range");
        if min == max {
            return min;
        }
        let span = (max as u64).wrapping_sub(min as u64);
        min.wrapping_add((self.step() % span) as i64)
    }

    /// Safe integer in `[0, 2^53)`.
    pub fn safe_integer(&mut self) -> i64 {
        (self.step() % 9_007_199_254_740_992) as i64
    }
}
