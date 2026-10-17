//! Wave-5 containers: HashMap, HashSet, MutableHashMap, MutableHashSet,
//! PrimaryKey, Trie, HashRing behavior ported from the tag's suites.
//! Modules load via `#[path]` includes (the .15 precedent): no facade
//! touch, no scope leak into lib.rs.
//!
//! Each test pins a behavior GROUP; residuals stay `unported` and are
//! listed in the close report instead of follow-up beads.

#[path = "../src/containers/hash_map.rs"]
mod hash_map;
#[path = "../src/containers/hash_ring.rs"]
mod hash_ring;
#[path = "../src/containers/hash_set.rs"]
mod hash_set;
#[path = "../src/containers/mutable_hash_map.rs"]
mod mutable_hash_map;
#[path = "../src/containers/mutable_hash_set.rs"]
mod mutable_hash_set;
#[path = "../src/containers/primary_key.rs"]
mod primary_key;
#[path = "../src/containers/trie.rs"]
mod trie;

use hash_map::HashMap;
use hash_ring::HashRing;
use hash_set::HashSet;

// `Equal` and `Hash` are reached by the pin's own `#[path]` include
// convention, the same route `port_containers` already uses for the container
// modules, so the test asserts the traits the library implements rather than a
// name the facade happens to re-export.
#[path = "../src/foundation.rs"]
mod foundation;

// Structural hash and equality for the map. These live beside the test rather
// than inside `hash_map.rs` because that file is `#[path]`-included into
// several test targets, so a `crate::foundation::` path inside it only resolves
// in the targets that also include `foundation.rs`.
impl<K, V> foundation::Hash for HashMap<K, V>
where
    K: std::hash::Hash + Eq,
    V: std::hash::Hash,
{
    fn hash_value(&self) -> u32 {
        // The per-entry hashes are combined commutatively (a wrapping sum)
        // because a std HashMap does not iterate in a defined order: two maps
        // holding the same entries can iterate differently depending on
        // insertion history, so hashing the iteration sequence would make
        // equal maps hash unequally.
        use std::hash::Hasher as _;
        let mut total: u32 = 0;
        for (key, value) in self.entries() {
            let mut entry = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(key, &mut entry);
            std::hash::Hash::hash(value, &mut entry);
            total = total.wrapping_add(entry.finish() as u32);
        }
        total
    }
}

impl<K, V> foundation::Equal for HashMap<K, V>
where
    K: std::hash::Hash + Eq,
    V: std::hash::Hash + PartialEq,
{
    fn equal(&self, other: &Self) -> bool {
        if self.size() != other.size() {
            return false;
        }
        self.entries()
            .all(|(key, value)| other.get(key) == Some(value))
    }
}
use mutable_hash_map::MutableHashMap;
use mutable_hash_set::MutableHashSet;
use primary_key::{PrimaryKey, value};
use trie::Trie;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Node {
    id: String,
}

impl PrimaryKey for Node {
    fn primary_key(&self) -> String {
        self.id.clone()
    }
}

fn node(id: &str) -> Node {
    Node { id: id.to_owned() }
}

// --- HashMap ---

#[test]
fn hash_map_empty_make_from_iterable() {
    let e: HashMap<String, i32> = HashMap::empty();
    assert!(e.is_empty());
    assert_eq!(e.size(), 0);
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    assert_eq!(m.size(), 2);
    assert_eq!(m.get(&"a".to_owned()), Some(&1));
    let f = HashMap::from_iter([("x".to_owned(), 9)]);
    assert_eq!(f.get(&"x".to_owned()), Some(&9));
}

#[test]
fn hash_map_set_remove_persist() {
    let m = HashMap::make([("a".to_owned(), 1)]);
    let m2 = m.set("b".to_owned(), 2);
    assert_eq!(m.size(), 1);
    assert_eq!(m2.size(), 2);
    let m3 = m2.remove(&"a".to_owned());
    assert!(m2.has(&"a".to_owned()));
    assert!(!m3.has(&"a".to_owned()));
    let m4 = m2.set_many([("c".to_owned(), 3), ("d".to_owned(), 4)]);
    assert_eq!(m4.size(), 4);
    let m5 = m4.remove_many(["c".to_owned(), "d".to_owned()]);
    assert_eq!(m5.size(), 2);
    // HashMap.test.ts:104 — removing a missing key changes nothing.
    let m6 = m2.remove(&"zzz".to_owned());
    assert_eq!(m6.size(), 2);
    assert!(m6.has(&"a".to_owned()));
    // :235 — bulk ops chain; :244 — any iterable works.
    assert_eq!(
        m2.set_many(vec![("e".to_owned(), 5)])
            .remove_many(["e".to_owned()])
            .size(),
        2
    );
}

#[test]
fn hash_map_views_and_union() {
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort();
    assert_eq!(keys, [&"a".to_owned(), &"b".to_owned()]);
    let mut vals: Vec<&i32> = m.values().collect();
    vals.sort();
    assert_eq!(vals, [&1, &2]);
    assert_eq!(m.entries().count(), 2);
    let u = m.union(&HashMap::make([("b".to_owned(), 20), ("c".to_owned(), 3)]));
    assert_eq!(u.get(&"b".to_owned()), Some(&20));
    assert_eq!(u.size(), 3);
}

#[test]
fn hash_map_modify_and_modify_at() {
    // HashMap.test.ts :378/:388/:394/:401.
    //
    // modifyAt takes (key, Option<V> -> Option<V>): Some replaces, None
    // removes an existing key and is a no-op on a missing one. `modify` is
    // the special case that maps the present value.
    //
    // :378 modifyAt - existing key replaces the value.
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    let replaced = m.modify_at("a".to_owned(), |current| Some(current.unwrap_or(&0) + 10));
    assert_eq!(replaced.get(&"a".to_owned()), Some(&11));
    assert_eq!(replaced.get(&"b".to_owned()), Some(&2));
    // The source is untouched, matching the pin's persistent map.
    assert_eq!(m.get(&"a".to_owned()), Some(&1));

    // :394 modifyAt - None removes an existing key.
    let removed = m.modify_at("a".to_owned(), |_| None);
    assert!(!removed.has(&"a".to_owned()));
    assert_eq!(removed.size(), 1);

    // :388 modifyAt - None on a non-existing key changes nothing.
    let missing = m.modify_at("zzz".to_owned(), |_| None);
    assert!(!missing.has(&"zzz".to_owned()));
    assert_eq!(missing.size(), 2);
    assert!(missing.has(&"a".to_owned()));

    // :401 modify multiplies the value at an existing key.
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    let tripled = m.modify("a".to_owned(), |v| v * 3);
    assert_eq!(tripled.get(&"a".to_owned()), Some(&3));
    assert_eq!(tripled.get(&"b".to_owned()), Some(&2));
}

