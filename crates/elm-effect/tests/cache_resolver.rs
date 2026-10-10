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
fn scoped_cache_make_capacity_and_ttl() {
    // ScopedCache.test.ts:50, :60, :73, :89 — capacity is fixed at
    // construction, the default TTL never expires, a fixed TTL is measured
    // against the clock, and a zero TTL expires immediately.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let lookup = |k: &String| Ok::<usize, String>(k.len());

    let fixed = ScopedCache::make(
        10,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(1)),
        lookup,
    );
    assert_eq!(fixed.capacity(), 10);
    assert_eq!(fixed.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_secs(1000 * 3600));
    assert!(fixed.has(&"test".to_string()));

    let hourly = ScopedCache::make(
        10,
        TimeToLive::For(Duration::from_minutes(60)),
        clock.clone(),
        Arc::new(Scope::new(2)),
        lookup,
    );
    assert_eq!(hourly.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_minutes(30));
    assert!(hourly.has(&"test".to_string()));
    clock.adjust(Duration::from_minutes(31));
    assert!(!hourly.has(&"test".to_string()));
    // ScopedCache.test.ts:481 — an expired entry reads as absent, and
    // getOption does not run the lookup to discover that.
    assert_eq!(hourly.get_option(&"test".to_string()), None);

    // Cache.test.ts:567 — the same expiry rule on the unscoped cache.
    let plain = Cache::new(10, lookup).with_ttl(
        TimeToLive::For(Duration::from_minutes(60)),
        clock.clone(),
    );
    assert_eq!(plain.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_minutes(120));
    assert_eq!(plain.get_option(&"test".to_string()), None);

    let zero = ScopedCache::make(
        10,
        TimeToLive::For(Duration::ZERO),
        clock.clone(),
        Arc::new(Scope::new(3)),
        lookup,
    );
    assert_eq!(zero.get(&"test".to_string()), Ok(4));
    assert!(!zero.has(&"test".to_string()));
}

#[test]
fn scoped_cache_set_fills_without_lookup() {
    // ScopedCache.test.ts:594, :605, :617, :628, :634 — set writes an entry
    // without running the lookup, overwrites in place, starts the TTL at the
    // write, and respects capacity by evicting the oldest entry.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let counting = {
        let calls = Arc::clone(&calls);
        move |k: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(k.len())
        }
    };
    let cache = ScopedCache::make(
        10,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(7)),
        counting,
    );
    cache.set("test".to_string(), 42);
    assert_eq!(cache.get(&"test".to_string()), Ok(42));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    cache.set("test".to_string(), 100);
    assert_eq!(cache.get(&"test".to_string()), Ok(100));
    cache.set("test2".to_string(), 43);
    cache.set("test3".to_string(), 44);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let hourly = ScopedCache::make(
        10,
        TimeToLive::For(Duration::from_minutes(60)),
        clock.clone(),
        Arc::new(Scope::new(8)),
        |k: &String| Ok::<usize, String>(k.len()),
    );
    hourly.set("test".to_string(), 42);
    assert!(hourly.has(&"test".to_string()));
    clock.adjust(Duration::from_minutes(120));
    assert!(!hourly.has(&"test".to_string()));

    let bounded = ScopedCache::make(
        2,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(9)),
        |k: &String| Ok::<usize, String>(k.len()),
    );
    bounded.set("a".to_string(), 1);
    bounded.set("b".to_string(), 2);
    assert_eq!(bounded.size(), 2);
    bounded.set("c".to_string(), 3);
    assert_eq!(bounded.size(), 2);
    // The oldest entry made room; the newest is present.
    assert!(!bounded.has(&"a".to_string()));
    assert!(bounded.has(&"c".to_string()));
}

