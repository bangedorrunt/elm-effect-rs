//! Cache single-flight, request batching, pools, and scoped caches.

#[path = "../src/cache.rs"]
mod cache;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use cache::{Cache, IdlePolicy, Pool, Request, RequestResolver, ScopedCache};
use elm_effect::{Exit, Runtime, Scope};

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
    let plain =
        Cache::new(10, lookup).with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
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
    let pool = Pool::new(4, IdlePolicy::default(), {
        let made = Arc::clone(&made);
        move || {
            made.fetch_add(1, Ordering::SeqCst);
            7u32
        }
    });
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
    let pool = Pool::new(2, IdlePolicy::default(), {
        let made = Arc::clone(&made);
        move || {
            made.fetch_add(1, Ordering::SeqCst);
            7u32
        }
    });
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
    let pool = Pool::new(2, IdlePolicy::default(), {
        let made = Arc::clone(&made);
        move || {
            made.fetch_add(1, Ordering::SeqCst);
            7u32
        }
    });
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
    let pool = Arc::new(Pool::new(3, IdlePolicy::default(), {
        let made = Arc::clone(&made);
        move || {
            made.fetch_add(1, Ordering::SeqCst);
            7u32
        }
    }));
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
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "only the get ran the lookup"
    );
}

#[test]
fn get_option_reports_absent_for_a_cached_failure() {
    // A cached failure is not a value: getOption reports absence rather than
    // resurrecting the error, which is what keeps it a "did I hold this key"
    // question.
    let cache =
        Cache::new(10, |_key: &String| Err::<usize, String>("boom".to_string())).cache_errors(true);
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
    let failing =
        Cache::new(10, |_: &String| Err::<usize, String>("error".to_string())).cache_errors(true);
    assert!(failing.get(&"test".to_string()).is_err());
    assert_eq!(
        failing.refresh(&"test".to_string()),
        Err("error".to_string())
    );
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
    let cache = Cache::new(10, |key: &String| Ok::<usize, String>(key.len()))
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    cache.get(&"test".to_string());
    clock.adjust(Duration::from_minutes(45));
    assert!(cache.has(&"test".to_string()));
    cache.refresh(&"test".to_string());
    clock.adjust(Duration::from_minutes(30));
    assert!(
        cache.has(&"test".to_string()),
        "45 + 30 is past the first TTL"
    );
    clock.adjust(Duration::from_minutes(31));
    assert!(
        !cache.has(&"test".to_string()),
        "61 minutes after the refresh"
    );
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
    assert!(
        !cache.has(&"a".to_string()),
        "refresh did not refresh recency"
    );
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
    let zero = Cache::new(1, |_: &String| Ok::<usize, String>(42))
        .with_ttl(TimeToLive::For(Duration::ZERO), clock.clone());
    assert_eq!(zero.get(&"key".to_string()), Ok(42));
    assert_eq!(zero.size(), 0, "a zero-TTL get stores nothing");
    assert!(!zero.has(&"key".to_string()));

    let failing = Cache::new(1, |_: &String| Err::<usize, String>("error".to_string()))
        .with_ttl(TimeToLive::For(Duration::ZERO), clock.clone());
    assert_eq!(
        failing.refresh(&"key".to_string()),
        Err("error".to_string())
    );
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
    assert_eq!(
        keys,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );

    endless.invalidate_all();
    assert!(endless.keys().is_empty());
    assert_eq!(endless.size(), 0);

    let hourly =
        Cache::new(10, lookup).with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
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
    assert!(
        !cache.has(&"key".to_string()),
        "cancel removed the pending row"
    );

    release.send(()).unwrap();
    assert_eq!(refresher.join().unwrap(), Ok(7), "the orphan still answers");
    assert!(
        !cache.has(&"key".to_string()),
        "and never reinstalls its row"
    );
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
    let cache =
        Arc::new(Cache::new(1, lookup).with_ttl_fn(failing_ttl, Arc::new(TestClock::new())));
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
    let cache =
        Arc::new(Cache::new(1, lookup).with_ttl_fn(failing_ttl, Arc::new(TestClock::new())));
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
    assert!(
        !cache.has(&"a".to_string()),
        "b evicted a during the lookup"
    );
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
    let cache = Arc::new(
        Cache::new(1, lookup).with_ttl(TimeToLive::For(Duration::ZERO), Arc::new(TestClock::new())),
    );
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
    assert_eq!(
        *seen.lock().unwrap(),
        vec![1],
        "the predicate saw the old value"
    );
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
    assert!(
        seen.lock().unwrap().is_empty(),
        "no predicate call for a failure"
    );
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

// Scoped entries: what an entry owns, and when it goes.

/// A tracker both events land in, so their order is observable.
type ResourceLog = Arc<std::sync::Mutex<Vec<String>>>;

fn log_of(log: &ResourceLog) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// The pin's `makeManagedResourceCache`: every entry acquires a resource on
/// its key and releases it when the entry stops being the entry.
fn managed_resource_cache(
    capacity: usize,
    scope: u64,
) -> (ScopedCache<String, String, String>, ResourceLog) {
    use cache::TimeToLive;

    let log: ResourceLog = Arc::new(std::sync::Mutex::new(Vec::new()));
    let cache = ScopedCache::make_resource(
        capacity,
        TimeToLive::Infinite,
        Arc::new(elm_effect::SystemClock),
        Arc::new(Scope::new(scope)),
        {
            let log = Arc::clone(&log);
            move |key: &String| -> (Result<String, String>, Option<cache::Release>) {
                log.lock().unwrap().push(format!("acquired-{key}"));
                let released = Arc::clone(&log);
                let key = key.clone();
                (
                    Ok(format!("value-{key}")),
                    Some(Box::new(move || {
                        released.lock().unwrap().push(format!("released-{key}"));
                    })),
                )
            }
        },
    );
    (cache, log)
}

#[test]
fn scoped_entry_resource_releases_on_eviction() {
    // ScopedCache.test.ts:400, :1660, :2033 — filling past capacity evicts the
    // oldest entry and releases its resource, and the release lands BEFORE the
    // lookup that displaced it acquires its own.
    let (cache, log) = managed_resource_cache(2, 20);
    assert_eq!(cache.get(&"a".to_string()), Ok("value-a".to_string()));
    assert_eq!(cache.get(&"b".to_string()), Ok("value-b".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-a", "acquired-b"]);

    assert_eq!(cache.get(&"c".to_string()), Ok("value-c".to_string()));
    assert_eq!(
        log_of(&log),
        vec!["acquired-a", "acquired-b", "released-a", "acquired-c"],
        "the evicted entry released before the new lookup acquired"
    );
    assert_eq!(cache.size(), 2);
}

#[test]
fn scoped_entry_resource_releases_on_overwrite() {
    // ScopedCache.test.ts:662 — a `set` over a live entry releases what that
    // entry held, and the written value takes its place.
    let (cache, log) = managed_resource_cache(10, 21);
    assert_eq!(cache.get(&"test".to_string()), Ok("value-test".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-test"]);

    cache.set("test".to_string(), "new-value".to_string());
    assert_eq!(log_of(&log), vec!["acquired-test", "released-test"]);
    assert_eq!(
        cache.get_option(&"test".to_string()),
        Some("new-value".to_string())
    );
}

#[test]
fn scoped_entry_resource_releases_on_invalidate() {
    // ScopedCache.test.ts:783 — invalidating an entry releases its resource.
    let (cache, log) = managed_resource_cache(10, 22);
    assert_eq!(cache.get(&"test".to_string()), Ok("value-test".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-test"]);

    cache.invalidate(&"test".to_string());
    assert_eq!(log_of(&log), vec!["acquired-test", "released-test"]);
    assert!(!cache.has(&"test".to_string()));
}

#[test]
fn scoped_entry_resource_releases_when_invalidate_when_matches() {
    // ScopedCache.test.ts:901 — the predicate's `true` releases the resource of
    // the entry it judged; a `false` leaves the entry (and its resource) held.
    let (cache, log) = managed_resource_cache(10, 23);
    assert_eq!(cache.get(&"test".to_string()), Ok("value-test".to_string()));

    assert!(!cache.invalidate_when(&"test".to_string(), |value| value == "value-other"));
    assert_eq!(
        log_of(&log),
        vec!["acquired-test"],
        "no match holds the resource"
    );

    assert!(cache.invalidate_when(&"test".to_string(), |value| value == "value-test"));
    assert_eq!(log_of(&log), vec!["acquired-test", "released-test"]);
    assert!(!cache.has(&"test".to_string()));
}

#[test]
fn scoped_entry_resource_releases_on_refresh() {
    // ScopedCache.test.ts:1074 — a refresh replaces the entry, so the old
    // entry's resource goes and the new lookup's rides the new entry.
    let (cache, log) = managed_resource_cache(10, 24);
    assert_eq!(cache.get(&"test".to_string()), Ok("value-test".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-test"]);

    assert_eq!(
        cache.refresh(&"test".to_string()),
        Ok("value-test".to_string())
    );
    let events = log_of(&log);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.as_str() == "released-test")
            .count(),
        1,
        "the replaced entry released its resource"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.as_str() == "acquired-test")
            .count(),
        2,
        "the refresh ran the lookup again"
    );
    assert_eq!(
        cache.get_option(&"test".to_string()),
        Some("value-test".to_string()),
        "the refreshed entry is held"
    );
}

#[test]
fn scoped_entry_resources_release_on_invalidate_all() {
    // ScopedCache.test.ts:1202 — invalidateAll releases every entry's resource.
    let (cache, log) = managed_resource_cache(10, 25);
    for key in ["test1", "test2", "test3"] {
        assert_eq!(cache.get(&key.to_string()), Ok(format!("value-{key}")));
    }
    assert_eq!(
        log_of(&log),
        vec!["acquired-test1", "acquired-test2", "acquired-test3"]
    );

    cache.invalidate_all();
    let mut released = log_of(&log);
    released.retain(|entry| entry.starts_with("released-"));
    released.sort();
    assert_eq!(
        released,
        vec!["released-test1", "released-test2", "released-test3"],
        "every entry released, whatever the map's order"
    );
    assert_eq!(cache.size(), 0);
}

#[test]
fn scoped_entry_resources_release_on_close() {
    // ScopedCache.test.ts:2058 — closing the cache releases every entry's
    // resource through the scope the cache owns.
    let (cache, log) = managed_resource_cache(10, 26);
    assert_eq!(
        cache.get(&"test1".to_string()),
        Ok("value-test1".to_string())
    );
    assert_eq!(
        cache.get(&"test2".to_string()),
        Ok("value-test2".to_string())
    );
    assert_eq!(log_of(&log), vec!["acquired-test1", "acquired-test2"]);

    cache.close();
    let mut released = log_of(&log);
    released.retain(|entry| entry.starts_with("released-"));
    released.sort();
    assert_eq!(released, vec!["released-test1", "released-test2"]);
    assert_eq!(cache.size(), 0);
}

#[test]
fn scoped_entry_resource_releases_when_has_prunes_expiry() {
    // ScopedCache.test.ts:2093 — reading past the TTL prunes the expired entry
    // and releases its resource.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let log: ResourceLog = Arc::new(std::sync::Mutex::new(Vec::new()));
    let clock = Arc::new(TestClock::new());
    let cache = ScopedCache::make_resource(
        10,
        TimeToLive::For(Duration::from_minutes(60)),
        clock.clone(),
        Arc::new(Scope::new(27)),
        {
            let log = Arc::clone(&log);
            move |key: &String| -> (Result<String, String>, Option<cache::Release>) {
                log.lock().unwrap().push(format!("acquired-{key}"));
                let released = Arc::clone(&log);
                let key = key.clone();
                (
                    Ok(format!("value-{key}")),
                    Some(Box::new(move || {
                        released.lock().unwrap().push(format!("released-{key}"));
                    })),
                )
            }
        },
    );
    assert_eq!(cache.get(&"test".to_string()), Ok("value-test".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-test"]);

    clock.adjust(Duration::from_minutes(120));
    assert!(!cache.has(&"test".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-test", "released-test"]);
}

#[test]
fn scoped_entry_resource_releases_when_a_walk_prunes_expiry() {
    // ScopedCache.test.ts:1340, :1433, :1525, :1733 — the walking reads (keys,
    // values, entries) prune expired rows as a side effect, and pruning
    // releases what the row owned.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let build = |scope: u64| {
        let log: ResourceLog = Arc::new(std::sync::Mutex::new(Vec::new()));
        let clock = Arc::new(TestClock::new());
        let cache = ScopedCache::make_resource(
            10,
            TimeToLive::For(Duration::from_minutes(60)),
            clock.clone(),
            Arc::new(Scope::new(scope)),
            {
                let log = Arc::clone(&log);
                move |key: &String| -> (Result<String, String>, Option<cache::Release>) {
                    log.lock().unwrap().push(format!("acquired-{key}"));
                    let released = Arc::clone(&log);
                    let key = key.clone();
                    (
                        Ok(format!("value-{key}")),
                        Some(Box::new(move || {
                            released.lock().unwrap().push(format!("released-{key}"));
                        })),
                    )
                }
            },
        );
        (cache, log, clock)
    };

    let (cache, log, clock) = build(28);
    cache.get(&"a".to_string());
    clock.adjust(Duration::from_minutes(120));
    assert!(cache.keys().is_empty());
    assert_eq!(
        log_of(&log),
        vec!["acquired-a", "released-a"],
        "keys pruned it"
    );

    let (cache, log, clock) = build(29);
    cache.get(&"b".to_string());
    clock.adjust(Duration::from_minutes(120));
    assert!(cache.values().is_empty());
    assert_eq!(
        log_of(&log),
        vec!["acquired-b", "released-b"],
        "values pruned it"
    );

    let (cache, log, clock) = build(30);
    cache.get(&"c".to_string());
    clock.adjust(Duration::from_minutes(120));
    assert!(cache.entries().is_empty());
    assert_eq!(
        log_of(&log),
        vec!["acquired-c", "released-c"],
        "entries pruned it"
    );
}

#[test]
fn scoped_failed_entry_keeps_its_resource_until_evicted() {
    // ScopedCache.test.ts:1976 — a failed lookup still occupies a row with its
    // resource alive, and eviction at capacity is what releases it.
    use cache::TimeToLive;

    let log: ResourceLog = Arc::new(std::sync::Mutex::new(Vec::new()));
    let cache = ScopedCache::make_resource(
        2,
        TimeToLive::Infinite,
        Arc::new(elm_effect::SystemClock),
        Arc::new(Scope::new(31)),
        {
            let log = Arc::clone(&log);
            move |key: &String| -> (Result<String, String>, Option<cache::Release>) {
                log.lock().unwrap().push(format!("acquired-{key}"));
                let released = Arc::clone(&log);
                let key = key.clone();
                (
                    Err(format!("error-{key}")),
                    Some(Box::new(move || {
                        released.lock().unwrap().push(format!("released-{key}"));
                    })),
                )
            }
        },
    );
    assert_eq!(
        cache.get(&"fail1".to_string()),
        Err("error-fail1".to_string())
    );
    assert_eq!(
        cache.get(&"fail2".to_string()),
        Err("error-fail2".to_string())
    );
    assert_eq!(
        log_of(&log),
        vec!["acquired-fail1", "acquired-fail2"],
        "both failed entries are held"
    );

    assert_eq!(
        cache.get(&"fail3".to_string()),
        Err("error-fail3".to_string())
    );
    assert_eq!(
        log_of(&log),
        vec![
            "acquired-fail1",
            "acquired-fail2",
            "released-fail1",
            "acquired-fail3"
        ],
        "evicting the oldest failure released its resource"
    );
}

#[test]
fn scoped_displaced_completion_releases_only_its_own_resource() {
    // ScopedCache.test.ts:797 — a lookup that no longer owns its row releases
    // the resource IT acquired, never the replacement's, and the replacement's
    // resource stays alive until the cache closes.
    use cache::TimeToLive;

    let log: ResourceLog = Arc::new(std::sync::Mutex::new(Vec::new()));
    let acquired = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let gate_rx = Arc::new(std::sync::Mutex::new(gate_rx));
    let cache = Arc::new(ScopedCache::make_resource(
        1,
        TimeToLive::Infinite,
        Arc::new(elm_effect::SystemClock),
        Arc::new(Scope::new(32)),
        {
            let log = Arc::clone(&log);
            let acquired = Arc::clone(&acquired);
            move |_key: &String| -> (Result<String, String>, Option<cache::Release>) {
                let id = acquired.fetch_add(1, Ordering::SeqCst) + 1;
                log.lock().unwrap().push(format!("acquired-{id}"));
                if id == 1 {
                    entered_tx.send(()).unwrap();
                    gate_rx.lock().unwrap().recv().unwrap();
                }
                let released = Arc::clone(&log);
                (
                    Ok(format!("resource-{id}")),
                    Some(Box::new(move || {
                        released.lock().unwrap().push(format!("released-{id}"));
                    })),
                )
            }
        },
    ));

    let first = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.get(&"key".to_string()))
    };
    entered_rx.recv().unwrap();
    // The first lookup is in flight and owns nothing yet, so this drops the
    // pending row without releasing anything.
    cache.invalidate(&"key".to_string());
    assert_eq!(cache.get(&"key".to_string()), Ok("resource-2".to_string()));
    assert_eq!(log_of(&log), vec!["acquired-1", "acquired-2"]);

    gate_tx.send(()).unwrap();
    assert_eq!(first.join().unwrap(), Ok("resource-1".to_string()));
    assert_eq!(
        log_of(&log),
        vec!["acquired-1", "acquired-2", "released-1"],
        "the displaced completion released its own resource only"
    );
    assert_eq!(
        cache.get_option(&"key".to_string()),
        Some("resource-2".to_string()),
        "the replacement still holds its resource"
    );

    cache.close();
    assert_eq!(
        log_of(&log),
        vec!["acquired-1", "acquired-2", "released-1", "released-2"]
    );
}

#[test]
fn scoped_invalidate_all_empties_the_cache_before_finalizers() {
    // ScopedCache.test.ts:1126 — every row leaves before any finalizer runs,
    // so a finalizer that reads the cache finds it empty.
    use cache::TimeToLive;

    type Observed = Arc<std::sync::Mutex<Vec<(usize, Vec<String>)>>>;
    let observed: Observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let holder: Arc<std::sync::Mutex<std::sync::Weak<ScopedCache<String, String, String>>>> =
        Arc::new(std::sync::Mutex::new(std::sync::Weak::new()));
    let cache = Arc::new(ScopedCache::make_resource(
        10,
        TimeToLive::Infinite,
        Arc::new(elm_effect::SystemClock),
        Arc::new(Scope::new(33)),
        {
            let observed = Arc::clone(&observed);
            let holder = Arc::clone(&holder);
            move |key: &String| -> (Result<String, String>, Option<cache::Release>) {
                let key = key.clone();
                let observed = Arc::clone(&observed);
                let holder = Arc::clone(&holder);
                (
                    Ok(format!("value-{key}")),
                    Some(Box::new(move || {
                        if let Some(cache) = holder.lock().unwrap().upgrade() {
                            observed.lock().unwrap().push((cache.size(), cache.keys()));
                        }
                    })),
                )
            }
        },
    ));
    *holder.lock().unwrap() = Arc::downgrade(&cache);

    assert_eq!(cache.get(&"key".to_string()), Ok("value-key".to_string()));
    cache.invalidate_all();
    assert_eq!(
        *observed.lock().unwrap(),
        vec![(0, Vec::<String>::new())],
        "the finalizer saw an empty cache"
    );
}

#[test]
fn scoped_finalizer_refresh_installs_an_entry_that_survives() {
    // ScopedCache.test.ts:1088 — a finalizer that refreshes while invalidateAll
    // runs owns the entry it installs: the invalidation releases only the
    // resource it displaced, and the refresh's resource goes at close. (The
    // pin also runs a yielded tuple; with no yield point between the two in a
    // synchronous port, this one test covers both.)
    use cache::TimeToLive;

    let released: Arc<std::sync::Mutex<Vec<usize>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let acquired = Arc::new(AtomicUsize::new(0));
    let holder: Arc<std::sync::Mutex<std::sync::Weak<ScopedCache<String, usize, String>>>> =
        Arc::new(std::sync::Mutex::new(std::sync::Weak::new()));
    let cache = Arc::new(ScopedCache::make_resource(
        10,
        TimeToLive::Infinite,
        Arc::new(elm_effect::SystemClock),
        Arc::new(Scope::new(34)),
        {
            let released = Arc::clone(&released);
            let acquired = Arc::clone(&acquired);
            let holder = Arc::clone(&holder);
            move |_key: &String| -> (Result<usize, String>, Option<cache::Release>) {
                let id = acquired.fetch_add(1, Ordering::SeqCst) + 1;
                let released = Arc::clone(&released);
                let holder = Arc::clone(&holder);
                (
                    Ok(id),
                    Some(Box::new(move || {
                        released.lock().unwrap().push(id);
                        if id == 1
                            && let Some(cache) = holder.lock().unwrap().upgrade()
                        {
                            let _ = cache.refresh(&"key".to_string());
                        }
                    })),
                )
            }
        },
    ));
    *holder.lock().unwrap() = Arc::downgrade(&cache);

    assert_eq!(cache.get(&"key".to_string()), Ok(1));
    cache.invalidate_all();
    assert_eq!(
        *released.lock().unwrap(),
        vec![1],
        "only the displaced resource"
    );
    assert_eq!(
        acquired.load(Ordering::SeqCst),
        2,
        "the finalizer's refresh ran"
    );
    assert_eq!(
        cache.get_option(&"key".to_string()),
        Some(2),
        "the entry the finalizer installed survived the invalidation"
    );

    cache.close();
    assert_eq!(*released.lock().unwrap(), vec![1, 2]);
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

#[test]
fn scoped_cache_inspection_family() {
    // ScopedCache.test.ts:1220, :1232, :1248, :1265 — size counts what is
    // stored, an empty cache is zero, and it falls after an invalidation.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let lookup = |k: &String| Ok::<usize, String>(k.len());
    let cache = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(70)),
        lookup,
    );
    assert_eq!(cache.size(), 0);
    cache.get(&"a".to_string());
    cache.get(&"bb".to_string());
    assert_eq!(cache.size(), 2);
    cache.invalidate(&"a".to_string());
    assert_eq!(cache.size(), 1);

    // :1290, :1306, :1324 — keys lists what is stored, minus what expired.
    let mut keys = cache.keys();
    keys.sort();
    assert_eq!(keys, vec!["bb".to_string()]);
    let ttl_cache = ScopedCache::make(
        16,
        TimeToLive::For(Duration::from_millis(1_000)),
        Arc::new(TestClock::new()),
        Arc::new(Scope::new(71)),
        lookup,
    );
    ttl_cache.get(&"live".to_string());
    // The entry is counted until something touches it, then it is gone.
    assert_eq!(ttl_cache.size(), 1);

    // :1379, :1397, :1415 — values lists stored values, skipping expired ones.
    let valued = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(72)),
        lookup,
    );
    valued.get(&"x".to_string());
    valued.get(&"yy".to_string());
    let mut values = valued.values();
    values.sort();
    assert_eq!(values, vec![1, 2]);

    // :1472, :1490, :1508 — entries pairs keys with values.
    let mut entries = valued.entries();
    entries.sort();
    assert_eq!(entries, vec![("x".to_string(), 1), ("yy".to_string(), 2)]);

    // :1148, :1166, :1183 — invalidateAll empties every view.
    valued.invalidate_all();
    assert_eq!(valued.size(), 0);
    assert!(valued.keys().is_empty());
    assert!(valued.values().is_empty());
    assert!(valued.entries().is_empty());
    assert!(valued.get_option(&"x".to_string()).is_none());
}

#[test]
fn scoped_cache_expiry_is_lazy_in_size_and_eager_in_the_views() {
    // ScopedCache.test.ts:1265, :1306, :1415, :1508, :575 — an expired entry
    // is still COUNTED until something reads it, but the keys/values/entries
    // views already leave it out, and reading the entry itself prunes it.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let lookup = |k: &String| Ok::<usize, String>(k.len());
    let cache = ScopedCache::make(
        16,
        TimeToLive::For(Duration::from_millis(1_000)),
        clock.clone(),
        Arc::new(Scope::new(80)),
        lookup,
    );
    // Stored at t=0, so this entry expires at t=1000.
    cache.get(&"a".to_string());
    assert_eq!(cache.size(), 1);

    clock.adjust(Duration::from_millis(1_001));
    cache.get(&"bb".to_string());

    // The expired row is still counted, but no view lists it.
    assert_eq!(cache.size(), 2, "an expired row is counted until accessed");
    assert_eq!(cache.keys(), vec!["bb".to_string()]);
    assert_eq!(cache.values(), vec![2]);
    assert_eq!(cache.entries(), vec![("bb".to_string(), 2)]);

    // Reading the expired entry prunes it, and the count drops to match.
    assert_eq!(cache.get_option(&"a".to_string()), None);
    assert_eq!(cache.size(), 1);
    assert_eq!(cache.get_option(&"bb".to_string()), Some(2));
}

#[test]
fn scoped_cache_hit_miss_and_get_option() {
    // ScopedCache.test.ts:280, :296, :311, :322, :343 — a hit serves the stored
    // value, the same key is looked up once, a miss runs the lookup, and a
    // failing lookup propagates.
    use cache::TimeToLive;
    use elm_effect::TestClock;

    let calls = Arc::new(AtomicUsize::new(0));
    let counting = {
        let calls = Arc::clone(&calls);
        move |k: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(k.len())
        }
    };
    let cache = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        Arc::new(TestClock::new()),
        Arc::new(Scope::new(85)),
        counting,
    );
    assert_eq!(cache.get(&"ab".to_string()), Ok(2));
    assert_eq!(cache.get(&"ab".to_string()), Ok(2));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the second get is a hit");

    // A different key is a fresh lookup.
    assert_eq!(cache.get(&"abc".to_string()), Ok(3));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // :456, :469, :537, :550, :563 — getOption answers Some on a stored value,
    // including one written by `set`, and None for a missing key.
    assert_eq!(cache.get_option(&"ab".to_string()), Some(2));
    assert_eq!(cache.get_option(&"zzz".to_string()), None);
    cache.set("written".to_string(), 9);
    assert_eq!(cache.get_option(&"written".to_string()), Some(9));

    // :343 — a failing lookup reaches the caller as the typed failure.
    let failing = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        Arc::new(TestClock::new()),
        Arc::new(Scope::new(86)),
        |_: &String| Err::<usize, String>("lookup failed".to_string()),
    );
    assert_eq!(cache.get(&"ab".to_string()), Ok(2));
    assert_eq!(
        failing.get(&"k".to_string()),
        Err("lookup failed".to_string())
    );
}

#[test]
fn scoped_cache_fixed_ttl_and_refresh() {
    // ScopedCache.test.ts:1681, :1698, :1718, :1836, :1849 — a fixed TTL
    // expires an entry, the next get runs the lookup again, `has` answers
    // false for the expired row, infinite TTL never expires, and a zero TTL
    // never stores at all.
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
        16,
        TimeToLive::For(Duration::from_millis(1_000)),
        clock.clone(),
        Arc::new(Scope::new(95)),
        counting,
    );
    assert_eq!(cache.get(&"ab".to_string()), Ok(2));
    assert!(cache.has(&"ab".to_string()));
    clock.adjust(Duration::from_millis(1_001));
    assert!(
        !cache.has(&"ab".to_string()),
        "an expired row is not present"
    );
    assert_eq!(
        cache.get(&"ab".to_string()),
        Ok(2),
        "and the lookup runs again"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // Infinite TTL never expires.
    let forever = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(96)),
        |k: &String| Ok::<usize, String>(k.len()),
    );
    forever.get(&"ab".to_string());
    clock.adjust(Duration::from_secs(100_000));
    assert_eq!(forever.get(&"ab".to_string()), Ok(2));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "no re-lookup, no expiry");

    // A zero TTL stores nothing, so every get is a miss that re-runs.
    let zero_calls = Arc::new(AtomicUsize::new(0));
    let zero = {
        let zero_calls = Arc::clone(&zero_calls);
        ScopedCache::make(
            16,
            TimeToLive::For(Duration::from_millis(0)),
            clock.clone(),
            Arc::new(Scope::new(97)),
            move |k: &String| {
                zero_calls.fetch_add(1, Ordering::SeqCst);
                Ok::<usize, String>(k.len())
            },
        )
    };
    assert_eq!(zero.get(&"ab".to_string()), Ok(2));
    assert_eq!(zero.size(), 0, "a zero TTL retains nothing");
    assert_eq!(zero.get(&"ab".to_string()), Ok(2));
    assert_eq!(zero_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn scoped_cache_refresh_reruns_the_lookup() {
    // ScopedCache.test.ts:916, :939, :967, :981, :1036, :1052, :1862, :1931 —
    // refresh re-runs the lookup on a hit, inserts a missing key, resets the
    // TTL, works after a failure, and each concurrent call runs its own.
    use cache::TimeToLive;
    use elm_effect::{Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = {
        let calls = Arc::clone(&calls);
        move |k: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(k.len())
        }
    };
    let cache = ScopedCache::make(
        4,
        TimeToLive::For(Duration::from_millis(1_000)),
        clock.clone(),
        Arc::new(Scope::new(98)),
        counted,
    );
    assert_eq!(cache.get(&"ab".to_string()), Ok(2));
    assert_eq!(cache.refresh(&"ab".to_string()), Ok(2));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "refresh re-runs the lookup"
    );

    // A missing key is inserted at the back and counted.
    assert_eq!(cache.refresh(&"abc".to_string()), Ok(3));
    assert!(cache.has(&"abc".to_string()));

    // The TTL restarts at the refresh, so the entry survives a clock move
    // that would have expired the original.
    clock.adjust(Duration::from_millis(500));
    assert_eq!(cache.refresh(&"ab".to_string()), Ok(2));
    clock.adjust(Duration::from_millis(600));
    assert_eq!(cache.get(&"ab".to_string()), Ok(2), "the TTL restarted");

    // Capacity is still enforced while refreshing.
    let small = ScopedCache::make(
        1,
        TimeToLive::Infinite,
        clock.clone(),
        Arc::new(Scope::new(99)),
        |k: &String| Ok::<usize, String>(k.len()),
    );
    small.refresh(&"a".to_string());
    small.refresh(&"b".to_string());
    assert_eq!(small.size(), 1);
    assert!(small.has(&"b".to_string()));
    assert!(!small.has(&"a".to_string()));
}

