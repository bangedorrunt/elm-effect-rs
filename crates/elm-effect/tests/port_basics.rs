//! Core basics fence: predicate combinators, duration helpers, seeded RNG.
//! The module loads via `#[path]` (the .15 precedent): no facade touch.

#[path = "../src/basics.rs"]
mod basics;

use basics::{
    Predicate, SeedRandom, all_of, always, any_of, between_ms, clamp_ms, compare_ms, divide_ms,
    equals_ms, every, from_input, from_input_unsafe, max_ms, min_ms, never, some,
};
use std::cmp::Ordering;

#[test]
fn predicates_compose() {
    let positive = Predicate::new(|v: &i32| *v > 0);
    let even = Predicate::new(|v: &i32| *v % 2 == 0);
    assert!(positive.and(&even).test(&4));
    assert!(!positive.and(&even).test(&3));
    assert!(positive.or(&even).test(&3));
    assert!(!positive.or(&even).test(&-3));
    assert!(positive.not().test(&-1));
    assert!(positive.xor(&even).test(&3));
    assert!(!positive.xor(&even).test(&4));
    assert!(positive.implies(&even).test(&-5));
    assert!(!positive.implies(&even).test(&3));
    assert!(positive.nor(&even).test(&-3));
    assert!(!positive.nor(&even).test(&4));
    assert!(positive.nand(&even).test(&3));
    assert!(!positive.nand(&even).test(&4));
    assert!(always::<i32>().test(&0));
    assert!(!never::<i32>().test(&0));
}

#[test]
fn predicates_quantify() {
    let positive = Predicate::new(|v: &i32| *v > 0);
    assert!(every(&positive, &[1, 2, 3]));
    assert!(every(&positive, &[]));
    assert!(!every(&positive, &[1, -2]));
    assert!(some(&positive, &[-1, 2]));
    assert!(!some(&positive, &[-1, -2]));
    assert!(!some(&positive, &[]));
}

#[test]
fn duration_helpers() {
    assert_eq!(compare_ms(1, 2), Ordering::Less);
    assert_eq!(compare_ms(2, 2), Ordering::Equal);
    assert!(equals_ms(5, 5));
    assert_eq!(min_ms(3, 7), 3);
    assert_eq!(max_ms(3, 7), 7);
    assert_eq!(clamp_ms(99, 0, 10), 10);
    assert!(between_ms(5, 0, 10));
    assert!(!between_ms(11, 0, 10));
    assert_eq!(divide_ms(7, 2), 3);
    assert_eq!(from_input("500ms"), Some(500));
    assert_eq!(from_input("30s"), Some(30_000));
    assert_eq!(from_input("2m"), Some(120_000));
    assert_eq!(from_input("1h"), Some(3_600_000));
    assert_eq!(from_input("1500"), Some(1500));
    assert_eq!(from_input("nope"), None);
    assert_eq!(from_input_unsafe("1s"), 1000);
}

#[test]
fn random_is_seeded_and_shaped() {
    let (a, b) = (SeedRandom::new(42), SeedRandom::new(42));
    let mut x = a;
    let mut y = b;
    let seq_x: Vec<f64> = (0..10).map(|_| x.next_f64()).collect();
    let seq_y: Vec<f64> = (0..10).map(|_| y.next_f64()).collect();
    assert_eq!(seq_x, seq_y);
    assert!(seq_x.iter().all(|v| (0.0..1.0).contains(v)));
    let mut rng = SeedRandom::new(7);
    let bools: Vec<bool> = (0..100).map(|_| rng.next_bool()).collect();
    assert!(bools.contains(&true) && bools.contains(&false));
    let mut rng = SeedRandom::new(9);
    assert!(
        (0..100)
            .map(|_| rng.range(-5, 5))
            .all(|v| (-5..5).contains(&v))
    );
    assert_eq!(SeedRandom::new(1).range(3, 3), 3);
    let mut rng = SeedRandom::new(11);
    assert!(
        (0..50)
            .map(|_| rng.safe_integer())
            .all(|v| (0..9_007_199_254_740_992).contains(&v))
    );
}

