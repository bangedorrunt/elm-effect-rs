//! Foundation behavior at the `effect@4.0.0` pin.

use std::cmp::Ordering;
use std::ops::Not;

use elm_effect::{
    Equal, Equivalence, Hash, Order, Predicate, array_equivalence, boolean_equivalence,
    boolean_order, combine_hash, combine_ordering, compose, const_false, const_null, const_true, constant,
    flip, flip_curried, hash_f64, hash_str, identity, match_ordering, number_equivalence, number_order,
    ord_order, reverse_ordering, string_equivalence, string_order, tuple_equivalence, tupled, untupled,
};

#[path = "../src/data/bigint.rs"]
mod bigint;

#[test]
#[allow(invalid_nan_comparisons, clippy::assertions_on_constants)]
fn equal_nan_is_nan() {
    // The deliberate divergence: Equal treats NaN as equal; f64's == stays IEEE.
    assert!(f64::NAN.equal(&f64::NAN));
    assert!(f32::NAN.equal(&f32::NAN));
    assert!(f64::NAN != f64::NAN);
    assert!(!1.0f64.equal(&f64::NAN));
    assert!(!f64::NAN.equal(&f64::INFINITY));
    assert!(1.0f64.equal(&1.0));
    assert!(vec![f64::NAN].equal(&vec![f64::NAN]));
    assert!(Some(f64::NAN).equal(&Some(f64::NAN)));
    assert!(!None::<f64>.equal(&Some(1.0)));
}

#[test]
fn equal_primitives_are_structural() {
    assert!("a".to_string().equal(&"a".to_string()));
    assert!(!"a".to_string().equal(&"b".to_string()));
    assert!(true.equal(&true));
    assert!(!true.equal(&false));
    assert!(Some(1u32).equal(&Some(1u32)));
    assert!((1u32, "x".to_string()).equal(&(1u32, "x".to_string())));
    assert!(!(1u32, "x".to_string()).equal(&(1u32, "y".to_string())));
}

#[test]
fn hash_nan_matches_and_combines() {
    assert_eq!(f64::NAN.hash_value(), f64::NAN.hash_value());
    assert_eq!(hash_f64(f64::NAN), hash_f64(f64::NAN));
    assert_eq!(1u32.hash_value(), 1u32.hash_value());
    assert_eq!(hash_str("abc"), hash_str("abc"));
    assert_ne!(hash_str("abc"), hash_str("abd"));
    assert_eq!(true.hash_value(), true.hash_value());
    assert_ne!(true.hash_value(), false.hash_value());
    let pair = (1u32, "x".to_string());
    assert_eq!(
        pair.hash_value(),
        combine_hash(1u32.hash_value(), "x".to_string().hash_value())
    );
    assert_eq!(hash_f64(0.0), hash_f64(-0.0));
    assert_eq!(vec![1u32, 2].hash_value(), vec![1u32, 2].hash_value());
    assert_ne!(vec![1u32, 2].hash_value(), vec![2u32, 1].hash_value());
    assert_ne!(vec![1u32].hash_value(), vec![1u32, 2].hash_value());
    assert_ne!(true.hash_value(), hash_str("true"));
    assert_ne!(None::<u32>.hash_value(), hash_str("None"));
}

