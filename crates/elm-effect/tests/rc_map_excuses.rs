//! RcMap rows the ledger excused as "RC internals not exposed".
//!
//! Each test witnesses one behavior row from `packages/effect/test/RcMap.test.ts`
//! at the effect@4.0.0 pin against the port in `../src/stdlib/rc_map.rs`. The
//! pin holds a value through a caller scope; this port models the reference
//! count explicitly, so "the caller scope closes" is `release_ref`.

#[path = "../src/stdlib/rc_map.rs"]
mod rc_map;

use rc_map::{RcClock, RcMap, RcMapError};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

/// Virtual milliseconds. The pin's TTL rows advance `TestClock`; the port's
/// clock is injected instead, so the same rows run without sleeping.
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }

    fn handle(&self) -> Arc<dyn RcClock> {
        Arc::new(Self(Arc::clone(&self.0)))
    }
}

impl RcClock for FakeClock {
    fn now_millis(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
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
/// RcMap.test.ts:150 `returns None without lookup or capacity acquisition when
/// missing`. The map has no capacity to give, and a miss costs no lookup.
#[test]
fn rcmap_get_option_reads_without_lookup() {
    let lookups = counter();
    let lookups_hook = lookups.clone();
    let mut map = RcMap::<String, String, String>::make(
        move |k: &String| -> Result<String, String> {
            lookups_hook.fetch_add(1, Ordering::SeqCst);
            Ok(k.clone())
        },
        |_: String| {},
    );
    map.set_capacity(Some(0));

    assert_eq!(map.get_option(&"missing".to_owned()), None);
    assert_eq!(map.get_option(&"missing".to_owned()), None);
    assert_eq!(lookups.load(Ordering::SeqCst), 0);
    assert_eq!(map.size(), 0);
}

/// RcMap.test.ts:444 `capacity`. Two idle entries hold both slots, so the third
/// key is refused; once their TTL elapses the slot is free.
#[test]
fn rcmap_capacity_refuses_until_the_idle_ttl_frees_a_slot() {
    let clock = FakeClock::new();
    let released = counter();
    let released_hook = released.clone();
    let mut map = RcMap::<String, String, String>::make(
        |k: &String| -> Result<String, String> { Ok(k.clone()) },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    )
    .with_clock(clock.handle());
    map.set_idle_ttl(Some(Duration::from_millis(1000)));
    map.set_capacity(Some(2));

    for key in ["foo", "bar"] {
        assert_eq!(map.get(&key.to_owned()), Some(Ok(key.to_owned())));
        map.release_ref(&key.to_owned());
    }
    assert_eq!(
        map.get(&"baz".to_owned()),
        Some(Err(RcMapError::ExceededCapacity(2)))
    );

    clock.advance(999);
    assert_eq!(map.expire_idle(), 0);
    clock.advance(1);
    assert_eq!(map.expire_idle(), 2);
    assert_eq!(released.load(Ordering::SeqCst), 2);
    assert_eq!(map.get(&"baz".to_owned()), Some(Ok("baz".to_owned())));
}

/// RcMap.test.ts:370 `idleTimeToLive`. An unreferenced entry is retained until
/// its TTL passes, then released; a later get re-acquires.
#[test]
fn rcmap_idle_ttl_releases_an_unreferenced_entry() {
    let clock = FakeClock::new();
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
    )
    .with_clock(clock.handle());
    map.set_idle_ttl(Some(Duration::from_millis(1000)));

    assert_eq!(map.get(&"foo".to_owned()), Some(Ok("foo".to_owned())));
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    map.release_ref(&"foo".to_owned());
    assert_eq!(released.load(Ordering::SeqCst), 0);

    clock.advance(999);
    assert_eq!(map.expire_idle(), 0);
    assert!(map.has(&"foo".to_owned()));
    clock.advance(1);
    assert_eq!(map.expire_idle(), 1);
    assert!(!map.has(&"foo".to_owned()));
    assert_eq!(released.load(Ordering::SeqCst), 1);

    // The lookup is not cached any more, so the next get acquires again.
    assert_eq!(map.get(&"foo".to_owned()), Some(Ok("foo".to_owned())));
    assert_eq!(acquired.load(Ordering::SeqCst), 2);
}

/// RcMap.test.ts:249 `retains an idle entry and restarts its TTL on release`.
/// A get on an idle entry does not look up again, and its release re-arms the
/// clock from that moment.
#[test]
fn rcmap_idle_reuse_restarts_the_ttl() {
    let clock = FakeClock::new();
    let acquired = counter();
    let acquired_hook = acquired.clone();
    let mut map = RcMap::<String, String, String>::make(
        move |k: &String| -> Result<String, String> {
            acquired_hook.fetch_add(1, Ordering::SeqCst);
            Ok(k.clone())
        },
        |_: String| {},
    )
    .with_clock(clock.handle());
    map.set_idle_ttl(Some(Duration::from_millis(1000)));

    assert_eq!(map.get(&"ttl".to_owned()), Some(Ok("ttl".to_owned())));
    map.release_ref(&"ttl".to_owned());

    clock.advance(500);
    assert_eq!(map.get(&"ttl".to_owned()), Some(Ok("ttl".to_owned())));
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    map.release_ref(&"ttl".to_owned());

    clock.advance(999);
    assert_eq!(map.expire_idle(), 0);
    clock.advance(1);
    assert_eq!(map.expire_idle(), 1);
}

/// RcMap.test.ts:413 `.touch`. Touching an idle entry restarts its clock, so the
/// TTL is counted from the touch.
#[test]
fn rcmap_touch_restarts_the_idle_clock() {
    let clock = FakeClock::new();
    let released = counter();
    let released_hook = released.clone();
    let mut map = RcMap::<String, String, String>::make(
        |k: &String| -> Result<String, String> { Ok(k.clone()) },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    )
    .with_clock(clock.handle());
    map.set_idle_ttl(Some(Duration::from_millis(1000)));

    assert_eq!(map.get(&"foo".to_owned()), Some(Ok("foo".to_owned())));
    map.release_ref(&"foo".to_owned());

    clock.advance(500);
    assert!(map.touch(&"foo".to_owned()));
    clock.advance(500);
    assert_eq!(map.expire_idle(), 0);
    assert_eq!(released.load(Ordering::SeqCst), 0);
    clock.advance(500);
    assert_eq!(map.expire_idle(), 1);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

/// RcMap.test.ts:492 `dynamic idleTimeToLive`. The TTL is read per key, so a
/// `short:` key expires while a `long:` one is still retained.
#[test]
fn rcmap_dynamic_ttl_is_read_per_key() {
    let clock = FakeClock::new();
    let released = counter();
    let released_hook = released.clone();
    let mut map = RcMap::<String, String, String>::make(
        |k: &String| -> Result<String, String> { Ok(k.clone()) },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    )
    .with_clock(clock.handle());
    map.set_idle_ttl_fn(|key: &String| {
        Some(Duration::from_millis(if key.starts_with("short:") {
            500
        } else {
            2000
        }))
    });

    for key in ["short:a", "long:b"] {
        assert_eq!(map.get(&key.to_owned()), Some(Ok(key.to_owned())));
        map.release_ref(&key.to_owned());
    }

    clock.advance(500);
    assert_eq!(map.expire_idle(), 1);
    assert!(!map.has(&"short:a".to_owned()));
    assert!(map.has(&"long:b".to_owned()));
    clock.advance(1500);
    assert_eq!(map.expire_idle(), 1);
    assert!(!map.has(&"long:b".to_owned()));
}

/// RcMap.test.ts:522 `dynamic idleTimeToLive with touch`. The per-key clock is
/// the one touch restarts.
#[test]
fn rcmap_dynamic_ttl_with_touch() {
    let clock = FakeClock::new();
    let released = counter();
    let released_hook = released.clone();
    let mut map = RcMap::<String, String, String>::make(
        |k: &String| -> Result<String, String> { Ok(k.clone()) },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    )
    .with_clock(clock.handle());
    map.set_idle_ttl_fn(|key: &String| {
        Some(Duration::from_millis(if key.starts_with("short:") {
            500
        } else {
            2000
        }))
    });

    assert_eq!(map.get(&"short:a".to_owned()), Some(Ok("short:a".to_owned())));
    map.release_ref(&"short:a".to_owned());

    clock.advance(250);
    assert!(map.touch(&"short:a".to_owned()));
    clock.advance(250);
    assert_eq!(map.expire_idle(), 0);
    clock.advance(250);
    assert_eq!(map.expire_idle(), 1);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

/// RcMap.test.ts:479 `keys lookup`. Every retained key is listed (order is the
/// port's HashMap, so the witness compares sets).
#[test]
fn rcmap_keys_lists_retained_entries() {
    let mut map = RcMap::<String, String, String>::make(
        |k: &String| -> Result<String, String> { Ok(k.clone()) },
        |_: String| {},
    );

    for key in ["foo", "bar", "baz"] {
        assert_eq!(map.get(&key.to_owned()), Some(Ok(key.to_owned())));
    }
    let mut keys = map.keys();
    keys.sort();
    assert_eq!(keys, ["bar", "baz", "foo"]);

    for key in ["foo", "bar", "baz"] {
        map.release_ref(&key.to_owned());
    }
    assert!(map.keys().is_empty());
}

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
