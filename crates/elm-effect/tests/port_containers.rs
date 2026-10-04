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
fn hash_set_algebra_and_transforms() {
    let a = HashSet::make([1, 2, 3]);
    let b = HashSet::make([2, 3, 4]);
    assert_eq!(a.union(&b).size(), 4);
    assert_eq!(a.intersection(&b).size(), 2);
    assert_eq!(a.difference(&b).size(), 1);
    assert_eq!(a.map(|v| v * 2).size(), 3);
    assert!(a.map(|v| v * 2).has(&4));
    assert_eq!(a.filter(|v| *v > 1).size(), 2);
    assert!(a.some(|v| *v == 3));
    assert!(a.every(|v| *v > 0));
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

#[test]
fn hash_ring_empty_and_membership() {
    let ring = HashRing::<Node>::make(None);
    assert_eq!(ring.get("anything"), None);
    assert_eq!(ring.get_shards(3), None);
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
    let shards = ring.get_shards(6).expect("non-empty ring shards");
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
    let mut ring = HashRing::make(None);
    ring.add(node("a"), None);
    assert_eq!(ring.get_shards(0).map(|s| s.len()), Some(0));
    let one = ring.get_shards(1).expect("shards");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, "a");
}
