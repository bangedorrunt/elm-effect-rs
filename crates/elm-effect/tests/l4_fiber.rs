//! Wave-1 fiber bead (.4): supervisor isolation, detach shutdown,
//! interrupt back-pressure. Bodies cooperate via [`Ctx`](elm_effect::Ctx).
#![cfg(feature = "tokio")]

use elm_effect::{Cause, Defect, Exit, FiberHandle, FiberId, ForkOpts, Runtime};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn is_set(f: &Arc<AtomicBool>) -> bool {
    f.load(Ordering::SeqCst)
}

#[test]
fn panic_in_child_is_die_and_sibling_lives() {
    let rt = Runtime::new();

    let panicker = rt.fork_detach(|_ctx| -> Exit<i32, String> {
        panic!("child blew up");
    });
    let sibling = rt.fork_detach(|_ctx| -> Exit<i32, String> { Exit::succeed(7) });

    let panic_exit = panicker.join();
    let sibling_exit = sibling.join();

    // The panic is a Die, never a typed error, and the process is still up.
    let cause = match panic_exit {
        Exit::Failure(c) => c,
        Exit::Success(_) => panic!("panicking child must not succeed"),
    };
    assert_eq!(cause.defects().len(), 1);
    assert!(cause.failure().is_none());
    assert!(cause.defects()[0].message.contains("child blew up"));

    assert_eq!(sibling_exit, Exit::Success(7));
}

#[test]
fn fork_detach_child_ends_at_runtime_shutdown() {
    let rt = Runtime::new();
    let saw_interrupt = flag();
    let body_saw = saw_interrupt.clone();

    let _detached: FiberHandle<(), String> = rt.fork_detach_with(
        move |ctx| {
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            body_saw.store(true, Ordering::SeqCst);
            Exit::succeed(())
        },
        ForkOpts::immediate(),
    );
    // Give the detached child a chance to start; it must still be parked.
    std::thread::sleep(Duration::from_millis(50));
    assert!(!is_set(&saw_interrupt));

    rt.shutdown();
    assert!(is_set(&saw_interrupt));
}

#[test]
fn interrupt_waits_for_finalizers() {
    let rt = Runtime::new();
    let finalized = flag();
    let body_finalized = finalized.clone();

    let child = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            ctx.add_finalizer(move || {
                // Slow finalizer: interrupt() must not return before this runs.
                std::thread::sleep(Duration::from_millis(100));
                body_finalized.store(true, Ordering::SeqCst);
            });
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::Failure(Cause::interrupt(None))
        },
        ForkOpts::immediate(),
    );

    // Wait until the child is parked in its loop.
    std::thread::sleep(Duration::from_millis(50));
    child.interrupt();
    // interrupt() waited: the finalizer ran before it returned.
    assert!(is_set(&finalized));

    let exit = child.join();
    assert!(matches!(exit, Exit::Failure(_)));
}

#[test]
fn deferred_start_begins_on_join() {
    let rt = Runtime::new();
    let started = flag();
    let body_started = started.clone();
    let child = rt.fork_detach_with(
        move |_ctx| -> Exit<i32, String> {
            body_started.store(true, Ordering::SeqCst);
            Exit::succeed(1)
        },
        ForkOpts::deferred(),
    );
    // Deferred: not started until the parent yields (join fires the latch).
    assert!(!is_set(&started));
    assert_eq!(child.join(), Exit::Success(1));
    assert!(is_set(&started));
}

#[test]
fn child_dies_with_parent_and_parent_waits_finalizers() {
    let rt = Runtime::new();
    let child_saw_interrupt = flag();
    let body_saw = child_saw_interrupt.clone();
    let child_finalized = flag();
    let body_fin = child_finalized.clone();
    let running = flag();
    let body_running = running.clone();

    let parent = rt.fork_detach(move |ctx| -> Exit<(), String> {
        let _child = ctx.fork_child(
            move |cctx| -> Exit<(), String> {
                let fin = body_fin.clone();
                cctx.add_finalizer(move || fin.store(true, Ordering::SeqCst));
                body_running.store(true, Ordering::SeqCst);
                while !cctx.is_interrupted() {
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                body_saw.store(true, Ordering::SeqCst);
                Exit::Failure(Cause::interrupt(None))
            },
            ForkOpts::immediate(),
        );
        // Wait until the child is parked before returning: the supervisor
        // must stop a running child, not preempt an unstarted one.
        while !is_set(&running) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        Exit::succeed(())
    });

    assert_eq!(parent.join(), Exit::Success(()));
    // Parent join returned only after the child was interrupted and finalized.
    assert!(is_set(&child_saw_interrupt));
    assert!(is_set(&child_finalized));
}

#[test]
fn uninterruptible_region_completes_before_interrupt_returns() {
    let rt = Runtime::new();
    let in_region = flag();
    let body_in_region = in_region.clone();
    let region_done = flag();
    let body_done = region_done.clone();

    let child = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            ctx.masked(|| {
                body_in_region.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
                body_done.store(true, Ordering::SeqCst);
            });
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::Failure(Cause::interrupt(None))
        },
        ForkOpts::immediate(),
    );

    while !is_set(&in_region) {
        std::thread::park_timeout(Duration::from_millis(1));
    }
    child.interrupt();
    // The masked region ran to completion even though interrupt arrived first.
    assert!(is_set(&region_done));
    let _ = child.join();
}