#[test]
fn hash_map_reduce_and_for_each() {
    // HashMap.test.ts :410 reduce and :416 forEach.
    //
    // reduce folds (acc, value, key); forEach visits every entry. The pin's
    // forEach passes (value, key), so the collected pairs are sorted before
    // comparing because HashMap's iteration order is not defined.
    let m = HashMap::make([
        ("a".to_owned(), 1),
        ("b".to_owned(), 2),
        ("c".to_owned(), 3),
    ]);

    let sum = m.reduce(0, |acc, value, _key| acc + value);
    assert_eq!(sum, 6);

    // :416 forEach visits each key and value, in the pin's (value, key) order.
    let mut seen: Vec<(String, i32)> = Vec::new();
    m.for_each(|value, key| seen.push((key.clone(), *value)));
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("a".to_owned(), 1),
            ("b".to_owned(), 2),
            ("c".to_owned(), 3)
        ]
    );
}

#[test]
fn hash_map_filter_map_keeps_some_entries() {
    // HashMap.test.ts :320 filterMap. A filter success maps the value through,
    // a failure drops the entry, which is the pin's Result.failVoid arm.
    let m = HashMap::make([
        ("a".to_owned(), 1),
        ("b".to_owned(), 2),
        ("c".to_owned(), 3),
        ("d".to_owned(), 4),
    ]);
    let doubled = m.filter_map(|_key, value| (value % 2 == 0).then(|| value * 2));
    assert_eq!(doubled.size(), 2);
    assert_eq!(doubled.get(&"b".to_owned()), Some(&4));
    assert_eq!(doubled.get(&"d".to_owned()), Some(&8));
    assert!(!doubled.has(&"a".to_owned()));
    assert!(!doubled.has(&"c".to_owned()));
}

#[test]
fn hash_map_handles_many_inserts_lookups_and_removals() {
    // HashMap.test.ts :618 — a thousand inserts, then a lookup range, then
    // removals. Pure stress against the existing set/has/get/remove surface.
    let mut m: HashMap<i32, String> = HashMap::empty();
    for i in 0..1000 {
        m = m.set(i, format!("value{i}"));
    }
    assert_eq!(m.size(), 1000);
    for i in 500..1000 {
        assert!(m.has(&i), "missing {i}");
        assert_eq!(m.get(&i), Some(&format!("value{i}")));
    }
    for i in 0..500 {
        m = m.remove(&i);
    }
    assert_eq!(m.size(), 500);
    for i in 0..500 {
        assert!(!m.has(&i));
    }
}

#[test]
fn hash_map_handles_hash_collisions() {
    // HashMap.test.ts :650 — keys whose Hash all write the same value. Three
    // entries survive, are addressable individually, and removing one leaves
    // the other two. This is the row that would catch a map that keyed on the
    // hash rather than on (hash, Eq).
    #[derive(Clone, Debug)]
    struct CollidingKey {
        id: i32,
    }
    impl PartialEq for CollidingKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }
    impl Eq for CollidingKey {}
    impl std::hash::Hash for CollidingKey {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            // Every key hashes identically, forcing collisions.
            42i32.hash(state);
        }
    }

    let (key1, key2, key3) = (
        CollidingKey { id: 1 },
        CollidingKey { id: 2 },
        CollidingKey { id: 3 },
    );
    let mut m: HashMap<CollidingKey, String> = HashMap::empty();
    m = m.set(key1.clone(), "value1".to_owned());
    m = m.set(key2.clone(), "value2".to_owned());
    m = m.set(key3.clone(), "value3".to_owned());

    assert_eq!(m.size(), 3, "colliding keys must stay distinct entries");
    assert_eq!(m.get(&key1), Some(&"value1".to_owned()));
    assert_eq!(m.get(&key2), Some(&"value2".to_owned()));
    assert_eq!(m.get(&key3), Some(&"value3".to_owned()));

    // Removing one colliding key must not disturb the others.
    m = m.remove(&key2);
    assert_eq!(m.size(), 2);
    assert!(m.has(&key1));
    assert!(!m.has(&key2));
    assert!(m.has(&key3));
}

#[test]
fn hash_map_equal_and_hash_ignore_insertion_order() {
    // HashMap.test.ts :480 Equal.equals and :489 Hash.hash. Two maps built in
    // different orders are structurally equal, and their hashes match, so
    // neither equality nor hashing may depend on insertion order.
    use foundation::{Equal as _, Hash as _};

    let forward = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    let reversed = HashMap::make([("b".to_owned(), 2), ("a".to_owned(), 1)]);
    let different = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 3)]);

    assert!(forward.equal(&reversed), "insertion order must not matter");
    assert!(!forward.equal(&different), "a different value is not equal");

    // A hash has to combine the per-entry hashes commutatively: a std HashMap
    // does not iterate in a defined order, so hashing the iteration sequence
    // would make the two orders disagree even when they are equal.
    assert_eq!(
        forward.hash_value(),
        reversed.hash_value(),
        "equal maps must hash equally"
    );
    assert_ne!(
        forward.hash_value(),
        different.hash_value(),
        "different maps should not normally collide"
    );
}

#[test]
fn hash_map_works_with_equal_objects_as_keys() {
    // HashMap.test.ts :512. A key type whose equality and hash are structural
    // makes two distinct instances the same key: the map is built with one
    // instance and read with a different one holding the same data.
    #[derive(Clone, Debug)]
    struct Person {
        name: String,
        age: i32,
    }
    impl PartialEq for Person {
        fn eq(&self, other: &Self) -> bool {
            self.name == other.name && self.age == other.age
        }
    }
    impl Eq for Person {}
    impl std::hash::Hash for Person {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.name.hash(state);
            self.age.hash(state);
        }
    }

    let alice = Person {
        name: "Alice".to_owned(),
        age: 25,
    };
    let alice_again = Person {
        name: "Alice".to_owned(),
        age: 25,
    };
    let bob = Person {
        name: "Bob".to_owned(),
        age: 30,
    };

    let map: HashMap<Person, &str> = HashMap::make([(alice, "value1"), (bob.clone(), "value3")]);
    assert_eq!(map.get(&alice_again), Some(&"value1"));
    assert_eq!(map.get(&bob), Some(&"value3"));
}

