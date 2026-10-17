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

use latch::{Await, Latch, Waiter};
use layer_map::{IdlePolicy, IdlePolicyFn, LayerMap, LayerMapService};
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
//
// The pin resumes a waiter through Effect's `Scheduler`: a `release` admits
// the waiters parked at that moment into a pending queue, and a scheduler
// tick resumes them. The module keeps that split (`run_pending` is the
// tick) because four of the eight rows assert it.

fn park(latch: &Latch) -> Waiter {
    match latch.await_open(&|| false) {
        Await::Waiting(waiter) => waiter,
        other => panic!("expected a parked waiter, got {other:?}"),
    }
}

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
    let latch = Latch::new_closed();
    let waiter = park(&latch);
    assert!(latch.open());
    assert_eq!(latch.run_pending(), vec![waiter]);
    assert!(latch.is_open(), "open admits future waiters too");
}

#[test]
fn latch_interrupted_waiter_leaves() {
    let latch = Latch::new_closed();
    assert_eq!(latch.await_open(&|| true), Await::Interrupted);
    assert_eq!(latch.waiters(), 0);
}

#[test]
fn latch_release_wakes_current_waiters_and_stays_closed() {
    // Latch row "release wakes current waiters and keeps the latch closed":
    // the waiter is parked (not resumed) until the release's resume runs,
    // and the latch reads closed afterwards.
    let latch = Latch::new_closed();
    let waiter = park(&latch);
    assert_eq!(latch.pending(), 0, "a parked waiter holds no resume");
    assert!(latch.release(), "releasing a closed latch reports it");
    assert_eq!(latch.pending(), 1, "the release queued the resume");
    assert!(!latch.is_open(), "release does not open the latch");
    assert_eq!(latch.run_pending(), vec![waiter]);
}

#[test]
fn latch_open_close_does_not_resume_late_waiters() {
    // Latch row "open then close does not resume waiters registered after
    // close": the open admitted the waiter parked before it; the close makes
    // the later one suspend again.
    let latch = Latch::new_closed();
    let before = park(&latch);
    assert!(latch.open());
    assert!(latch.close());
    let _after = park(&latch);
    assert_eq!(latch.run_pending(), vec![before]);
    assert!(latch.run_pending().is_empty(), "late waiter got no resume");
    assert_eq!(latch.waiters(), 1, "and it is still parked");
}

#[test]
fn latch_release_while_a_flush_is_pending_covers_new_waiters() {
    // Latch row "release while a flush is pending covers the new waiters":
    // the second release adds its waiter to the flush already pending
    // instead of queueing a second tick, and one tick resumes both.
    let latch = Latch::new_closed();
    let first = park(&latch);
    assert!(latch.release());
    assert_eq!(latch.pending(), 1);
    let second = park(&latch);
    assert!(latch.release());
    assert_eq!(latch.pending(), 2, "same tick covers the second waiter");
    assert_eq!(latch.run_pending(), vec![first, second]);
    assert!(!latch.is_open());
}

#[test]
fn latch_release_does_not_resume_waiters_registered_after_it() {
    // Latch row "release does not resume waiters registered after the
    // release": coverage is decided when release runs, not when the resume
    // does.
    let latch = Latch::new_closed();
    let covered = park(&latch);
    assert!(latch.release());
    let _late = park(&latch);
    assert_eq!(latch.run_pending(), vec![covered]);
    assert!(latch.run_pending().is_empty(), "late waiter got no resume");
    assert_eq!(latch.waiters(), 1);
}

#[test]
fn latch_open_unsafe_does_not_resume_waiters_after_a_reentrant_close() {
    // Latch row "openUnsafe does not resume waiters registered after a
    // reentrant close": the release already covered the only parked waiter,
    // so `openUnsafe` queues no second resume; the resumed waiter's
    // continuation closes the latch again, and the waiter it parks then
    // stays suspended.
    let latch = Latch::new_closed();
    let first = park(&latch);
    assert!(latch.release());
    assert!(latch.open_unsafe(), "openUnsafe flips the latch");
    assert_eq!(latch.pending(), 1, "no second resume was queued");
    assert_eq!(latch.run_pending(), vec![first]);
    assert!(latch.close_unsafe(), "the resumed waiter closes it again");
    let _second = park(&latch);
    assert!(
        latch.run_pending().is_empty(),
        "no resume for the new waiter"
    );
    assert_eq!(latch.waiters(), 1);
}

#[test]
fn latch_interrupting_a_waiter_removes_it_from_a_pending_flush() {
    // Latch row "interrupting a waiter removes it from a pending flush":
    // the queued resume finds an empty flush, so the waiter's body never
    // runs and the latch stays closed.
    let latch = Latch::new_closed();
    let waiter = park(&latch);
    assert!(latch.release());
    assert_eq!(latch.pending(), 1);
    assert!(latch.interrupt(waiter), "the waiter was parked");
    assert!(latch.run_pending().is_empty(), "nothing was resumed");
    assert_eq!(latch.waiters(), 0);
    assert!(!latch.is_open());
}