#[test]
fn order_number_nan_rules() {
    let order: Order<f64> = number_order();
    assert_eq!(order.compare(&f64::NAN, &f64::NAN), Ordering::Equal);
    assert_eq!(order.compare(&f64::NAN, &1.0), Ordering::Less);
    assert_eq!(order.compare(&1.0, &f64::NAN), Ordering::Greater);
    assert_eq!(order.compare(&1.0, &2.0), Ordering::Less);
    assert_eq!(order.compare(&2.0, &1.0), Ordering::Greater);
    // Pin behavior: ties report Greater.
    assert_eq!(order.compare(&1.0, &1.0), Ordering::Greater);
    assert_eq!(order.min(1.0, 1.0), 1.0);
    assert_eq!(order.clamp(5.0, 1.0, 3.0), 3.0);
    assert!(order.is_less_than(&1.0, &2.0));
    assert!(order.is_greater_than(&2.0, &1.0));
    assert!(order.is_less_than_or_equal(&1.0, &2.0));
    assert!(!order.is_less_than_or_equal(&2.0, &1.0));
    assert!(order.is_greater_than_or_equal(&2.0, &1.0));
    assert!(!order.is_greater_than_or_equal(&1.0, &2.0));
    assert!(order.is_between(&2.0, &1.0, &3.0));
    assert!(!order.is_between(&4.0, &1.0, &3.0));
    assert_eq!(order.max(1.0, 2.0), 2.0);
    let both = Order::combine_all(vec![number_order(), number_order()]);
    assert_eq!(both.compare(&1.0, &2.0), Ordering::Less);
}

#[test]
fn order_composes_lexicographically() {
    let by_first = string_order().map_input(|pair: &(String, u32)| pair.0.clone());
    let by_second = ord_order::<u32>().map_input(|pair: &(String, u32)| pair.1);
    let both = by_first.combine(by_second);
    // Distinct first members decide on the first order alone.
    assert_eq!(
        both.compare(&("a".to_string(), 9u32), &("b".to_string(), 1u32)),
        Ordering::Less
    );
    // Tied first members report Greater per the pin, which decides.
    assert_eq!(
        both.compare(&("x".to_string(), 1u32), &("x".to_string(), 2u32)),
        Ordering::Greater
    );
}

#[test]
fn equivalence_and_predicate_basics() {
    let eq: Equivalence<f64> = number_equivalence();
    assert!(eq.is_equivalent(&f64::NAN, &f64::NAN));
    assert!(!eq.is_equivalent(&1.0, &2.0));
    assert!(string_equivalence().is_equivalent(&"a".to_string(), &"a".to_string()));
    assert!(boolean_equivalence().is_equivalent(&true, &true));
    let both = number_equivalence().combine(number_equivalence());
    assert!(both.is_equivalent(&1.0, &1.0));
    let all = Equivalence::combine_all(vec![number_equivalence(), number_equivalence()]);
    assert!(all.is_equivalent(&1.0, &1.0));
    assert!(!all.is_equivalent(&1.0, &2.0));
    let by_len = string_equivalence().map_input(|v: &Vec<u32>| v.len().to_string());
    assert!(by_len.is_equivalent(&vec![1u32], &vec![2u32]));
    assert!(
        tuple_equivalence::<u32, String>()
            .is_equivalent(&(1u32, "a".to_string()), &(1u32, "a".to_string()))
    );
    assert!(array_equivalence::<u32>().is_equivalent(&vec![1u32], &vec![1u32]));
    assert!(!array_equivalence::<u32>().is_equivalent(&vec![1u32], &vec![1u32, 2u32]));

    let positive = Predicate::make(|n: &i32| *n > 0);
    let even = Predicate::make(|n: &i32| n % 2 == 0);
    assert!(positive.clone().and(even.clone()).test(&4));
    assert!(!positive.clone().and(even.clone()).test(&3));
    assert!(positive.clone().or(even.clone()).test(&3));
    assert!(positive.not().test(&-1));
    assert!(Predicate::<i32>::always().test(&0));
    assert!(!Predicate::<i32>::never().test(&0));
    assert!(Predicate::is_some().test(&Some(1)));
    assert!(Predicate::<Option<i32>>::is_none().test(&None));
    let projected = even.clone().map_input(|pair: &(i32, i32)| pair.0);
    assert!(projected.test(&(4, 1)));
}

#[test]
fn bigint_equivalence() {
    // Equivalence.test.ts:27 — BigInt equivalence compares values.
    let eq = Equivalence::make(|a: &bigint::Big, b: &bigint::Big| a == b);
    assert!(eq.is_equivalent(&bigint::Big::from_int(1), &bigint::Big::from_int(1)));
    assert!(!eq.is_equivalent(&bigint::Big::from_int(1), &bigint::Big::from_int(2)));
}

