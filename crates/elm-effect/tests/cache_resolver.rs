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

// Cache refresh and zero-TTL writes

#[test]
fn cache_refresh_reruns_the_lookup() {
    // Cache.test.ts:906, :1643 — refresh always runs the lookup, even on a
    // hit, and a cached failure can be refreshed into a success.
    let counter = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let counter = Arc::clone(&counter);
        move |_: &String| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(counter.load(Ordering::SeqCst))
        }
    });
    assert_eq!(cache.get(&"test".to_string()), Ok(1));
    assert_eq!(cache.refresh(&"test".to_string()), Ok(2));

    // The refresh replaces the cached failure.
    let failing = Cache::new(10, |_: &String| Err::<usize, String>("error".to_string()))
        .cache_errors(true);
    assert!(failing.get(&"test".to_string()).is_err());
    assert_eq!(failing.refresh(&"test".to_string()), Err("error".to_string()));
}

#[test]
fn cache_refresh_missing_key_enforces_capacity() {
    // Cache.test.ts:920 — refreshing a key that is not held runs the lookup
    // and inserts at the back of the queue, so the capacity check runs and
    // the oldest row makes room.
    let counter = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(1, {
        let counter = Arc::clone(&counter);
        move |_: &String| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(counter.load(Ordering::SeqCst))
        }
    });
    cache.set("other".to_string(), 99);
    assert_eq!(cache.refresh(&"test".to_string()), Ok(1));
    assert_eq!(cache.size(), 1);
    assert_eq!(cache.keys(), vec!["test".to_string()]);
}

#[test]
fn cache_refresh_updates_ttl() {
    // Cache.test.ts:1029, :1588 — the refresh restarts the lifetime, so an
    // entry refreshed inside its TTL outlives the original stamp.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let cache = Cache::new(10, |key: &String| Ok::<usize, String>(key.len())).with_ttl(
        TimeToLive::For(Duration::from_minutes(60)),
        clock.clone(),
    );
    cache.get(&"test".to_string());
    clock.adjust(Duration::from_minutes(45));
    assert!(cache.has(&"test".to_string()));
    cache.refresh(&"test".to_string());
    clock.adjust(Duration::from_minutes(30));
    assert!(cache.has(&"test".to_string()), "45 + 30 is past the first TTL");
    clock.adjust(Duration::from_minutes(31));
    assert!(!cache.has(&"test".to_string()), "61 minutes after the refresh");
}

#[test]
fn cache_refresh_is_not_a_use_for_eviction() {
    // The pin's refresh replaces the entry in place (a plain map set keeps
    // position), so a refreshed key keeps its old recency row and the next
    // write still evicts it. `get` moves the key, `refresh` does not.
    let cache = Cache::new(2, |key: &String| Ok::<usize, String>(key.len()));
    cache.get(&"a".to_string());
    cache.get(&"b".to_string());
    cache.refresh(&"a".to_string());
    cache.get(&"c".to_string());
    assert!(!cache.has(&"a".to_string()), "refresh did not refresh recency");
    assert!(cache.has(&"b".to_string()));
    assert!(cache.has(&"c".to_string()));
}

#[test]
fn concurrent_refreshes_each_run_the_lookup() {
    // Cache.test.ts:1045 — refresh is not single-flight: three refreshes of
    // one key run the lookup three times. The map lock serializes them, so
    // the results are a permutation of the three calls, never a repeat of a
    // cached value.
    let counter = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(Cache::new(10, {
        let counter = Arc::clone(&counter);
        move |_: &String| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(counter.load(Ordering::SeqCst))
        }
    }));
    cache.get(&"test".to_string());
    let mut results: Vec<usize> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..3 {
            handles.push(s.spawn(|| cache.refresh(&"test".to_string()).unwrap()));
        }
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    results.sort();
    assert_eq!(results, vec![2, 3, 4]);
}

#[test]
fn cache_zero_ttl_stores_nothing() {
    // Cache.test.ts:197, :1018 — a zero TTL skips the write entirely: a get
    // leaves no row (size 0, not an expired one), and a refresh removes its
    // own row rather than leaving it expired on arrival. A TTL that expires
    // LATER still counts until it is touched (Cache.test.ts:1172, pinned in
    // cache_size_counts_stored_entries), so this is a write-time rule, not
    // an expiry rule.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let zero = Cache::new(1, |_: &String| Ok::<usize, String>(42)).with_ttl(
        TimeToLive::For(Duration::ZERO),
        clock.clone(),
    );
    assert_eq!(zero.get(&"key".to_string()), Ok(42));
    assert_eq!(zero.size(), 0, "a zero-TTL get stores nothing");
    assert!(!zero.has(&"key".to_string()));

    let failing = Cache::new(1, |_: &String| Err::<usize, String>("error".to_string())).with_ttl(
        TimeToLive::For(Duration::ZERO),
        clock.clone(),
    );
    assert_eq!(failing.refresh(&"key".to_string()), Err("error".to_string()));
    assert!(!failing.has(&"key".to_string()));

    // A zero-TTL set also stores nothing, and drops a row that was there.
    zero.set("key".to_string(), 7);
    assert!(!zero.has(&"key".to_string()));
    assert_eq!(zero.size(), 0);
}

