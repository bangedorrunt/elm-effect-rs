//! `Duration.test.ts` rows, ported against [`Duration`].
//!
//! The pin keeps two representations (millis and nanos) plus two infinite
//! sentinels. Structural `==` distinguishes the representations, while
//! `equals`, `Order`, `Equivalence`, and `Hash` compare the value, so both
//! families are exercised here.

use elm_effect::{Duration, DurationJson, Equal, Hash, Order, is_duration, pipe};

#[test]
fn duration_order_compares_by_value() {
    // :158 - `Duration.Order` is the value order across representations.
    let order = Duration::order();
    assert_eq!(
        order.compare(&Duration::seconds(1.0), &Duration::millis(1000.0)),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        order.compare(&Duration::millis(1.0), &Duration::seconds(1.0)),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        order.compare(&Duration::infinity(), &Duration::seconds(1.0)),
        std::cmp::Ordering::Greater
    );
}

#[test]
fn duration_equivalence_matches_order() {
    // :179 - equivalence is order equivalence, not representation equality.
    let equivalence = Duration::equivalence();
    assert!(equivalence.is_equivalent(&Duration::seconds(1.0), &Duration::millis(1000.0)));
    assert!(!equivalence.is_equivalent(&Duration::millis(1.0), &Duration::nanos(1_000_001)));
    assert_ne!(Duration::seconds(1.0), Duration::nanos(1_000_000_000));
}

#[test]
fn duration_max_min_and_clamp() {
    // :200, :205, :210 - inclusive bounds and first-wins ties.
    let low = Duration::seconds(1.0);
    let high = Duration::seconds(2.0);
    assert_eq!(Duration::max(low, high), high);
    assert_eq!(Duration::max(high, low), high);
    assert_eq!(Duration::min(low, high), low);
    assert_eq!(Duration::clamp(Duration::zero(), low, high), low);
    assert_eq!(Duration::clamp(Duration::seconds(3.0), low, high), high);
    assert_eq!(Duration::clamp(Duration::seconds(15.0), low, high), high);
    assert_eq!(
        Duration::clamp(Duration::millis(1500.0), low, high),
        Duration::millis(1500.0)
    );
}

#[test]
fn duration_equals_is_value_equality() {
    // :227 - equals ignores the representation.
    assert!(Duration::seconds(1.0).equals(Duration::millis(1000.0)));
    assert!(Duration::nanos(1_500_000).equals(Duration::millis(1.5)));
    assert!(!Duration::seconds(1.0).equals(Duration::seconds(2.0)));
}

#[test]
fn duration_hash_agrees_with_equals_across_representations() {
    // :231 - equal values hash alike even when built differently.
    assert_eq!(
        Duration::seconds(1.0).hash_value(),
        Duration::nanos(1_000_000_000).hash_value()
    );
    assert_eq!(
        Duration::millis(1.0).hash_value(),
        Duration::nanos(1_000_000).hash_value()
    );
}

#[test]
fn duration_hash_handles_finite_millis_too_large_for_nanos() {
    // :245 - a finite millis value that overflows the exact-nanos path still
    // hashes deterministically.
    let huge = Duration::millis(1e300);
    assert!(huge.is_finite());
    assert_eq!(huge.to_nanos(), None);
    assert_eq!(huge.hash_value(), huge.hash_value());
    assert_ne!(huge.hash_value(), Duration::millis(1e299).hash_value());
}

#[test]
fn duration_between_is_inclusive() {
    // :252 - both bounds count as inside.
    let low = Duration::seconds(1.0);
    let high = Duration::seconds(2.0);
    assert!(Duration::seconds(1.0).between(low, high));
    assert!(Duration::seconds(2.0).between(low, high));
    assert!(Duration::millis(1500.0).between(low, high));
    assert!(!Duration::millis(999.0).between(low, high));
    assert!(!Duration::seconds(3.0).between(low, high));
}