#[test]
fn hash_grids_do_not_collide() {
    use elm_effect::{Hash, combine_hash, hash_str};
    use std::collections::{HashMap, HashSet};
    // Arrays: chained combines over index pairs.
    let arrays: std::collections::HashSet<u32> = (0..100u32)
        .flat_map(|i| (0..100u32).map(move |j| combine_hash(combine_hash(0, i), j)))
        .collect();
    assert_eq!(arrays.len(), 10_000);
    // Records: field hashes combined in order.
    let records: std::collections::HashSet<u32> = (0..100u32)
        .flat_map(|i| {
            (0..100u32)
                .map(move |j| combine_hash(hash_str(&format!("k{i}")), hash_str(&format!("v{j}"))))
        })
        .collect();
    assert_eq!(records.len(), 10_000);
    // Combined hashes: strings folded through the string hash.
    let combined: std::collections::HashSet<u32> = (0..100u32)
        .flat_map(|i| (0..100u32).map(move |j| hash_str(&format!("{i},{j}"))))
        .collect();
    assert_eq!(combined.len(), 10_000);
    // HashMap entries: key hashes stay distinct.
    let mut map = HashMap::new();
    for i in 0..100u32 {
        for j in 0..100u32 {
            map.insert((i, j), combine_hash(i, j));
        }
    }
    assert_eq!(map.len(), 10_000);
    assert_eq!(map[&(3, 4)], combine_hash(3, 4));
    // Native sets and hash sets of pairs below 150 stay collision-free.
    let native: HashSet<(u32, u32)> = (0..150u32)
        .flat_map(|i| (0..2u32).map(move |j| (i, j)))
        .collect();
    assert_eq!(native.len(), 300);
    struct Pair(u32, u32);
    impl Hash for Pair {
        fn hash_value(&self) -> u32 {
            combine_hash(self.0, self.1)
        }
    }
    let hashed: HashSet<u32> = (0..150u32)
        .flat_map(|i| (0..2u32).map(move |j| Pair(i, j).hash_value()))
        .collect();
    assert_eq!(hashed.len(), 300);
}

// ---------------------------------------------------------------------------
// Duration millis rows at the pin (Duration.test.ts describe max..between)
// ---------------------------------------------------------------------------

#[test]
fn duration_max_returns_the_larger() {
    // Duration.max(millis(1), millis(2)) === millis(2)
    // Duration.max(minutes(1), millis(2)) === minutes(1)
    assert_eq!(max_ms(1, 2), 2);
    assert_eq!(max_ms(60_000, 2), 60_000);
    // A match on both sides returns either bound.
    assert_eq!(max_ms(7, 7), 7);
}

#[test]
fn duration_min_returns_the_smaller() {
    // Duration.min(millis(1), millis(2)) === millis(1)
    // Duration.min(minutes(1), millis(2)) === millis(2)
    assert_eq!(min_ms(1, 2), 1);
    assert_eq!(min_ms(60_000, 2), 2);
    assert_eq!(min_ms(7, 7), 7);
}

#[test]
fn duration_clamp_bounds_both_ends() {
    // Below the minimum clamps up at it, above the maximum clamps down at it,
    // and a value inside the range is preserved exactly (minutes(1.5) stays).
    assert_eq!(clamp_ms(1, 2, 3), 2, "below the minimum clamps up");
    assert_eq!(clamp_ms(99, 2, 3), 3, "above the maximum clamps down");
    assert_eq!(clamp_ms(90_000, 60_000, 120_000), 90_000, "inside is preserved");
}

#[test]
fn duration_equals_across_units() {
    // Duration.equals(hours(1), minutes(60)): the comparison is on the
    // underlying millis, so differing units with the same span are equal.
    assert!(equals_ms(3_600_000, 3_600_000));
    assert!(equals_ms(3_600_000, 60 * 60_000));
    assert!(!equals_ms(1, 60_000));
}

#[test]
fn duration_between_is_inclusive_on_both_bounds() {
    // Duration.between(hours(1), { minimum: minutes(59), maximum: minutes(61) })
    assert!(between_ms(3_600_000, 59 * 60_000, 61 * 60_000));
    assert!(between_ms(60_000, 59_000, 61_000));
    // Just outside either bound is not between. 3_700_000 ms is past the
    // 61-minute maximum (3_660_000), so it is out.
    assert!(!between_ms(3_700_000, 59 * 60_000, 61 * 60_000));
    assert!(!between_ms(62_000, 60_000, 61_000));
}

// ---------------------------------------------------------------------------
// Predicate truth tables at the pin (Predicate.test.ts describe and/xor/
// eqv/implies/nor/nand, driven by constTrue / constFalse)
// ---------------------------------------------------------------------------