// Cache inspection: size, keys, values, entries

#[test]
fn cache_size_counts_stored_entries() {
    // Cache.test.ts:1127, :1139, :1155, :1172 — size is the number of stored
    // rows: 0 when empty, one per get, one fewer after invalidate, and an
    // expired row keeps counting until something touches it.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let cache = Cache::new(10, |key: &String| Ok::<usize, String>(key.len()))
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());

    assert_eq!(cache.size(), 0);
    for key in ["a", "b", "c"] {
        assert!(cache.get(&key.to_string()).is_ok());
    }
    assert_eq!(cache.size(), 3);
    cache.invalidate(&"b".to_string());
    assert_eq!(cache.size(), 2);

    // Two hours on, both rows are expired but still held: size counts what
    // is stored, not what is valid. Touching one drops it.
    clock.adjust(Duration::from_minutes(120));
    assert_eq!(cache.size(), 2);
    assert!(!cache.has(&"a".to_string()));
    assert_eq!(cache.size(), 1);
}

#[test]
fn cache_keys_lists_active_keys() {
    // Cache.test.ts:1197, :1213, :1231 — the unexpired keys; an expired row
    // is removed by the walk rather than merely filtered out, and
    // invalidateAll leaves nothing behind.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let lookup = |key: &String| Ok::<usize, String>(key.len());

    let endless = Cache::new(10, lookup);
    for key in ["a", "b", "c"] {
        endless.get(&key.to_string());
    }
    let mut keys = endless.keys();
    keys.sort();
    assert_eq!(keys, vec!["a".to_string(), "b".to_string(), "c".to_string()]);

    endless.invalidate_all();
    assert!(endless.keys().is_empty());
    assert_eq!(endless.size(), 0);

    let hourly = Cache::new(10, lookup)
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    hourly.get(&"a".to_string());
    hourly.get(&"b".to_string());
    clock.adjust(Duration::from_minutes(120));
    assert!(hourly.keys().is_empty());
    // Removing, not filtering: the expired rows are gone from the map too.
    assert_eq!(hourly.size(), 0);
}

#[test]
fn cache_values_and_entries_exclude_failures_and_expired_rows() {
    // Cache.test.ts:1249, :1267, :1285, :1306, :1324, :1342, :1361 — values
    // and entries report successful rows only, a cached failure is filtered
    // from both (but still counted by size), and expired rows are removed by
    // the walk, so size falls as a side effect.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let ttl = TimeToLive::For(Duration::from_minutes(60));
    let lookup = |key: &String| {
        if key == "fail" {
            Err::<usize, String>("error".to_string())
        } else {
            Ok(key.len())
        }
    };

    let cache = Cache::new(10, lookup).with_ttl(ttl, clock.clone());
    for key in ["a", "ab", "abc"] {
        assert!(cache.get(&key.to_string()).is_ok());
    }
    let mut values = cache.values();
    values.sort();
    assert_eq!(values, vec![1, 2, 3]);
    let mut entries = cache.entries();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            ("a".to_string(), 1),
            ("ab".to_string(), 2),
            ("abc".to_string(), 3)
        ]
    );

    // A cached failure is a stored row, so size counts it, but it is not a
    // value and not an entry.
    let failing = Cache::new(10, lookup)
        .with_ttl(ttl, clock.clone())
        .cache_errors(true);
    assert!(failing.get(&"a".to_string()).is_ok());
    assert!(failing.get(&"fail".to_string()).is_err());
    assert_eq!(failing.size(), 2);
    assert_eq!(failing.values(), vec![1]);
    assert_eq!(failing.entries(), vec![("a".to_string(), 1)]);

    let hourly = Cache::new(10, lookup).with_ttl(ttl, clock.clone());
    hourly.get(&"a".to_string());
    hourly.get(&"b".to_string());
    clock.adjust(Duration::from_minutes(120));
    assert_eq!(hourly.size(), 2);
    assert!(hourly.entries().is_empty());
    assert!(hourly.values().is_empty());
    assert_eq!(hourly.size(), 0, "the walk removed the expired rows");
}