#[test]
fn duration_divide() {
    // :265 - millis divide by a fraction; a nanos value needs a whole
    // divisor, and a zero, NaN, or infinite divisor yields None.
    assert_eq!(
        Duration::seconds(1.0).divide(2.0),
        Some(Duration::millis(500.0))
    );
    assert_eq!(Duration::seconds(1.0).divide(0.0), None);
    assert_eq!(Duration::seconds(1.0).divide(f64::NAN), None);
    assert_eq!(Duration::seconds(1.0).divide(f64::INFINITY), None);
    assert_eq!(
        Duration::nanos(10).divide(0.5),
        None,
        "the pin's BigInt(by) throws for a fractional divisor"
    );
    assert_eq!(
        Duration::infinity().divide(-1.0),
        Some(Duration::negative_infinity())
    );
}

#[test]
fn duration_divide_unsafe_uses_ieee_fallbacks() {
    // :304 - the unsafe variant answers instead of returning None.
    assert_eq!(
        Duration::seconds(1.0).divide_unsafe(2.0),
        Duration::millis(500.0)
    );
    assert_eq!(
        Duration::seconds(1.0).divide_unsafe(0.0),
        Duration::infinity()
    );
    assert_eq!(
        Duration::seconds(-1.0).divide_unsafe(0.0),
        Duration::negative_infinity()
    );
    assert_eq!(Duration::zero().divide_unsafe(0.0), Duration::zero());
    assert_eq!(
        Duration::seconds(1.0).divide_unsafe(f64::NAN),
        Duration::zero()
    );
}

#[test]
fn duration_times_scales_both_representations() {
    // :345 - millis scale in millis space, nanos by an integer multiplier.
    assert_eq!(Duration::seconds(2.0).times(3.0), Duration::seconds(6.0));
    assert_eq!(Duration::nanos(5).times(2.0), Duration::nanos(10));
    assert_eq!(Duration::seconds(2.0).times(0.0), Duration::zero());
    assert_eq!(
        Duration::infinity().times(-1.0),
        Duration::negative_infinity()
    );
}

#[test]
fn duration_sum_and_subtract() {
    // :351 - same-sign infinities keep the sign; opposites cancel to zero.
    assert_eq!(
        Duration::seconds(1.0).sum(Duration::millis(500.0)),
        Duration::millis(1500.0)
    );
    assert_eq!(
        Duration::infinity().sum(Duration::negative_infinity()),
        Duration::zero()
    );
    assert_eq!(
        Duration::infinity().sum(Duration::seconds(1.0)),
        Duration::infinity()
    );
    // :366 - subtract is sum with the negated operand.
    assert_eq!(
        Duration::seconds(2.0).subtract(Duration::millis(500.0)),
        Duration::millis(1500.0)
    );
    assert_eq!(
        Duration::seconds(1.0).subtract(Duration::infinity()),
        Duration::negative_infinity()
    );
}

#[test]
fn duration_relational_predicates() {
    // :392 through :430 - strict and inclusive comparisons.
    let one = Duration::seconds(1.0);
    let two = Duration::seconds(2.0);
    assert!(two.is_greater_than(one));
    assert!(two.is_greater_than_or_equal(one));
    assert!(one.is_greater_than_or_equal(Duration::millis(1000.0)));
    assert!(one.is_less_than(two));
    assert!(one.is_less_than_or_equal(two));
    assert!(one.is_less_than_or_equal(Duration::millis(1000.0)));
    assert!(!one.is_greater_than(one));
    assert!(!one.is_less_than(one));
}

#[test]
fn duration_to_string_renders_the_pin_spelling() {
    // :440 - toString names the stored representation.
    assert_eq!(Duration::millis(1500.0).to_string(), "1500 millis");
    assert_eq!(Duration::nanos(5).to_string(), "5 nanos");
    assert_eq!(Duration::millis(1.5).to_string(), "1500000 nanos");
    assert_eq!(Duration::infinity().to_string(), "Infinity");
    assert_eq!(Duration::negative_infinity().to_string(), "-Infinity");
}