/// `and`: true only when both hold.
#[test]
fn predicate_and_truth_table() {
    let (yes, no) = (always::<u8>(), never::<u8>());
    assert!(yes.and(&yes).test(&0));
    assert!(!yes.and(&no).test(&0));
    assert!(!no.and(&yes).test(&0));
    assert!(!no.and(&no).test(&0));
}

/// `xor`: true only when exactly one holds.
#[test]
fn predicate_xor_truth_table() {
    let (yes, no) = (always::<u8>(), never::<u8>());
    assert!(!yes.xor(&yes).test(&0));
    assert!(yes.xor(&no).test(&0));
    assert!(no.xor(&yes).test(&0));
    assert!(!no.xor(&no).test(&0));
}

/// `eqv`: true only when the two agree.
#[test]
fn predicate_eqv_truth_table() {
    let (yes, no) = (always::<u8>(), never::<u8>());
    assert!(yes.eqv(&yes).test(&0));
    assert!(!yes.eqv(&no).test(&0));
    assert!(!no.eqv(&yes).test(&0));
    assert!(no.eqv(&no).test(&0));
}

/// `implies`: true unless the antecedent holds and the consequent does not.
#[test]
fn predicate_implies_truth_table() {
    let (yes, no) = (always::<u8>(), never::<u8>());
    assert!(yes.implies(&yes).test(&0));
    assert!(!yes.implies(&no).test(&0));
    assert!(no.implies(&yes).test(&0));
    assert!(no.implies(&no).test(&0));
}

/// `nor`: true only when both are false.
#[test]
fn predicate_nor_truth_table() {
    let (yes, no) = (always::<u8>(), never::<u8>());
    assert!(!yes.nor(&yes).test(&0));
    assert!(!yes.nor(&no).test(&0));
    assert!(!no.nor(&yes).test(&0));
    assert!(no.nor(&no).test(&0));
}

/// `nand`: false only when both are true.
#[test]
fn predicate_nand_truth_table() {
    let (yes, no) = (always::<u8>(), never::<u8>());
    assert!(!yes.nand(&yes).test(&0));
    assert!(yes.nand(&no).test(&0));
    assert!(no.nand(&yes).test(&0));
    assert!(no.nand(&no).test(&0));
}

// ---------------------------------------------------------------------------
// Predicate combining form at the pin (describe every / some)
// ---------------------------------------------------------------------------

/// `Predicate.every` combining form: true only when all listed predicates
/// pass on the same input.
#[test]
fn predicate_all_of_requires_every_predicate_to_pass() {
    let is_positive = Predicate::new(|n: &i32| *n > 0);
    let under_two = Predicate::new(|n: &i32| *n < 2);
    let combined = all_of(vec![is_positive, under_two]);

    assert!(combined.test(&1), "both hold");
    assert!(!combined.test(&0), "first holds, second fails");
    assert!(!combined.test(&-2), "both fail");
}

/// An empty list is vacuously true (the pin's `every([])` row).
#[test]
fn predicate_all_of_empty_list_is_true() {
    assert!(all_of::<i32>(vec![]).test(&0));
}

/// `Predicate.some` combining form: true when any listed predicate passes.
#[test]
fn predicate_any_of_holds_when_any_predicate_passes() {
    let is_positive = Predicate::new(|n: &i32| *n > 0);
    let is_negative = Predicate::new(|n: &i32| *n < 0);
    let combined = any_of(vec![is_positive, is_negative]);

    assert!(!combined.test(&0), "neither holds");
    assert!(combined.test(&-1), "second holds");
    assert!(combined.test(&1), "first holds");
}

/// An empty list is false (the pin's `some([])` row).
#[test]
fn predicate_any_of_empty_list_is_false() {
    assert!(!any_of::<i32>(vec![]).test(&0));
}

/// A single-element list is exactly that predicate.
#[test]
fn predicate_combining_forms_with_one_predicate_are_it() {
    let is_positive = Predicate::new(|n: &i32| *n > 0);
    assert!(all_of(vec![is_positive.clone()]).test(&3));
    assert!(!all_of(vec![is_positive.clone()]).test(&-3));
    assert!(any_of(vec![is_positive]).test(&3));
}

// ---------------------------------------------------------------------------
// Duration value semantics (Duration.test.ts :14-:707)
// ---------------------------------------------------------------------------

use elm_effect::predicate::Value;
use elm_effect::{Duration, DurationJson, Equal, Hash, Order, Parts};

