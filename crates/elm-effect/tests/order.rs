//! [`elm_effect::Order`] laws.
//!
//! The crate has no date value: `SystemTime` already orders by instant
//! without a projection.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Order, combine_ordering, number_order, ord_order, string_order};

fn record(fields: &[(&str, i32)]) -> BTreeMap<String, i32> {
    fields
        .iter()
        .map(|(key, value)| (key.to_string(), *value))
        .collect()
}

#[test]
fn order_struct_compares_fields_in_declaration_order() {
    // :6 - first non-equal field decides; extra keys are ignored.
    let order = Order::struct_(vec![
        ("a".to_string(), ord_order::<i32>()),
        ("b".to_string(), ord_order::<i32>()),
    ]);
    let low = record(&[("a", 1), ("b", 9)]);
    let high = record(&[("a", 2), ("b", 0)]);
    assert_eq!(order.compare(&low, &high), std::cmp::Ordering::Less);
    assert_eq!(
        order.compare(&one_field(&low), &low),
        std::cmp::Ordering::Less,
        "a listed field missing on one side sorts below a present one"
    );
    assert_eq!(
        order.compare(
            &record(&[("a", 1), ("b", 2), ("c", 0)]),
            &record(&[("a", 1), ("b", 2)])
        ),
        std::cmp::Ordering::Equal
    );
}

fn one_field(record: &BTreeMap<String, i32>) -> BTreeMap<String, i32> {
    record
        .iter()
        .filter(|(key, _)| key.as_str() == "a")
        .map(|(key, value)| (key.clone(), *value))
        .collect()
}

#[test]
fn order_tuple_decides_left_then_right() {
    // :13 - the first component decides; the second only on a tie.
    let order: Order<(i32, i32)> = Order::make(|a: &(i32, i32), b: &(i32, i32)| {
        combine_ordering(
            ord_order::<i32>().compare(&a.0, &b.0),
            ord_order::<i32>().compare(&a.1, &b.1),
        )
    });
    assert_eq!(order.compare(&(1, 9), &(2, 0)), std::cmp::Ordering::Less);
    assert_eq!(order.compare(&(1, 9), &(1, 8)), std::cmp::Ordering::Greater);
    assert_eq!(order.compare(&(1, 9), &(1, 9)), std::cmp::Ordering::Equal);
}

#[test]
fn order_map_input_projects_both_sides() {
    // :21 - both values are projected before comparison.
    let by_length = ord_order::<usize>().map_input(|s: &String| s.len());
    assert_eq!(
        by_length.compare(&"ab".to_string(), &"c".to_string()),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        by_length.compare(&"ab".to_string(), &"cd".to_string()),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        by_length.compare(&"a".to_string(), &"bc".to_string()),
        std::cmp::Ordering::Less
    );
}

#[test]
fn order_number_treats_nan_as_the_lowest_value() {
    // :28 - NaN sits below every number and equals itself.
    let order = number_order();
    let nan = f64::NAN;
    assert_eq!(
        order.compare(&nan, &f64::NEG_INFINITY),
        std::cmp::Ordering::Less
    );
    assert_eq!(order.compare(&nan, &nan), std::cmp::Ordering::Equal);
    assert_eq!(order.compare(&1.0, &2.0), std::cmp::Ordering::Less);
    assert_eq!(order.compare(&2.0, &1.0), std::cmp::Ordering::Greater);
    assert_eq!(
        order.compare(&1.0, &1.0),
        std::cmp::Ordering::Equal,
        "a comparator must report a tie for equal values"
    );
    assert!(order.is_less_than_or_equal(&1.0, &1.0));
    assert!(order.is_greater_than_or_equal(&1.0, &1.0));
    assert!(!order.is_less_than(&1.0, &1.0));
    assert!(!order.is_greater_than(&1.0, &1.0));
    assert!(order.is_between(&1.0, &1.0, &1.0));
}

#[test]
fn order_clamp_keeps_values_inside_inclusive_bounds() {
    // :47 - below becomes the minimum, above the maximum, inside unchanged.
    let order = ord_order::<i32>();
    assert_eq!(order.clamp(-5, 0, 10), 0);
    assert_eq!(order.clamp(5, 0, 10), 5);
    assert_eq!(order.clamp(50, 0, 10), 10);
    assert_eq!(order.clamp(0, 0, 10), 0);
    assert_eq!(order.clamp(10, 0, 10), 10);
}