#[test]
fn latch_await_is_interruptible_and_cleans_up_interrupted_waiters() {
    // Latch row "await is interruptible and cleans up interrupted waiters":
    // an interrupted waiter leaves no resume behind, so a release with no
    // waiters queues nothing; a later waiter gets a resume of its own.
    let latch = Latch::new_closed();
    let interrupted = park(&latch);
    assert!(latch.interrupt(interrupted));
    assert_eq!(latch.waiters(), 0);
    assert!(latch.release());
    assert_eq!(latch.pending(), 0, "no waiter, no queued resume");
    assert!(latch.run_pending().is_empty());
    let live = park(&latch);
    assert!(latch.release());
    assert_eq!(latch.pending(), 1);
    assert_eq!(latch.run_pending(), vec![live]);
    assert!(!latch.is_open());
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

#[test]
fn resource_guard_identifies_resources() {
    // Resource.test.ts:11 — `isResource` accepts a resource and rejects a
    // plain `Set`. Rust types are the guard, so a foreign value cannot reach
    // the predicate and the positive case is the whole check.
    let res = Resource::manual(|| 0, |_| {});
    assert!(res.is_resource());
}

#[test]
fn resource_manual_refresh_updates_the_cached_value() {
    // Resource.test.ts:18 — after the source changes, refresh caches the new
    // value and every later get returns it.
    let source = std::sync::Arc::new(std::sync::Mutex::new(0));
    let read = source.clone();
    let res = Resource::manual(move || *read.lock().expect("source"), |_| {});
    assert_eq!(res.get(), 0);
    *source.lock().expect("source") = 1;
    assert_eq!(res.refresh(), 1);
    assert_eq!(res.get(), 1);
}

#[test]
fn resource_refresh_releases_before_it_acquires() {
    // Resource.test.ts:32 — the row's claim is that a refresh gives up the
    // previous acquisition. Its body drives that through `Resource.auto` on a
    // TestClock schedule; auto-refresh policies are this port's documented
    // residual (resource.rs header), so what is pinned here is the order the
    // row asserts around: release the old value, then acquire the new one,
    // and hand back the fresh value.
    let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let acquired = order.clone();
    let released = order.clone();
    let res = Resource::manual(
        move || {
            acquired.lock().expect("order").push("acquire");
            7
        },
        move |_| {
            released.lock().expect("order").push("release");
        },
    );
    assert_eq!(res.get(), 7);
    assert_eq!(res.refresh(), 7);
    assert_eq!(
        *order.lock().expect("order"),
        ["acquire", "release", "acquire"],
        "release lands between the two acquisitions"
    );
    assert_eq!(res.get(), 7);
    assert_eq!(
        order.lock().expect("order").len(),
        3,
        "the refreshed value is cached, not re-acquired per get"
    );
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

#[test]
fn layer_map_does_not_preload_with_default_idle_ttl() {
    // LayerMap row "make does not preload with the default idle TTL": with
    // no preload keys and the default TTL, nothing is built -- no key is
    // acquired and so none is released either.
    let built = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let built_factory = built.clone();
    let map = LayerMap::make_with_preload(
        move |key: &String| {
            built_factory.lock().unwrap().push(key.clone());
            key.clone()
        },
        None,
    );
    assert!(built.lock().unwrap().is_empty(), "no key is preloaded");
    assert!(map.keys().is_empty(), "and the map holds nothing");
}

#[test]
fn layer_map_preloads_listed_keys_eagerly() {
    // The companion shape: with preload keys, exactly those keys are built
    // up front. Without this, the row above cannot distinguish "preload is
    // off" from "preload is broken", which is the failure mode that makes a
    // negative row untrustworthy on its own.
    let built = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let built_factory = built.clone();
    let mut map = LayerMap::make_with_preload(
        move |key: &String| {
            built_factory.lock().unwrap().push(key.clone());
            key.clone()
        },
        Some(vec!["a".to_string(), "b".to_string()]),
    );
    assert_eq!(built.lock().unwrap().clone(), vec!["a", "b"]);
    assert_eq!(map.keys().len(), 2);
    // A second get does not rebuild: preloaded entries are memoized.
    assert_eq!(map.get(&"a".to_string()), Some("a".to_string()));
    assert_eq!(built.lock().unwrap().clone(), vec!["a", "b"]);
}

// --- LayerMap idle policy (LayerMap.test.ts rows) ---

/// The rows' acquire/release bookkeeping: which keys were built and which
/// were released, in order.
#[derive(Clone, Default)]
struct MapLogs {
    acquired: Arc<std::sync::Mutex<Vec<String>>>,
    released: Arc<std::sync::Mutex<Vec<String>>>,
}

impl MapLogs {
    fn acquired(&self) -> Vec<String> {
        self.acquired.lock().unwrap().clone()
    }

    fn released(&self) -> Vec<String> {
        self.released.lock().unwrap().clone()
    }

    /// A constructor that records the key it built, over a value that is the
    /// key text (so a release names the same key).
    fn build(&self) -> impl Fn(&String) -> Result<String, String> + Send + Sync + 'static {
        let acquired = self.acquired.clone();
        move |key: &String| {
            acquired.lock().unwrap().push(key.clone());
            Ok(key.clone())
        }
    }

    fn release(&self) -> impl Fn(&String) + Send + Sync + 'static {
        let released = self.released.clone();
        move |value: &String| released.lock().unwrap().push(value.clone())
    }
}

