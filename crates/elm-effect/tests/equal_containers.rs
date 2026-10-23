//! `Equal` for keyed collections: the pin's JavaScript `Map` and `Set` rows.
//!
//! The port's spellings are `BTreeMap` and `BTreeSet`. Their iteration order
//! is the keys' order rather than insertion order, so the
//! "same entries, different insertion order" rows are represented by building
//! the two values in different orders, which is the only way insertion order
//! could have shown up.
//!
//! Rows that need two distinct-but-equal keys in one collection (a JS `Map`
//! keyed by object identity), a `NaN` key, or a cycle are not representable:
//! a `BTreeMap` collapses equal keys and keys on `Ord`, which `f64` does not
//! implement. Those stay excused with that reason.

use elm_effect::Equal;
use std::collections::{BTreeMap, BTreeSet};

fn map(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

#[test]
fn equal_maps_hold_for_the_same_reference() {
    // Equal.test.ts:707 — a map compared with itself.
    let subject = map(&[("a", 1)]);
    assert!(subject.equal(&subject));
}

#[test]
fn equal_maps_hold_for_empty_maps() {
    // Equal.test.ts:712 — two empty maps.
    assert!(map(&[]).equal(&map(&[])));
}

#[test]
fn equal_maps_reject_different_values() {
    // Equal.test.ts:716 — same keys, one value differs.
    assert!(!map(&[("a", 1), ("b", 2)]).equal(&map(&[("a", 1), ("b", 3)])));
}

#[test]
fn equal_maps_reject_different_keys() {
    // Equal.test.ts:722 — same size, one key differs.
    assert!(!map(&[("a", 1), ("b", 2)]).equal(&map(&[("a", 1), ("c", 2)])));
}

#[test]
fn equal_maps_reject_different_sizes() {
    // Equal.test.ts:728 — a subset is not equal.
    assert!(!map(&[("a", 1), ("b", 2)]).equal(&map(&[("a", 1)])));
}

#[test]
fn equal_maps_compare_nested_maps_all_the_way_down() {
    // Equal.test.ts:754 — the value is itself a map.
    let nested = |inner: i32| -> BTreeMap<String, BTreeMap<String, i32>> {
        [("nested".to_string(), map(&[("x", inner)]))].into_iter().collect()
    };
    assert!(nested(1).equal(&nested(1)));
    assert!(!nested(1).equal(&nested(2)));
}

#[test]
fn equal_sets_hold_for_identical_contents() {
    // Equal.test.ts:770 — same elements, built in the same order.
    let set1: BTreeSet<i32> = [1, 2, 3].into_iter().collect();
    let set2: BTreeSet<i32> = [1, 2, 3].into_iter().collect();
    assert!(set1.equal(&set2));
}

#[test]
fn equal_sets_hold_for_the_same_reference() {
    // Equal.test.ts:776 — a set compared with itself.
    let subject: BTreeSet<i32> = [1, 2, 3].into_iter().collect();
    assert!(subject.equal(&subject));
}

#[test]
fn equal_sets_hold_for_empty_sets() {
    // Equal.test.ts:781 — two empty sets.
    assert!(BTreeSet::<i32>::new().equal(&BTreeSet::<i32>::new()));
}

#[test]
fn equal_sets_reject_different_elements() {
    // Equal.test.ts:785 — one member differs.
    let set1: BTreeSet<i32> = [1, 2, 3].into_iter().collect();
    let set2: BTreeSet<i32> = [1, 2, 4].into_iter().collect();
    assert!(!set1.equal(&set2));
}

#[test]
fn equal_sets_reject_different_sizes() {
    // Equal.test.ts:791 — a subset is not equal.
    let set1: BTreeSet<i32> = [1, 2, 3].into_iter().collect();
    let set2: BTreeSet<i32> = [1, 2].into_iter().collect();
    assert!(!set1.equal(&set2));
}

#[test]
fn equal_sets_ignore_insertion_order() {
    // Equal.test.ts:797 — same members, inserted in a different order. The
    // container sorts them, so the two agree member by member.
    let set1: BTreeSet<i32> = [1, 2, 3].into_iter().collect();
    let set2: BTreeSet<i32> = [3, 1, 2].into_iter().collect();
    assert!(set1.equal(&set2));
}

#[test]
fn equal_sets_compare_array_elements_all_the_way_down() {
    // Equal.test.ts:823 — the members are arrays, so equality runs into them.
    let set1: BTreeSet<Vec<i32>> = [vec![1, 2], vec![3, 4]].into_iter().collect();
    let set2: BTreeSet<Vec<i32>> = [vec![3, 4], vec![1, 2]].into_iter().collect();
    assert!(set1.equal(&set2));

    let differs: BTreeSet<Vec<i32>> = [vec![1, 2], vec![3, 5]].into_iter().collect();
    assert!(!set1.equal(&differs));
}
