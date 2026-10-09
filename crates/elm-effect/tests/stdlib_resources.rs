//! Wave-9 stdlib resources: RcMap, RcRef, ScopedRef, LayerMap, LayerRef,
//! Latch, Resource behavior ported from the tag's suites. Modules load via
//! `#[path]` includes (the .15 precedent): no facade touch.
//!
//! Each test pins a behavior GROUP; residuals stay `unported` and are
//! listed in the close report instead of follow-up beads.

#[path = "../src/stdlib/latch.rs"]
mod latch;
#[path = "../src/stdlib/layer_map.rs"]
mod layer_map;
#[path = "../src/stdlib/layer_ref.rs"]
mod layer_ref;
#[path = "../src/stdlib/rc_map.rs"]
mod rc_map;
#[path = "../src/stdlib/rc_ref.rs"]
mod rc_ref;
#[path = "../src/stdlib/resource.rs"]
mod resource;
#[path = "../src/stdlib/scoped_ref.rs"]
mod scoped_ref;

use latch::Latch;
use layer_map::LayerMap;
use layer_ref::LayerRef;
use rc_map::RcMap;
use rc_ref::RcRef;
use resource::Resource;
use scoped_ref::ScopedRef;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

// --- Latch ---

#[test]
fn latch_open_close_state() {
    let latch = Latch::new_closed();
    assert!(!latch.is_open());
    assert!(latch.open());
    assert!(!latch.open());
    assert!(latch.is_open());
    assert!(latch.close());
    assert!(!latch.close());
    assert!(!latch.is_open());
    assert!(Latch::new_open().is_open());
    assert!(Latch::new_closed().release());
}