fn key(text: &str) -> String {
    text.to_owned()
}

// LayerMap.test.ts:19 — `contextEffectOption` answers from the cache and
// never builds, including from a scope that only borrows.
#[test]
fn layer_map_option_does_not_build_missing_entries() {
    let logs = MapLogs::default();
    let mut map = LayerMap::try_make_idle(
        logs.build(),
        logs.release(),
        Vec::new(),
        Some(Box::new(|_: &String| IdlePolicy::Never)),
    )
    .expect("nothing is preloaded, so nothing can fail");

    assert_eq!(map.context_effect_option(&key("key")), None);
    assert!(logs.acquired().is_empty(), "an option read builds nothing");

    // The owner scope builds and holds; the borrower only reads.
    assert_eq!(map.context_effect(&key("key")), Some(key("key")));
    assert_eq!(map.context_effect_option(&key("key")), Some(key("key")));
    assert_eq!(logs.acquired(), ["key"], "the entry is built once");
}

// LayerMap.test.ts:40 — the same rule through `LayerMap.Service`.
#[test]
fn layer_map_service_option_does_not_build_missing_entries() {
    let logs = MapLogs::default();
    let service = LayerMapService::install(
        "LayerMapTest/ContextEffectOption",
        logs.build(),
        logs.release(),
        Vec::new(),
        Some(Box::new(|_: &String| IdlePolicy::Never)),
    )
    .expect("nothing is preloaded");

    assert_eq!(service.context_effect_option(&key("key")), None);
    assert!(logs.acquired().is_empty(), "an option read builds nothing");

    assert_eq!(service.context_effect(&key("key")), Some(key("key")));
    assert_eq!(service.context_effect_option(&key("key")), Some(key("key")));
    assert_eq!(logs.acquired(), ["key"], "the entry is built once");
    assert_eq!(service.name(), "LayerMapTest/ContextEffectOption");
}

// LayerMap.test.ts:75 — preload skips a zero-TTL key, a finite TTL releases
// its key when the idle window closes, and an infinite one never does.
#[test]
fn layer_map_preloads_only_keys_with_non_zero_idle_ttl() {
    let logs = MapLogs::default();
    let mut map = LayerMap::try_make_idle(
        logs.build(),
        logs.release(),
        vec![key("zero"), key("finite"), key("infinite")],
        Some(Box::new(|key: &String| match key.as_str() {
            "zero" => IdlePolicy::Keep,
            "finite" => IdlePolicy::Finite(Duration::from_millis(1000)),
            _ => IdlePolicy::Never,
        })),
    )
    .expect("the preloads succeed");

    assert_eq!(logs.acquired(), ["finite", "infinite"]);
    assert!(logs.released().is_empty());

    // A scoped read reuses the preloaded entry instead of rebuilding.
    assert_eq!(map.scoped(&key("finite")), Some(key("finite")));
    assert_eq!(logs.acquired(), ["finite", "infinite"]);

    map.advance(Duration::from_millis(999));
    assert!(logs.released().is_empty(), "999ms is inside the window");
    map.advance(Duration::from_millis(1));
    assert_eq!(logs.released(), ["finite"], "the window closed at 1000ms");

    assert_eq!(map.scoped(&key("infinite")), Some(key("infinite")));
    assert_eq!(
        logs.acquired(),
        ["finite", "infinite"],
        "the infinite entry is still cached"
    );
    assert_eq!(logs.released(), ["finite"]);
}