#[test]
fn fiber_ids_are_unique_per_runtime() {
    let rt = Runtime::new();
    let a = rt.fork_detach(|_ctx| -> Exit<(), String> { Exit::succeed(()) });
    let b = rt.fork_detach(|_ctx| -> Exit<(), String> { Exit::succeed(()) });
    let FiberId(x) = a.id();
    let FiberId(y) = b.id();
    assert_ne!(x, y);
    let _ = a.join();
    let _ = b.join();
}

#[test]
fn defect_carries_panic_message() {
    let d = Defect::new("x");
    assert_eq!(d.message, "x");
    assert!(!d.backtrace.is_empty());
}

#[test]
fn scoped_binding_reports_parent_and_scope() {
    use std::sync::Mutex;

    let rt = Runtime::new();
    let scope = rt.create_scope();
    let scope_id = scope.id();
    let seen: Arc<Mutex<(Option<FiberId>, Vec<u64>)>> = Arc::new(Mutex::new((None, Vec::new())));
    let body_seen = seen.clone();
    let spin_seen = seen.clone();

    let parent = rt.fork_detach(move |ctx| -> Exit<(), String> {
        let child = ctx.fork_scoped(
            &scope,
            move |cctx| -> Exit<(), String> {
                *body_seen.lock().unwrap() = (cctx.parent(), cctx.bound_scopes());
                while !cctx.is_interrupted() {
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                Exit::Failure(Cause::interrupt(None))
            },
            ForkOpts::immediate(),
        );
        // Wait until the child recorded, then return: supervisor stops it.
        while spin_seen.lock().unwrap().0.is_none() {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        let _ = child;
        Exit::succeed(())
    });

    let parent_id = parent.id();
    assert_eq!(parent.join(), Exit::Success(()));
    let (par, bound) = seen.lock().unwrap().clone();
    assert_eq!(par, Some(parent_id));
    assert!(bound.contains(&scope_id));
}

#[test]
fn closing_scope_interrupts_members() {
    let rt = Runtime::new();
    let scope = rt.create_scope();
    let saw = flag();
    let body_saw = saw.clone();

    let child = rt.fork_in(
        &scope,
        move |ctx| -> Exit<(), String> {
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            body_saw.store(true, Ordering::SeqCst);
            Exit::Failure(Cause::interrupt(None))
        },
        ForkOpts::immediate(),
    );
    std::thread::sleep(Duration::from_millis(50));
    scope.close();
    let exit = child.join();
    assert!(is_set(&saw));
    assert!(matches!(exit, Exit::Failure(_)));
}

// ---------------------------------------------------------------------------
// awaitAllChildren at the pin (Effect.test.ts describe "awaitAllChildren")
// ---------------------------------------------------------------------------

#[test]
fn await_all_children_completes_immediately_without_children() {
    // Effect.succeed(1).pipe(Effect.awaitAllChildren) === 1: nothing is
    // forked, so there is nothing to await. The runtime must outlive the
    // join here: dropping it shuts down and interrupts every live fiber.
    let rt = Runtime::new();
    let exit = rt.fork_detach(|_ctx| -> Exit<u32, String> { Exit::succeed(1) });
    assert_eq!(exit.join(), Exit::Success(1));
}

#[test]
fn await_all_children_waits_for_the_child_the_body_forked() {
    // The wrapped effect forks a child that stays parked. The parent's join
    // returns only after that child's finalizers run, which is the
    // observable claim of Effect.awaitAllChildren in this port (a parent
    // waits for its children's finalizers).
    //
    // The child must poll `is_interrupted` (the proven idiom in this file):
    // a body that never checks it spins forever once the supervisor
    // interrupts it, which is what hung the first version of this test.
    let running = flag();
    let body_running = running.clone();
    let finalized = flag();
    let body_finalized = finalized.clone();

    let rt = Runtime::new();
    let parent = rt.fork_detach(move |ctx| -> Exit<(), String> {
        ctx.fork_child(
            move |cctx| -> Exit<(), String> {
                cctx.add_finalizer(move || body_finalized.store(true, Ordering::SeqCst));
                body_running.store(true, Ordering::SeqCst);
                while !cctx.is_interrupted() {
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                Exit::Failure(Cause::interrupt(None))
            },
            ForkOpts::immediate(),
        );
        while !is_set(&running) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        Exit::succeed(())
    });

    assert_eq!(parent.join(), Exit::Success(()));
    // The child was interrupted and finalized before join returned.
    assert!(is_set(&finalized));
}