#[test]
fn order_is_between_checks_inclusive_bounds() {
    // :58 - both bounds count.
    let order = ord_order::<i32>();
    assert!(order.is_between(&0, &0, &10));
    assert!(order.is_between(&10, &0, &10));
    assert!(order.is_between(&5, &0, &10));
    assert!(!order.is_between(&-1, &0, &10));
    assert!(!order.is_between(&11, &0, &10));
}

#[test]
fn order_flip_reverses_an_existing_order() {
    // :69 - reverse is argument order plus verdict flip.
    let forward = ord_order::<i32>();
    let reversed = forward.clone().reverse();
    assert_eq!(forward.compare(&1, &2), std::cmp::Ordering::Less);
    assert_eq!(reversed.compare(&1, &2), std::cmp::Ordering::Greater);
    assert_eq!(reversed.compare(&2, &1), std::cmp::Ordering::Less);
}

#[test]
fn order_relational_predicates_are_strict_where_named() {
    // :76 through :97 - the four comparisons.
    let order = ord_order::<i32>();
    assert!(order.is_less_than(&1, &2));
    assert!(!order.is_less_than(&2, &2));
    assert!(order.is_less_than_or_equal(&2, &2));
    assert!(order.is_greater_than(&3, &2));
    assert!(!order.is_greater_than(&2, &2));
    assert!(order.is_greater_than_or_equal(&2, &2));
}

#[test]
fn order_min_and_max_break_ties_toward_the_first() {
    // :104, :119 - min keeps the first on a tie, max keeps the first on a tie.
    let order = ord_order::<i32>();
    assert_eq!(order.min(1, 2), 1);
    assert_eq!(order.min(2, 1), 1);
    assert_eq!(order.min(2, 2), 2);
    assert_eq!(order.max(1, 2), 2);
    assert_eq!(order.max(2, 1), 2);
    assert_eq!(order.max(2, 2), 2);
}

#[test]
fn order_combine_and_combine_all_stop_at_the_first_non_equal_verdict() {
    // :134, :140 - a criterion that decides prevents every later criterion
    // from running, which is what makes an expensive tie-breaker cheap.
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = {
        let calls = Arc::clone(&calls);
        Order::make(move |a: &i32, b: &i32| {
            calls.fetch_add(1, Ordering::SeqCst);
            a.cmp(b)
        })
    };
    let first = Order::make(|a: &i32, b: &i32| {
        if a == b {
            std::cmp::Ordering::Equal
        } else {
            a.cmp(b)
        }
    });

    let combined = first.clone().combine(counted.clone());
    assert_eq!(combined.compare(&1, &2), std::cmp::Ordering::Less);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    assert_eq!(combined.compare(&1, &1), std::cmp::Ordering::Equal);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let all = Order::combine_all(vec![first, counted.clone()]);
    calls.store(0, Ordering::SeqCst);
    assert_eq!(all.compare(&3, &2), std::cmp::Ordering::Greater);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(all.compare(&3, &3), std::cmp::Ordering::Equal);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn order_string_is_lexicographic() {
    let order = string_order();
    assert_eq!(
        order.compare(&"a".to_string(), &"a".to_string()),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        order.compare(&"a".to_string(), &"b".to_string()),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        order.compare(&"b".to_string(), &"a".to_string()),
        std::cmp::Ordering::Greater
    );
}

#[test]
fn order_reuses_the_same_criteria_across_comparisons() {
    // :134 - combineAll is a reusable value: comparing twice does not consume
    // the criteria, and each comparison starts from the first criterion.
    let all = Order::combine_all(vec![ord_order::<i32>(), ord_order::<i32>()]);
    assert_eq!(all.compare(&1, &2), std::cmp::Ordering::Less);
    assert_eq!(all.compare(&1, &2), std::cmp::Ordering::Less);
    assert_eq!(all.compare(&2, &2), std::cmp::Ordering::Equal);
}
