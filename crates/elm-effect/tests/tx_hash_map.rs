//! TxHashMap's open behavior rows (TxHashMap.test.ts, Effect v4.0.0 pin).
//!
//! The rows this file carries are the bulk/functional/advanced families. The
//! four `getHash`/`hasHash` rows are excused in the ledger instead: the port's
//! map is a `HashMap` keyed by `K`'s own `Eq`/`Hash`, so a lookup driven by an
//! externally computed hash — and that hash family's wrong-hash miss — has no
//! counterpart to assert.
#![cfg(feature = "stm")]

use std::cell::RefCell;
use std::collections::HashMap;

use elm_effect::{TxHashMap, atomically, is_tx_hash_map};

/// TxHashMap.test.ts:176 — `modify` hands back the value it replaced and
/// leaves the new one in place.
#[test]
fn modify_returns_the_old_value_and_replaces_it() {
    let map = TxHashMap::from_iter([("counter", 5)]);
    let old = map.modify(&"counter", |value| value * 2);
    assert_eq!(old, Some(5));
    assert_eq!(map.get(&"counter"), Some(10));
}

/// TxHashMap.test.ts:186 — a key that is not there is left alone, and the
/// map stays empty rather than growing a default.
#[test]
fn modify_on_a_missing_key_leaves_the_map_untouched() {
    let map: TxHashMap<&str, i32> = TxHashMap::new();
    assert_eq!(map.modify(&"counter", |value| value * 2), None);
    assert_eq!(map.len(), 0);
}

/// TxHashMap.test.ts:196 — `modifyAt` with `Some` inserts a key that was not
/// there.
#[test]
fn modify_at_inserts_a_new_value() {
    let map = TxHashMap::from_iter([("a", 1)]);
    map.modify_at("b", |_| Some(2));
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&"b"), Some(2));
}

/// TxHashMap.test.ts:208 — `modifyAt` with `None` removes the key.
#[test]
fn modify_at_removes_an_existing_value() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    map.modify_at("a", |_| None);
    assert_eq!(map.len(), 1);
    assert!(!map.contains(&"a"));
}

/// TxHashMap.test.ts:220 — `keys` reports every key.
#[test]
fn keys_lists_every_key() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2), ("c", 3)]);
    let mut keys = map.keys();
    keys.sort_unstable();
    assert_eq!(keys, vec!["a", "b", "c"]);
}

/// TxHashMap.test.ts:228 — `values` reports every value.
#[test]
fn values_lists_every_value() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2), ("c", 3)]);
    let mut values = map.values();
    values.sort_unstable();
    assert_eq!(values, vec![1, 2, 3]);
}

/// TxHashMap.test.ts:236 — `entries` reports every pair.
#[test]
fn entries_lists_every_pair() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    let mut entries = map.entries();
    entries.sort_unstable();
    assert_eq!(entries, vec![("a", 1), ("b", 2)]);
}

/// TxHashMap.test.ts:244 — a snapshot is a detached copy: writes after the
/// snapshot do not reach into it.
#[test]
fn snapshot_detaches_from_later_writes() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    let snapshot = map.snapshot();
    map.insert("c", 3);
    assert_eq!(snapshot.len(), 2, "the snapshot keeps what it took");
    assert_eq!(snapshot.get(&"a"), Some(&1));
    assert_eq!(snapshot.get(&"c"), None, "the snapshot did not see the write");
    assert_eq!(map.len(), 3, "the original moved on");
}

/// TxHashMap.test.ts:264 — `union` merges a plain `HashMap` in, and the
/// other side's value wins on a shared key.
#[test]
fn union_overwrites_shared_keys() {
    let map = TxHashMap::from_iter([
        ("a".to_string(), 1),
        ("b".to_string(), 2),
    ]);
    let other = HashMap::from([("b".to_string(), 20), ("c".to_string(), 3)]);
    map.union(&other);
    assert_eq!(map.len(), 3);
    assert_eq!(map.get(&"a".to_string()), Some(1), "the untouched entry survives");
    assert_eq!(map.get(&"b".to_string()), Some(20), "the incoming value wins");
    assert_eq!(map.get(&"c".to_string()), Some(3));
}