#[test]
fn hash_map_keeps_entries_addressable_when_bit_31_is_present() {
    // HashMap.test.ts :543. Keys whose Hash all write the same fixed value
    // force the map's collision path. The pin uses hashes 31 and 30, which
    // exercise the bit-31 branch of its indexed-leaf insert; Rust's std map
    // has no such branch, but the observable these rows assert is the one
    // that matters: entries at the same hash stay distinct and addressable.
    //
    // This is the row that would catch a map keyed on the hash rather than on
    // (hash, Eq), which is the same invariant the :650 collision row asserts
    // through a single constant hash.
    #[derive(Clone, Debug)]
    struct FixedHashKey {
        id: &'static str,
        hash: u32,
    }
    impl PartialEq for FixedHashKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id && self.hash == other.hash
        }
    }
    impl Eq for FixedHashKey {}
    impl std::hash::Hash for FixedHashKey {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.hash.hash(state);
        }
    }

    let bit31 = FixedHashKey {
        id: "bit31",
        hash: 31,
    };
    let bit30 = FixedHashKey {
        id: "bit30",
        hash: 30,
    };
    let mut m: HashMap<FixedHashKey, &str> = HashMap::empty();
    m = m.set(bit31.clone(), "session1");
    m = m.set(bit30.clone(), "session2");

    assert_eq!(m.size(), 2, "distinct keys must stay distinct entries");
    assert_eq!(m.get(&bit31), Some(&"session1"));
    assert_eq!(m.get(&bit30), Some(&"session2"));
}

#[test]
fn hash_map_keeps_entries_addressable_when_merge_leaves_sees_bit_31() {
    // HashMap.test.ts :556. Two keys collide on the SAME fixed hash (31) and a
    // third sits at a different one (30). Both colliding entries and the
    // distinct one must all remain addressable, which is the case where a map
    // that deduplicated on hash alone would silently drop one of them.
    #[derive(Clone, Debug)]
    struct FixedHashKey {
        id: &'static str,
        hash: u32,
    }
    impl PartialEq for FixedHashKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id && self.hash == other.hash
        }
    }
    impl Eq for FixedHashKey {}
    impl std::hash::Hash for FixedHashKey {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.hash.hash(state);
        }
    }

    let collision_a = FixedHashKey {
        id: "collisionA",
        hash: 31,
    };
    let collision_b = FixedHashKey {
        id: "collisionB",
        hash: 31,
    };
    let bit30 = FixedHashKey {
        id: "bit30",
        hash: 30,
    };
    let mut m: HashMap<FixedHashKey, &str> = HashMap::empty();
    m = m.set(collision_a.clone(), "a");
    m = m.set(collision_b.clone(), "b");
    m = m.set(bit30.clone(), "c");

    assert_eq!(m.size(), 3);
    assert_eq!(m.get(&collision_a), Some(&"a"));
    assert_eq!(m.get(&collision_b), Some(&"b"));
    assert_eq!(m.get(&bit30), Some(&"c"));
}

#[test]
fn hash_map_keeps_entries_addressable_across_bit31_heavy_shapes() {
    // HashMap.test.ts :572, the property form over random bit-31-heavy shapes.
    //
    // The pin runs 200 fast-check cases, each an array of hashes in 0..=31
    // prefixed with [31, 31, 30], with unique ids, inserting every entry and
    // asserting each one is still addressable. The property is reproduced
    // deterministically with a small LCG so the shapes are varied and
    // reproducible without a property-testing dependency: the invariant is
    // that NO shape loses an entry, not that one particular shape works.
    #[derive(Clone, Debug)]
    struct FixedHashKey {
        id: String,
        hash: u32,
    }
    impl PartialEq for FixedHashKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }
    impl Eq for FixedHashKey {}
    impl std::hash::Hash for FixedHashKey {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.hash.hash(state);
        }
    }

    // A small deterministic generator: same sequence every run, so a failure
    // is reproducible rather than dependent on the test harness's RNG.
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for run in 0..200u32 {
        // 0..=80 hashes, matching the pin's minLength/maxLength.
        let count = (next() % 81) as usize;
        let mut hashes: Vec<u32> = vec![31, 31, 30];
        for _ in 0..count {
            hashes.push((next() % 32) as u32);
        }
        let mut m: HashMap<FixedHashKey, String> = HashMap::empty();
        let mut inserted: Vec<(FixedHashKey, String)> = Vec::with_capacity(hashes.len());
        for (i, hash) in hashes.iter().enumerate() {
            let id = format!("{run}-{i}");
            let key = FixedHashKey {
                id: id.clone(),
                hash: *hash,
            };
            let value = format!("value-{id}");
            m = m.set(key.clone(), value.clone());
            inserted.push((key, value));
        }
        assert_eq!(m.size(), inserted.len(), "run {run} lost an entry");
        for (key, value) in &inserted {
            assert!(m.has(key), "run {run} lost {:?}", key.id);
            assert_eq!(m.get(key), Some(value), "run {run} changed {:?}", key.id);
        }
    }
}

#[test]
fn hash_map_mutate_and_begin_mutation() {
    // HashMap.test.ts :427/:442/:459.
    //
    // mutate applies a batch of in-place updates and returns the new map. A
    // mutation replaces the held map rather than mutating through it, so the
    // source is untouched: that is the property all three rows assert.
    let source = HashMap::make([("a".to_owned(), 1)]);

    // :427 mutate sets two entries on top of one.
    let grown = source.mutate(|m| {
        m.set("b".to_owned(), 2);
        m.set("c".to_owned(), 3);
    });
    assert_eq!(source.size(), 1, "mutate must not touch the original");
    assert_eq!(grown.size(), 3);
    assert_eq!(grown.get(&"a".to_owned()), Some(&1));
    assert_eq!(grown.get(&"b".to_owned()), Some(&2));
    assert_eq!(grown.get(&"c".to_owned()), Some(&3));

    // :442 beginMutation produces a distinct instance, and its updates do not
    // reach the source. `into_map` freezes it.
    let mutable = source.begin_mutation();
    mutable.set("b".to_owned(), 2);
    assert_eq!(mutable.size(), 2);
    assert_eq!(source.size(), 1, "the source stays at one entry");
    assert_eq!(mutable.into_map().size(), 2);

    // :459 mutations are isolated from the original across set, remove and set
    // again, so a remove during the batch does not remove from the source.
    let original = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    let changed = original.mutate(|m| {
        m.set("a".to_owned(), 100);
        m.remove(&"b".to_owned());
        m.set("c".to_owned(), 3);
    });
    assert_eq!(original.get(&"a".to_owned()), Some(&1));
    assert_eq!(original.get(&"b".to_owned()), Some(&2));
    assert!(!original.has(&"c".to_owned()));
    assert_eq!(changed.get(&"a".to_owned()), Some(&100));
    assert!(!changed.has(&"b".to_owned()));
    assert_eq!(changed.get(&"c".to_owned()), Some(&3));
    // The source is still usable as a source for another mutation.
    assert_eq!(original.size(), 2);
}

