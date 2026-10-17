//! Laws L6/L7: every exit releases, reverse order, exactly once; releases run
//! to completion even under cancellation.
#![cfg(feature = "tokio")]
//!
//! `acquire_release` runs its acquire eagerly: the caller is already inside
//! the runtime (a fiber body or a test driving one), i.e. the imperative
//! shell edge. Laziness of descriptions (L1) is untouched: nothing here
//! builds a deferred description.

use elm_effect::{Cause, Effect, Exit, ForkOpts, Reason, Runtime, Scope, ScopeTag};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

// ---------------------------------------------------------------------------
// Effect.scoped at the pin (Effect.test.ts:79)
// ---------------------------------------------------------------------------

/// Which exit the scoped body ends on.
#[derive(Clone, Copy)]
enum BodyExit {
    Succeed,
    FailTyped,
    Interrupt,
}

/// Install a finalizer on the ambient scope, then end the body the given way.
/// Records the scope id it saw, so the caller can tell the minted scope from
/// any other.
fn install_finalizer_then(
    exit: BodyExit,
    finalized: Arc<AtomicUsize>,
    seen: Arc<Mutex<Option<u64>>>,
) -> Effect<(), String, ScopeTag> {
    elm_effect::use_service::<ScopeTag>()
        .map(move |scope| {
            *seen.lock().unwrap() = Some(scope.id());
            let counter = Arc::clone(&finalized);
            scope.add_finalizer(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            });
        })
        // Reading a service cannot fail, so the typed channel is widened once
        // here to the body's own error type instead of a second door.
        .map_err(|never| match never {})
        .flat_map(move |()| match exit {
            BodyExit::Succeed => Effect::<(), String, ScopeTag>::succeed(()),
            BodyExit::FailTyped => Effect::<(), String, ScopeTag>::fail("body failed".to_string()),
            BodyExit::Interrupt => Effect::<(), String, ScopeTag>::interrupt(),
        })
}

#[test]
fn scoped_closes_its_scope_when_interrupted_after_installing_it() {
    // Effect.test.ts:79. A finalizer installed inside `scoped` runs exactly
    // once when the run ends, on all three exits. Interruption is cooperative
    // in this carrier, so the interrupted leg carries the interrupt back as
    // the body's cause rather than being signalled mid-body.
    for (exit, label) in [
        (BodyExit::Succeed, "success"),
        (BodyExit::FailTyped, "typed failure"),
        (BodyExit::Interrupt, "interrupt"),
    ] {
        let finalized = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(None));
        let program = Effect::scoped(install_finalizer_then(
            exit,
            Arc::clone(&finalized),
            Arc::clone(&seen),
        ));
        let out = Runtime::run(program);

        assert_eq!(
            finalized.load(Ordering::SeqCst),
            1,
            "finalizer runs exactly once on {label}"
        );
        let scope_id = seen.lock().unwrap().expect("body saw the ambient scope");
        match exit {
            BodyExit::Succeed => assert!(matches!(out, Exit::Success(()))),
            BodyExit::FailTyped => match out {
                Exit::Failure(cause) => {
                    assert_eq!(cause.failure(), Some(&"body failed".to_string()));
                    assert!(cause.defects().is_empty(), "no finalizer defect appended");
                }
                Exit::Success(_) => panic!("a typed failure must propagate"),
            },
            BodyExit::Interrupt => match out {
                Exit::Failure(cause) => {
                    assert!(cause.is_interrupted(), "the interrupt survives (L6)");
                    assert!(cause.failure().is_none(), "interruption is not a failure");
                }
                Exit::Success(_) => panic!("an interrupt must not succeed"),
            },
        }
        // The scope the body saw is not a shared global: every run mints one.
        assert_ne!(scope_id, 0, "minted ids come from the fiber id source");
    }
}