#[test]
fn duration_from_input_parses_every_input_shape() {
    // :14/:116 — the fromInput table: a Duration round-trips, numbers are
    // millis, bigints are nanos, and the string, [seconds, nanos] and object
    // forms each parse. Ties round away from zero, and the huge-nanos case
    // keeps its exact value.
    let cases: Vec<(Value, Duration)> = vec![
        (Value::Number(100.0), Duration::millis(100.0)),
        (Value::BigInt(10), Duration::nanos(10)),
        (Value::String("1 nano".into()), Duration::nanos(1)),
        (Value::String("10 nanos".into()), Duration::nanos(10)),
        (Value::String("1.5 nanos".into()), Duration::nanos(2)),
        (Value::String("-1.5 nanos".into()), Duration::nanos(-2)),
        (
            Value::String("9007199254740993.1 nanos".into()),
            Duration::nanos(9_007_199_254_740_993),
        ),
        (
            Value::String("-9007199254740993.5 nanos".into()),
            Duration::nanos(-9_007_199_254_740_994),
        ),
        (Value::String("1 micro".into()), Duration::micros(1)),
        (Value::String("1.5 micros".into()), Duration::nanos(1500)),
        (Value::String("1 milli".into()), Duration::millis(1.0)),
        (Value::String("1 second".into()), Duration::seconds(1.0)),
        (Value::String("1 minute".into()), Duration::minutes(1.0)),
        (Value::String("1 hour".into()), Duration::hours(1.0)),
        (Value::String("1 day".into()), Duration::days(1.0)),
        (Value::String("1 week".into()), Duration::weeks(1.0)),
        (Value::String("1.5 seconds".into()), Duration::seconds(1.5)),
        (Value::String("-1.5 seconds".into()), Duration::seconds(-1.5)),
        (Value::String("Infinity".into()), Duration::infinity()),
        (Value::String("-Infinity".into()), Duration::negative_infinity()),
        (
            Value::Array(vec![Value::Number(500.0), Value::Number(123456789.0)]),
            Duration::nanos(500_123_456_789),
        ),
        (
            Value::Array(vec![Value::Number(-500.0), Value::Number(123456789.0)]),
            Duration::nanos(-500_000_000_000 + 123_456_789),
        ),
        (
            Value::Array(vec![Value::Number(0.0), Value::Number(1.5)]),
            Duration::nanos(2),
        ),
        (
            Value::Array(vec![Value::Number(0.0000000005), Value::Number(0.5)]),
            Duration::nanos(1),
        ),
        (Value::Array(vec![Value::Number(f64::NAN), Value::Number(0.0)]), Duration::zero()),
        (Value::Array(vec![Value::Number(0.0), Value::Number(f64::NAN)]), Duration::zero()),
    ];
    for (input, expected) in cases {
        assert_eq!(Duration::from_input(&input), Some(expected), "{input:?}");
        assert_eq!(Duration::from_input_unsafe(&input), expected, "{input:?}");
    }
    // The object form, and the invalid string only `from_input` refuses.
    let mut hours = std::collections::BTreeMap::new();
    hours.insert("hours".to_string(), Value::Number(2.0));
    assert_eq!(Duration::from_input(&Value::Object(hours)), Some(Duration::hours(2.0)));
    assert_eq!(Duration::from_input(&Value::Object(std::collections::BTreeMap::new())), Some(Duration::zero()));
    assert_eq!(Duration::from_input(&Value::String("invalid".into())), None);
}

#[test]
fn duration_order_spans_both_representations_and_the_sentinels() {
    // :158 — NegativeInfinity < finite < Infinity, and millis and nanos
    // compare by value rather than by representation.
    assert_eq!(Duration::millis(1.0).compare(&Duration::millis(2.0)), Ordering::Less);
    assert_eq!(Duration::millis(2.0).compare(&Duration::millis(1.0)), Ordering::Greater);
    assert_eq!(Duration::millis(2.0).compare(&Duration::millis(2.0)), Ordering::Equal);
    assert_eq!(Duration::millis(1.0).compare(&Duration::nanos(2_000_000)), Ordering::Less);
    assert_eq!(Duration::nanos(2_000_000).compare(&Duration::millis(1.0)), Ordering::Greater);
    assert_eq!(Duration::infinity().compare(&Duration::infinity()), Ordering::Equal);
    assert_eq!(Duration::infinity().compare(&Duration::millis(1.0)), Ordering::Greater);
    assert_eq!(Duration::millis(1.0).compare(&Duration::infinity()), Ordering::Less);
    assert_eq!(Duration::negative_infinity().compare(&Duration::millis(1.0)), Ordering::Less);
    let order = Duration::order();
    assert_eq!(order.compare(&Duration::nanos(1), &Duration::nanos(2)), Ordering::Less);
}

