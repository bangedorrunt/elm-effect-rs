//! RcMap rows the ledger excused as "RC internals not exposed".
//!
//! Each test witnesses one behavior row from `packages/effect/test/RcMap.test.ts`
//! at the effect@4.0.0 pin against the port in `../src/stdlib/rc_map.rs`. The
//! pin holds a value through a caller scope; this port models the reference
//! count explicitly, so "the caller scope closes" is `release_ref`.

#[path = "../src/stdlib/rc_map.rs"]
mod rc_map;

use rc_map::{RcClock, RcMap, RcMapError};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// RcMap.test.ts:54 `releases only the invalidated entry with TTL %s`. Turning
/// the entry over on invalidate leaves the old borrower holding generation one;
/// when that borrower returns, generation one is released and the replacement
/// is untouched, whatever the TTL.
#[test]
fn rcmap_invalidate_spares_the_replacement_for_every_ttl() {
    let key = "key".to_owned();
    for ttl in [Duration::ZERO, Duration::from_millis(1000), Duration::MAX] {
        let clock = FakeClock::new();
        let acquired = Arc::new(Mutex::new(Vec::<i32>::new()));
        let released = Arc::new(Mutex::new(Vec::<i32>::new()));
        let acquired_hook = acquired.clone();
        let released_hook = released.clone();
        let mut map = RcMap::<String, i32, String>::make(
            move |_: &String| -> Result<i32, String> {
                let mut values = acquired_hook.lock().unwrap();
                let value = values.len() as i32 + 1;
                values.push(value);
                Ok(value)
            },
            move |value: i32| released_hook.lock().unwrap().push(value),
        )
        .with_clock(clock.handle());
        map.set_idle_ttl(Some(ttl));

        assert_eq!(map.get(&key), Some(Ok(1)));
        assert!(map.invalidate(&key));
        assert!(!map.has(&key));
        assert!(released.lock().unwrap().is_empty());

        assert_eq!(map.get(&key), Some(Ok(2)));
        // The old scope closes: the invalidated generation goes, the
        // replacement stays reachable.
        map.release_ref(&key);
        assert_eq!(*released.lock().unwrap(), [1]);
        assert!(map.has(&key));

        // A scoped getOption borrows and returns in one step, then the new
        // scope closes.
        assert_eq!(map.get_option(&key), Some(Ok(2)));
        map.release_ref(&key);
        map.release_ref(&key);
        let after_new_release = released.lock().unwrap().clone();
        if ttl.is_zero() {
            assert_eq!(after_new_release, [1, 2]);
        } else {
            assert_eq!(after_new_release, [1]);
        }

        if ttl == Duration::from_millis(1000) {
            clock.advance(999);
            assert!(map.has(&key));
            clock.advance(1);
            assert_eq!(map.expire_idle(), 1);
            assert!(!map.has(&key));
        }
        if ttl == Duration::MAX {
            assert!(map.has(&key));
        }

        map.close();
        assert_eq!(*released.lock().unwrap(), [1, 2]);
        assert_eq!(*acquired.lock().unwrap(), [1, 2]);
    }
}

