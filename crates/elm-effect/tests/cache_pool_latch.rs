//! Cache, pool, and latch witnesses for the kernel envelope.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration as StdDuration;

use elm_effect::{
    Cache, Duration, IdlePolicy, Latch, LatchInterrupted, Pool, Release, Scope, ScopedCache,
    TestClock, TimeToLive,
};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + StdDuration::from_secs(2);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "condition was not reached before the deadline"
        );
        std::thread::sleep(StdDuration::from_millis(1));
    }
}

#[test]
fn cache_single_flights_identical_misses() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(Cache::new(16, {
        let calls = Arc::clone(&calls);
        move |key: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(StdDuration::from_millis(20));
            Ok::<u32, String>(key * 2)
        }
    }));

    let results: Vec<u32> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| cache.get(&21))).collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect()
    });

    assert!(results.iter().all(|value| *value == 42));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(cache.size(), 1);
}

#[test]
fn cache_keeps_least_recently_used_entries() {
    let cache = Cache::new(2, |key: &u32| Ok::<u32, String>(*key));
    assert_eq!(cache.get(&1), Ok(1));
    assert_eq!(cache.get(&2), Ok(2));
    assert_eq!(cache.get(&1), Ok(1));
    assert_eq!(cache.get(&3), Ok(3));

    assert!(cache.has(&1));
    assert!(!cache.has(&2));
    assert!(cache.has(&3));
    assert!(!cache.is_empty());
    assert_eq!(cache.size(), 2);
}

#[test]
fn cache_ttl_reads_the_injected_clock() {
    let clock = Arc::new(TestClock::new());
    let cache = Cache::new(8, |key: &u32| Ok::<u32, String>(*key))
        .with_ttl(TimeToLive::For(Duration::from_secs(60)), clock.clone());

    assert_eq!(cache.get(&1), Ok(1));
    clock.adjust(Duration::from_secs(30));
    assert!(cache.has(&1));
    clock.adjust(Duration::from_secs(31));
    assert!(!cache.has(&1));
    assert_eq!(cache.get_option(&1), None);
}

#[test]
fn cache_does_not_keep_failures_by_default() {
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup = {
        let calls = Arc::clone(&calls);
        move |_: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<u32, String>("down".to_owned())
        }
    };

    let cache = Cache::new(8, lookup.clone());
    assert!(cache.get(&1).is_err());
    assert!(cache.get(&1).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let sticky = Cache::new(8, lookup).cache_errors(true);
    assert!(sticky.get(&1).is_err());
    assert!(sticky.get(&1).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn cache_set_and_invalidate_skip_the_lookup() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(8, {
        let calls = Arc::clone(&calls);
        move |key: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<u32, String>(key * 2)
        }
    });

    cache.set(1, 11);
    assert_eq!(cache.get(&1), Ok(11));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    cache.invalidate(&1);
    assert!(!cache.has(&1));
    assert_eq!(cache.get(&1), Ok(2));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_releases_an_entry_when_it_is_evicted_or_cleared() {
    let released = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new_owned(1, {
        let released = Arc::clone(&released);
        move |key: &u32| {
            let released = Arc::clone(&released);
            (
                Ok::<u32, String>(*key),
                Some(Box::new(move || {
                    released.fetch_add(1, Ordering::SeqCst);
                }) as Release),
            )
        }
    });

    assert_eq!(cache.get(&1), Ok(1));
    assert_eq!(cache.get(&2), Ok(2));
    assert_eq!(released.load(Ordering::SeqCst), 1);
    cache.clear();
    assert_eq!(released.load(Ordering::SeqCst), 2);
}

#[test]
fn scoped_cache_clears_when_its_scope_closes() {
    let scope = Arc::new(Scope::new(90));
    let cache = ScopedCache::make(
        8,
        TimeToLive::Infinite,
        Arc::new(TestClock::new()),
        Arc::clone(&scope),
        |key: &u32| Ok::<u32, String>(*key + 1),
    );

    assert_eq!(cache.get(&1), Ok(2));
    assert!(cache.has(&1));
    scope.close();
    assert!(!cache.has(&1));
}

#[test]
fn pool_reuses_values_and_blocks_when_exhausted() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Arc::new(Pool::new(1, IdlePolicy::default(), {
        let made = Arc::clone(&made);
        move || {
            made.fetch_add(1, Ordering::SeqCst);
            7u32
        }
    }));

    let first = pool.acquire();
    assert_eq!(pool.in_use(), 1);
    let (tx, rx) = mpsc::channel();
    let worker = {
        let pool = Arc::clone(&pool);
        std::thread::spawn(move || {
            let value = pool.acquire();
            tx.send(*value).unwrap();
        })
    };

    wait_until(|| pool.in_use() == 1);
    assert!(rx.recv_timeout(StdDuration::from_millis(20)).is_err());
    drop(first);
    assert_eq!(rx.recv_timeout(StdDuration::from_secs(2)), Ok(7));
    worker.join().unwrap();
    assert_eq!(made.load(Ordering::SeqCst), 1);
}

#[test]
fn pool_invalidate_and_reserve_remove_items_from_circulation() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Pool::new(2, IdlePolicy::default(), {
        let made = Arc::clone(&made);
        move || {
            let next = made.fetch_add(1, Ordering::SeqCst) + 1;
            next as u32
        }
    });

    let broken = pool.acquire();
    broken.invalidate();
    assert_eq!(pool.in_use(), 0);
    let replacement = pool.acquire();
    assert_eq!(*replacement, 2);
    drop(replacement);

    let reserved = pool.reserve();
    assert_eq!(*reserved, 2);
    drop(reserved);
    let fresh = pool.acquire();
    assert_eq!(*fresh, 3);
}

#[test]
fn latch_release_admits_parked_waiters_and_stays_closed() {
    let latch = Arc::new(Latch::new_closed());
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..3 {
        let latch = Arc::clone(&latch);
        let tx = tx.clone();
        workers.push(std::thread::spawn(move || {
            latch.await_open(&|| false).unwrap();
            tx.send(()).unwrap();
        }));
    }
    wait_until(|| latch.waiters() == 3);

    assert!(latch.release());
    for _ in 0..3 {
        rx.recv_timeout(StdDuration::from_secs(2)).unwrap();
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(!latch.is_open());
    assert_eq!(latch.waiters(), 0);

    let late = {
        let latch = Arc::clone(&latch);
        let tx = tx.clone();
        std::thread::spawn(move || {
            latch.await_open(&|| false).unwrap();
            tx.send(()).unwrap();
        })
    };
    wait_until(|| latch.waiters() == 1);
    assert!(rx.recv_timeout(StdDuration::from_millis(20)).is_err());
    assert!(latch.open());
    rx.recv_timeout(StdDuration::from_secs(2)).unwrap();
    late.join().unwrap();
}

#[test]
fn latch_observes_interruption_without_admitting() {
    let latch = Latch::new_closed();
    assert_eq!(latch.await_open(&|| true), Err(LatchInterrupted));
    assert_eq!(latch.waiters(), 0);
    assert!(!latch.is_open());

    assert!(latch.open());
    assert_eq!(latch.await_open(&|| false), Ok(()));
}