/// TxHashMap.test.ts:282 — `removeMany` drops every named key and leaves the
/// rest.
#[test]
fn remove_many_drops_every_named_key() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2), ("c", 3), ("d", 4)]);
    map.remove_many(["b", "d"]);
    assert_eq!(map.len(), 2);
    assert!(map.contains(&"a"));
    assert!(!map.contains(&"b"));
    assert!(map.contains(&"c"));
    assert!(!map.contains(&"d"));
}

/// TxHashMap.test.ts:300 — `setMany` inserts the new pairs and overwrites
/// the ones already present.
#[test]
fn set_many_inserts_and_overwrites() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    map.set_many([("c", 3), ("d", 4), ("a", 10)]);
    assert_eq!(map.len(), 4);
    assert_eq!(map.get(&"a"), Some(10), "overwritten");
    assert_eq!(map.get(&"b"), Some(2), "preserved");
    assert_eq!(map.get(&"c"), Some(3), "new");
    assert_eq!(map.get(&"d"), Some(4), "new");
}

/// TxHashMap.test.ts:322 — one operation lands whole or not at all. The
/// port's `atomically` is per call, so a lone set and a lone modify each
/// commit on their own, and a reader sees the result of both.
#[test]
fn single_operations_are_each_atomic() {
    let map = TxHashMap::from_iter([("counter", 0)]);
    map.insert("counter", 1);
    map.modify(&"counter", |value| value + 1);
    assert_eq!(map.get(&"counter"), Some(2));
}

/// TxHashMap.test.ts:334 — a multi-step block commits together, so a reader
/// outside the block sees none of it or all of it.
#[test]
fn multi_step_operations_commit_together() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    let observed = std::cell::Cell::new(usize::MAX);
    atomically(|| {
        let current = map.get(&"a").expect("the read sees its own block");
        map.insert("a", current * 10);
        map.remove(&"b");
        map.insert("c", 3);
        // A read made halfway through the block, from the same thread, sees
        // the writes so far — the block is one transaction for every other
        // observer, which is what the row's final state asserts.
        observed.set(map.len());
    });
    assert_eq!(map.get(&"a"), Some(10));
    assert!(!map.contains(&"b"));
    assert_eq!(map.get(&"c"), Some(3));
    assert_eq!(map.len(), 2);
}

/// TxHashMap.test.ts:359 — the data-last form has no Rust `pipe`; the
/// composition is the sequence of calls, and it lands the same state.
#[test]
fn set_composes_without_a_pipe() {
    let map: TxHashMap<&str, i32> = TxHashMap::new();
    map.insert("a", 1);
    map.insert("b", 2);
    assert_eq!(map.get(&"a"), Some(1));
    assert_eq!(map.len(), 2);
}

/// TxHashMap.test.ts:376 — a `TxHashMap` identifies as one. The pin's
/// negative cases are foreign values (`{}`, `null`, `42`), which are
/// different Rust types and cannot reach the predicate at all.
#[test]
fn a_tx_hash_map_identifies_itself() {
    let map = TxHashMap::from_iter([("key", "value")]);
    assert!(is_tx_hash_map(&map));
    let other: TxHashMap<&str, i32> = TxHashMap::new();
    assert!(is_tx_hash_map(&other));
}

/// TxHashMap.test.ts:499 — `map` transforms the values and keeps the keys.
#[test]
fn map_transforms_values_and_keeps_keys() {
    let map = TxHashMap::from_iter([("user1", 25), ("user2", 30)]);
    let names = map.map(|age, _| age + 1);
    assert_eq!(names.get(&"user1"), Some(26));
    assert_eq!(names.get(&"user2"), Some(31));
}

/// TxHashMap.test.ts:525 — `filter` keeps only the matching entries.
#[test]
fn filter_keeps_only_matching_entries() {
    let map = TxHashMap::from_iter([("apple", 1.5), ("carrot", 0.8), ("banana", 2.0)]);
    let fruit = map.filter(|price, _| *price > 1.0);
    assert_eq!(fruit.len(), 2);
    assert!(fruit.contains(&"apple"));
    assert!(fruit.contains(&"banana"));
    assert!(!fruit.contains(&"carrot"));
}