#[test]
fn hash_map_transforms_and_predicates() {
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    assert_eq!(m.map(|v| v * 10).get(&"a".to_owned()), Some(&10));
    assert_eq!(m.filter(|_, v| *v > 1).size(), 1);
    assert!(m.some(|_, v| *v == 2));
    assert!(m.every(|_, v| *v > 0));
    assert!(!m.every(|_, v| *v > 1));
    let fm = m.filter_map(|_, v| if *v > 1 { Some(v * 100) } else { None });
    assert_eq!(fm.size(), 1);
    let o: HashMap<String, Option<i32>> =
        HashMap::make([("a".to_owned(), Some(1)), ("b".to_owned(), None)]);
    let c = o.compact();
    assert_eq!(c.size(), 1);
    assert_eq!(c.get(&"a".to_owned()), Some(&1));
}

// --- HashSet ---

#[test]
fn hash_set_add_remove_membership() {
    let s = HashSet::make([1, 2, 3]);
    assert!(s.has(&2));
    assert!(!s.has(&9));
    let s2 = s.add(4);
    assert_eq!(s.size(), 3);
    assert_eq!(s2.size(), 4);
    let s3 = s2.remove(&1);
    assert!(s2.has(&1));
    assert!(!s3.has(&1));
    assert!(HashSet::<i32>::empty().is_empty());
}

#[test]
fn hash_set_guard_identifies_sets() {
    // HashSet.test.ts:286 — `isHashSet` separates a set from the array and
    // object values the guard rejects. Rust's types are the guard, so the
    // negatives (an array, an object, null, undefined) cannot reach it at
    // all; the port pins the difference the guard exists to make — a set
    // answers membership by value and collapses a duplicate on add, a plain
    // array does neither.
    let set = HashSet::make(["a", "b", "c"]);
    assert!(set.has(&"a"));
    assert!(!set.has(&"z"));
    assert_eq!(set.size(), 3);
    assert_eq!(set.add("a").size(), 3, "a set de-duplicates on add");
    let mut array = vec!["a", "b", "c"];
    array.push("a");
    assert_eq!(array.len(), 4, "an array keeps the duplicate");
}

#[test]
fn hash_set_algebra_and_transforms() {
    let a = HashSet::make([1, 2, 3]);
    let b = HashSet::make([2, 3, 4]);
    assert_eq!(a.union(&b).size(), 4);
    assert_eq!(a.intersection(&b).size(), 2);
    assert_eq!(a.difference(&b).size(), 1);
    assert_eq!(a.map(|v| v * 2).size(), 3);
    assert!(a.map(|v| v * 2).has(&4));
    assert_eq!(a.map(|_| 0).size(), 1);
    assert_eq!(a.filter(|v| *v > 1).size(), 2);
    assert!(a.some(|v| *v == 3));
    assert!(a.every(|v| *v > 0));
}

#[test]
fn hash_set_subset_reduce_iterate() {
    let a = HashSet::make([1, 2]);
    let b = HashSet::make([1, 2, 3]);
    assert!(a.is_subset(&b));
    assert!(!b.is_subset(&a));
    assert!(a.is_subset(&a));
    assert!(HashSet::<i32>::empty().is_subset(&b));
    assert_eq!(b.reduce(0, |acc, v| acc + v), 6);
    assert_eq!(b.reduce(1, |acc, v| acc * v), 6);
    let mut seen: Vec<i32> = b.iter().copied().collect();
    seen.sort();
    assert_eq!(seen, vec![1, 2, 3]);
    let mut total = 0;
    for v in &b {
        total += v;
    }
    assert_eq!(total, 6);
}

#[test]
fn hash_set_equality_hash_membership() {
    let a = HashSet::make([1, 2, 3]);
    let b = HashSet::make([3, 2, 1]);
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));
    let c = HashSet::make([1, 2, 3, 4]);
    assert_ne!(hash_of(&a), hash_of(&c));
    #[derive(Clone, Debug)]
    struct Ci(String);
    impl PartialEq for Ci {
        fn eq(&self, other: &Self) -> bool {
            self.0.eq_ignore_ascii_case(&other.0)
        }
    }
    impl Eq for Ci {}
    impl std::hash::Hash for Ci {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.0.to_ascii_lowercase().hash(state);
        }
    }
    let s = HashSet::make([Ci("A".to_owned())]);
    assert!(s.has(&Ci("a".to_owned())));
    assert_eq!(s.add(Ci("a".to_owned())).size(), 1);
}

