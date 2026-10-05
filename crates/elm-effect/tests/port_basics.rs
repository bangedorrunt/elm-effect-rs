//! Core basics fence: predicate combinators, duration helpers, seeded RNG.
//! The module loads via `#[path]` (the .15 precedent): no facade touch.

#[path = "../src/basics.rs"]
mod basics;

use basics::{
    Predicate, SeedRandom, always, between_ms, clamp_ms, compare_ms, divide_ms, equals_ms, every,
    from_input, from_input_unsafe, max_ms, min_ms, never, some,
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
