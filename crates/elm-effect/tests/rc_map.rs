//! RcMap witnesses: reference-counted retention, idle TTL, capacity, and
//! generation-safe invalidation.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use elm_effect::{Duration, LookupFailure, RcMap, RcMapError, RcMapOptions, TestClock};

fn key(value: &str) -> String {
    value.to_owned()
}

#[test]
fn leases_share_one_lookup_and_release_on_the_last_return() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let map = RcMap::<String, usize, String>::make(
        {
            let lookups = Arc::clone(&lookups);
            move |key: &String| {
                lookups.fetch_add(1, Ordering::SeqCst);
                Ok::<usize, LookupFailure<String>>(key.len())
            }
        },
        {
            let released = Arc::clone(&released);
            move |_| {
                released.fetch_add(1, Ordering::SeqCst);
            }
        },
    );

    let first = map.get(&key("abc")).unwrap();
    let second = map.get(&key("abc")).unwrap();
    assert_eq!(first.value(), Some(&3));
    assert_eq!(second.value(), Some(&3));
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(map.ref_count(&key("abc")), 2);

    drop(first);
    assert_eq!(released.load(Ordering::SeqCst), 0);
    assert!(map.has(&key("abc")));

    drop(second);
    assert_eq!(released.load(Ordering::SeqCst), 1);
    assert!(!map.has(&key("abc")));
    assert!(map.is_empty());
}

#[test]
fn invalidation_retires_the_old_generation() {
    let acquired = Arc::new(Mutex::new(Vec::<u32>::new()));
    let released = Arc::new(Mutex::new(Vec::<u32>::new()));
    let map = RcMap::<String, u32, String>::make(
        {
            let acquired = Arc::clone(&acquired);
            move |_key: &String| {
                let mut values = acquired.lock().unwrap();
                let value = values.len() as u32 + 1;
                values.push(value);
                Ok::<u32, LookupFailure<String>>(value)
            }
        },
        {
            let released = Arc::clone(&released);
            move |value| released.lock().unwrap().push(value)
        },
    );

    let old = map.get(&key("key")).unwrap();
    assert!(map.invalidate(&key("key")));
    assert!(!map.has(&key("key")));

    let replacement = map.get(&key("key")).unwrap();
    assert_eq!(old.value(), Some(&1));
    assert_eq!(replacement.value(), Some(&2));
    assert!(replacement.generation() > old.generation());

    drop(old);
    assert_eq!(*released.lock().unwrap(), vec![1]);
    assert!(map.has(&key("key")));
    assert_eq!(map.ref_count(&key("key")), 1);

    drop(replacement);
    assert_eq!(*released.lock().unwrap(), vec![1, 2]);
}

#[test]
fn capacity_refuses_until_idle_ttl_frees_a_slot() {
    let clock = Arc::new(TestClock::new());
    let released = Arc::new(AtomicUsize::new(0));
    let options = RcMapOptions::<String>::new()
        .with_clock(clock.clone())
        .with_idle_ttl(Some(Duration::from_millis(1000)))
        .with_capacity(Some(2));
    let map = RcMap::<String, String, String>::make_with_options(
        |key: &String| Ok::<String, LookupFailure<String>>(key.clone()),
        {
            let released = Arc::clone(&released);
            move |_| {
                released.fetch_add(1, Ordering::SeqCst);
            }
        },
        options,
    );

    drop(map.get(&key("a")).unwrap());
    drop(map.get(&key("b")).unwrap());
    let refused = map.get(&key("c")).unwrap();
    assert_eq!(
        refused.result(),
        &Err(RcMapError::ExceededCapacity(2)),
        "a full map refuses instead of evicting"
    );
    drop(refused);

    clock.adjust(Duration::from_millis(999));
    assert_eq!(map.expire_idle(), 0);
    clock.adjust(Duration::from_millis(1));
    assert_eq!(map.expire_idle(), 2);
    assert_eq!(released.load(Ordering::SeqCst), 2);
    assert_eq!(map.get(&key("c")).unwrap().value(), Some(&key("c")));
}

#[test]
fn get_option_never_runs_the_lookup() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let options = RcMapOptions::<String>::new().with_idle_ttl(Some(Duration::from_secs(60)));
    let map = RcMap::<String, usize, String>::make_with_options(
        {
            let lookups = Arc::clone(&lookups);
            move |key: &String| {
                lookups.fetch_add(1, Ordering::SeqCst);
                Ok::<usize, LookupFailure<String>>(key.len())
            }
        },
        |_| {},
        options,
    );

    assert!(map.get_option(&key("missing")).is_none());
    assert_eq!(lookups.load(Ordering::SeqCst), 0);
    assert_eq!(map.get_value(&key("present")), Some(Ok(7)));
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(map.get_option_value(&key("present")), Some(Ok(7)));
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
}

#[test]
fn cached_failures_are_replayed() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let options = RcMapOptions::<String>::new().with_idle_ttl(Some(Duration::from_secs(60)));
    let map = RcMap::<String, u32, String>::make_with_options(
        {
            let lookups = Arc::clone(&lookups);
            move |_key: &String| {
                lookups.fetch_add(1, Ordering::SeqCst);
                Err::<u32, LookupFailure<String>>(LookupFailure::Fail("down".to_owned()))
            }
        },
        |_| {},
        options,
    );

    let first = map.get(&key("bad")).unwrap();
    assert_eq!(first.result(), &Err(RcMapError::Lookup("down".to_owned())));
    drop(first);

    let second = map.get(&key("bad")).unwrap();
    assert_eq!(second.result(), &Err(RcMapError::Lookup("down".to_owned())));
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(map.ref_count(&key("bad")), 1);
}

#[test]
fn close_releases_once_even_with_an_outstanding_lease() {
    let released = Arc::new(AtomicUsize::new(0));
    let map = RcMap::<String, u32, String>::make(
        |key: &String| Ok::<u32, LookupFailure<String>>(key.len() as u32),
        {
            let released = Arc::clone(&released);
            move |_| {
                released.fetch_add(1, Ordering::SeqCst);
            }
        },
    );

    let lease = map.get(&key("held")).unwrap();
    map.close();
    assert!(map.is_closed());
    assert!(map.get(&key("held")).is_none());
    assert_eq!(released.load(Ordering::SeqCst), 1);

    drop(lease);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}