fn hash_of<T: std::hash::Hash>(v: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

// --- MutableHashMap ---

#[test]
fn mutable_hash_map_in_place() {
    let m = MutableHashMap::make([("a".to_owned(), 1)]);
    assert_eq!(m.get(&"a".to_owned()), Some(1));
    m.set("b".to_owned(), 2);
    assert_eq!(m.size(), 2);
    assert!(m.has(&"b".to_owned()));
    assert_eq!(m.remove(&"a".to_owned()), Some(1));
    assert!(!m.has(&"a".to_owned()));
    let mut keys = m.keys();
    keys.sort();
    assert_eq!(keys, ["b".to_owned()]);
    assert_eq!(m.values(), [2]);
    assert!(MutableHashMap::<String, i32>::empty().is_empty());
}

#[test]
fn mutable_hash_map_modify_from_iterate_and_pipe() {
    // MutableHashMap.test.ts:124 — fromIterable builds from entries.
    let m = MutableHashMap::make(vec![("a".to_owned(), 1), ("b".to_owned(), 2)]);
    assert_eq!(m.size(), 2);
    assert!(m.has(&"a".to_owned()));
    // :135 — iterate visits every entry.
    assert_eq!(m.keys().len(), 2);
    // :314 — modify updates existing keys, ignores missing ones.
    m.modify(&"a".to_owned(), |v| v + 10);
    assert_eq!(m.get(&"a".to_owned()), Some(11));
    m.modify(&"missing".to_owned(), |v| v + 1);
    assert_eq!(m.size(), 2);
    // MutableHashMap.test.ts:285 — set overwrites equal keys without growing.
    m.set("a".to_owned(), 99);
    assert_eq!(m.get(&"a".to_owned()), Some(99));
    assert_eq!(m.size(), 2);
    // :340 — pipe(): chained in-place ops equal a built map.
    let piped = MutableHashMap::<String, String>::empty();
    piped.set("key".to_owned(), "value".to_owned());
    assert_eq!(piped.get(&"key".to_owned()), Some("value".to_owned()));
    assert_eq!(piped.size(), 1);
}

#[test]
fn mutable_hash_map_runtime_shapes() {
    // MutableHashMap.test.ts:77 — `String(map)` renders the tagged map;
    // :98 — `inspect` is the same shape (Rust's spelling is `Debug`);
    // :89 — `toJSON` is the `values` shape. Entries render sorted by their
    // rendering because the backing map has no insertion order.
    let m = MutableHashMap::make([(0, "a"), (1, "b")]);
    assert_eq!(m.to_string(), "MutableHashMap([[0,\"a\"],[1,\"b\"]])");
    assert_eq!(format!("{m:?}"), "MutableHashMap([[0,\"a\"],[1,\"b\"]])");
    assert_eq!(
        m.json_shape(),
        "{\"_id\":\"MutableHashMap\",\"values\":[[0,\"a\"],[1,\"b\"]]}"
    );
}

#[test]
fn mutable_hash_map_guard_identifies_mutable_maps() {
    // MutableHashMap.test.ts:72 — `isMutableHashMap` accepts the mutable map
    // and rejects a plain JS `Map`. Rust types are the guard, so the negative
    // is a foreign value that cannot reach the predicate; what the guard is
    // for is pinned instead: the mutable map updates in place through a shared
    // reference, the persistent one hands back a new map.
    let mutable = MutableHashMap::make([("a".to_owned(), 1)]);
    let persistent = HashMap::make([("a".to_owned(), 1)]);
    mutable.set("b".to_owned(), 2);
    assert_eq!(mutable.size(), 2, "the shared map grew in place");
    let grown = persistent.set("b".to_owned(), 2);
    assert_eq!(persistent.size(), 1, "the persistent map did not");
    assert_eq!(grown.size(), 2);
}

// --- MutableHashSet ---

#[test]
fn mutable_hash_set_in_place() {
    let s = MutableHashSet::make([1, 2]);
    assert!(s.add(3));
    assert!(!s.add(3));
    assert_eq!(s.size(), 3);
    assert!(s.remove(&2));
    assert!(!s.has(&2));
    let mut vals = s.values();
    vals.sort();
    assert_eq!(vals, [1, 3]);
}

// --- PrimaryKey ---

#[test]
fn primary_key_value_contract() {
    assert_eq!(value(&node("db")), "db".to_owned());
    assert_eq!(value(&"svc".to_owned()), "svc".to_owned());
}

// --- Trie ---

#[test]
fn trie_make_insert_get() {
    let t = Trie::make([("app/db".to_owned(), 1), ("app/cache".to_owned(), 2)]);
    assert_eq!(t.get("app/db"), Some(&1));
    assert_eq!(t.get("app/missing"), None);
    assert!(t.has("app/cache"));
    assert_eq!(t.size(), 2);
    let t2 = Trie::from_iter([("x".to_owned(), 1)]);
    assert_eq!(t2.get("x"), Some(&1));
    assert!(Trie::<i32>::empty().is_empty());
}

#[test]
fn trie_remove_persists_and_prefix() {
    let t = Trie::make([("app/db".to_owned(), 1), ("app/cache".to_owned(), 2)]);
    let t2 = t.remove("app/db");
    assert!(t.has("app/db"));
    assert!(!t2.has("app/db"));
    assert_eq!(t2.size(), 1);
    let mut prefixed = t.keys_with_prefix("app/");
    prefixed.sort();
    assert_eq!(prefixed, ["app/cache".to_owned(), "app/db".to_owned()]);
    assert!(t.keys_with_prefix("zzz").is_empty());
    assert_eq!(t.entries().len(), 2);
}

// --- HashRing ---

/// Node with a payload we can tell apart from another node sharing its
/// primary key: rows :6 / :27 update a node in place.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RingNode {
    id: String,
    payload: &'static str,
}

impl PrimaryKey for RingNode {
    fn primary_key(&self) -> String {
        self.id.clone()
    }
}

fn ring_node(id: &str, payload: &'static str) -> RingNode {
    RingNode {
        id: id.to_owned(),
        payload,
    }
}

#[test]
fn hash_ring_add_replaces_node_for_the_same_primary_key() {
    // HashRing.test.ts:6 — re-adding an existing primary key updates the
    // stored node, so routing hands back the newest payload.
    let mut ring = HashRing::make(None);
    ring.add(ring_node("node", "first"), None);
    assert_eq!(ring.get("request").map(|n| n.payload), Some("first"));
    ring.add(ring_node("node", "updated"), None);
    assert_eq!(ring.get("request").map(|n| n.payload), Some("updated"));
    assert_eq!(ring.nodes().count(), 1, "one node per primary key");
}

#[test]
fn hash_ring_add_with_new_weight_replaces_the_stored_node() {
    // HashRing.test.ts:27 — a weight change re-points the node, and the
    // payload handed back is the updated one.
    let mut ring = HashRing::make(None);
    ring.add(ring_node("node", "first"), None);
    ring.add(ring_node("node", "updated"), Some(2.0));
    assert_eq!(ring.get("request").map(|n| n.payload), Some("updated"));
    assert_eq!(ring.nodes().count(), 1, "one node per primary key");
}

#[test]
fn hash_ring_empty_and_membership() {
    let ring = HashRing::<Node>::make(None);
    assert_eq!(ring.get("anything"), None);
    assert_eq!(ring.get_shards(3.0), None);
    let mut ring = HashRing::make(None);
    ring.add(node("a"), None);
    assert!(ring.has(&node("a")));
    assert!(!ring.has(&node("b")));
    assert_eq!(ring.get("k").as_ref().map(|n| n.id.as_str()), Some("a"));
    ring.remove(&node("a"));
    assert!(!ring.has(&node("a")));
    assert_eq!(ring.get("k"), None);
}

