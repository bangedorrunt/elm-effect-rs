//! RcMap rows the ledger excused as "RC internals not exposed".
//!
//! Each test witnesses one behavior row from `packages/effect/test/RcMap.test.ts`
//! at the effect@4.0.0 pin against the port in `../src/stdlib/rc_map.rs`. The
//! pin holds a value through a caller scope; this port models the reference
//! count explicitly, so "the caller scope closes" is `release_ref`.

#[path = "../src/stdlib/rc_map.rs"]
mod rc_map;

use rc_map::RcMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

/// RcMap.test.ts:163 `retains a ready entry until the caller scope closes`.
/// Two holders share one acquisition, and the release runs only when the last
/// one returns its reference.
#[test]
fn rcmap_refs_defer_the_release_until_the_last_holder() {
    let released = counter();
    let released_hook = released.clone();
    let mut map = RcMap::<String, String, String>::make(
        |k: &String| -> Result<String, String> { Ok(k.clone()) },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    );

    let owner = map.get(&"ready".to_owned());
    let borrower = map.get(&"ready".to_owned());
    assert_eq!(owner, Some(Ok("ready".to_owned())));
    assert_eq!(borrower, Some(Ok("ready".to_owned())));
    assert_eq!(map.ref_count(&"ready".to_owned()), 2);

    // The first holder closes: the entry stays, so nothing is released.
    map.release_ref(&"ready".to_owned());
    assert_eq!(released.load(Ordering::SeqCst), 0);

    // The last holder closes: the release runs exactly once.
    map.release_ref(&"ready".to_owned());
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

/// RcMap.test.ts:319 `deallocation`.
/// The lookup acquires on the first get (`acquired` grows) and the release runs
/// once, on the get's own reference, not on the lookup.
#[test]
fn rcmap_deallocation_releases_once_on_the_last_ref() {
    let acquired = counter();
    let released = counter();
    let acquired_hook = acquired.clone();
    let released_hook = released.clone();
    let mut map = RcMap::<String, String, String>::make(
        move |k: &String| -> Result<String, String> {
            acquired_hook.fetch_add(1, Ordering::SeqCst);
            Ok(k.clone())
        },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    );

    assert_eq!(acquired.load(Ordering::SeqCst), 0);
    assert_eq!(map.get(&"foo".to_owned()), Some(Ok("foo".to_owned())));
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    assert_eq!(released.load(Ordering::SeqCst), 0);

    map.release_ref(&"foo".to_owned());
    assert_eq!(released.load(Ordering::SeqCst), 1);

    // A second release on the same key has nothing left to release.
    map.release_ref(&"foo".to_owned());
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

/// RcMap.test.ts:466 `complex key`.
/// The key is selected by value, not by object identity: a fresh instance
/// naming the same field reads the retained entry.
#[test]
fn rcmap_complex_key_is_hashed_by_value() {
    #[derive(Clone, Hash, PartialEq, Eq, Debug)]
    struct Key {
        id: u32,
    }

    let lookups = counter();
    let lookups_hook = lookups.clone();
    let mut map = RcMap::<Key, u32, String>::make(
        move |k: &Key| -> Result<u32, String> {
            lookups_hook.fetch_add(1, Ordering::SeqCst);
            Ok(k.id)
        },
        |_: u32| {},
    );

    assert_eq!(map.get(&Key { id: 1 }), Some(Ok(1)));
    // A different instance of the same value is the same key.
    assert_eq!(map.get(&Key { id: 1 }), Some(Ok(1)));
    assert_eq!(map.ref_count(&Key { id: 1 }), 2);
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
}