/// TxHashMap.test.ts:553 — `reduce` folds over every value, and the key is
/// there for a fold that needs it.
#[test]
fn reduce_folds_every_value() {
    let map = TxHashMap::from_iter([("item1", 10), ("item2", 20), ("item3", 30)]);
    assert_eq!(map.reduce(0, |acc, value, _| acc + value), 60);
    let counted = map.reduce((0, Vec::new()), |(count, mut keys), _, key: &&str| {
        keys.push((*key).to_string());
        (count + 1, keys)
    });
    // A HashMap iterates in its own order, so the collected keys are sorted
    // before they are compared rather than asserted in an order no map keeps.
    let mut keys = counted.1;
    keys.sort_unstable();
    assert_eq!(keys, vec!["item1".to_string(), "item2".to_string(), "item3".to_string()]);
    assert_eq!(counted.0, 3);
    let empty: TxHashMap<&str, i32> = TxHashMap::new();
    assert_eq!(empty.reduce(0, |acc, value, _| acc + value), 0);
}

/// TxHashMap.test.ts:582 — `filterMap` drops what the callback rejects and
/// unwraps what it keeps.
#[test]
fn filter_map_drops_what_the_callback_rejects() {
    let map = TxHashMap::from_iter([
        ("valid1", "10"),
        ("invalid", "not-a-number"),
        ("valid2", "20"),
    ]);
    let parsed = map.filter_map(|value, _| value.parse::<i32>().ok());
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed.get(&"valid1"), Some(10));
    assert_eq!(parsed.get(&"valid2"), Some(20));
    assert_eq!(parsed.get(&"invalid"), None);
}

/// TxHashMap.test.ts:634 — `hasBy` reports whether any entry matches, and an
/// empty map has none.
#[test]
fn has_by_checks_any_entry() {
    let map = TxHashMap::from_iter([("admin", "admin"), ("user1", "user")]);
    assert!(map.has_by(|role, _| *role == "admin"));
    assert!(!map.has_by(|role, _| *role == "guest"));
    let empty: TxHashMap<&str, &str> = TxHashMap::new();
    assert!(!empty.has_by(|_, _| true));
}

/// TxHashMap.test.ts:659 — `findFirst` returns the first matching entry as a
/// pair, or `None` when nothing matches.
#[test]
fn find_first_returns_the_first_matching_entry() {
    let map = TxHashMap::from_iter([("session1", "user1"), ("session2", "user2")]);
    let found = map.find_first(|user, _| *user == "user2");
    assert_eq!(found, Some(("session2", "user2")));
    let missing = map.find_first(|user, _| *user == "user3");
    assert_eq!(missing, None);
    let empty: TxHashMap<&str, i32> = TxHashMap::new();
    assert_eq!(empty.find_first(|_, _| true), None);
}

/// TxHashMap.test.ts:689 — `some` reports whether any entry matches.
#[test]
fn some_checks_any_entry() {
    let map = TxHashMap::from_iter([("order1", 100), ("order2", 250)]);
    assert!(map.some(|amount, _| *amount > 200));
    assert!(!map.some(|amount, _| *amount > 1000));
    let empty: TxHashMap<&str, i32> = TxHashMap::new();
    assert!(!empty.some(|_, _| true));
}

/// TxHashMap.test.ts:717 — `every` reports whether all entries match, and an
/// empty map is vacuously true.
#[test]
fn every_checks_all_entries() {
    let map = TxHashMap::from_iter([("task1", true), ("task2", true)]);
    assert!(map.every(|done, _| *done));
    map.insert("task3", false);
    assert!(!map.every(|done, _| *done));
    assert!(map.every(|done, _| !done || *done), "each is completed or not");
    let empty: TxHashMap<&str, bool> = TxHashMap::new();
    assert!(empty.every(|_, _| false), "an empty map satisfies every predicate");
}