#[test]
fn function_and_ordering_basics() {
    assert_eq!(identity(3), 3);
    assert_eq!(constant(7)(), 7);
    assert!(const_true()());
    assert!(!const_false()());
    assert_eq!(flip(|a: u32, b: u32| a - b)(1, 3), 2);
    assert_eq!(compose(|a: u32| a + 1, |b: u32| b * 2)(3), 8);

    assert_eq!(reverse_ordering(Ordering::Less), Ordering::Greater);
    assert_eq!(
        combine_ordering(Ordering::Equal, Ordering::Less),
        Ordering::Less
    );
    assert_eq!(
        match_ordering(Ordering::Greater, || "lt", || "eq", || "gt"),
        "gt"
    );
    assert_eq!(
        string_order().compare(&"a".to_string(), &"b".to_string()),
        Ordering::Less
    );
    assert_eq!(boolean_order().compare(&false, &true), Ordering::Less);
    assert_eq!(
        boolean_order().reverse().compare(&false, &true),
        Ordering::Greater
    );
}

#[test]
fn predicate_and_stops_at_first_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = calls.clone();
    let failing = Predicate::make(|_: &u32| false);
    let counting = Predicate::make(move |_: &u32| {
        probe.fetch_add(1, Ordering::SeqCst);
        true
    });
    assert!(!failing.and(counting).test(&1));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let probe2 = calls.clone();
    let counting2 = Predicate::make(move |_: &u32| {
        probe2.fetch_add(1, Ordering::SeqCst);
        true
    });
    assert!(counting2.and(Predicate::make(|_: &u32| true)).test(&1));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn predicate_or_needs_any_and_not_flips() {
    assert!(Predicate::make(|_: &u32| false).or(Predicate::make(|_: &u32| true)).test(&0));
    assert!(!Predicate::make(|_: &u32| false).or(Predicate::make(|_: &u32| false)).test(&0));
    assert!(!Predicate::make(|_: &u32| true).not().test(&0));
    assert!(Predicate::make(|_: &u32| false).not().test(&0));
}

#[test]
fn predicate_xor_needs_exactly_one() {
    assert!(Predicate::make(|_: &u32| true).xor(Predicate::make(|_: &u32| false)).test(&0));
    assert!(!Predicate::make(|_: &u32| true).xor(Predicate::make(|_: &u32| true)).test(&0));
    assert!(!Predicate::make(|_: &u32| false).xor(Predicate::make(|_: &u32| false)).test(&0));
}

#[test]
fn function_combinators() {
    // Function.test.ts:93/101 - identity returns values and references.
    assert_eq!(identity(3), 3);
    let obj = vec![1];
    assert!(std::ptr::eq(identity(&obj), &obj));
    // :108/:114 - satisfies/cast are identity at runtime.
    assert_eq!(identity(5), 5);
    assert_eq!(identity(42), 42);
    // :120 - constant returns the value every time.
    let c = constant(42);
    assert_eq!(c(), 42);
    assert_eq!(c(), 42);
    // :135/:141/:147 - nullary constants.
    assert!(const_true()());
    assert!(!const_false()());
    assert_eq!(const_null::<i32>(), None);
    // :165 - flip reverses curried arguments.
    fn subtract_len(a: i64) -> Box<dyn Fn(&str) -> i64> {
        Box::new(move |s: &str| a - s.len() as i64)
    }
    assert_eq!(flip_curried(subtract_len)("aaa")(2), -1);
    // :175 - compose runs left to right.
    let increment = |n: i32| n + 1;
    let double = |n: i32| n * 2;
    assert_eq!(compose(increment, double)(2), 6);
    // :193/:201 - tupled and untupled convert both ways.
    fn add(a: i32, b: i32) -> i32 {
        a + b
    }
    assert_eq!(tupled(add)((1, 2)), 3);
    assert_eq!(untupled(|pair: (i32, i32)| pair.0 + pair.1)(1, 2), 3);
}