#[test]
fn scoped_cache_has_reports_presence_without_touching_lru() {
    // ScopedCache.test.ts:677, :690, :702, :717 — has reports a held,
    // unexpired entry, and reading it must not refresh recency.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let lookup = |k: &String| Ok::<usize, String>(k.len());

    let plain = ScopedCache::make(
        10,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(11)),
        lookup,
    );
    assert!(!plain.has(&"test".to_string()));
    assert_eq!(plain.get(&"test".to_string()), Ok(4));
    assert!(plain.has(&"test".to_string()));

    let hourly = ScopedCache::make(
        10,
        TimeToLive::For(Duration::from_minutes(60)),
        clock.clone(),
        Arc::new(Scope::new(12)),
        lookup,
    );
    assert_eq!(hourly.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_minutes(120));
    assert!(!hourly.has(&"test".to_string()));

    // With capacity 2, adding "c" must still evict "a" (the oldest), not
    // the key whose presence was just checked.
    let bounded = ScopedCache::make(
        2,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(13)),
        lookup,
    );
    bounded.get(&"a".to_string());
    bounded.get(&"b".to_string());
    assert!(bounded.has(&"a".to_string()));
    bounded.get(&"c".to_string());
    assert!(!bounded.has(&"a".to_string()));
    assert!(bounded.has(&"b".to_string()));
    assert!(bounded.has(&"c".to_string()));

    // The row's own sequence above cannot see a recency touch: with two
    // entries and a capacity of two, the eviction victim is the queue head
    // either way. A longer run can: if has refreshes recency, the check
    // leaves a stale name in the queue and a later write evicts a ghost
    // instead of a live entry, so capacity ends up violated.
    for key in ["d", "e"] {
        bounded.get(&key.to_string());
    }
    assert_eq!(bounded.size(), 2);
}

#[test]
fn scoped_cache_evicts_least_recently_used() {
    // ScopedCache.test.ts:1572, :1597, :1620, :1639 — eviction takes the
    // least recently used entry, a hit counts as a use, a capacity of one
    // keeps only the newest key, and a large capacity keeps everything.
    use cache::TimeToLive;
    use elm_effect::TestClock;

    let clock = Arc::new(TestClock::new());
    let lookup = |k: &String| Ok::<usize, String>(k.len());

    let three = ScopedCache::make(
        3,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(31)),
        lookup,
    );
    for key in ["first", "second", "third"] {
        three.get(&key.to_string());
    }
    assert!(three.has(&"first".to_string()));
    assert!(three.has(&"second".to_string()));
    assert!(three.has(&"third".to_string()));
    three.get(&"fourth".to_string());
    assert!(!three.has(&"first".to_string()));
    assert!(three.has(&"second".to_string()));
    assert!(three.has(&"third".to_string()));
    assert!(three.has(&"fourth".to_string()));

    let accessed = ScopedCache::make(
        3,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(32)),
        lookup,
    );
    for key in ["a", "b", "c"] {
        accessed.get(&key.to_string());
    }
    // Touching "a" makes "b" the least recently used.
    accessed.get(&"a".to_string());
    accessed.get(&"d".to_string());
    assert!(accessed.has(&"a".to_string()));
    assert!(!accessed.has(&"b".to_string()));
    assert!(accessed.has(&"c".to_string()));
    assert!(accessed.has(&"d".to_string()));

    let single = ScopedCache::make(
        1,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(33)),
        lookup,
    );
    single.get(&"a".to_string());
    assert!(single.has(&"a".to_string()));
    single.get(&"b".to_string());
    assert!(!single.has(&"a".to_string()));
    assert!(single.has(&"b".to_string()));
    single.get(&"c".to_string());
    assert!(!single.has(&"b".to_string()));
    assert!(single.has(&"c".to_string()));

    let roomy = ScopedCache::make(
        10_000,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(34)),
        lookup,
    );
    for index in 0..100 {
        roomy.get(&format!("key-{index}"));
    }
    for index in 0..100 {
        assert!(roomy.has(&format!("key-{index}")));
    }
    assert_eq!(roomy.size(), 100);
}

#[test]
fn scoped_cache_invalidate_forgets_entry() {
    // ScopedCache.test.ts:740, :754, :767 — invalidate drops the entry,
    // tolerates a missing key, and forces the next get to run the lookup
    // again. What invalidate does to the stamp map beyond that is not
    // observable (see `Cache::invalidate`), so nothing here asserts it.
    use cache::TimeToLive;
    use elm_effect::TestClock;

    let clock = Arc::new(TestClock::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let counting = {
        let calls = Arc::clone(&calls);
        move |_k: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(1)
        }
    };
    let cache = ScopedCache::make(
        10,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(21)),
        counting,
    );
    assert_eq!(cache.get(&"test".to_string()), Ok(1));
    cache.invalidate(&"test".to_string());
    assert!(!cache.has(&"test".to_string()));
    // Invalidating an absent key is a no-op, not an error.
    cache.invalidate(&"test".to_string());
    assert!(!cache.has(&"test".to_string()));
    assert_eq!(cache.get(&"test".to_string()), Ok(1));
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

#[test]
fn pool_preallocates_idle_values() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Pool::new(
        4,
        IdlePolicy::default(),
        {
            let made = Arc::clone(&made);
            move || {
                made.fetch_add(1, Ordering::SeqCst);
                7u32
            }
        },
    );
    pool.preallocate(3);
    assert_eq!(made.load(Ordering::SeqCst), 3);
    assert_eq!(pool.idle_count(), 3);
    assert_eq!(pool.in_use(), 0);
    let a = pool.acquire();
    assert_eq!(*a, 7);
    assert_eq!(made.load(Ordering::SeqCst), 3, "acquire reuses, not makes");
    drop(a);
}