/// RcMap.test.ts:103 `an invalidated entry's idle timer cannot remove its
/// replacement`. The invalidated generation keeps its own clock, and releasing
/// it must not decrement the generation that replaced it.
#[test]
fn rcmap_invalidated_entry_release_spares_the_replacement() {
    let clock = FakeClock::new();
    let key = "key".to_owned();
    let acquired = Arc::new(Mutex::new(Vec::<i32>::new()));
    let released = Arc::new(Mutex::new(Vec::<i32>::new()));
    let acquired_hook = acquired.clone();
    let released_hook = released.clone();
    let mut map = RcMap::<String, i32, String>::make(
        move |_: &String| -> Result<i32, String> {
            let mut values = acquired_hook.lock().unwrap();
            let value = values.len() as i32 + 1;
            values.push(value);
            Ok(value)
        },
        move |value: i32| released_hook.lock().unwrap().push(value),
    )
    .with_clock(clock.handle());
    map.set_idle_ttl(Some(Duration::from_millis(1000)));

    assert_eq!(map.get(&key), Some(Ok(1)));
    map.release_ref(&key);
    clock.advance(500);
    // Reuse inside the idle window keeps the value.
    assert_eq!(map.get(&key), Some(Ok(1)));
    assert_eq!(acquired.lock().unwrap().len(), 1);

    assert!(map.invalidate(&key));
    assert!(released.lock().unwrap().is_empty());
    assert_eq!(map.get(&key), Some(Ok(2)));

    // The invalidated holder returns; the replacement keeps its reference.
    map.release_ref(&key);
    assert_eq!(*released.lock().unwrap(), [1]);
    assert_eq!(map.ref_count(&key), 1);

    clock.advance(1000);
    assert_eq!(map.get_option(&key), Some(Ok(2)));
    assert_eq!(map.ref_count(&key), 2);
    map.release_ref(&key);
    map.release_ref(&key);

    map.close();
    assert_eq!(*released.lock().unwrap(), [1, 2]);
    assert_eq!(*acquired.lock().unwrap(), [1, 2]);
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

    assert_eq!(
        map.get(&"short:a".to_owned()),
        Some(Ok("short:a".to_owned()))
    );
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

// ---------------------------------------------------------------------------
// :281 linearizes retention before invalidation
// ---------------------------------------------------------------------------

#[test]
fn invalidate_retires_a_referenced_entry_until_the_last_borrower_returns() {
    // The pin removes the entry from the map at invalidation but lets the
    // scope that still holds it close normally; only then does the value's
    // finalizer run. Releasing at invalidation would tear the value down
    // under a live borrower.
    let released = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = released.clone();
    let mut map = RcMap::<String, String, ()>::make(
        |key: &String| Ok(key.clone()),
        move |value: String| log.lock().unwrap().push(value),
    );

    assert_eq!(map.get(&"key".to_owned()), Some(Ok("key".to_owned())));
    assert_eq!(
        map.get_option(&"key".to_owned()),
        Some(Ok("key".to_owned()))
    );

    assert!(map.invalidate(&"key".to_owned()));
    assert!(!map.has(&"key".to_owned()));
    assert!(
        released.lock().unwrap().is_empty(),
        "invalidation must not release a value a borrower still holds"
    );

    map.release_ref(&"key".to_owned());
    assert!(
        released.lock().unwrap().is_empty(),
        "the second borrower still holds it"
    );

    map.release_ref(&"key".to_owned());
    assert_eq!(
        *released.lock().unwrap(),
        vec!["key".to_owned()],
        "the last borrower releases exactly once"
    );
}

/// RcMap.test.ts:188 `shares an in-flight entry`.
///
/// The pin hands the second caller the entry while the first caller's
/// acquisition is still pending (`refCount` 2 before the lookup's own effect
/// completes). This port serializes acquisition on the map, so the second
/// caller waits for the first and then joins the cached entry: one lookup, two
/// references, and the release runs on the last return.
#[test]
fn rcmap_shares_an_in_flight_acquisition_between_threads() {
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let gate_rx = Arc::new(Mutex::new(Some(gate_rx)));
    let lookups = counter();
    let released = counter();
    let (looked_up_tx, looked_up_rx) = std::sync::mpsc::channel::<()>();

    let lookups_hook = lookups.clone();
    let released_hook = released.clone();
    let map = RcMap::<String, String, ()>::make(
        move |key: &String| -> Result<String, ()> {
            lookups_hook.fetch_add(1, Ordering::SeqCst);
            started_tx.send(()).ok();
            if let Some(gate) = gate_rx.lock().unwrap().take() {
                gate.recv().ok();
            }
            Ok(key.clone())
        },
        move |_: String| {
            released_hook.fetch_add(1, Ordering::SeqCst);
        },
    );
    let map = Arc::new(Mutex::new(map));

    let owner_map = map.clone();
    let owner = std::thread::spawn(move || owner_map.lock().unwrap().get(&"pending".to_owned()));

    started_rx.recv().expect("the owner reaches the lookup");

    let borrower_map = map.clone();
    let borrower = std::thread::spawn(move || {
        let mut guard = borrower_map.lock().unwrap();
        looked_up_tx.send(()).ok();
        guard.get_option(&"pending".to_owned())
    });

    // The borrower cannot reach the map while the owner is inside `lookup`:
    // that serialization is how this port shares one acquisition.
    assert!(
        looked_up_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err(),
        "the second caller must wait for the acquisition in flight"
    );
    assert_eq!(
        lookups.load(Ordering::SeqCst),
        1,
        "the waiting caller must not start a second lookup"
    );

    gate_tx.send(()).ok();

    assert_eq!(owner.join().unwrap(), Some(Ok("pending".to_owned())));
    assert_eq!(borrower.join().unwrap(), Some(Ok("pending".to_owned())));
    assert_eq!(map.lock().unwrap().ref_count(&"pending".to_owned()), 2);
    assert_eq!(lookups.load(Ordering::SeqCst), 1);

    let mut guard = map.lock().unwrap();
    guard.release_ref(&"pending".to_owned());
    assert_eq!(released.load(Ordering::SeqCst), 0, "one holder is left");
    guard.release_ref(&"pending".to_owned());
    assert_eq!(
        released.load(Ordering::SeqCst),
        1,
        "the shared value is released once, on the last return"
    );
}

/// RcMap.test.ts:551 `closing the map while an idle entry is being released`.
///
/// The pin's finalizer may await, so `close` overlaps a release already in
/// flight and every finalizer still runs. Here the release hook itself is the
/// in-flight finalizer: a blocking hook runs under the map lock, so `close`
/// waits for it and then releases what remains, with no value released twice.
#[test]
fn rcmap_close_waits_out_an_in_flight_release_and_runs_every_finalizer() {
    let clock = FakeClock::new();
    let released = Arc::new(Mutex::new(Vec::<String>::new()));
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let gate_rx = Arc::new(Mutex::new(Some(gate_rx)));
    let log = released.clone();

    let mut map = RcMap::<String, String, ()>::make(
        |key: &String| Ok(key.clone()),
        move |value: String| {
            log.lock().unwrap().push(value.clone());
            if value == "a" {
                started_tx.send(()).ok();
                if let Some(gate) = gate_rx.lock().unwrap().take() {
                    gate.recv().ok();
                }
            }
        },
    )
    .with_clock(clock.handle());
    map.set_idle_ttl(Some(Duration::from_millis(1000)));

    assert_eq!(map.get(&"a".to_owned()), Some(Ok("a".to_owned())));
    assert_eq!(map.get(&"b".to_owned()), Some(Ok("b".to_owned())));
    // Only "a" goes idle, so the timer has one entry to release and "b" is
    // still held when close arrives.
    map.release_ref(&"a".to_owned());
    clock.advance(1000);

    let map = Arc::new(Mutex::new(map));

    let expiring_map = map.clone();
    let expiring = std::thread::spawn(move || {
        let mut guard = expiring_map.lock().unwrap();
        let expired = guard.expire_idle();
        (
            expired,
            guard.has(&"a".to_owned()),
            guard.has(&"b".to_owned()),
        )
    });

    started_rx.recv().expect("the idle timer starts releasing");

    let closing_map = map.clone();
    let (closed_tx, closed_rx) = std::sync::mpsc::channel::<()>();
    let closing = std::thread::spawn(move || {
        let mut guard = closing_map.lock().unwrap();
        guard.close();
        closed_tx.send(()).ok();
        guard.is_closed()
    });

    assert!(
        closed_rx.recv_timeout(Duration::from_millis(150)).is_err(),
        "close must wait for the release already in flight"
    );

    gate_tx.send(()).ok();

    assert_eq!(
        expiring.join().unwrap(),
        (1, false, true),
        "the timer takes the idle key and leaves the held one"
    );
    assert!(closing.join().unwrap(), "close finishes after the release");
    assert_eq!(
        *released.lock().unwrap(),
        vec!["a".to_owned(), "b".to_owned()],
        "every value is released exactly once, none dropped or repeated"
    );
}
