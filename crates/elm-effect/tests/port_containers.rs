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
    let m = HashMap::make([("a".to_owned(), 1), ("b".to_owned(), 2), ("c".to_owned(), 3)]);

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
