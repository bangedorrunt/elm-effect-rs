//! Laws L6/L7: every exit releases, reverse order, exactly once; releases run
//! to completion even under cancellation.
#![cfg(feature = "tokio")]
//!
//! `acquire_release` runs its acquire eagerly: the caller is already inside
//! the runtime (a fiber body or a test driving one), i.e. the imperative
//! shell edge. Laziness of descriptions (L1) is untouched: nothing here
//! builds a deferred description.

use elm_effect::{Cause, Effect, Exit, ForkOpts, Reason, Runtime, Scope};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

type Log = Arc<Mutex<Vec<&'static str>>>;

fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn acquire_named(name: &'static str) -> Effect<&'static str, String> {
    Effect::succeed(name)
}

fn release_rec(log: Log, name: &'static str) -> impl FnOnce(&'static str) -> Effect<(), String> {
    move |_| {
        log.lock().unwrap().push(name);
        Effect::succeed(())
    }
}

#[test]
fn release_on_three_exits() {
    // Success exit: reverse order of acquisition, exactly once.
    {
        let scope = Scope::new(1);
        let log = log();
        let a = scope.acquire_release(acquire_named("first"), release_rec(log.clone(), "first"));
        let b = scope.acquire_release(acquire_named("second"), release_rec(log.clone(), "second"));
        assert!(a.is_ok() && b.is_ok());
        let spills = scope.close();
        assert!(spills.is_empty());
        assert_eq!(*log.lock().unwrap(), vec!["second", "first"]);
        // Closing again releases nothing more.
        let spills = scope.close();
        assert!(spills.is_empty());
        assert_eq!(*log.lock().unwrap(), vec!["second", "first"]);
    }

    // Typed failure exit: the failed acquire registers nothing; the
    // earlier resource still releases.
    {
        let scope = Scope::new(2);
        let log = log();
        let a = scope.acquire_release(acquire_named("first"), release_rec(log.clone(), "first"));
        assert!(a.is_ok());
        let b: Result<&str, Cause<String>> = scope.acquire_release(
            Effect::fail("nope".to_string()),
            release_rec(log.clone(), "never"),
        );
        assert!(b.is_err());
        scope.close();
        assert_eq!(*log.lock().unwrap(), vec!["first"]);
    }

    // Interruption exit: everything acquired still releases, once each.
    {
        let rt = Runtime::new();
        let log = log();
        let body_log = log.clone();
        let parked = Arc::new(AtomicBool::new(false));
        let body_parked = parked.clone();
        let child = rt.fork_detach_with(
            move |ctx| -> Exit<(), String> {
                let l1 = body_log.clone();
                let l2 = body_log.clone();
                ctx.acquire_release(acquire_named("first"), move |_| -> Effect<(), String> {
                    l1.lock().unwrap().push("first");
                    Effect::succeed(())
                })
                .expect("acquire succeeds");
                ctx.acquire_release(acquire_named("second"), move |_| -> Effect<(), String> {
                    l2.lock().unwrap().push("second");
                    Effect::succeed(())
                })
                .expect("acquire succeeds");
                body_parked.store(true, Ordering::SeqCst);
                while !ctx.is_interrupted() {
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                Exit::Failure(Cause::interrupt(None))
            },
            ForkOpts::immediate(),
        );
        while !parked.load(Ordering::SeqCst) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        child.interrupt();
        assert!(matches!(child.join(), Exit::Failure(_)));
        assert_eq!(*log.lock().unwrap(), vec!["second", "first"]);
    }
}

#[test]
fn cancel_during_release_finishes_release() {
    let rt = Runtime::new();
    let released = Arc::new(AtomicBool::new(false));
    let body_released = released.clone();
    // Gate inside the release: it cannot finish until the test opens it.
    let gate: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
    let body_gate = gate.clone();
    let interrupt_done = Arc::new(AtomicBool::new(false));
    let probe_done = interrupt_done.clone();
    let parked = Arc::new(AtomicBool::new(false));
    let body_parked = parked.clone();

    let child = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            ctx.add_finalizer(move || {
                // Slow release: wait for the test gate, then record.
                let mut open = body_gate.0.lock().unwrap();
                while !*open {
                    open = body_gate.1.wait(open).unwrap();
                }
                body_released.store(true, Ordering::SeqCst);
            });
            body_parked.store(true, Ordering::SeqCst);
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::Failure(Cause::interrupt(None))
        },
        ForkOpts::immediate(),
    );
    while !parked.load(Ordering::SeqCst) {
        std::thread::park_timeout(Duration::from_millis(1));
    }

    // Interrupt in the background: it must block inside the unfinished release.
    std::thread::spawn(move || {
        child.interrupt();
        probe_done.store(true, Ordering::SeqCst);
    });
    // Give the interrupter time to arrive mid-release; it must still be waiting.
    std::thread::sleep(Duration::from_millis(100));
    assert!(!interrupt_done.load(Ordering::SeqCst));
    assert!(!released.load(Ordering::SeqCst));

    // Open the gate: the release runs to completion, then interrupt returns.
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !interrupt_done.load(Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "interrupt never returned"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
    assert!(released.load(Ordering::SeqCst));
}

#[test]
fn finalizer_failure_is_appended() {
    // End-to-end through a fiber: Fail exit + failing release.
    {
        let rt = Runtime::new();
        let child = rt.fork_detach(|ctx| -> Exit<&'static str, String> {
            ctx.acquire_release(
                Effect::<&str, String>::succeed("res"),
                |_| -> Effect<(), String> { Effect::fail("release blew up".to_string()) },
            )
            .expect("acquire succeeds");
            Exit::Failure(Cause::fail("original".to_string()))
        });
        let exit = child.join();
        let cause = match exit {
            Exit::Failure(c) => c,
            Exit::Success(_) => panic!("must fail"),
        };
        assert!(matches!(
            cause.reasons.as_slice(),
            [Reason::Fail(_), Reason::Die(_)]
        ));
        assert_eq!(cause.failure(), Some(&"original".to_string()));
        assert!(cause.defects()[0].message.contains("release blew up"));
    }
    // Success + failing release: success becomes a Die-only failure.
    {
        let rt = Runtime::new();
        let child = rt.fork_detach(|ctx| -> Exit<&'static str, String> {
            ctx.acquire_release(
                Effect::<&str, String>::succeed("res"),
                |_| -> Effect<(), String> { Effect::fail("release blew up".to_string()) },
            )
            .expect("acquire succeeds");
            Exit::Success("done")
        });
        let exit = child.join();
        let cause = match exit {
            Exit::Failure(c) => c,
            Exit::Success(_) => panic!("bad release must fail the fiber"),
        };
        assert!(cause.failure().is_none());
        assert_eq!(cause.defects().len(), 1);
    }
}

#[test]
fn acquire_into_closed_scope_releases_immediately() {
    // Unhappy path: the scope closed between decision and acquire. The
    // resource must not leak: release runs inline and the caller sees an
    // interruption, not a silent success.
    let scope = Scope::new(4);
    let log = log();
    scope.close();
    let r = scope.acquire_release(acquire_named("res"), release_rec(log.clone(), "res"));
    let cause = r.expect_err("acquire into a closed scope must not succeed");
    assert!(matches!(
        cause.reasons.as_slice(),
        [Reason::Interrupt { .. }]
    ));
    assert_eq!(*log.lock().unwrap(), vec!["res"]);
}