#[test]
fn duration_equals_and_hashes_across_representations() {
    // :179/:231/:245 — the equivalence sees through the representation, and
    // the hash agrees with it, including for a millis value far too large to
    // convert to nanos.
    assert!(Duration::millis(1.0).equal(&Duration::nanos(1_000_000)));
    assert!(Duration::nanos(1_000_000).equal(&Duration::millis(1.0)));
    assert!(!Duration::millis(1.0).equal(&Duration::millis(2.0)));
    assert!(Duration::infinity().equal(&Duration::infinity()));
    assert!(!Duration::infinity().equal(&Duration::millis(1.0)));
    assert_eq!(
        Duration::seconds(5.0).hash_value(),
        Duration::nanos(5_000_000_000).hash_value()
    );
    assert_eq!(
        Duration::millis(-5000.0).hash_value(),
        Duration::nanos(-5_000_000_000).hash_value()
    );
    assert_ne!(
        Duration::infinity().hash_value(),
        Duration::negative_infinity().hash_value()
    );
    let huge = Duration::millis(1e303);
    assert!(huge.equal(&Duration::millis(1e303)));
    assert_eq!(huge.hash_value(), Duration::millis(1e303).hash_value());
    let equivalence = Duration::equivalence();
    assert!(equivalence.is_equivalent(&Duration::seconds(5.0), &Duration::nanos(5_000_000_000)));
}

#[test]
fn duration_divide_and_times_follow_the_pins_arithmetic() {
    // :265/:304/:345 — divide is exact or None; divideUnsafe fills the IEEE
    // 754 fallbacks; times scales each representation and zeroes an
    // infinity.
    assert_eq!(Duration::divide(Duration::minutes(1.0), 2.0), Some(Duration::seconds(30.0)));
    assert_eq!(Duration::divide(Duration::seconds(1.0), 3.0), Some(Duration::nanos(333_333_333)));
    assert_eq!(Duration::divide(Duration::zero(), 2.0), Some(Duration::zero()));
    assert_eq!(Duration::divide(Duration::minutes(1.0), 1.5), Some(Duration::seconds(40.0)));
    assert_eq!(Duration::divide(Duration::nanos(1), 3.0), Some(Duration::zero()));
    assert_eq!(Duration::divide(Duration::nanos(1), 0.5), None);
    assert_eq!(Duration::divide(Duration::minutes(1.0), 0.0), None);
    assert_eq!(Duration::divide(Duration::minutes(1.0), f64::NAN), None);
    assert_eq!(Duration::divide(Duration::infinity(), 2.0), Some(Duration::infinity()));
    assert_eq!(Duration::divide(Duration::infinity(), -2.0), Some(Duration::negative_infinity()));

    assert_eq!(Duration::divide_unsafe(Duration::minutes(1.0), 2.0), Duration::seconds(30.0));
    assert_eq!(Duration::divide_unsafe(Duration::nanos(1), 0.5), Duration::zero());
    assert_eq!(Duration::divide_unsafe(Duration::minutes(1.0), 0.0), Duration::infinity());
    assert_eq!(Duration::divide_unsafe(Duration::minutes(1.0), -0.0), Duration::negative_infinity());
    assert_eq!(Duration::divide_unsafe(Duration::nanos(-1), 0.0), Duration::negative_infinity());
    assert_eq!(Duration::divide_unsafe(Duration::zero(), 0.0), Duration::zero());
    assert_eq!(Duration::divide_unsafe(Duration::minutes(1.0), f64::NAN), Duration::zero());
    assert_eq!(Duration::divide_unsafe(Duration::infinity(), f64::INFINITY), Duration::zero());

    assert_eq!(Duration::times(Duration::seconds(1.0), 60.0), Duration::minutes(1.0));
    assert_eq!(Duration::times(Duration::nanos(2), 10.0), Duration::nanos(20));
    assert_eq!(Duration::times(Duration::infinity(), 60.0), Duration::infinity());
    assert_eq!(Duration::times(Duration::infinity(), 0.0), Duration::zero());
}