#[test]
fn pool_invalidate_drops_item_from_circulation() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Pool::new(
        2,
        IdlePolicy::default(),
        {
            let made = Arc::clone(&made);
            move || {
                made.fetch_add(1, Ordering::SeqCst);
                7u32
            }
        },
    );
    let bad = pool.acquire();
    assert_eq!(made.load(Ordering::SeqCst), 1);
    bad.invalidate();
    assert_eq!(pool.idle_count(), 0, "invalidated item never returns");
    assert_eq!(pool.in_use(), 0);
    let fresh = pool.acquire();
    assert_eq!(made.load(Ordering::SeqCst), 2, "replacement is made");
    drop(fresh);
}

#[test]
fn pool_reserve_removes_item_permanently() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Pool::new(
        2,
        IdlePolicy::default(),
        {
            let made = Arc::clone(&made);
            move || {
                made.fetch_add(1, Ordering::SeqCst);
                7u32
            }
        },
    );
    let held = pool.reserve();
    assert_eq!(*held, 7);
    drop(held);
    assert_eq!(pool.idle_count(), 0, "reserved item leaves circulation");
    assert_eq!(pool.in_use(), 0);
    let next = pool.acquire();
    assert_eq!(made.load(Ordering::SeqCst), 2, "pool makes a replacement");
    drop(next);
}

#[test]
fn pool_max_size_with_concurrency() {
    let made = Arc::new(AtomicUsize::new(0));
    let pool = Arc::new(Pool::new(
        3,
        IdlePolicy::default(),
        {
            let made = Arc::clone(&made);
            move || {
                made.fetch_add(1, Ordering::SeqCst);
                7u32
            }
        },
    ));
    let a = pool.acquire();
    let b = pool.acquire();
    let c = pool.acquire();
    assert_eq!(pool.in_use(), 3);
    let done = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        for _ in 0..3 {
            let (pool, done) = (Arc::clone(&pool), Arc::clone(&done));
            s.spawn(move || {
                let guard = pool.acquire();
                assert_eq!(*guard, 7);
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(done.load(Ordering::SeqCst), 0, "waiters block at max");
        drop(a);
        drop(b);
        drop(c);
    });
    assert_eq!(done.load(Ordering::SeqCst), 3);
    assert_eq!(made.load(Ordering::SeqCst), 3, "no over-make past max");
}

#[test]
fn pool_reclaims_after_concurrent_use() {
    let pool = Pool::new(2, IdlePolicy::default(), || 7u32);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                let guard = pool.acquire();
                assert_eq!(*guard, 7);
            });
        }
    });
    assert_eq!(pool.in_use(), 0, "everything returned");
    assert!(pool.idle_count() <= 2);
    let _ = pool.acquire();
}

// ---------------------------------------------------------------------------
// Cache.getOption at the pin (describe "getOption")
// ---------------------------------------------------------------------------

#[test]
fn get_option_returns_some_after_a_get() {
    // Cache.get(cache, "test") then Cache.getOption(cache, "test") === Some(4)
    let cache = Cache::new(10, |key: &String| Ok::<usize, String>(key.len()));
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert_eq!(cache.get_option(&"test".to_string()), Some(4));
}

#[test]
fn get_option_returns_none_for_a_missing_key_and_never_runs_the_lookup() {
    // Cache.getOption(cache, "test") === None. The observable claim that
    // distinguishes getOption from get is that a MISS does NOT invoke the
    // lookup: this is a pure read of the map.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |key: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(key.len())
        }
    });
    assert_eq!(cache.get_option(&"test".to_string()), None);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a getOption miss must not run the lookup"
    );
    // A later get still populates and then reports the value.
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert_eq!(cache.get_option(&"test".to_string()), Some(4));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "only the get ran the lookup");
}

#[test]
fn get_option_reports_absent_for_a_cached_failure() {
    // A cached failure is not a value: getOption reports absence rather than
    // resurrecting the error, which is what keeps it a "did I hold this key"
    // question.
    let cache = Cache::new(10, |_key: &String| Err::<usize, String>("boom".to_string()))
        .cache_errors(true);
    assert!(cache.get(&"k".to_string()).is_err());
    assert_eq!(cache.len(), 1, "the failure is cached");
    assert_eq!(cache.get_option(&"k".to_string()), None);
}