#[test]
fn hash_ring_routes_stably_and_shards_cover() {
    let mut ring = HashRing::make(None);
    ring.add_many([node("a"), node("b"), node("c")], None);
    let first: Vec<Option<String>> = ["k1", "k2", "k3", "k4"]
        .iter()
        .map(|k| ring.get(k).map(|n| n.id))
        .collect();
    // Deterministic: same inputs route the same way every time.
    let second: Vec<Option<String>> = ["k1", "k2", "k3", "k4"]
        .iter()
        .map(|k| ring.get(k).map(|n| n.id))
        .collect();
    assert_eq!(first, second);
    assert!(first.iter().all(|n| n.is_some()));
    // Shards cover every index exactly once across the node set.
    let shards = ring.get_shards(6.0).expect("non-empty ring shards");
    assert_eq!(shards.len(), 6);
    // Weighted caps: removing a node remaps only its shards.
    let before: Vec<String> = ["s1", "s2", "s3", "s4", "s5", "s6"]
        .iter()
        .map(|k| ring.get(k).map(|n| n.id).unwrap())
        .collect();
    ring.remove(&node("b"));
    let after: Vec<String> = ["s1", "s2", "s3", "s4", "s5", "s6"]
        .iter()
        .map(|k| ring.get(k).map(|n| n.id).unwrap())
        .collect();
    for (b, a) in before.iter().zip(after.iter()) {
        if b != "b" {
            assert_eq!(b, a);
        }
    }
    assert!(after.iter().all(|n| n != "b"));
}

#[test]
fn hash_ring_shard_count_normalizes() {
    // HashRing.test.ts:48 — the tag's count is a `number`: NaN, a negative
    // count, and anything under one mean zero shards, and a fractional count
    // truncates (1.9 -> 1, 2.9 -> 2).
    let mut ring = HashRing::make(None);
    ring.add(node("a"), None);
    assert_eq!(ring.get_shards(f64::NAN).map(|s| s.len()), Some(0));
    assert_eq!(ring.get_shards(-1.0).map(|s| s.len()), Some(0));
    assert_eq!(ring.get_shards(0.0).map(|s| s.len()), Some(0));
    assert_eq!(ring.get_shards(0.5).map(|s| s.len()), Some(0));
    let one = ring.get_shards(1.9).expect("shards");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, "a");
    assert_eq!(ring.get_shards(2.9).map(|s| s.len()), Some(2));
}

#[test]
fn hash_ring_shards_overflow_to_the_first_ring_entry() {
    // HashRing.test.ts:63 — with one ring entry per node, these keys make
    // every shard nearest `second`, so `first` at ring index 0 is allocated
    // only by the exclusion scan, which has to spiral back to the ring's
    // first entry to find it.
    let mut ring = HashRing::make(Some(1));
    ring.add(ring_node("node-0", "first"), None);
    ring.add(ring_node("node-1", "second"), None);
    // `get` hashes its input with the same function the shard count uses on
    // `shard-N`, so this is the node each shard sees first.
    for shard in 0..3 {
        let nearest = ring.get(&format!("shard-{shard}")).expect("nearest");
        assert_eq!(nearest.payload, "second", "shard-{shard} nearest");
    }
    let shards = ring.get_shards(3.0).expect("allocated");
    let payloads: Vec<&str> = shards.iter().map(|n| n.payload).collect();
    assert_eq!(payloads.len(), 3);
    assert_eq!(
        payloads.iter().filter(|p| **p == "first").count(),
        1,
        "the overflow reaches the first ring entry: {payloads:?}"
    );
    assert_eq!(
        payloads.iter().filter(|p| **p == "second").count(),
        2,
        "`second` keeps its one-shard cap plus the last shard: {payloads:?}"
    );
}