#[test]
fn scoped_mints_a_scope_per_run() {
    // A description is a value: running the same one twice must not let the
    // first run's scope observe the second's, and each run closes its own.
    let finalized = Arc::new(AtomicUsize::new(0));
    let run_once = || {
        let id = Arc::new(Mutex::new(None));
        let program = Effect::scoped(install_finalizer_then(
            BodyExit::Succeed,
            Arc::clone(&finalized),
            Arc::clone(&id),
        ));
        let out = Runtime::run(program);
        assert!(matches!(out, Exit::Success(())));
        id.lock().unwrap().expect("body saw the ambient scope")
    };
    let first = run_once();
    let second = run_once();
    assert_eq!(
        finalized.load(Ordering::SeqCst),
        2,
        "each run installs and runs its own finalizer"
    );
    assert_ne!(first, second, "the two runs got two scopes");
}

// ---------------------------------------------------------------------------
// acquireUseRelease at the pin (Effect.test.ts describe "acquireUseRelease")
// ---------------------------------------------------------------------------

#[test]
fn acquire_use_release_returns_the_usage_result() {
    // acquireUseRelease(Effect.void, () => Effect.succeed(42), () => Effect.void)
    // === 42. The release runs at the fiber's end, after use.
    let rt = Runtime::new();
    let released = Arc::new(AtomicBool::new(false));
    let fin_released = released.clone();
    let exit = rt
        .fork_detach(move |ctx| -> Exit<u32, String> {
            let res = ctx.acquire_release(
                Effect::<u32, String>::succeed(1),
                move |_| -> Effect<(), String> {
                    fin_released.store(true, Ordering::SeqCst);
                    Effect::succeed(())
                },
            );
            match res {
                Ok(_) => Exit::succeed(42),
                Err(cause) => Exit::Failure(cause),
            }
        })
        .join();
    assert!(matches!(exit, Exit::Success(42)));
    assert!(released.load(Ordering::SeqCst), "release runs after use");
}

#[test]
fn acquire_use_release_error_in_just_acquisition() {
    // The acquire fails: no use runs, no release registers, and the fiber
    // carries the typed failure.
    let rt = Runtime::new();
    let exit = rt
        .fork_detach(move |ctx| -> Exit<u32, String> {
            match ctx.acquire_release(
                Effect::<u32, String>::fail("acq boom".to_string()),
                |_| -> Effect<(), String> { Effect::succeed(()) },
            ) {
                Ok(n) => Exit::succeed(n),
                Err(cause) => Exit::Failure(cause),
            }
        })
        .join();
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"acq boom".to_string())),
        Exit::Success(_) => panic!("expected the acquisition failure"),
    }
}

#[test]
fn acquire_use_release_error_in_just_release() {
    // The release dies. A failing release appends as a Die, so a successful
    // use ends as a defect-only failure (L6: the original reason survives
    // as the release's own failure, never erased).
    let rt = Runtime::new();
    let release_calls = Arc::new(AtomicBool::new(false));
    let fin_calls = release_calls.clone();
    let exit = rt
        .fork_detach(move |ctx| -> Exit<u32, String> {
            let res = ctx.acquire_release(
                Effect::<u32, String>::succeed(1),
                move |_| -> Effect<(), String> {
                    fin_calls.store(true, Ordering::SeqCst);
                    Effect::die(elm_effect::Defect::new("rel boom"))
                },
            );
            match res {
                Ok(n) => Exit::succeed(n),
                Err(cause) => Exit::Failure(cause),
            }
        })
        .join();
    assert!(release_calls.load(Ordering::SeqCst), "release must run");
    match exit {
        Exit::Failure(cause) => {
            assert!(cause.failure().is_none(), "a die is not a typed failure");
            assert_eq!(cause.defects().len(), 1);
        }
        Exit::Success(_) => panic!("a failing release must fail the fiber"),
    }
}

#[test]
fn acquire_use_release_error_in_just_usage() {
    // The use fails with a typed error; a successful release appends nothing.
    let rt = Runtime::new();
    let exit = rt
        .fork_detach(move |ctx| -> Exit<u32, String> {
            let res = ctx.acquire_release(
                Effect::<u32, String>::succeed(1),
                |_| -> Effect<(), String> { Effect::succeed(()) },
            );
            match res {
                Ok(_) => Exit::Failure(Cause::fail("usage boom".to_string())),
                Err(cause) => Exit::Failure(cause),
            }
        })
        .join();
    match exit {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), Some(&"usage boom".to_string()));
            assert!(cause.defects().is_empty(), "no release failure to append");
        }
        Exit::Success(_) => panic!("expected the usage failure"),
    }
}