#[test]
fn scoped_cache_failures_are_kept_out_of_the_views() {
    // ScopedCache.test.ts:1397, :1490 — a failed lookup is not a stored value,
    // so keys/values/entries never list it.
    use cache::TimeToLive;
    use elm_effect::TestClock;

    let cache = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        Arc::new(TestClock::new()),
        Arc::new(Scope::new(100)),
        |_: &String| Err::<usize, String>("lookup failed".to_string()),
    );
    assert_eq!(
        cache.get(&"k".to_string()),
        Err("lookup failed".to_string())
    );
    assert!(cache.keys().is_empty());
    assert!(cache.values().is_empty());
    assert!(cache.entries().is_empty());
    assert_eq!(cache.size(), 0);
}

#[test]
fn scoped_cache_number_keys() {
    // ScopedCache.test.ts:2371 — a numeric key works the same as a string one.
    use cache::TimeToLive;
    use elm_effect::TestClock;

    let cache = ScopedCache::make(
        16,
        TimeToLive::Infinite,
        Arc::new(TestClock::new()),
        Arc::new(Scope::new(101)),
        |k: &u32| Ok::<usize, String>(*k as usize * 2),
    );
    assert_eq!(cache.get(&21), Ok(42));
    assert_eq!(cache.get(&21), Ok(42));
    assert!(cache.has(&21));
    assert_eq!(cache.keys(), vec![21]);
    assert_eq!(cache.values(), vec![42]);
    assert_eq!(cache.size(), 1);
}