#[test]
fn duration_format_renders_normalized_parts() {
    // :449 - format is the human reading of the normalized parts.
    assert_eq!(Duration::zero().format(), "0");
    assert_eq!(
        Duration::hours(1.0).sum(Duration::minutes(30.0)).format(),
        "1h 30m"
    );
    assert_eq!(Duration::seconds(-1.5).format(), "-1s 500ms");
    assert_eq!(Duration::infinity().format(), "Infinity");
}

#[test]
fn duration_parts_are_signed_and_normalized() {
    // :461 - every field carries the sign and stays inside its unit.
    let one_day = Duration::nanos(90_061_001_000_001);
    let parts = one_day.parts();
    assert_eq!(parts.days, 1.0);
    assert_eq!(parts.hours, 1.0);
    assert_eq!(parts.minutes, 1.0);
    assert_eq!(parts.seconds, 1.0);
    assert_eq!(parts.millis, 1.0);
    assert_eq!(parts.nanos, 1.0);

    let negative = Duration::seconds(-1.5).parts();
    assert_eq!(negative.seconds, -1.0);
    assert_eq!(negative.millis, -500.0);
}

#[test]
fn duration_json_and_inspect() {
    // :490, :497 - toJSON tags the representation; nanos render as a string.
    assert_eq!(
        Duration::seconds(1.0).to_json(),
        DurationJson::Millis(1000.0)
    );
    assert_eq!(
        Duration::nanos(5).to_json(),
        DurationJson::Nanos("5".to_string())
    );
    assert_eq!(
        Duration::millis(1.5).to_json(),
        DurationJson::Nanos("1500000".to_string())
    );
    assert_eq!(Duration::infinity().to_json(), DurationJson::Infinity);
    assert_eq!(
        Duration::negative_infinity().to_json(),
        DurationJson::NegativeInfinity
    );
    assert_eq!(
        Duration::seconds(2.0).inspect(),
        Duration::seconds(2.0).to_json()
    );
}

#[test]
fn duration_pipes_through_the_chain_macro() {
    // :505 - a duration is a value like any other in a pipe chain.
    let total = pipe!(Duration::seconds(1.0), |d: Duration| d
        .sum(Duration::millis(500.0)));
    assert_eq!(total, Duration::millis(1500.0));
    let doubled = pipe!(Duration::seconds(1.0), |d: Duration| d.times(2.0));
    assert_eq!(doubled, Duration::seconds(2.0));
}

#[test]
fn duration_is_duration_recognizes_the_value() {
    // :509 - the runtime guard answers for the duration type only.
    assert!(is_duration(&Duration::zero()));
    assert!(is_duration(&Duration::infinity()));
    assert!(!is_duration(&1u32));
    assert!(!is_duration(&"1 millis"));
}

#[test]
fn duration_zero_and_weeks() {
    // :514, :518 - zero is the millis zero; weeks scale by seven days.
    assert_eq!(Duration::zero(), Duration::millis(0.0));
    assert!(Duration::zero().is_zero());
    assert_eq!(Duration::weeks(2.0).to_days(), 14.0);
    assert_eq!(Duration::weeks(1.0).to_millis(), 604_800_000.0);
}

#[test]
fn duration_millis_seconds_nanos_and_hr_time() {
    // :523, :529, :535, :552 - exact readings from the stored representation.
    assert_eq!(Duration::nanos(1).to_millis(), 0.000001);
    assert_eq!(Duration::nanos(1_000_000_000).to_seconds(), 1.0);
    assert_eq!(Duration::nanos(1).to_seconds(), 1e-9);
    assert_eq!(Duration::nanos(42).to_nanos(), Some(42));
    assert_eq!(Duration::millis(1.0).to_nanos(), Some(1_000_000));
    assert_eq!(Duration::infinity().to_nanos(), None);
    assert_eq!(Duration::millis(1500.0).to_hr_time(), (1.0, 500_000_000.0));
    assert_eq!(
        Duration::millis(-1500.0).to_hr_time(),
        (-1.0, -500_000_000.0)
    );
}