#[test]
fn latch_waiter_wakes_on_open() {
    let latch = Arc::new(Latch::new_closed());
    let woken = counter();
    let woken_body = woken.clone();
    let moved = latch.clone();
    let admitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let admitted_body = admitted.clone();
    std::thread::spawn(move || {
        if moved.await_open(&|| false) {
            woken_body.fetch_add(1, Ordering::SeqCst);
        }
        admitted_body.store(true, Ordering::SeqCst);
    });
    std::thread::sleep(Duration::from_millis(50));
    assert!(latch.open());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !admitted.load(Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "waiter never admitted"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
    assert_eq!(woken.load(Ordering::SeqCst), 1);
}

#[test]
fn latch_interrupted_waiter_leaves() {
    let latch = Latch::new_closed();
    assert!(!latch.await_open(&|| true));
    assert_eq!(latch.waiters(), 0);
}

// --- Resource ---

#[test]
fn resource_acquires_refreshes_releases() {
    let acquired = counter();
    let released = counter();
    let a = acquired.clone();
    let r = released.clone();
    let res = Resource::manual(
        move || {
            a.fetch_add(1, Ordering::SeqCst);
            42
        },
        move |_| {
            r.fetch_add(1, Ordering::SeqCst);
        },
    );
    assert!(!res.is_acquired());
    assert_eq!(res.get(), 42);
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    assert_eq!(res.get(), 42);
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    assert_eq!(res.refresh(), 42);
    assert_eq!(acquired.load(Ordering::SeqCst), 2);
    assert_eq!(released.load(Ordering::SeqCst), 1);
    res.invalidate();
    assert!(!res.is_acquired());
    assert_eq!(released.load(Ordering::SeqCst), 2);
}

// --- ScopedRef ---

#[test]
fn scoped_ref_get_set() {
    let cell = ScopedRef::make(|| 1);
    assert_eq!(cell.get(), 1);
    cell.set(2);
    assert_eq!(cell.get(), 2);
    let built = ScopedRef::from_acquire(|| 7);
    assert_eq!(built.get(), 7);
}

// --- RcRef ---

#[test]
fn rcref_shares_and_deallocates() {
    let acquired = counter();
    let released = counter();
    let a = acquired.clone();
    let r = released.clone();
    let cell = RcRef::make(
        move || {
            a.fetch_add(1, Ordering::SeqCst);
            "v".to_owned()
        },
        move |_| {
            r.fetch_add(1, Ordering::SeqCst);
        },
    );
    let v1 = cell.get();
    let v2 = cell.get();
    assert_eq!(v1, "v");
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    assert_eq!(cell.ref_count(), 2);
    cell.release_ref();
    assert!(cell.is_acquired());
    cell.release_ref();
    assert_eq!(released.load(Ordering::SeqCst), 1);
    assert!(!cell.is_acquired());
    assert_eq!(v2, "v");
    cell.invalidate();
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn rcref_idle_ttl_reuses_then_releases() {
    let acquired = counter();
    let a = acquired.clone();
    let cell = RcRef::make(
        move || {
            a.fetch_add(1, Ordering::SeqCst);
            1
        },
        |_| {},
    );
    cell.set_idle_ttl(Some(Duration::from_millis(50)));
    assert_eq!(cell.get(), 1);
    cell.release_ref();
    assert!(cell.is_acquired());
    assert!(cell.touch());
    assert_eq!(cell.get(), 1);
    cell.release_ref();
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(cell.get(), 1);
    cell.release_ref();
    assert_eq!(acquired.load(Ordering::SeqCst), 2);
}

// --- RcMap ---

fn test_map() -> RcMap<String, i32, String> {
    RcMap::make(|k: &String| Ok(k.len() as i32), |_| {})
}

#[test]
fn rcmap_lookup_shares_and_releases() {
    let mut map = test_map();
    assert_eq!(map.get(&"ab".to_owned()), Some(Ok(2)));
    assert_eq!(map.get(&"ab".to_owned()), Some(Ok(2)));
    assert_eq!(map.ref_count(&"ab".to_owned()), 2);
    assert_eq!(map.keys(), ["ab".to_owned()]);
    map.release_ref(&"ab".to_owned());
    map.release_ref(&"ab".to_owned());
    assert_eq!(map.ref_count(&"ab".to_owned()), 0);
    map.set_idle_ttl(Some(Duration::from_secs(30)));
}

#[test]
fn rcmap_failure_propagates_and_caches() {
    let calls = counter();
    let calls_body = calls.clone();
    let mut map = RcMap::make(
        move |k: &String| {
            calls_body.fetch_add(1, Ordering::SeqCst);
            if k == "bad" {
                Err("nope".to_owned())
            } else {
                Ok(1)
            }
        },
        |_: i32| {},
    );
    assert_eq!(map.get(&"bad".to_owned()), Some(Err("nope".to_owned())));
    assert_eq!(map.get(&"bad".to_owned()), Some(Err("nope".to_owned())));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    map.release_ref(&"bad".to_owned());
    map.release_ref(&"bad".to_owned());
}

#[test]
fn rcmap_invalidate_touch_close() {
    let mut map = test_map();
    assert_eq!(map.get(&"ab".to_owned()), Some(Ok(2)));
    assert!(map.touch(&"ab".to_owned()));
    assert!(!map.touch(&"zz".to_owned()));
    map.release_ref(&"ab".to_owned());
    assert!(map.invalidate(&"ab".to_owned()));
    assert!(!map.invalidate(&"ab".to_owned()));
    assert_eq!(map.size(), 0);
    assert_eq!(map.get(&"x".to_owned()), Some(Ok(1)));
    map.release_ref(&"x".to_owned());
    map.close();
    assert!(map.is_closed());
    assert_eq!(map.get(&"y".to_owned()), None);
}

#[test]
fn rcmap_capacity_evicts_idle() {
    let mut map = test_map();
    map.set_capacity(Some(2));
    for k in ["a", "b", "c"] {
        map.get(&k.to_owned());
        map.release_ref(&k.to_owned());
    }
    assert!(map.size() <= 2);
}

// --- LayerMap / LayerRef ---

#[test]
fn layermap_memoizes_and_invalidates() {
    let builds = counter();
    let builds_body = builds.clone();
    let mut map = LayerMap::make(move |k: &String| {
        builds_body.fetch_add(1, Ordering::SeqCst);
        k.len()
    });
    assert_eq!(map.get(&"ab".to_owned()), Some(2));
    assert_eq!(map.get(&"ab".to_owned()), Some(2));
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert!(map.invalidate(&"ab".to_owned()));
    assert_eq!(map.get(&"ab".to_owned()), Some(2));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert_eq!(map.keys(), ["ab".to_owned()]);
    map.set_idle_ttl(Some(Duration::from_secs(60)));
    assert!(map.touch(&"ab".to_owned()));
}

#[test]
fn layerref_builds_once_and_refreshes() {
    let builds = counter();
    let builds_body = builds.clone();
    let cell = LayerRef::make(move || {
        builds_body.fetch_add(1, Ordering::SeqCst);
        5
    });
    assert!(!cell.is_built());
    assert_eq!(cell.get(), 5);
    assert_eq!(cell.get(), 5);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    cell.refresh();
    assert!(!cell.is_built());
    assert_eq!(cell.get(), 5);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[test]
fn latch_is_open_tracks_open_and_close() {
    // Latch row "isOpen reflects the state of the latch": closed at
    // construction, open after open(), and back to closed after close().
    // The pin interposes a yield between open and the assertion, so the
    // state must survive a context switch, not just be readable inline.
    let latch = Latch::new_closed();
    assert!(!latch.is_open(), "a latch built closed is not open");

    assert!(latch.open(), "opening a closed latch flips it");
    // Give any parked/contending observer a chance to run before asserting.
    std::thread::yield_now();
    assert!(latch.is_open(), "the latch stays open across a yield");

    assert!(latch.close(), "closing an open latch flips it");
    assert!(!latch.is_open(), "the latch reads closed again");
}

#[test]
fn latch_open_is_idempotent_and_reports_the_flip() {
    // The return value distinguishes "I opened it" from "it was already
    // open" -- the pin's open/close return these same booleans.
    let latch = Latch::new_open();
    assert!(!latch.open(), "an already-open latch reports no flip");
    assert!(latch.close());
    assert!(!latch.close(), "an already-closed latch reports no flip");
}