// `Effect.cached` / `Effect.cachedWithTTL` (Effect.test.ts :4292-:4412).
// The pin memoizes an effect value: concurrent callers share one run and
// every later caller replays the recorded exit. This port's descriptions
// are sync-only and interruption is observed by polling, so the memoized
// unit is the same `Body` a fiber runs and the memo itself is an
// imperative handle: the caller that finds the memo empty becomes the
// owner and runs the body inline; every other caller waits on the
// recorded exit, dropping out with an interrupt when its own fiber is
// interrupted (only an owner publishes, so an interrupted waiter never
// poisons the memo).

/// Run `cached.get` inside a throwaway fiber and take the exit.
fn memoized<A, E>(cached: &Arc<cache::Cached<A, E>>) -> Exit<A, E>
where
    A: Clone + Send + 'static,
    E: Clone + Send + 'static,
{
    let cached = Arc::clone(cached);
    Runtime::new()
        .fork_detach(move |ctx| cached.get(ctx))
        .join()
}

#[test]
fn cached_runs_the_effect_once_for_concurrent_callers() {
    use elm_effect::{Deferred, Exit, Runtime};

    let rt = Runtime::new();
    let started = Arc::new(Deferred::<(), ()>::new());
    let release = Arc::new(Deferred::<(), ()>::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let cached = Arc::new(cache::Cached::new({
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        let calls = Arc::clone(&calls);
        move |_ctx| {
            calls.fetch_add(1, Ordering::SeqCst);
            started.complete(());
            while !release.is_done() {
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            Exit::Success(1usize)
        }
    }));

    let mut fibers = Vec::new();
    for _ in 0..3 {
        let cached = Arc::clone(&cached);
        fibers.push(rt.fork_detach_with(
            move |ctx| cached.get(ctx),
            elm_effect::ForkOpts::immediate(),
        ));
    }
    // The owner started; the other two are parked on its run.
    started.wait();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    release.complete(());
    let exits: Vec<Exit<usize, String>> = fibers.into_iter().map(|f| f.join()).collect();
    assert_eq!(
        exits,
        vec![Exit::Success(1), Exit::Success(1), Exit::Success(1)]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cached_replays_failures() {
    use elm_effect::{Cause, Exit};

    let calls = Arc::new(AtomicUsize::new(0));
    let cached = Arc::new(cache::Cached::new({
        let calls = Arc::clone(&calls);
        move |_ctx| {
            calls.fetch_add(1, Ordering::SeqCst);
            Exit::<usize, String>::Failure(Cause::fail("boom".to_string()))
        }
    }));

    let failure = Exit::<usize, String>::Failure(Cause::fail("boom".to_string()));
    assert_eq!(memoized(&cached), failure);
    assert_eq!(memoized(&cached), failure);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cached_interrupting_a_waiter_leaves_the_owner_running() {
    use elm_effect::{Cause, Deferred, Exit, ForkOpts, Runtime};

    let rt = Runtime::new();
    let started = Arc::new(Deferred::<(), ()>::new());
    let release = Arc::new(Deferred::<(), ()>::new());
    let cached = Arc::new(cache::Cached::new({
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        move |ctx| {
            started.complete(());
            while !release.is_done() {
                if ctx.is_interrupted() {
                    return Exit::<usize, String>::Failure(Cause::interrupt(None));
                }
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            Exit::Success(42usize)
        }
    }));

    let owner = {
        let cached = Arc::clone(&cached);
        rt.fork_detach_with(move |ctx| cached.get(ctx), ForkOpts::immediate())
    };
    started.wait();
    let waiter = {
        let cached = Arc::clone(&cached);
        rt.fork_detach_with(move |ctx| cached.get(ctx), ForkOpts::immediate())
    };

    waiter.interrupt();
    let waiter_exit = waiter.join();
    assert!(
        matches!(&waiter_exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the interrupted waiter ends interrupted: {waiter_exit:?}"
    );

    release.complete(());
    assert_eq!(owner.join(), Exit::Success(42));
    assert_eq!(memoized(&cached), Exit::Success(42));
    rt.shutdown();
}

#[test]
fn cached_replays_the_owners_interrupted_exit() {
    use elm_effect::{Cause, Deferred, Exit, ForkOpts, Runtime};

    let rt = Runtime::new();
    let started = Arc::new(Deferred::<(), ()>::new());
    let release = Arc::new(Deferred::<(), ()>::new());
    let cached = Arc::new(cache::Cached::new({
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        move |ctx| {
            started.complete(());
            while !release.is_done() {
                if ctx.is_interrupted() {
                    return Exit::<usize, String>::Failure(Cause::interrupt(None));
                }
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            Exit::Success(42usize)
        }
    }));

    let owner = {
        let cached = Arc::clone(&cached);
        rt.fork_detach_with(move |ctx| cached.get(ctx), ForkOpts::immediate())
    };
    started.wait();

    // The owner's own body observes the interruption, so the exit the
    // owner publishes is the interrupted one and every later caller
    // replays it.
    owner.interrupt();
    let owner_exit = owner.join();
    assert!(
        matches!(&owner_exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the interrupted owner ends interrupted: {owner_exit:?}"
    );
    assert_eq!(memoized(&cached), owner_exit);
    release.complete(());
    rt.shutdown();
}

#[test]
fn cached_with_ttl_selects_ttl_from_each_fresh_exit() {
    use elm_effect::{Duration, Exit, TestClock};
    use std::sync::Mutex;

    let clock = Arc::new(TestClock::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let seen: Arc<Mutex<Vec<Exit<usize, String>>>> = Arc::new(Mutex::new(Vec::new()));
    let cached = Arc::new(
        cache::Cached::new({
            let calls = Arc::clone(&calls);
            move |_ctx| {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                Exit::Success(n)
            }
        })
        .with_ttl(
            {
                let seen = Arc::clone(&seen);
                move |exit: &Exit<usize, String>| {
                    seen.lock().unwrap().push(exit.clone());
                    match exit {
                        Exit::Success(1) => Duration::from_secs(1),
                        _ => Duration::from_secs(3),
                    }
                }
            },
            Arc::clone(&clock) as Arc<dyn elm_effect::ClockImpl>,
        ),
    );

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(*seen.lock().unwrap(), vec![]);
    assert_eq!(memoized(&cached), Exit::Success(1));
    clock.adjust(Duration::from_millis(999));
    assert_eq!(memoized(&cached), Exit::Success(1));
    assert_eq!(*seen.lock().unwrap(), vec![Exit::Success(1)]);

    clock.adjust(Duration::from_millis(1));
    assert_eq!(memoized(&cached), Exit::Success(2));
    clock.adjust(Duration::from_secs(1));
    assert_eq!(memoized(&cached), Exit::Success(2));
    clock.adjust(Duration::from_secs(2));
    assert_eq!(memoized(&cached), Exit::Success(3));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Exit::Success(1), Exit::Success(2), Exit::Success(3)]
    );
}

#[test]
fn cached_with_ttl_skips_failures_and_caches_successes() {
    use elm_effect::{Cause, Duration, Exit, TestClock};
    use std::sync::Mutex;

    let clock = Arc::new(TestClock::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let seen: Arc<Mutex<Vec<Exit<usize, String>>>> = Arc::new(Mutex::new(Vec::new()));
    let cached = Arc::new(
        cache::Cached::new({
            let calls = Arc::clone(&calls);
            move |_ctx| {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    Exit::Failure(Cause::fail("boom".to_string()))
                } else {
                    Exit::Success(n)
                }
            }
        })
        .with_ttl(
            {
                let seen = Arc::clone(&seen);
                move |exit: &Exit<usize, String>| {
                    seen.lock().unwrap().push(exit.clone());
                    if matches!(exit, Exit::Failure(_)) {
                        Duration::ZERO
                    } else {
                        Duration::from_secs(1)
                    }
                }
            },
            Arc::clone(&clock) as Arc<dyn elm_effect::ClockImpl>,
        ),
    );

    assert_eq!(
        memoized(&cached),
        Exit::<usize, String>::Failure(Cause::fail("boom".to_string()))
    );
    assert_eq!(memoized(&cached), Exit::Success(2));
    assert_eq!(memoized(&cached), Exit::Success(2));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            Exit::<usize, String>::Failure(Cause::fail("boom".to_string())),
            Exit::Success(2)
        ]
    );

    clock.adjust(Duration::from_secs(1));
    assert_eq!(memoized(&cached), Exit::Success(3));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            Exit::<usize, String>::Failure(Cause::fail("boom".to_string())),
            Exit::Success(2),
            Exit::Success(3)
        ]
    );
}

#[test]
fn cached_with_ttl_starts_ttl_from_value_creation() {
    use elm_effect::{Cause, ClockImpl, Duration, Exit, ForkOpts, Runtime, TestClock};

    let rt = Runtime::new();
    let clock = Arc::new(TestClock::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let cached = Arc::new(
        cache::Cached::new({
            let calls = Arc::clone(&calls);
            let clock = Arc::clone(&clock);
            move |ctx| {
                let clock = Arc::clone(&clock);
                let interrupted = move || ctx.is_interrupted();
                // The body sleeps before producing: the memo's TTL starts
                // when the value lands, not when the caller asked.
                if clock.sleep(Duration::from_secs(2), &interrupted)
                    != elm_effect::SleepOutcome::Elapsed
                {
                    return Exit::<usize, String>::Failure(Cause::interrupt(None));
                }
                Exit::Success(calls.fetch_add(1, Ordering::SeqCst) + 1)
            }
        })
        .with_ttl(
            |_: &Exit<usize, String>| Duration::from_secs(1),
            Arc::clone(&clock) as Arc<dyn elm_effect::ClockImpl>,
        ),
    );

    let first = {
        let cached = Arc::clone(&cached);
        rt.fork_detach_with(move |ctx| cached.get(ctx), ForkOpts::immediate())
    };
    // Wait until the owner's sleep is scheduled: a TestClock adjust
    // never wakes a sleeper registered after it (the port's stand-in for
    // the pin's `Effect.yieldNow` before the adjust).
    while clock.pending_sleepers() == 0 {
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    // The owner sleeps for two virtual seconds, then the value lands and
    // its one-second TTL starts.
    clock.adjust(Duration::from_secs(2));
    assert_eq!(first.join(), Exit::Success(1));

    let second = {
        let cached = Arc::clone(&cached);
        rt.fork_detach_with(move |ctx| cached.get(ctx), ForkOpts::immediate())
    };
    // The pin's `Effect.yieldNow` before its adjust: the caller replays
    // the recorded exit first (a replay needs no time, so it never parks
    // on the clock), and only then does virtual time move. Were the TTL
    // measured from the fiber's request rather than the value's creation,
    // the entry would already be expired here and the run would sleep.
    while !second.is_done() {
        assert_eq!(
            clock.pending_sleepers(),
            0,
            "the second run must replay, not re-run"
        );
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    clock.adjust(Duration::from_secs(2));
    assert_eq!(second.join(), Exit::Success(1));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    rt.shutdown();
}

// ---------------------------------------------------------------------------
// Cache.test.ts's open rows, carried on the port's imperative Cache
// (`Cache::new` + `with_ttl` / `with_ttl_fn` + TestClock). The pin's
// `Cache.make({ capacity, lookup })` with an Effect-returning lookup
// renders as the port's closure-returning-`Result` lookup; the TTL
// families render as `TimeToLive` fixed or per-exit/key/value functions.
// ---------------------------------------------------------------------------

use cache::TimeToLive;
use elm_effect::{Duration, TestClock};

fn clock() -> Arc<TestClock> {
    Arc::new(TestClock::new())
}

fn len_lookup() -> impl Fn(&String) -> Result<usize, String> + Send + Sync + 'static {
    |key: &String| Ok(key.len())
}

#[test]
fn cache_default_ttl_never_expires() {
    // Cache.test.ts:46 — an unconfigured cache has an infinite TTL.
    let clock = clock();
    let cache = Cache::new(10, len_lookup()).with_ttl(TimeToLive::Infinite, clock.clone());
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_secs(1000 * 3600));
    assert!(cache.has(&"test".to_string()));
}

#[test]
fn cache_custom_ttl_expires_between_advances() {
    // Cache.test.ts:59 — a one-hour TTL: live at 30 minutes, gone at 61.
    let clock = clock();
    let cache = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_minutes(30));
    assert!(cache.has(&"test".to_string()));
    clock.adjust(Duration::from_minutes(31));
    assert!(!cache.has(&"test".to_string()));
}

#[test]
fn cache_zero_ttl_expires_immediately() {
    // Cache.test.ts:75 — a numeric zero TTL stores nothing live.
    let clock = clock();
    let cache = Cache::new(10, len_lookup()).with_ttl(TimeToLive::For(Duration::ZERO), clock);
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert!(!cache.has(&"test".to_string()));
}

#[test]
fn cache_make_with_ttl_by_outcome_and_key() {
    // Cache.test.ts:133 — makeWith's TTL function selects by the
    // lookup's exit and the key: a failure lives one second, "k" lives a
    // minute, everything else an hour.
    let clock = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new_owned(10, {
        let calls = Arc::clone(&calls);
        move |key: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            if key == "fail" {
                (Err("error".to_string()), None)
            } else {
                (Ok(key.len()), None)
            }
        }
    })
    .with_ttl_fn(
        move |outcome: &Result<usize, String>, key: &String| {
            if outcome.is_err() {
                TimeToLive::For(Duration::from_secs(1))
            } else if key == "k" {
                TimeToLive::For(Duration::from_minutes(1))
            } else {
                TimeToLive::For(Duration::from_minutes(60))
            }
        },
        clock.clone(),
    );
    assert_eq!(cache.get(&"short".to_string()), Ok(5));
    assert!(cache.get(&"fail".to_string()).is_err());
    assert_eq!(cache.get(&"k".to_string()), Ok(1));

    clock.adjust(Duration::from_secs(2));
    assert!(
        !cache.has(&"fail".to_string()),
        "a failure's one-second TTL lapsed"
    );
    clock.adjust(Duration::from_minutes(1));
    assert!(
        !cache.has(&"k".to_string()),
        "\"k\"'s one-minute TTL lapsed"
    );
    assert!(cache.has(&"short".to_string()), "an hour is still live");
}

#[test]
fn cache_hit_returns_the_cached_value_once() {
    // Cache.test.ts:304.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(4)
        }
    });
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_hit_serves_distinct_keys() {
    // Cache.test.ts:316.
    let cache = Cache::new(10, len_lookup());
    assert_eq!(cache.get(&"a".to_string()), Ok(1));
    assert_eq!(cache.get(&"ab".to_string()), Ok(2));
    assert_eq!(cache.get(&"abc".to_string()), Ok(3));
}

#[test]
fn cache_miss_runs_the_lookup() {
    // Cache.test.ts:347.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(42)
        }
    });
    assert_eq!(cache.get(&"test".to_string()), Ok(42));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_miss_after_ttl_expiry_reruns_the_lookup() {
    // Cache.test.ts:358.
    let clock = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| Ok::<usize, String>(calls.fetch_add(1, Ordering::SeqCst) + 1)
    })
    .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    assert_eq!(cache.get(&"test".to_string()), Ok(1));
    clock.adjust(Duration::from_minutes(120));
    assert_eq!(cache.get(&"test".to_string()), Ok(2));
}

