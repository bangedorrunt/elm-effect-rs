//! `TxHashSet`'s set algebra: union, intersection, difference, subset, the
//! three predicate folds, map, filter and reduce, plus the immutable snapshot
//! the operations return.

//! The snapshot type is std's Hash Set, which is what TxHashSet stores, so the
//! test reaches it the same way the crate does.
#[path = "../src/containers/hash_map.rs"]
mod hash_map;
#[path = "../src/containers/hash_set.rs"]
mod hash_set;

use elm_effect::{TxHashSet, atomically};
use std::collections::HashSet;

fn set<T: Eq + std::hash::Hash + Clone>(values: &[T]) -> TxHashSet<T> {
    values.iter().cloned().collect()
}

fn contents<T: Eq + std::hash::Hash + Ord + Clone>(s: &TxHashSet<T>) -> Vec<T> {
    let mut out: Vec<T> = s.to_hash_set().iter().cloned().collect();
    out.sort();
    out.dedup();
    out
}

#[test]
fn union_intersection_difference() {
    // TxHashSet.test.ts:163, :175, :187.
    let ab = set(&[1, 2, 3]);
    let bc = set(&[3, 4, 5]);

    assert_eq!(contents(&ab.union(&bc)), vec![1, 2, 3, 4, 5]);
    assert_eq!(contents(&ab.intersection(&bc)), vec![3]);
    assert_eq!(contents(&ab.difference(&bc)), vec![1, 2]);

    // :412 — the empty sets behave as identities.
    let empty: TxHashSet<u32> = set(&[]);
    assert_eq!(contents(&empty.union(&ab)), vec![1, 2, 3]);
    assert!(empty.intersection(&ab).is_empty());
    assert_eq!(contents(&ab.difference(&empty)), vec![1, 2, 3]);
    assert!(empty.difference(&ab).is_empty());
}

#[test]
fn is_subset_is_reflexive_and_ordering_sensitive() {
    // TxHashSet.test.ts:199.
    let small = set(&[1, 2]);
    let large = set(&[1, 2, 3]);
    assert!(small.is_subset(&large));
    assert!(large.is_subset(&large), "a set is its own subset");
    assert!(!large.is_subset(&small));
    assert!(
        set(&[]).is_subset(&large),
        "the empty set is a subset of all"
    );

    // :431 — a one-element set is a subset only of a set holding it exactly.
    assert!(set(&[1]).is_subset(&large));
    assert!(!set(&[9]).is_subset(&large));
}

#[test]
fn some_and_every_read_each_element() {
    // TxHashSet.test.ts:213, :224.
    let s = set(&[2, 4, 6]);
    assert!(s.some(|v| *v == 4));
    assert!(!s.some(|v| *v == 5));
    assert!(s.every(|v| *v % 2 == 0));
    assert!(!s.every(|v| *v > 4));

    // The empty set: `some` is false, `every` is true (vacuous truth).
    let empty: TxHashSet<u32> = set(&[]);
    assert!(!empty.some(|_| true));
    assert!(empty.every(|_| false), "no element refutes it");
}

#[test]
fn map_filter_and_reduce() {
    // TxHashSet.test.ts:237, :248, :258, :269.
    let s = set(&[1, 2, 3]);

    // :237 — map transforms each value.
    assert_eq!(contents(&s.map(|v| v * 10)), vec![10, 20, 30]);

    // :248 — map can reduce size when two inputs collide.
    let collided = set(&[1, 2, 3, 4, 5]);
    assert_eq!(contents(&collided.map(|v| v % 2)), vec![0, 1]);

    // :258 — filter keeps only matching values.
    assert_eq!(contents(&s.filter(|v| *v > 1)), vec![2, 3]);
    assert!(s.filter(|_| false).is_empty());

    // :269, :396 — reduce accumulates.
    assert_eq!(s.reduce(0, |acc, v| acc + v), 6);
    let big = set(&(0..500).collect::<Vec<_>>());
    assert_eq!(big.reduce(0, |acc, v| acc + v), 124_750);
    assert_eq!(big.len(), 500, "and the big set kept every element");
}

#[test]
fn to_hash_set_snapshots_and_detaches() {
    // TxHashSet.test.ts:288.
    let s = set(&[1, 2, 3]);
    let snapshot: HashSet<u32> = s.to_hash_set();
    assert_eq!(snapshot.len(), 3);

    // The snapshot detaches: later writes to the transactional set do not
    // reach it.
    s.insert(4);
    assert_eq!(snapshot.len(), 3, "the snapshot did not move");
    assert_eq!(s.len(), 4, "the transactional set did");
}

/// A type with a hand-written `Eq`/`Hash` — the Rust shape of the tag's
/// "custom Equal implementation" (a case-insensitive key).
#[derive(Clone, Debug)]
struct Folded(String);

impl PartialEq for Folded {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_lowercase() == other.0.to_lowercase()
    }
}

impl Eq for Folded {}

impl std::hash::Hash for Folded {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_lowercase().hash(state);
    }
}

#[test]
fn a_custom_hash_and_eq_decide_membership() {
    // TxHashSet.test.ts:304 — the set defers to the element's own Hash and
    // Equal, so two values that compare equal are one member.
    let s: TxHashSet<Folded> = [Folded("Alpha".into()), Folded("BETA".into())]
        .into_iter()
        .collect();
    assert_eq!(s.len(), 2);
    assert!(
        !s.insert(Folded("alpha".into())),
        "already present under Fold"
    );
    assert_eq!(s.len(), 2);
    assert!(s.contains(&Folded("beta".into())));
    assert!(s.remove(&Folded("ALPHA".into())));
    assert_eq!(s.len(), 1);
}

#[test]
fn a_multi_op_block_runs_as_one_transaction() {
    // TxHashSet.test.ts:329 — a run of operations counted once, so a reader
    // outside sees none or all of them, never a half-applied middle.
    let left = set(&[1, 2, 3]);
    let right = set(&[3, 4, 5]);
    let observer = set(&[]);

    atomically(|| {
        observer.insert(1);
        observer.insert(2);
        left.insert(9);
        right.remove(&3);
    });

    assert_eq!(observer.len(), 2, "both writes landed");
    assert_eq!(contents(&left), vec![1, 2, 3, 9]);
    assert_eq!(contents(&right), vec![4, 5]);
}

#[test]
fn multiple_set_operations_work_together() {
    // TxHashSet.test.ts:346 — a composed run: mutate both sets, intersect, and
    // let the intersection's size decide a further write. This is the row that
    // checks the algebra composes rather than each op in isolation.
    let set1 = set(&["a", "b", "c"]);
    let set2 = set(&["c", "d", "e"]);

    assert!(set1.insert("f"));
    assert!(set2.remove(&"d"));

    let common = set1.intersection(&set2);
    if common.len() > 0 {
        assert!(set1.insert("shared"));
        assert!(set2.insert("shared"));
    }

    assert!(set1.contains(&"f"), ":346 asserts f landed on set1");
    assert!(set1.contains(&"shared"), "and the conditional write");
    assert!(!set2.contains(&"d"), "set2 lost d");
    assert!(set2.contains(&"shared"), "and gained the shared marker");
}