#[test]
fn trie_modify_many_and_longest_prefix() {
    // Trie.test.ts:480 - modify updates an existing key, ignores a missing one.
    let t = Trie::make([("app/db".to_owned(), 1)]);
    let bumped = t.modify("app/db", |v| v + 1);
    assert_eq!(bumped.get("app/db"), Some(&2));
    assert_eq!(t.get("app/db"), Some(&1));
    assert_eq!(t.modify("app/none", |v| v + 1).size(), 1);
    // :504 - insertMany inserts every entry.
    let many = t.insert_many([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    assert_eq!(many.size(), 3);
    // :491 - removeMany deletes every matching key.
    let pruned = many.remove_many(["app/db", "a"]);
    assert_eq!(pruned.size(), 1);
    assert!(pruned.has("b"));
    assert!(many.has("app/db"));
    // :376 / :389 - longestPrefixOf returns the longest stored key.
    let prefixed = Trie::make([
        ("ab".to_owned(), 1),
        ("abc".to_owned(), 2),
        ("zz".to_owned(), 3),
    ]);
    assert_eq!(
        prefixed.longest_prefix_of("abcd").map(|(k, _)| k),
        Some("abc".to_owned())
    );
    assert_eq!(
        prefixed.longest_prefix_of("abx").map(|(k, _)| k),
        Some("ab".to_owned())
    );
    assert_eq!(prefixed.longest_prefix_of("q"), None);
}

/// The pin's `Array.from(trie)`: entries as owned pairs, in iteration order.
fn trie_i32(entries: &[(&str, i32)]) -> Vec<(String, i32)> {
    entries.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn trie_pairs(trie: &Trie<i32>) -> Vec<(String, i32)> {
    trie.entries().into_iter().map(|(k, v)| (k, *v)).collect()
}

#[test]
fn trie_empty_iterates_nothing_and_insert_orders_by_key() {
    // :62 - an empty trie iterates no entries.
    let empty = Trie::<String>::empty();
    assert_eq!(empty.size(), 0);
    assert!(empty.entries().is_empty());

    // :69 - every insert returns a new trie, and entries come out in key
    // order rather than insertion order: "mid" is stored last and reads third.
    let t1 = Trie::empty().insert("call", 0);
    let t2 = t1.insert("me", 1);
    let t3 = t2.insert("mind", 2);
    let t4 = t3.insert("mid", 3);
    assert_eq!(trie_pairs(&t1), trie_i32(&[("call", 0)]));
    assert_eq!(trie_pairs(&t2), trie_i32(&[("call", 0), ("me", 1)]));
    assert_eq!(
        trie_pairs(&t3),
        trie_i32(&[("call", 0), ("me", 1), ("mind", 2)])
    );
    assert_eq!(
        trie_pairs(&t4),
        trie_i32(&[("call", 0), ("me", 1), ("mid", 3), ("mind", 2)])
    );
}

#[test]
fn trie_insert_replaces_without_mutating_or_growing() {
    // :84 - replacing a key leaves the source trie alone and keeps the size.
    let before = Trie::make([("a".to_owned(), 1)]);
    let after = before.insert("a", 2);
    assert_eq!(trie_pairs(&before), trie_i32(&[("a", 1)]));
    assert_eq!(before.size(), 1);
    assert_eq!(trie_pairs(&after), trie_i32(&[("a", 2)]));
    assert_eq!(after.size(), 1);
}

#[test]
fn trie_from_iterable_keeps_every_shape_and_orders_by_key() {
    // :95 - an empty iterable stays empty.
    let none: Vec<(String, i32)> = Vec::new();
    assert_eq!(trie_pairs(&Trie::from_iter(none)), Vec::new());

    // :114 - fromIterable orders by key, not by input order.
    let shuffled = Trie::from_iter([
        ("call".to_owned(), 0),
        ("me".to_owned(), 1),
        ("mind".to_owned(), 2),
        ("mid".to_owned(), 3),
    ]);
    assert_eq!(
        trie_pairs(&shuffled),
        trie_i32(&[("call", 0), ("me", 1), ("mid", 3), ("mind", 2)])
    );

    // :120 - an already sorted iterable comes back unchanged.
    let sorted = Trie::from_iter([("a".to_owned(), 0), ("b".to_owned(), 1)]);
    assert_eq!(trie_pairs(&sorted), trie_i32(&[("a", 0), ("b", 1)]));

    // :126 - a single entry.
    let single = Trie::from_iter([("a".to_owned(), 0)]);
    assert_eq!(trie_pairs(&single), trie_i32(&[("a", 0)]));

    // :132 - a key that prefixes another sorts before it.
    let prefixed = Trie::from_iter([("shells".to_owned(), 0), ("she".to_owned(), 1)]);
    assert_eq!(
        trie_pairs(&prefixed),
        trie_i32(&[("she", 1), ("shells", 0)])
    );
}

#[test]
fn trie_keeps_option_payloads_and_reads_only_complete_keys() {
    // :171 - a stored payload with no value inside it is still an entry.
    let trie = Trie::make([("a".to_owned(), None::<i32>)]);
    assert_eq!(trie.size(), 1);
    assert_eq!(trie.get("a"), Some(&None));
    assert_eq!(trie.entries().len(), 1);

    // :179 - a prefix of a stored key is not a stored key.
    let prefixed = Trie::empty()
        .insert("shells", 0)
        .insert("sells", 1)
        .insert("she", 2);
    assert_eq!(prefixed.get("sell"), None);
    assert_eq!(prefixed.get("sells"), Some(&1));
    assert_eq!(prefixed.get("shell"), None);
    assert_eq!(prefixed.get("she"), Some(&2));
}

#[test]
fn trie_get_unsafe_returns_or_panics() {
    // :209 - getUnsafe hands back the value, and refuses a key it does not
    // have with a tagged message rather than a silent default.
    let trie = Trie::empty().insert("call", 0).insert("me", 1);
    assert_eq!(*trie.get_unsafe("call"), 0);
    assert_eq!(*trie.get_unsafe("me"), 1);
    let absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| trie.get_unsafe("mae")));
    let message = absent.expect_err("a missing key is refused");
    let message = message
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| message.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(message.contains("Trie.getUnsafe"), "{message}");
}

#[test]
fn trie_keys_values_and_entries_come_out_in_key_order() {
    // :243 - keys are sorted, not insertion ordered.
    let trie = Trie::empty()
        .insert("cab", 0)
        .insert("abc", 1)
        .insert("bca", 2);
    assert_eq!(trie.keys(), ["abc", "bca", "cab"]);

    // :254 - a valued node sorts with its own extensions: "a" before "ab",
    // "ca" before "cac", and every branch of a level before the next.
    let nested = Trie::from_iter(
        [
            "abc", "bac", "b", "ca", "cac", "c", "abb", "ba", "a", "bca", "cab", "dca", "ab", "adc",
        ]
        .map(|k| (k.to_owned(), 0)),
    );
    assert_eq!(
        nested.keys(),
        [
            "a", "ab", "abb", "abc", "adc", "b", "ba", "bac", "bca", "c", "ca", "cab", "cac", "dca"
        ]
    );

    // :291 - values follow that same key order.
    let values = Trie::empty()
        .insert("call", 0)
        .insert("me", 1)
        .insert("and", 2);
    let ordered: Vec<i32> = values.values().into_iter().copied().collect();
    assert_eq!(ordered, [2, 0, 1]);

    // :302 / :312 - entries, and the materialized `toEntries` form of them,
    // are that same order as pairs.
    let pairs = Trie::empty().insert("call", 0).insert("me", 1);
    assert_eq!(trie_pairs(&pairs), trie_i32(&[("call", 0), ("me", 1)]));
}

#[test]
fn trie_prefix_readers_return_keys_values_and_entries() {
    // :322 - keysWithPrefix hands back every matching key, sorted.
    let wide = Trie::empty()
        .insert("she", 0)
        .insert("shells", 1)
        .insert("sea", 2)
        .insert("sells", 3)
        .insert("by", 4)
        .insert("the", 5)
        .insert("sea", 6)
        .insert("shore", 7);
    assert_eq!(wide.keys_with_prefix("she"), ["she", "shells"]);

    let narrow = Trie::empty()
        .insert("shells", 0)
        .insert("sells", 1)
        .insert("sea", 2)
        .insert("she", 3);

    // :338 - valuesWithPrefix pairs those keys with their values.
    let values: Vec<i32> = narrow
        .values_with_prefix("she")
        .into_iter()
        .copied()
        .collect();
    assert_eq!(values, [3, 0]);

    // :351 / :364 - entriesWithPrefix carries both, and is the array form.
    let entries: Vec<(String, i32)> = narrow
        .entries_with_prefix("she")
        .into_iter()
        .map(|(k, v)| (k, *v))
        .collect();
    assert_eq!(entries, trie_i32(&[("she", 3), ("shells", 0)]));
    assert!(narrow.entries_with_prefix("zzz").is_empty());
}

#[test]
fn trie_map_filter_filter_map_and_compact() {
    let trie = Trie::empty()
        .insert("shells", 0)
        .insert("sells", 1)
        .insert("she", 2);

    // :399 - map transforms values, and the key-aware form is handed the key.
    let by_value = trie.map(|v| v + 1);
    assert_eq!(
        trie_pairs(&by_value),
        trie_i32(&[("sells", 2), ("she", 3), ("shells", 1)])
    );
    let by_key = trie.map_with_key(|_, k| k.len() as i32);
    assert_eq!(
        trie_pairs(&by_key),
        trie_i32(&[("sells", 5), ("she", 3), ("shells", 6)])
    );

    // :422 - filter keeps entries by value, or by key.
    assert_eq!(
        trie_pairs(&trie.filter(|_, v| *v > 1)),
        trie_i32(&[("she", 2)])
    );
    assert_eq!(
        trie_pairs(&trie.filter(|k, _| k.len() > 3)),
        trie_i32(&[("sells", 1), ("shells", 0)])
    );

    // :442 - filterMap keeps the successes, by value or by key.
    assert_eq!(
        trie_pairs(&trie.filter_map(|_, v| (*v > 1).then_some(*v))),
        trie_i32(&[("she", 2)])
    );
    assert_eq!(
        trie_pairs(&trie.filter_map(|k, v| (k.len() > 3).then_some(*v))),
        trie_i32(&[("sells", 1), ("shells", 0)])
    );

    // :465 - compact drops the empty payloads and unwraps the rest.
    let options = Trie::empty()
        .insert("shells", Some(0))
        .insert("sells", None::<i32>)
        .insert("she", Some(2));
    assert_eq!(
        trie_pairs(&options.compact()),
        trie_i32(&[("she", 2), ("shells", 0)])
    );
}

#[test]
fn trie_reduce_and_for_each_visit_in_key_order() {
    let trie = Trie::empty()
        .insert("shells", 0)
        .insert("sells", 1)
        .insert("she", 2);

    // :524 - reduce folds left to right in key order, and its fold is handed
    // the key: the concatenation spells sells, she, shells.
    assert_eq!(trie.reduce(0, |acc, v, _| acc + v), 3);
    assert_eq!(trie.reduce(10, |acc, v, _| acc + v), 13);
    assert_eq!(
        trie.reduce(String::new(), |acc, _, k| acc + k),
        "sellssheshells"
    );

    // :551 - forEach visits the same order, value then key.
    let mut total = 0;
    trie.for_each(|v, k| total += v + k.len() as i32);
    assert_eq!(total, 17);
}

#[test]
fn trie_renders_entries_in_iteration_order() {
    let trie = Trie::empty().insert("a", 0).insert("b", 1);

    // :23 - toString is the class rendering, in key order.
    assert_eq!(trie.to_string(), "Trie([[\"a\",0],[\"b\",1]])");
    assert_eq!(format!("{trie:?}"), trie.to_string());

    // :36 - toJSON is the tagged object.
    assert_eq!(
        trie.json_shape(),
        "{\"_id\":\"Trie\",\"values\":[[\"a\",0],[\"b\",1]]}"
    );
}

#[test]
fn trie_equality_is_by_entries() {
    // :566 - two tries with the same entries are equal, including empty ones.
    assert_eq!(Trie::<i32>::empty(), Trie::empty());
    assert_eq!(
        Trie::make([("call".to_owned(), 0), ("me".to_owned(), 1)]),
        Trie::make([("call".to_owned(), 0), ("me".to_owned(), 1)])
    );

    // Either half of an entry can differ, and the size is checked before the
    // entries are walked, which is the part of :11 this port can build.
    assert_ne!(
        Trie::make([("call".to_owned(), 0)]),
        Trie::make([("call".to_owned(), 1)])
    );
    assert_ne!(Trie::<i32>::empty(), Trie::make([("a".to_owned(), 1)]));
}

#[test]
fn hash_map_unsafe_and_flat_map() {
    // HashMap.test.ts:111 - getUnsafe returns existing values.
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2)]);
    assert_eq!(*m.get_unsafe(&"a".to_owned()), 1);
    assert_eq!(*m.get_unsafe(&"b".to_owned()), 2);
    // :117 - getUnsafe panics with a tagged message when absent.
    let single = HashMap::make([("a".to_owned(), 1)]);
    let absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        single.get_unsafe(&"b".to_owned())
    }));
    let message = absent.unwrap_err();
    let message = message
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| message.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(message.contains("HashMap.getUnsafe"));
    // :285 - flatMap merges one map per entry.
    let flat = m.flat_map(|v, k| HashMap::make([(k.clone() + "1", *v), (k.clone() + "2", v * 2)]));
    assert_eq!(flat.size(), 4);
    assert_eq!(flat.get(&"a1".to_owned()), Some(&1));
    assert_eq!(flat.get(&"a2".to_owned()), Some(&2));
    assert_eq!(flat.get(&"b1".to_owned()), Some(&2));
    assert_eq!(flat.get(&"b2".to_owned()), Some(&4));
}