#[test]
fn cache_get_success_shapes() {
    // Cache.test.ts:604, :617, :630, :642 — the pin's getSuccess is Some
    // only for a live successful entry: absent, failed, and expired all
    // read None. The port's get_option is that door (a miss never runs
    // the lookup; a cached failure and a pending row are not values).
    let clock = clock();
    let good = Cache::new(10, len_lookup()).with_ttl(TimeToLive::Infinite, clock.clone());
    assert_eq!(good.get(&"test".to_string()), Ok(4));
    assert_eq!(good.get_option(&"test".to_string()), Some(4));
    assert_eq!(good.get_option(&"absent".to_string()), None);

    let failing =
        Cache::new(10, |_: &String| Err::<usize, String>("error".to_string())).cache_errors(true);
    assert!(failing.get(&"test".to_string()).is_err());
    assert_eq!(failing.get_option(&"test".to_string()), None);

    let expiring = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    assert_eq!(expiring.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_minutes(120));
    assert_eq!(expiring.get_option(&"test".to_string()), None);
}

#[test]
fn cache_get_option_waits_for_the_in_flight_lookup() {
    // Cache.test.ts:582 — a waiter for an in-flight lookup sees the same
    // value the owner produced (the port's single-flight: the second
    // caller blocks on the pending row instead of running the lookup).
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok::<usize, String>(42)
        }
    }));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let cache = Arc::clone(&cache);
        handles.push(std::thread::spawn(move || cache.get(&"test".to_string())));
    }
    let results: Vec<Result<usize, String>> =
        handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results, vec![Ok(42), Ok(42), Ok(42), Ok(42)]);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the lookup ran once for every caller"
    );
    assert_eq!(cache.get_option(&"test".to_string()), Some(42));
}