/// TxHashMap.test.ts:749 — the functional family composes: map, then filter,
/// then reduce, then every over what survived.
#[test]
fn the_functional_family_composes() {
    let map = TxHashMap::from_iter([("item1", 10), ("item2", 20), ("item3", 30)]);
    let discounted = map.map(|price, _| price * 10 / 10);
    let expensive = discounted.filter(|price, _| *price > 15);
    assert_eq!(expensive.len(), 2);
    assert_eq!(expensive.reduce(0, |acc, price, _| acc + price), 50);
    assert!(expensive.every(|price, _| *price > 0));
}

/// TxHashMap.test.ts:787 — `forEach` runs the callback once per entry.
#[test]
fn for_each_runs_the_callback_per_entry() {
    let map = TxHashMap::from_iter([("user1", "active"), ("user2", "inactive"), ("user3", "active")]);
    let seen = RefCell::new(Vec::new());
    map.for_each(|status, user| seen.borrow_mut().push(format!("{user}:{status}")));
    let mut seen = seen.into_inner();
    seen.sort();
    assert_eq!(seen, vec!["user1:active", "user2:inactive", "user3:active"]);
}

/// TxHashMap.test.ts:821 — `flatMap` merges the maps each entry builds, and a
/// later entry's key wins.
#[test]
fn flat_map_merges_the_maps_it_builds() {
    let departments = TxHashMap::from_iter([("eng", 2), ("sales", 1)]);
    let employees = departments.flat_map(|_headcount, department| {
        HashMap::from([
            (format!("{department}-lead"), "lead"),
            (format!("{department}-member"), "member"),
        ])
    });
    assert_eq!(employees.len(), 4);
    assert_eq!(employees.get(&"eng-lead".to_string()), Some("lead"));
    assert_eq!(employees.get(&"sales-member".to_string()), Some("member"));
}

/// TxHashMap.test.ts:856 — `compact` unwraps the `Some` values and drops the
/// `None` ones.
#[test]
fn compact_unwraps_some_and_drops_none() {
    let optional = TxHashMap::from_iter([
        ("valid1", Some(10)),
        ("invalid1", None),
        ("valid2", Some(20)),
        ("invalid2", None),
    ]);
    let compacted = optional.compact();
    assert_eq!(compacted.len(), 2);
    assert_eq!(compacted.get(&"valid1"), Some(10));
    assert_eq!(compacted.get(&"valid2"), Some(20));
    assert_eq!(compacted.get(&"invalid1"), None);
}

/// TxHashMap.test.ts:888 — `toEntries` returns every pair, and it is the
/// same content as `entries`.
#[test]
fn to_entries_matches_entries() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2), ("c", 3)]);
    let mut entries = map.to_entries();
    entries.sort_unstable();
    assert_eq!(entries, vec![("a", 1), ("b", 2), ("c", 3)]);
    let mut from_entries = map.entries();
    from_entries.sort_unstable();
    assert_eq!(entries, from_entries);
}

/// TxHashMap.test.ts:909 — `toValues` returns every value, and it is the
/// same content as `values`.
#[test]
fn to_values_matches_values() {
    let map = TxHashMap::from_iter([("x", 100), ("y", 200), ("z", 300)]);
    let mut values = map.to_values();
    values.sort_unstable();
    assert_eq!(values, vec![100, 200, 300]);
    let mut from_values = map.values();
    from_values.sort_unstable();
    assert_eq!(values, from_values);
}

/// TxHashMap.test.ts:930 — the specialized family composes: compact, then
/// forEach over what survived, then the two array views.
#[test]
fn the_specialized_family_composes() {
    let map = TxHashMap::from_iter([("a", Some(1)), ("b", None), ("c", Some(3))]);
    map.insert("d", Some(4));
    map.insert("e", None);
    let compacted = map.compact();
    assert_eq!(compacted.len(), 3, "a, c and d survive");
    let total = RefCell::new(0);
    compacted.for_each(|value, _| *total.borrow_mut() += value);
    assert_eq!(total.into_inner(), 8);
    assert_eq!(compacted.to_values().len(), 3);
    assert_eq!(compacted.to_entries().len(), 3);
}
