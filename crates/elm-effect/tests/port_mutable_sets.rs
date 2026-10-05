//! hi5 tail slice: mutable-vs-persistent set distinction.
//! Ports packages/effect/test/MutableHashSet.test.ts (Effect mutable hash
//! sets are distinguished from native sets), left uncovered by the area
//! beads. Modules load via `#[path]` like port_containers.rs: no facade
//! touch.
#![cfg(feature = "tokio")]

#[path = "../src/containers/hash_map.rs"]
mod hash_map;
#[path = "../src/containers/hash_set.rs"]
mod hash_set;
#[path = "../src/containers/mutable_hash_set.rs"]
mod mutable_hash_set;

use hash_set::HashSet;
use mutable_hash_set::MutableHashSet;

// MutableHashSet.test.ts:6 — the Effect mutable set is a different kind
// from the persistent set: same membership, but updates apply in place
// through a shared reference instead of returning a new set.
#[test]
fn mutable_set_distinguishes_from_persistent_set() {
    let mutable = MutableHashSet::make([1, 2, 3]);
    let persistent = HashSet::make([1, 2, 3]);
    assert!(mutable.has(&2));
    assert!(persistent.has(&2));
    assert_eq!(mutable.size(), persistent.size());
    let shared = &mutable;
    assert!(shared.add(4), "in-place add through a shared reference");
    assert!(mutable.has(&4));
    assert_eq!(mutable.size(), 4);
    let grown = persistent.add(4);
    assert!(grown.has(&4));
    assert!(!persistent.has(&4), "persistent add leaves the original alone");
    assert_eq!(persistent.size(), 3);
}