#[test]
fn cache_set_writes_without_the_lookup() {
    // Cache.test.ts:661, :672, :701, :712 — a set stores a value, never
    // invokes the lookup, overwrites in place, and respects capacity.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(2, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<usize, String>(0)
        }
    });
    cache.set("test".to_string(), 42);
    assert_eq!(cache.get(&"test".to_string()), Ok(42));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a hit must not run the lookup"
    );

    cache.set("test".to_string(), 100);
    assert_eq!(cache.get(&"test".to_string()), Ok(100));

    cache.set("a".to_string(), 1);
    assert_eq!(cache.size(), 2, "capacity 2 holds test and a");
    cache.set("b".to_string(), 2);
    assert_eq!(cache.size(), 2, "a third write evicts the oldest");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no write ran the lookup");
}

#[test]
fn cache_set_with_ttl_expires() {
    // Cache.test.ts:686 — a set value lives under the cache's TTL.
    let clock = clock();
    let cache = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    cache.set("test".to_string(), 42);
    assert!(cache.has(&"test".to_string()));
    clock.adjust(Duration::from_minutes(120));
    assert!(!cache.has(&"test".to_string()));
}

#[test]
fn cache_has_reports_presence_and_expiry() {
    // Cache.test.ts:731, :744, :756 — has is true only for a populated,
    // unexpired key; a never-populated key is false.
    let clock = clock();
    let cache = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    assert!(
        cache.has(&"test".to_string()),
        "a populated key reads present"
    );
    assert!(!cache.has(&"absent".to_string()));
    clock.adjust(Duration::from_minutes(120));
    assert!(!cache.has(&"test".to_string()));
}

