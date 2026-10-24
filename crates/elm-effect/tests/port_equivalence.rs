//! `Equivalence.test.ts` rows, ported against [`elm_effect::Equivalence`].
//!
//! A JS object is a string-keyed record; the Rust carrier for that shape is
//! `BTreeMap<String, _>`, and `Equal` already walks two maps entry by entry.
//! The `Date` row stays excused: the crate has no date value, and comparing
//! two instants is `SystemTime`'s own `==`, not a structural equivalence.

use std::collections::{BTreeMap, BTreeSet};

use elm_effect::{Equal, Equivalence};

#[test]
fn equivalence_string_number_and_boolean() {
    // :6, :11, :20 - the primitive equivalences are value equality.
    let string = elm_effect::string_equivalence();
    assert!(string.is_equivalent(&"a".to_string(), &"a".to_string()));
    assert!(!string.is_equivalent(&"a".to_string(), &"b".to_string()));

    let number = elm_effect::number_equivalence();
    assert!(number.is_equivalent(&1.5, &1.5));
    assert!(!number.is_equivalent(&1.5, &2.5));
    assert!(number.is_equivalent(&f64::NAN, &f64::NAN));

    let boolean = elm_effect::boolean_equivalence();
    assert!(boolean.is_equivalent(&true, &true));
    assert!(!boolean.is_equivalent(&true, &false));
}

#[test]
fn equivalence_bigint_through_the_integer_carrier() {
    // :27 - Rust's i64 is the crate's fixed-width integer carrier; the row's
    // claim is value equality, which `strict` gives.
    let bigint = Equivalence::<i64>::strict();
    assert!(bigint.is_equivalent(&9_007_199_254_740_993, &9_007_199_254_740_993));
    assert!(!bigint.is_equivalent(&9_007_199_254_740_993, &1));
}

#[test]
fn equivalence_map_input_projects_both_sides() {
    // :32 - both inputs go through the projection before comparison.
    let by_length = elm_effect::string_equivalence().map_input(|s: &String| s.len().to_string());
    assert!(by_length.is_equivalent(&"ab".to_string(), &"cd".to_string()));
    assert!(!by_length.is_equivalent(&"ab".to_string(), &"cde".to_string()));
}

#[test]
fn equivalence_combine_requires_both() {
    // :44 - combine is conjunction.
    let even = Equivalence::make(|a: &i32, b: &i32| (a + b) % 2 == 0);
    let close = Equivalence::make(|a: &i32, b: &i32| (a - b).abs() < 5);
    let both = even.combine(close);
    assert!(both.is_equivalent(&2, &4));
    assert!(
        !both.is_equivalent(&0, &10),
        "the sum is even, but the values are far apart"
    );
    assert!(
        !both.is_equivalent(&0, &1),
        "the values are close, but the sum is odd"
    );
    assert!(!both.is_equivalent(&0, &11));
}

#[test]
fn equivalence_combine_all_is_vacuous_on_empty() {
    // :55 - every member must hold, and an empty set holds trivially.
    let all = Equivalence::combine_all(vec![
        Equivalence::make(|a: &i32, b: &i32| a <= b),
        Equivalence::make(|a: &i32, b: &i32| (a - b).abs() < 10),
    ]);
    assert!(all.is_equivalent(&1, &2));
    assert!(!all.is_equivalent(&1, &20));
    assert!(Equivalence::<i32>::combine_all(Vec::new()).is_equivalent(&1, &2));
}

#[test]
fn equivalence_tuple_and_array() {
    // :67, :75 - tuples and arrays compare element by element.
    let tuple = elm_effect::tuple_equivalence::<i32, String>();
    assert!(tuple.is_equivalent(&(1, "a".to_string()), &(1, "a".to_string())));
    assert!(!tuple.is_equivalent(&(1, "a".to_string()), &(1, "b".to_string())));

    let array = elm_effect::array_equivalence::<i32>();
    assert!(array.is_equivalent(&vec![1, 2, 3], &vec![1, 2, 3]));
    assert!(!array.is_equivalent(&vec![1, 2, 3], &vec![1, 2]));
    assert!(!array.is_equivalent(&vec![1, 2, 3], &vec![3, 2, 1]));
}

#[test]
fn equivalence_struct_uses_string_keys_not_order() {
    // :85 - two records with the same fields are equivalent regardless of
    // which key was inserted first; keys are matched by name, not position.
    let equivalence = Equivalence::struct_(vec![
        ("a".to_string(), Equivalence::<i32>::strict()),
        ("b".to_string(), Equivalence::<i32>::strict()),
    ]);
    let first: BTreeMap<String, i32> = [("a".to_string(), 1), ("b".to_string(), 2)]
        .into_iter()
        .collect();
    let second: BTreeMap<String, i32> = [("b".to_string(), 2), ("a".to_string(), 1)]
        .into_iter()
        .collect();
    assert!(equivalence.is_equivalent(&first, &second));

    let mut changed = second.clone();
    changed.insert("b".to_string(), 3);
    assert!(!equivalence.is_equivalent(&first, &changed));
}

#[test]
fn equivalence_struct_ignores_extra_keys() {
    // :123 - the second "string keys" row: only the declared fields are
    // compared, so an unrelated key on either side does not matter.
    let equivalence = Equivalence::struct_(vec![("a".to_string(), Equivalence::<i32>::strict())]);
    let two: BTreeMap<String, i32> = [("a".to_string(), 1), ("b".to_string(), 2)]
        .into_iter()
        .collect();
    let one: BTreeMap<String, i32> = [("a".to_string(), 1)].into_iter().collect();
    assert!(equivalence.is_equivalent(&two, &one));

    let missing: BTreeMap<String, i32> = [("b".to_string(), 1)].into_iter().collect();
    assert!(
        !equivalence.is_equivalent(&missing, &one),
        "a listed key present on only one side is not a match"
    );
}

#[test]
fn equivalence_set_membership_ignores_order() {
    // The set spelling of the same "string keys" claim.
    let equivalence = Equivalence::make(|a: &BTreeSet<String>, b: &BTreeSet<String>| a.equal(b));
    let first: BTreeSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
    let second: BTreeSet<String> = ["b".to_string(), "a".to_string()].into_iter().collect();
    assert!(equivalence.is_equivalent(&first, &second));
    assert!(!equivalence.is_equivalent(
        &first,
        &["a".to_string()].into_iter().collect::<BTreeSet<_>>()
    ));
}
