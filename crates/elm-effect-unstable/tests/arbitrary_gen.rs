//! Arbitrary fence: deterministic seeded samples, bounded integers with
//! edges, shrink-to-boundary, and composition.

use elm_effect_unstable::arbitrary::{
    Rng, array, ascii_string, boolean, constant, hex_string, integer, number, option, record, tuple,
};

#[test]
fn samples_are_deterministic_per_seed() {
    let g = integer(-100, 100);
    assert_eq!(g.sample_n(42, 20), integer(-100, 100).sample_n(42, 20));
    assert_ne!(g.sample_n(42, 20), integer(-100, 100).sample_n(43, 20));
}

#[test]
fn bounded_integers_cover_domain_and_edges() {
    let g = integer(0, 10);
    let mut rng = Rng::new(7);
    let samples: Vec<i64> = (0..500).map(|_| g.sample(&mut rng)).collect();
    assert!(samples.iter().all(|v| (0..=10).contains(v)));
    assert!(samples.contains(&0) && samples.contains(&10));
    assert!(
        integer(i64::MIN, i64::MAX).sample(&mut Rng::new(1)) != 0
            || integer(i64::MIN, i64::MAX).sample(&mut Rng::new(2)) != 0
    );
}

#[test]
fn integer_bounds_reject_inverted_ranges() {
    let result = std::panic::catch_unwind(|| integer(5, 4));
    assert!(result.is_err());
}

#[test]
fn integers_shrink_toward_the_boundary() {
    let g = integer(0, 100);
    let shrunk = g.shrink(&90);
    assert!(shrunk.contains(&0));
    assert!(shrunk.iter().all(|v| (0..=100).contains(v)));
    assert!(shrunk.iter().all(|v| *v < 90));
    assert_eq!(g.shrink(&0), Vec::<i64>::new());
}

#[test]
fn primitives_and_constants() {
    let bools: Vec<bool> = boolean().sample_n(99, 200);
    assert!(bools.contains(&true) && bools.contains(&false));
    assert_eq!(constant(7).sample(&mut Rng::new(0)), 7);
    let s = ascii_string(8).sample(&mut Rng::new(3));
    assert_eq!(s.len(), 8);
    assert!(s.bytes().all(|b| b.is_ascii_lowercase()));
    let h = hex_string(8).sample(&mut Rng::new(3));
    assert_eq!(h.len(), 8);
    assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn composition_map_filter_tuple_record_option() {
    let doubled = integer(0, 5).map(|v| v * 2);
    assert!(doubled.sample_n(11, 50).iter().all(|v| v % 2 == 0));
    let evens = integer(0, 100).filter(|v| v % 2 == 0);
    assert!(evens.sample_n(12, 50).iter().all(|v| v % 2 == 0));
    let pair = tuple(integer(0, 1), boolean()).sample(&mut Rng::new(4));
    assert!(pair.0 == 0 || pair.0 == 1);
    let rec = record(ascii_string(1), integer(0, 9), 3).sample(&mut Rng::new(5));
    assert_eq!(rec.len(), 3);
    let maybe = option(integer(0, 1)).sample_n(6, 100);
    assert!(maybe.contains(&None));
    assert!(maybe.iter().any(Option::is_some));
}

#[test]
fn arrays_honor_length_bounds() {
    let g = array(integer(0, 9), 2, 4);
    for v in g.sample_n(8, 100) {
        assert!((2..=4).contains(&v.len()));
    }
    assert_eq!(array(integer(0, 9), 3, 3).sample(&mut Rng::new(9)).len(), 3);
}

#[test]
fn array_rejects_invalid_bounds_immediately() {
    let result = std::panic::catch_unwind(|| array(integer(0, 9), 5, 2));
    assert!(result.is_err());
    // Upstream also rejects -1/0.5/NaN/Infinity/>u32 bounds: unrepresentable
    // as `usize`, so the type already refuses them; the largest upstream
    // bound constructs (sampling it would not terminate, as upstream).
    let _ = array(integer(0, 9), 0, u32::MAX as usize);
}

#[test]
fn array_honors_minima_maxima_and_exact_lengths() {
    assert_eq!(
        array(integer(0, 9), 0, 0).sample(&mut Rng::new(21)).len(),
        0
    );
    assert_eq!(
        array(integer(0, 9), 4, 4).sample(&mut Rng::new(22)).len(),
        4
    );
    // Upstream { maxLength: 2 } and { minLength: 3 } across sizes: no size
    // parameter exists here, so pin the bounds over repeated samples.
    for v in array(integer(0, 9), 0, 2).sample_n(23, 50) {
        assert!(v.len() <= 2);
    }
    for v in array(integer(0, 9), 3, 9).sample_n(24, 50) {
        assert!((3..=9).contains(&v.len()));
    }
}

#[test]
fn safe_integers_cover_both_parities() {
    // Arbitrary.test.ts:1037 — the complete safe range reaches both parities.
    const MIN_SAFE: i64 = -9_007_199_254_740_991;
    const MAX_SAFE: i64 = 9_007_199_254_740_991;
    let g = integer(MIN_SAFE, MAX_SAFE);
    let mut rng = Rng::new(99);
    let values: Vec<i64> = (0..200).map(|_| g.sample(&mut rng)).collect();
    assert!(values.iter().all(|v| *v >= MIN_SAFE && *v <= MAX_SAFE));
    assert!(values.iter().any(|v| v.rem_euclid(2) == 0));
    assert!(values.iter().any(|v| v.rem_euclid(2) == 1));
}

#[test]
fn numbers_cover_binade_buckets() {
    // Arbitrary.test.ts:1072 — [1, 2) spreads across 4 buckets, 10-40% each.
    let values = number(1.0, 2.0, false, true).sample_n(11, 8192);
    let mut buckets = [0usize; 4];
    for v in &values {
        buckets[(((v - 1.0) * 4.0) as usize).min(3)] += 1;
    }
    for b in buckets {
        assert!((819..=3277).contains(&b), "bucket {b} outside 10-40%");
    }
}

#[test]
fn numbers_reach_nan_below_upper_bound() {
    // Arbitrary.test.ts:1603 — one-sided (..= 0] reaches NaN.
    let values = number(f64::NEG_INFINITY, 0.0, false, false).sample_n(12, 1000);
    assert!(values.iter().all(|v| *v <= 0.0 || v.is_nan()));
    assert!(values.iter().any(|v| v.is_nan()));
}

#[test]
fn numbers_hit_sole_value_between_adjacent_bounds() {
    // Arbitrary.test.ts:1738 — exclusive-adjacent bounds yield the one value.
    let minimum = 2f64.powi(100);
    let unit = 2f64.powi(48);
    let values = number(minimum, minimum + unit * 2.0, true, true).sample_n(13, 20);
    assert!(values.iter().all(|&v| v == minimum + unit));
}