#[test]
fn cache_invalidate_all_empties_every_key() {
    // Cache.test.ts:1104.
    let cache = Cache::new(10, len_lookup());
    assert_eq!(cache.get(&"a".to_string()), Ok(1));
    assert_eq!(cache.get(&"b".to_string()), Ok(1));
    cache.invalidate_all();
    assert_eq!(cache.get_option(&"a".to_string()), None);
    assert_eq!(cache.get_option(&"b".to_string()), None);
    assert_eq!(cache.size(), 0);
}

#[test]
fn cache_capacity_evicts_lru_order() {
    // Cache.test.ts:1390, :1408, :1429 — capacity evicts the least
    // recently used entry, and a hit refreshes recency.
    let cache = Cache::new(3, len_lookup());
    for key in ["a", "b", "c"] {
        assert!(cache.get(&key.to_string()).is_ok());
    }
    assert!(cache.get(&"d".to_string()).is_ok());
    assert!(
        !cache.has(&"a".to_string()),
        "a is the oldest and is evicted"
    );
    assert!(
        cache.has(&"b".to_string()) && cache.has(&"c".to_string()) && cache.has(&"d".to_string())
    );

    // Reading "b" makes it the most recent, so the next insert evicts "c".
    let cache = Cache::new(3, len_lookup());
    for key in ["a", "b", "c"] {
        assert!(cache.get(&key.to_string()).is_ok());
    }
    assert!(cache.get(&"a".to_string()).is_ok());
    assert!(cache.get(&"d".to_string()).is_ok());
    assert!(cache.has(&"a".to_string()), "the refreshed key survives");
    assert!(!cache.has(&"b".to_string()), "the stale key is evicted");
}