#[test]
fn duration_sum_subtract_and_comparisons() {
    // :351/:366/:392-:430 — sum keeps the wider representation and the
    // signed infinities cancel; the comparison family agrees with order.
    assert_eq!(Duration::seconds(30.0).sum(Duration::seconds(30.0)), Duration::minutes(1.0));
    assert_eq!(Duration::nanos(30).sum(Duration::nanos(30)), Duration::nanos(60));
    assert_eq!(Duration::infinity().sum(Duration::seconds(30.0)), Duration::infinity());
    assert_eq!(Duration::seconds(30.0).sum(Duration::infinity()), Duration::infinity());
    assert_eq!(Duration::infinity().sum(Duration::negative_infinity()), Duration::zero());
    assert_eq!(Duration::seconds(5.0).subtract(Duration::seconds(2.0)), Duration::seconds(3.0));

    assert!(Duration::seconds(2.0).compare(&Duration::seconds(1.0)).is_gt());
    assert!(Duration::seconds(2.0).compare(&Duration::seconds(2.0)).is_ge());
    assert!(Duration::seconds(1.0).compare(&Duration::seconds(2.0)).is_lt());
    assert!(Duration::seconds(2.0).compare(&Duration::seconds(2.0)).is_le());
}

#[test]
fn duration_renders_string_format_parts_and_json() {
    // :440/:449/:461/:490/:497 — the three renderings agree with the pin:
    // toString names the stored representation (a fractional millis renders
    // in nanos), format is the compact signed form, parts is the signed
    // decomposition, and toJSON/inspect are the tagged shapes.
    assert_eq!(Duration::infinity().to_string(), "Infinity");
    assert_eq!(Duration::nanos(10).to_string(), "10 nanos");
    assert_eq!(Duration::millis(2.0).to_string(), "2 millis");
    assert_eq!(Duration::millis(2.125).to_string(), "2125000 nanos");
    assert_eq!(Duration::seconds(2.0).to_string(), "2000 millis");
    assert_eq!(Duration::seconds(2.5).to_string(), "2500 millis");

    assert_eq!(Duration::format(Duration::infinity()), "Infinity");
    assert_eq!(Duration::format(Duration::minutes(5.0)), "5m");
    assert_eq!(Duration::format(Duration::minutes(5.325)), "5m 19s 500ms");
    assert_eq!(Duration::format(Duration::hours(3.0)), "3h");
    assert_eq!(Duration::format(Duration::hours(3.11125)), "3h 6m 40s 500ms");
    assert_eq!(Duration::format(Duration::days(2.0)), "2d");
    assert_eq!(Duration::format(Duration::days(2.25)), "2d 6h");
    assert_eq!(Duration::format(Duration::weeks(1.0)), "7d");
    assert_eq!(Duration::format(Duration::zero()), "0");
    assert_eq!(Duration::format(Duration::seconds(-5.0)), "-5s");
    assert_eq!(Duration::format(Duration::negative_infinity()), "-Infinity");

    let infinite = Duration::infinity().parts();
    assert_eq!(
        infinite,
        Parts {
            days: f64::INFINITY,
            hours: f64::INFINITY,
            minutes: f64::INFINITY,
            seconds: f64::INFINITY,
            millis: f64::INFINITY,
            nanos: f64::INFINITY
        }
    );
    assert_eq!(
        Duration::minutes(5.325).parts(),
        Parts { days: 0.0, hours: 0.0, minutes: 5.0, seconds: 19.0, millis: 500.0, nanos: 0.0 }
    );
    assert_eq!(
        Duration::minutes(3.11125).parts(),
        Parts { days: 0.0, hours: 0.0, minutes: 3.0, seconds: 6.0, millis: 675.0, nanos: 0.0 }
    );

    assert_eq!(Duration::seconds(2.0).to_json(), DurationJson::Millis(2000.0));
    assert_eq!(Duration::nanos(5).to_json(), DurationJson::Nanos("5".to_string()));
    assert_eq!(Duration::millis(1.5).to_json(), DurationJson::Nanos("1500000".to_string()));
    assert_eq!(Duration::infinity().to_json(), DurationJson::Infinity);
    assert_eq!(Duration::millis(1000.0).inspect(), DurationJson::Millis(1000.0));
}

