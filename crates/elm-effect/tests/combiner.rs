//! [`elm_effect::Combiner`] and [`elm_effect::Reducer`] call order.
//!
//! A combiner is just its function: these tests pin the order each
//! constructor promises. They do not assert an associativity law, which an
//! arbitrary closing function cannot state.

use elm_effect::{Combiner, Reducer, ord_order, string_order};

#[test]
fn combiner_flip_swaps_the_arguments() {
    // Combiner.test.ts:6 - flip reverses the call order without changing the
    // underlying combiner.
    let concat = Combiner::make(|a: &String, b: &String| format!("{a}{b}"));
    assert_eq!(concat.combine(&"a".to_string(), &"b".to_string()), "ab");
    assert_eq!(
        concat.flip().combine(&"a".to_string(), &"b".to_string()),
        "ba"
    );
}

#[test]
fn combiner_min_and_max_follow_an_order() {
    // :11, :17 - the smaller / larger value wins, ties keep the first.
    let min = Combiner::min(ord_order::<i32>());
    assert_eq!(min.combine(&2, &1), 1);
    assert_eq!(min.combine(&1, &2), 1);
    let max = Combiner::max(ord_order::<i32>());
    assert_eq!(max.combine(&2, &1), 2);
    assert_eq!(max.combine(&1, &2), 2);
}

#[test]
fn combiner_first_and_last_keep_their_side() {
    // :23, :28 - first ignores the right value, last the left.
    assert_eq!(
        Combiner::<String>::first().combine(&"a".to_string(), &"b".to_string()),
        "a"
    );
    assert_eq!(
        Combiner::<String>::last().combine(&"a".to_string(), &"b".to_string()),
        "b"
    );
}

#[test]
fn combiner_constant_ignores_both_inputs() {
    // :33 - every combination answers the captured value.
    let constant = Combiner::constant("fixed".to_string());
    assert_eq!(
        constant.combine(&"a".to_string(), &"b".to_string()),
        "fixed"
    );
}

#[test]
fn combiner_intercalate_joins_with_the_middle() {
    // :38 - the middle text sits between the two values.
    let join = Combiner::intercalate(", ");
    assert_eq!(join.combine(&"a".to_string(), &"b".to_string()), "a, b");
    assert_eq!(
        join.combine(&"".to_string(), &"b".to_string()),
        ", b",
        "no special case for an empty side; Effect 4 concatenates"
    );
}

#[test]
fn combiner_works_under_a_string_order() {
    // min/max are generic in the order, so a custom order decides the winner.
    let min = Combiner::min(string_order());
    assert_eq!(
        min.combine(&"b".to_string(), &"a".to_string()),
        "a".to_string()
    );
}

#[test]
fn reducer_flip_swaps_the_combine_direction() {
    // Reducer.test.ts:6 - flip keeps the empty value and reverses the fold.
    let subtract = Reducer::make(0, |a: &i32, b: &i32| a - b);
    assert_eq!(subtract.combine(&10, &3), 7);
    let flipped = subtract.flip();
    assert_eq!(flipped.combine(&10, &3), -7);
    assert_eq!(flipped.empty(), 0);
}