#[test]
fn cache_capacity_of_one_and_very_large_capacity() {
    // Cache.test.ts:1429, :1444.
    let tiny = Cache::new(1, len_lookup());
    assert_eq!(tiny.get(&"a".to_string()), Ok(1));
    assert!(tiny.has(&"a".to_string()));
    assert_eq!(tiny.get(&"b".to_string()), Ok(1));
    assert!(!tiny.has(&"a".to_string()));
    assert!(tiny.has(&"b".to_string()));

    let wide = Cache::new(10_000, len_lookup());
    for i in 0..100 {
        assert!(wide.get(&format!("key{i}")).is_ok());
    }
    assert_eq!(wide.size(), 100);
    assert!(wide.has(&"key0".to_string()) && wide.has(&"key99".to_string()));
}

#[test]
fn cache_ttl_family_on_a_fixed_duration() {
    // Cache.test.ts:1464, :1480, :1497.
    let clock = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| Ok::<usize, String>(calls.fetch_add(1, Ordering::SeqCst) + 1)
    })
    .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    assert_eq!(cache.get(&"test".to_string()), Ok(1));
    clock.adjust(Duration::from_minutes(59));
    assert!(cache.has(&"test".to_string()));
    clock.adjust(Duration::from_minutes(2));
    assert!(!cache.has(&"test".to_string()), "expired at 61 minutes");
    assert_eq!(
        cache.get(&"test".to_string()),
        Ok(2),
        "a get after expiry re-runs the lookup"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cache_ttl_function_by_outcome() {
    // Cache.test.ts:1512 — successes live an hour, failures a minute.
    let clock = clock();
    let cache = Cache::new_owned(10, |key: &String| {
        if key == "fail" {
            (Err("error".to_string()), None)
        } else {
            (Ok(key.len()), None)
        }
    })
    .with_ttl_fn(
        |outcome: &Result<usize, String>, _key: &String| {
            if outcome.is_ok() {
                TimeToLive::For(Duration::from_minutes(60))
            } else {
                TimeToLive::For(Duration::from_minutes(1))
            }
        },
        clock.clone(),
    )
    .cache_errors(true);
    assert!(cache.get(&"success".to_string()).is_ok());
    assert!(cache.get(&"fail".to_string()).is_err());
    clock.adjust(Duration::from_minutes(30));
    assert!(cache.has(&"success".to_string()));
    assert!(!cache.has(&"fail".to_string()));
}

#[test]
fn cache_ttl_function_by_key_and_value() {
    // Cache.test.ts:1530, :1545 — the TTL function reads the key and the
    // decoded value, not just the outcome.
    let mut first = clock();
    let by_key = Cache::new(10, len_lookup()).with_ttl_fn(
        |_outcome: &Result<usize, String>, key: &String| {
            if key == "short" {
                TimeToLive::For(Duration::from_minutes(1))
            } else {
                TimeToLive::For(Duration::from_minutes(60))
            }
        },
        first.clone(),
    );
    assert!(by_key.get(&"short".to_string()).is_ok());
    assert!(by_key.get(&"long".to_string()).is_ok());
    first.adjust(Duration::from_minutes(30));
    assert!(!by_key.has(&"short".to_string()));
    assert!(by_key.has(&"long".to_string()));

    let mut clock2 = clock();
    let by_value = Cache::new(10, len_lookup()).with_ttl_fn(
        |outcome: &Result<usize, String>, _key: &String| {
            let value = outcome.as_ref().copied().unwrap_or(0);
            if value > 3 {
                TimeToLive::For(Duration::from_minutes(60))
            } else {
                TimeToLive::For(Duration::from_minutes(1))
            }
        },
        clock2.clone(),
    );
    assert!(by_value.get(&"ab".to_string()).is_ok());
    assert!(by_value.get(&"abcd".to_string()).is_ok());
    clock2.adjust(Duration::from_minutes(30));
    assert!(!by_value.has(&"ab".to_string()));
    assert!(by_value.has(&"abcd".to_string()));
}

#[test]
fn cache_ttl_infinite_and_zero_from_the_function() {
    // Cache.test.ts:1563, :1575 — the TTL function's infinity and zero
    // behave like the fixed forms.
    let mut first = clock();
    let infinite = Cache::new(10, len_lookup()).with_ttl_fn(
        |_: &Result<usize, String>, _: &String| TimeToLive::Infinite,
        first.clone(),
    );
    assert!(infinite.get(&"test".to_string()).is_ok());
    first.adjust(Duration::from_minutes(60_000));
    assert!(infinite.has(&"test".to_string()));

    let mut later = clock();
    let zero = Cache::new(10, len_lookup()).with_ttl_fn(
        |_: &Result<usize, String>, _: &String| TimeToLive::For(Duration::ZERO),
        later.clone(),
    );
    assert!(zero.get(&"test".to_string()).is_ok());
    assert!(!zero.has(&"test".to_string()));
}

#[test]
fn cache_refresh_resets_the_ttl() {
    // Cache.test.ts:1588 — a refresh re-runs the lookup and restarts the
    // TTL from the refreshed value, not from the original store.
    let clock = clock();
    let cache = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), clock.clone());
    assert_eq!(cache.get(&"test".to_string()), Ok(4));
    clock.adjust(Duration::from_minutes(45));
    cache.refresh(&"test".to_string());
    clock.adjust(Duration::from_minutes(30));
    assert!(
        cache.has(&"test".to_string()),
        "45 + 30 minutes, but the TTL restarted at 45"
    );
    clock.adjust(Duration::from_minutes(31));
    assert!(
        !cache.has(&"test".to_string()),
        "61 minutes after the refresh"
    );
}

#[test]
fn cache_failed_lookups_are_cached_when_opted_in() {
    // Cache.test.ts:1613, :1628, :1643 — a failure caches like a value
    // when cacheErrors is on, replays to later gets, and a refresh can
    // replace it once the lookup succeeds.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<usize, String>("error".to_string())
        }
    })
    .cache_errors(true);
    assert_eq!(cache.get(&"test".to_string()), Err("error".to_string()));
    assert_eq!(cache.get(&"test".to_string()), Err("error".to_string()));
    assert_eq!(cache.get(&"test".to_string()), Err("error".to_string()));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the failure replay: one lookup"
    );

    let succeed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flaky = Cache::new(10, {
        let succeed = Arc::clone(&succeed);
        move |_: &String| {
            if succeed.load(Ordering::SeqCst) {
                Ok::<usize, String>(42)
            } else {
                Err("error".to_string())
            }
        }
    })
    .cache_errors(true);
    assert!(flaky.get(&"test".to_string()).is_err());
    succeed.store(true, Ordering::SeqCst);
    assert_eq!(flaky.refresh(&"test".to_string()), Ok(42));
}