#[test]
fn duration_pipes_its_combinators() {
    // :505 — the pipe form is the method chain in Rust.
    assert_eq!(
        Duration::seconds(1.0).sum(Duration::seconds(1.0)),
        Duration::seconds(2.0)
    );
}

#[test]
fn duration_identifies_itself() {
    // :509 — the type guard answers for a Duration and false for anything
    // else.
    assert!(elm_effect::is_duration(&Duration::millis(100.0) as &dyn std::any::Any));
    assert!(!elm_effect::is_duration(&7i32 as &dyn std::any::Any));
}

#[test]
fn duration_zero_weeks_and_units() {
    // :514/:518/:523-:549/:675-:693 — zero is the sum identity, weeks are
    // days by another name, and every unit conversion lands where the pin
    // puts it, with the infinite sentinels mapping to infinities.
    assert_eq!(Duration::seconds(1.0).sum(Duration::zero()), Duration::seconds(1.0));
    assert!(Duration::weeks(1.0).equal(&Duration::days(7.0)));
    assert!(!Duration::weeks(1.0).equal(&Duration::days(1.0)));

    assert_eq!(Duration::millis(1.0).to_millis(), 1.0);
    assert_eq!(Duration::nanos(1).to_millis(), 0.000001);
    assert_eq!(Duration::infinity().to_millis(), f64::INFINITY);
    assert_eq!(Duration::millis(1.0).to_seconds(), 0.001);
    assert_eq!(Duration::nanos(1).to_seconds(), 1e-9);
    assert_eq!(Duration::infinity().to_seconds(), f64::INFINITY);

    assert_eq!(Duration::nanos(1).to_nanos(), Some(1));
    assert_eq!(Duration::infinity().to_nanos(), None);
    assert_eq!(Duration::negative_infinity().to_nanos(), None);
    assert_eq!(Duration::millis(1.0005).to_nanos(), Some(1_000_500));
    assert_eq!(Duration::millis(100.0).to_nanos(), Some(100_000_000));

    assert_eq!(Duration::nanos(1).to_nanos_unsafe(), 1);
    assert_eq!(Duration::millis(1.0005).to_nanos_unsafe(), 1_000_500);
    assert_eq!(Duration::millis(0.0000015).to_nanos_unsafe(), 2);
    assert_eq!(Duration::millis(-0.0000015).to_nanos_unsafe(), -2);
    assert_eq!(Duration::millis(100.0).to_nanos_unsafe(), 100_000_000);
    let panics = std::panic::catch_unwind(|| Duration::infinity().to_nanos_unsafe());
    assert!(panics.is_err(), "an infinite duration has no nanos");

    assert_eq!(Duration::millis(1.0).to_hr_time(), (0.0, 1_000_000.0));
    assert_eq!(Duration::nanos(1).to_hr_time(), (0.0, 1.0));
    assert_eq!(Duration::nanos(1_000_000_001).to_hr_time(), (1.0, 1.0));
    assert_eq!(Duration::millis(1001.0).to_hr_time(), (1.0, 1_000_000.0));
    assert_eq!(Duration::infinity().to_hr_time(), (f64::INFINITY, 0.0));
    assert_eq!(Duration::negative_infinity().to_hr_time(), (f64::NEG_INFINITY, 0.0));

    assert_eq!(Duration::millis(60000.0).to_minutes(), 1.0);
    assert_eq!(Duration::nanos(60_000_000_000).to_minutes(), 1.0);
    assert_eq!(Duration::infinity().to_minutes(), f64::INFINITY);
    assert_eq!(Duration::millis(3_600_000.0).to_hours(), 1.0);
    assert_eq!(Duration::nanos(3_600_000_000_000).to_hours(), 1.0);
    assert_eq!(Duration::millis(86_400_000.0).to_days(), 1.0);
    assert_eq!(Duration::nanos(86_400_000_000_000).to_days(), 1.0);
    assert_eq!(Duration::millis(604_800_000.0).to_weeks(), 1.0);
    assert_eq!(Duration::nanos(604_800_000_000_000).to_weeks(), 1.0);
}