// LayerMap.test.ts:99 — a preload that fails fails construction.
#[test]
fn layer_map_make_fails_when_a_preload_fails() {
    let failure = LayerMap::try_make_idle(
        |key: &String| Err::<String, String>(key.clone()),
        |_: &String| {},
        vec![key("broken")],
        Some(Box::new(|_: &String| {
            IdlePolicy::Finite(Duration::from_millis(1000))
        })),
    );
    assert_eq!(failure.err(), Some(key("broken")));

    // The zero-TTL key in the same position is skipped, so it cannot fail
    // the build: that is the difference the policy makes.
    let skipped = LayerMap::try_make_idle(
        |key: &String| Err::<String, String>(key.clone()),
        |_: &String| {},
        vec![key("broken")],
        Some(Box::new(|_: &String| IdlePolicy::Keep)),
    );
    assert!(
        skipped.is_ok(),
        "a skipped preload cannot fail construction"
    );
}

// LayerMap.test.ts:108 — a per-key TTL function: each key expires on its own
// window rather than one map-wide TTL.
#[test]
fn layer_map_supports_dynamic_idle_ttl() {
    let logs = MapLogs::default();
    let mut map = LayerMap::try_make_idle(
        logs.build(),
        logs.release(),
        Vec::new(),
        Some(Box::new(|key: &String| {
            if key.starts_with("short:") {
                IdlePolicy::Finite(Duration::from_millis(500))
            } else {
                IdlePolicy::Finite(Duration::from_millis(2000))
            }
        })),
    )
    .expect("nothing is preloaded");

    assert_eq!(map.scoped(&key("short:a")), Some(key("short:a")));
    assert_eq!(map.scoped(&key("long:b")), Some(key("long:b")));
    assert_eq!(logs.acquired(), ["short:a", "long:b"]);
    assert!(logs.released().is_empty());

    map.advance(Duration::from_millis(500));
    assert_eq!(logs.released(), ["short:a"]);

    map.advance(Duration::from_millis(1500));
    assert_eq!(logs.released(), ["short:a", "long:b"]);
}

// LayerMap.test.ts:129 — `fromRecord` keeps the same per-key TTL rule over a
// fixed set of layers.
#[test]
fn layer_map_from_record_supports_dynamic_idle_ttl() {
    let logs = MapLogs::default();
    let ctor = |name: &str| {
        let logs = logs.clone();
        let name = name.to_owned();
        Box::new(move || {
            logs.acquired.lock().unwrap().push(name.clone());
            name.clone()
        }) as Box<dyn Fn() -> String + Send + Sync>
    };
    let mut map = LayerMap::from_record(
        vec![(key("short"), ctor("short")), (key("long"), ctor("long"))],
        logs.release(),
        Some(Box::new(|key: &String| {
            if key == "short" {
                IdlePolicy::Finite(Duration::from_millis(500))
            } else {
                IdlePolicy::Finite(Duration::from_millis(2000))
            }
        })),
    );

    assert_eq!(map.scoped(&key("short")), Some(key("short")));
    assert_eq!(map.scoped(&key("long")), Some(key("long")));
    assert_eq!(logs.acquired(), ["short", "long"]);
    assert!(logs.released().is_empty());

    map.advance(Duration::from_millis(500));
    assert_eq!(logs.released(), ["short"]);

    map.advance(Duration::from_millis(1500));
    assert_eq!(logs.released(), ["short", "long"]);

    // A key the record does not carry is not this map's to build.
    assert_eq!(map.context_effect(&key("other")), None);
    assert_eq!(logs.acquired(), ["short", "long"]);
}

// LayerMap.test.ts:159 — both service forms carry the dynamic TTL rule.
#[test]
fn layer_map_service_supports_dynamic_idle_ttl() {
    let logs = MapLogs::default();
    let short_window = || {
        Some(Box::new(|key: &String| {
            if key.starts_with("short:") || key == "short" {
                IdlePolicy::Finite(Duration::from_millis(500))
            } else {
                IdlePolicy::Finite(Duration::from_millis(2000))
            }
        }) as IdlePolicyFn<String>)
    };

    let lookup = LayerMapService::install(
        "LayerMapTest/LookupMap",
        logs.build(),
        logs.release(),
        Vec::new(),
        short_window(),
    )
    .expect("nothing is preloaded");
    let record_ctor = |name: &str| {
        let logs = logs.clone();
        let name = name.to_owned();
        Box::new(move || {
            logs.acquired.lock().unwrap().push(name.clone());
            name.clone()
        }) as Box<dyn Fn() -> String + Send + Sync>
    };
    let record = LayerMapService::install_record(
        "LayerMapTest/RecordMap",
        vec![(key("short"), record_ctor("short"))],
        logs.release(),
        short_window(),
    );

    assert_eq!(lookup.scoped(&key("short:a")), Some(key("short:a")));
    assert_eq!(record.scoped(&key("short")), Some(key("short")));
    assert_eq!(logs.acquired(), ["short:a", "short"]);
    assert!(logs.released().is_empty());

    lookup.advance(Duration::from_millis(500));
    record.advance(Duration::from_millis(500));
    assert_eq!(logs.released(), ["short:a", "short"]);
}