#[test]
fn duration_to_nanos_unsafe_panics_on_infinities() {
    // :543 - the unsafe accessor throws where toNanos answers None.
    assert_eq!(Duration::seconds(1.0).to_nanos_unsafe(), 1_000_000_000);
    let panicked = std::panic::catch_unwind(|| Duration::infinity().to_nanos_unsafe());
    assert!(panicked.is_err());
}

#[test]
fn duration_negative_values_keep_their_sign() {
    // :560 - negative durations render, decompose, and compare sign-first.
    let negative = Duration::seconds(-2.0);
    assert!(negative.is_negative());
    assert_eq!(negative.to_millis(), -2000.0);
    assert_eq!(negative.abs(), Duration::seconds(2.0));
    assert_eq!(negative.negate(), Duration::seconds(2.0));
    assert_eq!(negative.parts().seconds, -2.0);
    assert!(negative.is_less_than(Duration::zero()));
}

#[test]
fn duration_match_dispatches_on_the_representation() {
    // :649 - one arm per representation, including both infinities.
    let label = |d: Duration| {
        d.match_duration(
            |millis| format!("millis {millis}"),
            |nanos| format!("nanos {nanos}"),
            || "infinity".to_string(),
            || "negative infinity".to_string(),
        )
    };
    assert_eq!(label(Duration::millis(5.0)), "millis 5");
    assert_eq!(label(Duration::nanos(5)), "nanos 5");
    assert_eq!(label(Duration::infinity()), "infinity");
    assert_eq!(label(Duration::negative_infinity()), "negative infinity");
}

#[test]
fn duration_is_finite_and_zero() {
    // :660, :666 - the sentinels are neither finite nor zero.
    assert!(Duration::millis(1.0).is_finite());
    assert!(Duration::nanos(1).is_finite());
    assert!(!Duration::infinity().is_finite());
    assert!(!Duration::negative_infinity().is_finite());
    assert!(Duration::zero().is_zero());
    assert!(!Duration::millis(1.0).is_zero());
    assert!(!Duration::infinity().is_zero());
}

#[test]
fn duration_minutes_hours_days_and_weeks() {
    // :675 through :693 - every accessor divides by its own unit.
    assert_eq!(Duration::seconds(90.0).to_minutes(), 1.5);
    assert_eq!(Duration::seconds(5400.0).to_hours(), 1.5);
    assert_eq!(Duration::days(2.0).to_hours(), 48.0);
    assert_eq!(Duration::days(2.0).to_minutes(), 2880.0);
    assert_eq!(Duration::days(2.0).to_days(), 2.0);
    assert_eq!(Duration::days(7.0).to_weeks(), 1.0);
}

#[test]
fn duration_reducer_sum_and_combiner_extremes() {
    // :699, :703, :707 - the fold starts at zero and the combiners pick an
    // extreme under the duration order.
    let reducer = Duration::reducer_sum();
    assert_eq!(
        reducer.fold(&[
            Duration::seconds(1.0),
            Duration::millis(500.0),
            Duration::zero()
        ]),
        Duration::millis(1500.0)
    );
    assert_eq!(reducer.empty(), Duration::zero());

    let max = Duration::combiner_max();
    assert_eq!(
        max.combine(&Duration::seconds(1.0), &Duration::seconds(2.0)),
        Duration::seconds(2.0)
    );
    let min = Duration::combiner_min();
    assert_eq!(
        min.combine(&Duration::seconds(1.0), &Duration::millis(500.0)),
        Duration::millis(500.0)
    );
}

#[test]
fn duration_equality_through_the_equal_trait() {
    // The trait spelling of `equals`, used by containers.
    assert!(Duration::seconds(1.0).equal(&Duration::millis(1000.0)));
    assert!(!Duration::seconds(1.0).equal(&Duration::seconds(2.0)));
    assert_eq!(
        Order::make(|a: &Duration, b: &Duration| a.compare(b))
            .max(Duration::seconds(1.0), Duration::seconds(2.0)),
        Duration::seconds(2.0)
    );
}