#[test]
fn hash_map_find_compact_and_keyed() {
    // HashMap.test.ts:346/352 - findFirst returns the entry or nothing.
    let m = HashMap::make([
        ("a".to_owned(), 1),
        ("b".to_owned(), 2),
        ("c".to_owned(), 3),
    ]);
    assert_eq!(
        m.find_first(|_, v| *v > 2).map(|(k, v)| (k.clone(), *v)),
        Some(("c".to_owned(), 3))
    );
    assert_eq!(m.find_first(|_, v| *v > 5), None);
    // :307 - compact drops Nones.
    let opt = HashMap::make([("a".to_owned(), Some(1)), ("b".to_owned(), None)]);
    let flat = opt.compact();
    assert_eq!(flat.size(), 1);
    assert_eq!(flat.get(&"a".to_owned()), Some(&1));
    // :330 - filterMap can use the key.
    let keyed = m.filter_map(|k, v| if k == "b" { Some(v * 10) } else { None });
    assert_eq!(keyed.get(&"b".to_owned()), Some(&20));
    assert_eq!(keyed.size(), 1);
    // :370 - hasBy checks key and value together.
    assert!(m.has_by(|k, v| k == "b" && *v == 2));
    assert!(!m.has_by(|k, v| k == "b" && *v == 5));
}
