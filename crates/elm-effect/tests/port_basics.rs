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