// Pending entries: the interleavings the pin's suspended lookups allow.

/// A lookup that signals `entered` when it starts, blocks until `released`
/// sends, then answers with `outcome`: the pin's suspended lookup, as threads.
fn gated(
    outcome: Result<usize, String>,
) -> (
    impl Fn(&String) -> Result<usize, String>,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
    let lookup = move |_key: &String| {
        entered_tx.send(()).unwrap();
        release_rx.lock().unwrap().recv().unwrap();
        outcome.clone()
    };
    (lookup, entered_rx, release_tx)
}

#[test]
fn refresh_pending_row_is_visible_and_cancel_removes_it() {
    // Cache.test.ts:978 — while a missing-key refresh runs its pending row is
    // a real row (`has` is true), and interrupting the refresh removes only
    // that row: the orphaned lookup still answers its caller, and it never
    // reinstalls the row.
    let (lookup, entered, release) = gated(Ok(7));
    let cache = Arc::new(Cache::new(1, lookup));
    let refresher = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.refresh(&"key".to_string()))
    };
    entered.recv().unwrap();
    assert!(cache.has(&"key".to_string()), "the pending row is visible");
    assert_eq!(cache.size(), 1);

    assert!(cache.cancel_pending(&"key".to_string()));
    assert!(!cache.has(&"key".to_string()), "cancel removed the pending row");

    release.send(()).unwrap();
    assert_eq!(refresher.join().unwrap(), Ok(7), "the orphan still answers");
    assert!(!cache.has(&"key".to_string()), "and never reinstalls its row");
}

#[test]
fn cancelling_a_missing_key_refresh_keeps_a_newer_set_value() {
    // Cache.test.ts:960 — a value written while a missing-key refresh runs is
    // never removed by the refresh's completion or by an interrupt.
    let (lookup, entered, release) = gated(Ok(1));
    let cache = Arc::new(Cache::new(1, lookup));
    let refresher = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.refresh(&"key".to_string()))
    };
    entered.recv().unwrap();
    cache.set("key".to_string(), 99);
    assert_eq!(cache.get_option(&"key".to_string()), Some(99));
    assert!(
        !cache.cancel_pending(&"key".to_string()),
        "the row belongs to the set now"
    );

    release.send(()).unwrap();
    assert_eq!(refresher.join().unwrap(), Ok(1));
    assert_eq!(
        cache.get_option(&"key".to_string()),
        Some(99),
        "the newer value survived the orphaned completion"
    );
}

#[test]
fn zero_ttl_refresh_with_replacement_rules() {
    // Cache.test.ts:995 — a zero-TTL refresh that was UPDATING an existing
    // entry removes the row that is there now, even a value written during
    // the lookup; one that was CREATING an entry removes only its own row, so
    // a replacement survives.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    fn failing_ttl(outcome: &Result<usize, String>, _key: &String) -> TimeToLive {
        match outcome {
            Err(_) => TimeToLive::For(Duration::ZERO),
            Ok(_) => TimeToLive::For(Duration::from_minutes(60)),
        }
    }

    // existing: true — the refresh replaces a live entry.
    let (lookup, entered, release) = gated(Err("error".to_string()));
    let cache = Arc::new(Cache::new(1, lookup).with_ttl_fn(failing_ttl, Arc::new(TestClock::new())));
    cache.set("key".to_string(), 1);
    let refresher = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.refresh(&"key".to_string()))
    };
    entered.recv().unwrap();
    cache.set("key".to_string(), 99);
    assert_eq!(cache.get_option(&"key".to_string()), Some(99));
    release.send(()).unwrap();
    assert_eq!(refresher.join().unwrap(), Err("error".to_string()));
    assert_eq!(
        cache.get_option(&"key".to_string()),
        None,
        "the updating refresh removed the current row"
    );

    // existing: false — the refresh was creating the entry, so the value
    // written mid-lookup is not its row to remove.
    let (lookup, entered, release) = gated(Err("error".to_string()));
    let cache = Arc::new(Cache::new(1, lookup).with_ttl_fn(failing_ttl, Arc::new(TestClock::new())));
    let refresher = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.refresh(&"key".to_string()))
    };
    entered.recv().unwrap();
    cache.set("key".to_string(), 99);
    release.send(()).unwrap();
    assert_eq!(refresher.join().unwrap(), Err("error".to_string()));
    assert_eq!(
        cache.get_option(&"key".to_string()),
        Some(99),
        "the replacement survived"
    );
}