#[test]
fn duration_negative_family_agrees_with_the_pin() {
    // :560 — isNegative/isPositive, abs, negate and the sentinel behavior.
    assert!(Duration::millis(-1.0).is_negative());
    assert!(Duration::nanos(-1).is_negative());
    assert!(Duration::seconds(-5.0).is_negative());
    assert!(!Duration::zero().is_negative());
    assert!(!Duration::seconds(5.0).is_negative());
    assert!(!Duration::infinity().is_negative());
    assert!(Duration::negative_infinity().is_negative());
    assert!(Duration::seconds(5.0).is_positive());
    assert!(!Duration::zero().is_positive());
    assert!(!Duration::seconds(-5.0).is_positive());
    assert!(Duration::infinity().is_positive());
    assert!(!Duration::negative_infinity().is_positive());

    assert_eq!(Duration::abs(Duration::seconds(-5.0)), Duration::seconds(5.0));
    assert_eq!(Duration::abs(Duration::seconds(5.0)), Duration::seconds(5.0));
    assert_eq!(Duration::abs(Duration::zero()), Duration::zero());
    assert_eq!(Duration::abs(Duration::negative_infinity()), Duration::infinity());
    assert_eq!(Duration::abs(Duration::infinity()), Duration::infinity());

    assert_eq!(Duration::negate(Duration::seconds(5.0)), Duration::seconds(-5.0));
    assert_eq!(Duration::negate(Duration::seconds(-5.0)), Duration::seconds(5.0));
    assert_eq!(Duration::negate(Duration::zero()), Duration::zero());
    assert_eq!(Duration::negate(Duration::infinity()), Duration::negative_infinity());
    assert_eq!(Duration::negate(Duration::negative_infinity()), Duration::infinity());

    assert!(!Duration::negative_infinity().is_finite());
    assert!(!Duration::zero().is_negative());
    assert_eq!(Duration::negative_infinity().to_millis(), f64::NEG_INFINITY);
    assert_eq!(Duration::negative_infinity().to_seconds(), f64::NEG_INFINITY);
    assert_eq!(
        Duration::negative_infinity().compare(&Duration::negative_infinity()),
        Ordering::Equal
    );
    assert_eq!(
        Duration::negative_infinity().compare(&Duration::seconds(-5.0)),
        Ordering::Less
    );
    assert_eq!(
        Duration::seconds(-5.0).compare(&Duration::negative_infinity()),
        Ordering::Greater
    );
}

#[test]
fn duration_match_finite_and_sentinels() {
    // :649 — one arm per representation; the sentinels take the infinity arm.
    let rendered = Duration::millis(100.0).match_duration(
        |millis| format!("millis: {millis}"),
        |nanos| format!("nanos: {nanos}"),
        || "infinity".to_string(),
        || "infinity".to_string(),
    );
    assert_eq!(rendered, "millis: 100");
    let rendered = Duration::nanos(10).match_duration(
        |millis| format!("millis: {millis}"),
        |nanos| format!("nanos: {nanos}"),
        || "infinity".to_string(),
        || "infinity".to_string(),
    );
    assert_eq!(rendered, "nanos: 10");
    let rendered = Duration::infinity().match_duration(
        |millis| format!("millis: {millis}"),
        |nanos| format!("nanos: {nanos}"),
        || "infinity".to_string(),
        || "infinity".to_string(),
    );
    assert_eq!(rendered, "infinity");
}

#[test]
fn duration_finite_zero_and_reducers() {
    // :660/:666/:699-:707 — isFinite, isZero, and the three combiners.
    assert!(Duration::millis(100.0).is_finite());
    assert!(Duration::nanos(100).is_finite());
    assert!(!Duration::infinity().is_finite());
    assert!(!Duration::negative_infinity().is_finite());

    assert!(Duration::zero().is_zero());
    assert!(Duration::millis(0.0).is_zero());
    assert!(Duration::nanos(0).is_zero());
    assert!(!Duration::infinity().is_zero());
    assert!(!Duration::millis(1.0).is_zero());
    assert!(!Duration::nanos(1).is_zero());

    let sum = elm_effect::Reducer::make(Duration::zero(), |a: &Duration, b: &Duration| a.sum(*b));
    assert_eq!(sum.combine(&Duration::millis(1.0), &Duration::millis(2.0)), Duration::millis(3.0));
    let max = elm_effect::Combiner::make(|a: &Duration, b: &Duration| (*a).max(*b));
    assert_eq!(max.combine(&Duration::millis(1.0), &Duration::millis(2.0)), Duration::millis(2.0));
    let min = elm_effect::Combiner::make(|a: &Duration, b: &Duration| (*a).min(*b));
    assert_eq!(min.combine(&Duration::millis(1.0), &Duration::millis(2.0)), Duration::millis(1.0));
}