#[test]
fn cache_failure_single_flight_across_threads() {
    // Cache.test.ts:1659 — concurrent callers share one failing lookup.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(
        Cache::new(10, {
            let calls = Arc::clone(&calls);
            move |_: &String| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(50));
                Err::<usize, String>("error".to_string())
            }
        })
        .cache_errors(true),
    );
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let cache = Arc::clone(&cache);
            std::thread::spawn(move || cache.get(&"key".to_string()))
        })
        .collect();
    let results: Vec<Result<usize, String>> =
        handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results, vec![Err("error".to_string()); 3]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_ttl_measured_per_entry_against_the_clock() {
    // Cache.test.ts:1690, :1712 — each entry's TTL runs from its own
    // store, so staggered gets expire in order; the pin's setTime is
    // adjust from zero in the port.
    let mut first = clock();
    let cache = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_minutes(60)), first.clone());
    assert!(cache.get(&"a".to_string()).is_ok());
    first.adjust(Duration::from_minutes(30));
    assert!(cache.get(&"b".to_string()).is_ok());
    first.adjust(Duration::from_minutes(30));
    assert!(cache.get(&"c".to_string()).is_ok());
    first.adjust(Duration::from_minutes(30));
    assert!(!cache.has(&"a".to_string()), "stored at 0, expired at 90");
    assert!(!cache.has(&"b".to_string()), "stored at 30, expired at 90");
    assert!(cache.has(&"c".to_string()), "stored at 60, expires at 120");

    let mut later = clock();
    let millis = Cache::new(10, len_lookup())
        .with_ttl(TimeToLive::For(Duration::from_millis(1000)), later.clone());
    assert!(millis.get(&"test".to_string()).is_ok());
    later.adjust(Duration::from_millis(500));
    assert!(millis.has(&"test".to_string()));
    later.adjust(Duration::from_millis(1000));
    assert!(!millis.has(&"test".to_string()));
}

#[test]
fn cache_serves_any_hashable_key_type() {
    // Cache.test.ts:1728, :1742, :1758 — strings, numbers, and structural
    // object keys all work, and an equal-by-value key hits the entry.
    let strings = Cache::new(10, len_lookup());
    assert!(strings.get(&"hello".to_string()).is_ok());
    assert!(strings.get(&"world".to_string()).is_ok());
    assert_eq!(strings.get(&"hello".to_string()), Ok(5));

    let numbers = Cache::new(10, |key: &i64| {
        Ok::<String, String>(format!("number: {key}"))
    });
    for key in [1, 2, 3] {
        assert!(numbers.get(&key).is_ok());
    }
    assert_eq!(numbers.get(&1), Ok("number: 1".to_string()));

    #[derive(Clone, Debug, PartialEq, Eq, std::hash::Hash)]
    struct CacheKey {
        id: i64,
        name: String,
    }
    let objects = Cache::new(10, |key: &CacheKey| {
        Ok::<String, String>(format!("{}-{}", key.name, key.id))
    });
    let key1 = CacheKey {
        id: 1,
        name: "test".to_string(),
    };
    let key2 = CacheKey {
        id: 2,
        name: "test".to_string(),
    };
    let key3 = CacheKey {
        id: 1,
        name: "test".to_string(),
    };
    assert!(objects.get(&key1).is_ok());
    assert!(objects.get(&key2).is_ok());
    assert_eq!(
        objects.get(&key3),
        Ok("test-1".to_string()),
        "an equal key hits the entry"
    );
}

#[test]
fn cache_zero_ttl_lookup_does_not_evict_a_live_entry() {
    // Cache.test.ts:244 — a failing lookup whose TTL is zero stores
    // nothing, so the live entries around it stay and size holds.
    let clock = clock();
    let k_lookups = Arc::new(AtomicUsize::new(0));
    let cache = Cache::new_owned(2, {
        let k_lookups = Arc::clone(&k_lookups);
        move |key: &String| {
            if key == "k" {
                let attempt = k_lookups.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt == 2 {
                    return (Err("error".to_string()), None);
                }
            }
            (Ok(key.len()), None)
        }
    })
    .with_ttl_fn(
        |outcome: &Result<usize, String>, key: &String| {
            if outcome.is_err() {
                TimeToLive::For(Duration::ZERO)
            } else if key == "k" {
                TimeToLive::For(Duration::from_minutes(1))
            } else {
                TimeToLive::For(Duration::from_minutes(60))
            }
        },
        clock.clone(),
    );
    assert_eq!(cache.get(&"a".to_string()), Ok(1));
    assert_eq!(cache.get(&"k".to_string()), Ok(1));
    clock.adjust(Duration::from_minutes(2));
    assert_eq!(cache.get(&"k".to_string()), Err("error".to_string()));
    assert_eq!(cache.get(&"b".to_string()), Ok(1));
    assert_eq!(cache.get_option(&"a".to_string()), Some(1));
    assert_eq!(cache.get_option(&"b".to_string()), Some(1));
    assert_eq!(cache.size(), 2, "the zero-TTL failure stored nothing");
}

#[test]
fn cache_zero_ttl_completion_keeps_a_newer_set_value() {
    // Cache.test.ts:268 — a lookup that sets the same key while it runs
    // and then fails completes into nothing: the newer set value is still
    // held. The lookup reaches the cache through a weak handle to the
    // slot, the port's stand-in for the pin's fiber interleaving (the
    // set lands mid-lookup).
    use std::sync::OnceLock;
    let slot: Arc<OnceLock<Cache<String, i64, String>>> = Arc::new(OnceLock::new());
    let weak = Arc::downgrade(&slot);
    let cache = Cache::new_owned(1, move |_key: &String| {
        if let Some(slot) = weak.upgrade()
            && let Some(cache) = slot.get()
        {
            cache.set("key".to_string(), 99);
        }
        (Err("error".to_string()), None)
    });
    if slot.set(cache).is_err() {
        panic!("the slot must be empty until the first lookup runs");
    }
    let cache = slot.get().expect("just stored");
    assert_eq!(cache.get(&"key".to_string()), Err("error".to_string()));
    assert_eq!(cache.get_option(&"key".to_string()), Some(99));
    assert_eq!(cache.size(), 1);
}

#[test]
fn cache_get_and_invalidate_race_reruns_the_lookup() {
    // Cache.test.ts:409 — invalidating while a lookup is in flight
    // detaches the pending row: the owner's completion is dropped and
    // the next get runs the lookup again. Real sleeps stand in for the
    // pin's virtual-time delay.
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(Cache::new(10, {
        let calls = Arc::clone(&calls);
        move |_: &String| {
            let attempt = calls.fetch_add(1, Ordering::SeqCst) + 1;
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok::<i64, String>(attempt as i64)
        }
    }));
    let in_flight = {
        let cache = Arc::clone(&cache);
        std::thread::spawn(move || cache.get(&"test".to_string()))
    };
    std::thread::sleep(std::time::Duration::from_millis(20));
    cache.invalidate(&"test".to_string());
    assert_eq!(cache.get(&"test".to_string()), Ok(2));
    assert_eq!(
        in_flight.join().unwrap(),
        Ok(1),
        "the detached owner still returns its own value"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the second get ran a fresh lookup"
    );
}

#[test]
fn cache_interrupted_lookup_keeps_a_newer_set_value() {
    // Cache.test.ts:518 — the getter's interruption detaches the pending
    // row (the port's cancel_pending, the stand-in for the pin's fiber
    // interrupt), so the lookup's failing completion no longer owns the
    // row, and the set that landed mid-lookup survives.
    use std::sync::OnceLock;
    let slot: Arc<OnceLock<Cache<String, i64, String>>> = Arc::new(OnceLock::new());
    let weak = Arc::downgrade(&slot);
    let built = Cache::new_owned(1, move |_key: &String| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Some(slot) = weak.upgrade()
            && let Some(cache) = slot.get()
        {
            // The set lands while the lookup is still in flight.
            cache.set("key".to_string(), 99);
        }
        (Err("error".to_string()), None)
    });
    if Arc::clone(&slot).set(built).is_err() {
        panic!("the slot must be empty until the getter starts");
    }

    let slot_for_getter = Arc::clone(&slot);
    let getter = std::thread::spawn(move || slot_for_getter.get().unwrap().get(&"key".to_string()));
    std::thread::sleep(std::time::Duration::from_millis(20));
    // The interruption: the pending row is detached from the map.
    assert!(slot.get().unwrap().cancel_pending(&"key".to_string()));
    assert_eq!(getter.join().unwrap(), Err("error".to_string()));
    let cache = slot.get().unwrap();
    assert_eq!(cache.get_option(&"key".to_string()), Some(99));
    assert_eq!(cache.size(), 1);
}
