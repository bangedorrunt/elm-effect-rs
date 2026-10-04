//! The remote stack: limiter and bulkhead waits precede the attempt timeout;
//! attempts are finite; releases run per attempt; the breaker is typed.

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use elm_effect::{
    BreakerConfig, ClockImpl, Duration, Effect, Exit, RateLimiter, Remote, RemoteError,
    RemotePolicy, Runtime, Scope, TestClock,
};

fn fast_policy<A: Clone + Send + 'static, E: Send + 'static>() -> RemotePolicy<A, E> {
    RemotePolicy {
        retries: 2,
        backoff: Duration::ZERO,
        ..Default::default()
    }
}

fn clock() -> Arc<dyn ClockImpl> {
    Arc::new(TestClock::new())
}

#[test]
fn attempt_timeout_does_not_cover_the_queue() {
    struct GateLimiter {
        gate: Mutex<bool>,
        parked: AtomicBool,
        release: Condvar,
    }

    impl RateLimiter for GateLimiter {
        fn acquire(&self) {
            self.parked.store(true, Ordering::SeqCst);
            let mut gate = self.gate.lock().unwrap();
            while !*gate {
                gate = self.release.wait(gate).unwrap();
            }
        }
    }

    let limiter = Arc::new(GateLimiter {
        gate: Mutex::new(false),
        parked: AtomicBool::new(false),
        release: Condvar::new(),
    });
    let policy = RemotePolicy {
        limiter: Some(limiter.clone()),
        ..fast_policy()
    };
    let remote = Remote::new(policy);
    let rt = Runtime::new();
    let test_clock = Arc::new(TestClock::new());
    let clock: Arc<dyn ClockImpl> = test_clock.clone();
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let exit = std::thread::scope(|s| {
        let handle = s.spawn(|| {
            let factory_calls = Arc::clone(&factory_calls);
            let done = Arc::clone(&done);
            let exit = remote.call(&rt, &clock, move || {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::succeed(7)
            });
            done.store(true, Ordering::SeqCst);
            exit
        });
        while !limiter.parked.load(Ordering::SeqCst) {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        // Still queued: advancing virtual time past the attempt timeout
        // fires nothing, and the factory never started.
        test_clock.adjust(Duration::from_secs(60));
        assert!(!done.load(Ordering::SeqCst));
        assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
        *limiter.gate.lock().unwrap() = true;
        limiter.release.notify_all();
        handle.join().unwrap()
    });
    assert!(matches!(exit, Exit::Success(7)));
    assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn remote_policy_is_capped() {
    let policy = RemotePolicy {
        retries: 3,
        backoff: Duration::ZERO,
        ..Default::default()
    };
    // Defaults are exactly the specified caps.
    let defaults = RemotePolicy::<u32, String>::default();
    assert_eq!(defaults.retries, 3);
    assert_eq!(defaults.backoff.as_millis(), 250);
    assert_eq!(defaults.ceiling.as_millis(), 10_000);
    assert_eq!(defaults.attempt_timeout.as_millis(), 30_000);
    assert_eq!(defaults.budget.as_millis(), 120_000);
    assert_eq!(defaults.bulkhead.permits, 8);
    assert_eq!(defaults.bulkhead.queue, 32);
    assert_eq!(defaults.bulkhead.wait.as_millis(), 5_000);
    assert!(defaults.limiter.is_none());
    assert!(defaults.breaker.is_none());

    let remote = Remote::new(policy);
    let rt = Runtime::new();
    let clock = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = remote.call(&rt, &clock, {
        let calls = Arc::clone(&calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Effect::<u32, String, ()>::fail("down".to_string())
        }
    });
    // One initial attempt plus three retries: finite.
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    match exit {
        Exit::Failure(c) => assert_eq!(c.failure(), Some(&RemoteError::Inner("down".to_string()))),
        Exit::Success(_) => panic!("expected failure"),
    }
}

#[test]
fn retry_releases_per_attempt() {
    let scope = Arc::new(Scope::new(80));
    let releases = Arc::new(AtomicUsize::new(0));
    let outer_scope = Scope::new(81);
    let outer_releases = Arc::new(AtomicUsize::new(0));
    outer_scope
        .acquire_release(Effect::<(), String>::succeed(()), {
            let outer_releases = Arc::clone(&outer_releases);
            move |_| {
                outer_releases.fetch_add(1, Ordering::SeqCst);
                Effect::<(), String>::succeed(())
            }
        })
        .expect("acquire cannot fail");

    let remote = Remote::new(fast_policy());
    let rt = Runtime::new();
    let clock = clock();
    let exit = remote.call(&rt, &clock, {
        let (scope, releases) = (Arc::clone(&scope), Arc::clone(&releases));
        move || {
            scope
                .acquire_release(Effect::<(), String>::succeed(()), {
                    let releases = Arc::clone(&releases);
                    move |_| {
                        releases.fetch_add(1, Ordering::SeqCst);
                        Effect::<(), String>::succeed(())
                    }
                })
                .expect("acquire cannot fail");
            Effect::<u32, String, ()>::fail("flaky".to_string())
        }
    });
    assert!(matches!(exit, Exit::Failure(_)));
    scope.close();
    // Three attempts, three releases; the outer resource survives.
    assert_eq!(releases.load(Ordering::SeqCst), 3);
    assert_eq!(outer_releases.load(Ordering::SeqCst), 0);
}

#[test]
fn breaker_open_is_typed() {
    let policy = RemotePolicy {
        retries: 5,
        backoff: Duration::ZERO,
        breaker: Some(BreakerConfig {
            max_failures: 2,
            reset: Duration::from_secs(3600),
        }),
        ..Default::default()
    };
    let remote = Remote::new(policy);
    let rt = Runtime::new();
    let clock = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let failing = || {
        let calls = Arc::clone(&calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Effect::<u32, String, ()>::fail("down".to_string())
        }
    };
    // First attempt fails and opens the breaker; the second sees it open and
    // returns BreakerOpen without consuming further retries.
    let exit = remote.call(&rt, &clock, failing());
    match exit {
        Exit::Failure(c) => assert_eq!(c.failure(), Some(&RemoteError::BreakerOpen)),
        Exit::Success(_) => panic!("expected breaker open"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // Still open for the next operation: no new attempts.
    let exit = remote.call(&rt, &clock, failing());
    match exit {
        Exit::Failure(c) => assert_eq!(c.failure(), Some(&RemoteError::BreakerOpen)),
        Exit::Success(_) => panic!("expected breaker open"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