#[test]
fn refresh_reenters_and_trims_when_evicted_during_lookup() {
    // Cache.test.ts:936 — a refreshed key that was evicted while its lookup
    // ran comes back at the back of the queue, and capacity runs again.
    let (lookup, entered, release) = gated(Ok(2));
    let cache = Arc::new(Cache::new(1, lookup));
    cache.set("a".to_string(), 1);
    let refresher = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.refresh(&"a".to_string()))
    };
    entered.recv().unwrap();
    cache.set("b".to_string(), 99);
    assert!(!cache.has(&"a".to_string()), "b evicted a during the lookup");
    release.send(()).unwrap();
    assert_eq!(refresher.join().unwrap(), Ok(2));
    assert_eq!(cache.size(), 1);
    assert_eq!(cache.keys(), vec!["a".to_string()]);
    assert_eq!(cache.get_option(&"a".to_string()), Some(2));
}

#[test]
fn suspended_zero_ttl_get_is_removed_when_it_completes() {
    // Cache.test.ts:208 — a suspended zero-TTL get is a visible row while it
    // runs (size counts it) and leaves nothing when it completes.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let (lookup, entered, release) = gated(Ok(42));
    let cache = Arc::new(Cache::new(1, lookup).with_ttl(
        TimeToLive::For(Duration::ZERO),
        Arc::new(TestClock::new()),
    ));
    let getter = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.get(&"key".to_string()))
    };
    entered.recv().unwrap();
    assert_eq!(cache.size(), 1, "the in-flight row counts");
    release.send(()).unwrap();
    assert_eq!(getter.join().unwrap(), Ok(42));
    assert_eq!(cache.size(), 0, "the completed lookup left no row");
    assert!(!cache.has(&"key".to_string()));
}

#[test]
fn invalidate_when_judges_a_pending_lookup_and_spares_a_newer_value() {
    // Cache.test.ts:818 — the invalidator waits for the in-flight lookup and
    // judges ITS value; when a `set` replaced the row in the meantime the row
    // is spared and the verdict is false. The failing tuple never calls the
    // predicate at all.
    use std::time::Duration as StdDuration;

    let (lookup, entered, release) = gated(Ok(1));
    let cache = Arc::new(Cache::new(1, lookup));
    let getter = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.get(&"key".to_string()))
    };
    entered.recv().unwrap();

    let seen = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let invalidator = {
        let cache = Arc::clone(&cache);
        let seen = Arc::clone(&seen);
        std::thread::spawn(move || {
            cache.invalidate_when(&"key".to_string(), |value| {
                seen.lock().unwrap().push(value);
                value == 1
            })
        })
    };
    // Let the invalidator park on the pending cell before the write lands, as
    // the pin's forked invalidator does.
    std::thread::sleep(StdDuration::from_millis(50));
    cache.set("key".to_string(), 99);
    release.send(()).unwrap();

    assert_eq!(getter.join().unwrap(), Ok(1));
    assert!(!invalidator.join().unwrap());
    assert_eq!(*seen.lock().unwrap(), vec![1], "the predicate saw the old value");
    assert_eq!(cache.get_option(&"key".to_string()), Some(99));

    // Failing tuple: the predicate is never called and the row is spared.
    let (lookup, entered, release) = gated(Err("error".to_string()));
    let cache = Arc::new(Cache::new(1, lookup));
    let getter = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.get(&"key".to_string()))
    };
    entered.recv().unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let invalidator = {
        let cache = Arc::clone(&cache);
        let seen = Arc::clone(&seen);
        std::thread::spawn(move || {
            cache.invalidate_when(&"key".to_string(), |value| {
                seen.lock().unwrap().push(value);
                true
            })
        })
    };
    std::thread::sleep(StdDuration::from_millis(50));
    cache.set("key".to_string(), 99);
    release.send(()).unwrap();

    assert_eq!(getter.join().unwrap(), Err("error".to_string()));
    assert!(!invalidator.join().unwrap());
    assert!(seen.lock().unwrap().is_empty(), "no predicate call for a failure");
    assert_eq!(cache.get_option(&"key".to_string()), Some(99));
}

#[test]
fn zero_ttl_failure_does_not_evict_a_live_entry_at_capacity() {
    // Cache.test.ts:226 — a zero-TTL failure row leaves again without running
    // capacity against the entries that were there first.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    fn failure_is_instant(outcome: &Result<usize, String>, _key: &String) -> TimeToLive {
        match outcome {
            Err(_) => TimeToLive::For(Duration::ZERO),
            Ok(_) => TimeToLive::For(Duration::from_minutes(60)),
        }
    }

    let lookup = |key: &String| {
        if key == "bad" {
            Err("error".to_string())
        } else {
            Ok(key.len())
        }
    };
    let cache = Cache::new(2, lookup).with_ttl_fn(failure_is_instant, Arc::new(TestClock::new()));
    cache.get(&"a".to_string());
    assert_eq!(cache.get(&"bad".to_string()), Err("error".to_string()));
    assert_eq!(cache.size(), 1);
    cache.get(&"b".to_string());
    assert_eq!(cache.get_option(&"a".to_string()), Some(1));
    assert_eq!(cache.get_option(&"b".to_string()), Some(1));
}

