//! Cache single-flight, request batching, pools, and scoped caches.

#[path = "../src/cache.rs"]
mod cache;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use cache::{Cache, IdlePolicy, Pool, Request, RequestResolver, ScopedCache};
use elm_effect::Scope;

#[test]
fn single_flight_runs_lookup_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(Cache::new(16, {
        let calls = Arc::clone(&calls);
        move |key: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(10));
            Ok::<u32, String>(key * 2)
        }
    }));
    let results: Vec<u32> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(s.spawn(|| cache.get(&21)));
        }
        handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect()
    });
    assert!(results.iter().all(|v| *v == 42));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(cache.len(), 1);
}

#[test]
fn batching_makes_one_downstream_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = RequestResolver::new({
        let calls = Arc::clone(&calls);
        move |keys: Vec<u32>| {
            calls.fetch_add(1, Ordering::SeqCst);
            keys.into_iter()
                .map(|k| Ok::<u32, String>(k * 10))
                .collect()
        }
    });
    let outcomes = resolver.resolve_all(vec![1, 2, 3]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcomes, vec![Ok(10), Ok(20), Ok(30)]);

    let handles: Vec<(u32, Request<u32, String>)> = vec![(4, Request::new()), (5, Request::new())];
    let completed = resolver.resolve_requests(handles);
    assert_eq!(completed, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let request = Request::<u32, String>::new();
    assert!(request.complete(Ok(9)));
    assert!(!request.complete(Ok(10)));
    assert_eq!(request.wait(), Ok(9));
}

#[test]
fn failures_are_not_cached_by_default() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(16, {
        let calls = Arc::clone(&calls);
        move |_: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<u32, String>("down".to_string())
        }
    });
    assert!(cache.get(&1).is_err());
    assert!(cache.get(&1).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let sticky = Cache::new(16, {
        let calls = Arc::clone(&calls);
        move |_: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<u32, String>("down".to_string())
        }
    })
    .cache_errors(true);
    assert!(sticky.get(&1).is_err());
    assert!(sticky.get(&1).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    let small = Cache::new(2, |key: &u32| Ok::<u32, String>(key + 100));
    assert_eq!(small.get(&1), Ok(101));
    assert_eq!(small.get(&2), Ok(102));
    assert_eq!(small.get(&3), Ok(103));
    assert_eq!(small.len(), 2);
}

#[test]
fn pool_releases_idle_extras() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Pool::new(
        2,
        IdlePolicy {
            max_idle: 1,
            max_idle_time: None,
        },
        {
            let made = Arc::clone(&made);
            move || {
                made.fetch_add(1, Ordering::SeqCst);
                7u32
            }
        },
    );
    let a = pool.acquire();
    let b = pool.acquire();
    assert_eq!(pool.in_use(), 2);
    assert_eq!(made.load(Ordering::SeqCst), 2);
    assert_eq!(*a + *b, 14);
    drop(a);
    assert_eq!(pool.idle_count(), 1);
    drop(b);
    assert_eq!(pool.idle_count(), 1);
    let c = pool.acquire();
    assert_eq!(*c, 7);
    assert_eq!(made.load(Ordering::SeqCst), 2);
    drop(c);
}

#[test]
fn scoped_cache_close_clears() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(16, {
        let calls = Arc::clone(&calls);
        move |key: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<u32, String>(key + 1)
        }
    });
    let scoped = ScopedCache::new(cache, Arc::new(Scope::new(90)));
    assert_eq!(scoped.get(&1), Ok(2));
    assert_eq!(scoped.get(&1), Ok(2));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    scoped.close();
    assert_eq!(scoped.get(&1), Ok(2));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn pool_blocks_while_exhausted() {
    let pool = Pool::new(1, IdlePolicy::default(), || 1u32);
    let held = pool.acquire();
    let parked = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        s.spawn(|| {
            parked.fetch_add(1, Ordering::SeqCst);
            let _guard = pool.acquire();
            done.fetch_add(1, Ordering::SeqCst);
        });
        while parked.load(Ordering::SeqCst) == 0 {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        // Waiter is blocked: nothing completed, pool fully in use.
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(done.load(Ordering::SeqCst), 0);
        assert_eq!(pool.in_use(), 1);
        drop(held);
    });
    assert_eq!(done.load(Ordering::SeqCst), 1);
}