#[test]
fn cache_invalidate_when_judges_only_a_held_success_value() {
    // Cache.test.ts:851, :865, :879, :891 — invalidateWhen reports true and
    // drops the row only when the predicate accepts the held success value;
    // a miss keeps the row, and a missing key or a cached failure never
    // calls the predicate at all: there is no value to judge.
    let lookup = |key: &String| {
        if key == "fail" {
            Err::<usize, String>("error".to_string())
        } else {
            Ok(key.len())
        }
    };
    let cache = Cache::new(10, lookup);
    assert_eq!(cache.get(&"test".to_string()), Ok(4));

    let mut called = 0;
    assert!(cache.invalidate_when(&"test".to_string(), |value| {
        called += 1;
        value == 4
    }));
    assert_eq!(called, 1);
    assert!(!cache.has(&"test".to_string()));

    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert!(!cache.invalidate_when(&"test".to_string(), |value| {
        called += 1;
        value == 5
    }));
    assert_eq!(called, 2);
    assert!(cache.has(&"test".to_string()), "a miss keeps the row");

    assert!(!cache.invalidate_when(&"absent".to_string(), |_| {
        called += 1;
        true
    }));
    assert_eq!(called, 2, "no predicate call for a missing key");

    let failing = Cache::new(10, lookup).cache_errors(true);
    assert_eq!(failing.get(&"fail".to_string()), Err("error".to_string()));
    assert!(!failing.invalidate_when(&"fail".to_string(), |_| {
        called += 1;
        true
    }));
    assert_eq!(called, 2, "no predicate call for a cached failure");
    assert!(failing.has(&"fail".to_string()), "the failed row stays");
}

#[test]
fn cache_invalidate_and_invalidate_all_units() {
    // Cache.test.ts:773, :787, :800, :1069, :1087 — invalidate drops exactly
    // one held row and an unknown key is a no-op, the next get reruns the
    // lookup, and invalidateAll empties the map so size reads zero.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(calls.load(Ordering::SeqCst))
        }
    });
    assert_eq!(cache.get(&"a".to_string()), Ok(1));
    cache.invalidate(&"a".to_string());
    assert!(!cache.has(&"a".to_string()));
    cache.invalidate(&"missing".to_string());
    assert_eq!(
        cache.get(&"a".to_string()),
        Ok(2),
        "a get after invalidate reruns the lookup"
    );

    for key in ["a", "b", "c"] {
        assert!(cache.get(&key.to_string()).is_ok());
    }
    cache.invalidate_all();
    for key in ["a", "b", "c"] {
        assert!(!cache.has(&key.to_string()));
    }
    assert_eq!(cache.size(), 0);
}

#[test]
fn scoped_cache_invalidate_when_shares_the_predicate_rule() {
    // ScopedCache.test.ts:848, :862, :876, :888 — the scoped cache answers
    // invalidateWhen exactly like the plain one.
    use cache::TimeToLive;
    use elm_effect::SystemClock;

    let lookup = |key: &String| Ok::<usize, String>(key.len());
    let cache = ScopedCache::make(
        10,
        TimeToLive::Infinite,
        Arc::new(SystemClock),
        Arc::new(Scope::new(11)),
        lookup,
    );
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert!(cache.invalidate_when(&"test".to_string(), |value| value == 4));
    assert!(!cache.has(&"test".to_string()));

    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert!(!cache.invalidate_when(&"test".to_string(), |value| value == 5));
    assert!(cache.has(&"test".to_string()));

    let mut called = 0;
    assert!(!cache.invalidate_when(&"missing".to_string(), |_| {
        called += 1;
        true
    }));
    assert_eq!(called, 0, "no predicate call for a missing key");

    let failing = ScopedCache::make(
        10,
        TimeToLive::Infinite,
        Arc::new(SystemClock),
        Arc::new(Scope::new(12)),
        |_: &String| Err::<usize, String>("error".to_string()),
    );
    assert!(failing.get(&"test".to_string()).is_err());
    assert!(!failing.invalidate_when(&"test".to_string(), |_| true));
}
