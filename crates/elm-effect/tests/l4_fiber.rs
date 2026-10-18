//! Wave-1 fiber bead (.4): supervisor isolation, detach shutdown,
//! interrupt back-pressure. Bodies cooperate via [`Ctx`](elm_effect::Ctx).
#![cfg(feature = "tokio")]

use elm_effect::{Cause, Defect, Effect, Exit, FiberHandle, FiberId, ForkOpts, Runtime};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

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

/// A body that cannot return must not be the reason its children keep
/// running: interruption reaches the whole subtree at the moment it is
/// requested. Here the parent is blocked inside `join`, so nothing it does
/// after the body would ever run.
///
/// Without recursive delivery this test does not fail, it hangs: the child
/// parks until an interrupt that only the parent's (never reached) body end
/// would deliver. The watchdog turns the hang into a loud abort.
#[test]
fn interrupting_a_parent_reaches_the_child_it_is_blocked_on() {
    let done = flag();
    let watchdog_done = done.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        while !is_set(&watchdog_done) {
            if start.elapsed() > Duration::from_secs(10) {
                eprintln!(
                    "the blocked parent never finished: interruption is not reaching its children"
                );
                std::process::abort();
            }
            std::thread::park_timeout(Duration::from_millis(10));
        }
    });

    let rt = Runtime::new();
    let child_running = flag();
    let running = child_running.clone();
    let child_stopped = flag();
    let stopped = child_stopped.clone();

    let parent = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            let child = ctx.fork_child(
                move |cctx| -> Exit<(), String> {
                    running.store(true, Ordering::SeqCst);
                    while !cctx.is_interrupted() {
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                    stopped.store(true, Ordering::SeqCst);
                    Exit::Failure(Cause::interrupt(None))
                },
                ForkOpts::immediate(),
            );
            // The parent's body cannot return until the child ends, so the
            // request has to reach the child while the parent is still here.
            let _ = child.join();
            Exit::succeed(())
        },
        ForkOpts::immediate(),
    );

    while !is_set(&child_running) {
        std::thread::park_timeout(Duration::from_millis(1));
    }
    parent.interrupt();
    done.store(true, Ordering::SeqCst);

    assert!(is_set(&child_stopped), "the child never saw the interrupt");
    // The parent ends interrupted, not successful: a body that returns after
    // its interrupt was requested still ends interrupted.
    match parent.join() {
        Exit::Success(()) => panic!("an interrupted parent reported success"),
        Exit::Failure(cause) => assert!(cause.is_interrupted(), "{cause:?}"),
    }
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

// ---------------------------------------------------------------------------
// Effect-level forkChild + awaitAllChildren (Effect.test.ts :2457 - :2576).
// The pin forks children *inside* an effect; this port could not until
// `Effect::fork_child` carried the spawn entry on the ambient fiber, so these
// rows were unported for want of a fork, not for want of an await.
// ---------------------------------------------------------------------------

/// The pin's `Deferred.await(latch)`: park until the fiber is interrupted,
/// then end interrupted. A child that lives exactly as long as the interrupt
/// takes to arrive.
fn until_interrupted(started: Arc<AtomicBool>) -> Effect<(), String> {
    Effect::with_fiber(move |fiber| {
        let fiber = fiber.clone();
        Effect::<(), String>::sync(move || {
            started.store(true, Ordering::SeqCst);
            while !fiber.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
        })
        .flat_map(|()| Effect::<(), String>::interrupt())
    })
}

/// Park until `released`, ignoring interruption. A child that outlives the
/// parent's interrupt request is what tells "the await returned early" apart
/// from "the parent waited the child out".
fn until_released(released: Arc<AtomicBool>, started: Arc<AtomicBool>) -> Effect<(), String> {
    Effect::<(), String>::sync(move || {
        started.store(true, Ordering::SeqCst);
        while !released.load(Ordering::SeqCst) {
            std::thread::park_timeout(Duration::from_millis(1));
        }
    })
}

/// Poll a flag up to `within`. A witness that would hang instead fails on the
/// assertion, which is the loud failure these rows need.
fn wait_for(f: &Arc<AtomicBool>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if f.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    f.load(Ordering::SeqCst)
}

/// Run `effect` on a fiber of its own and hand the handle to the test thread.
/// The forking parent then parks, so nothing but the witness can end the
/// effect's fiber.
fn spawn_effect(effect: Effect<u8, String>) -> (Runtime, FiberHandle<u8, String>) {
    let rt = Runtime::new();
    let (tx, rx) = mpsc::channel();
    let _parent = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            let handle = ctx.spawn_effect(effect);
            // Yield: fires the deferred start latch of the effect's fiber.
            let dummy = ctx.fork_child(
                |_| -> Exit<(), String> { Exit::succeed(()) },
                ForkOpts::immediate(),
            );
            dummy.join();
            let _ = tx.send(handle);
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::succeed(())
        },
        ForkOpts::immediate(),
    );
    let handle = rx.recv().expect("the parent hands its effect handle over");
    (rt, handle)
}

/// :2457 `can be interrupted while awaiting children after success`.
/// The body's value is already in hand when the await takes over; an
/// interrupt ends the await, and both fibers exit interrupts only.
#[test]
fn await_all_children_is_interruptible_after_success() {
    let started = flag();
    let child_started = started.clone();
    let child_slot: Arc<Mutex<Option<FiberHandle<(), String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&child_slot);

    let effect = Effect::suspend(move || {
        Effect::fork_child(until_interrupted(child_started.clone())).flat_map(move |child| {
            *slot.lock().unwrap() = Some(child);
            Effect::succeed(1u8)
        })
    })
    .await_all_children();

    let (rt, target) = spawn_effect(effect);
    assert!(
        wait_for(&started, Duration::from_secs(5)),
        "the forked child never started"
    );
    // The await is what is running now.
    std::thread::sleep(Duration::from_millis(20));
    target.interrupt_unsafe();

    let cause = match target.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("an interrupted await must not report success"),
    };
    assert!(cause.is_interrupted(), "the parent exit lost its interrupt");
    assert!(cause.failure().is_none(), "no typed failure was raised");
    assert!(cause.defects().is_empty(), "no defect was raised");

    let child = child_slot
        .lock()
        .unwrap()
        .take()
        .expect("the effect forked a child");
    let child_cause = match child.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("an interrupted child must not report success"),
    };
    assert!(child_cause.is_interrupted(), "the child lost its interrupt");
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2489 `can be interrupted while awaiting children after failure`.
/// The body's failure survives the interrupt that lands during the await: the
/// exit carries both, which is why the interrupt is appended, never
/// substituted for the cause.
#[test]
fn await_all_children_keeps_a_body_failure_when_interrupted() {
    let started = flag();
    let child_started = started.clone();
    let child_slot: Arc<Mutex<Option<FiberHandle<(), String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&child_slot);

    let effect = Effect::suspend(move || {
        Effect::fork_child(until_interrupted(child_started.clone()))
    })
    .flat_map(move |child| {
        *slot.lock().unwrap() = Some(child);
        Effect::<u8, String>::fail("boom".to_owned())
    })
    .await_all_children();

    let (rt, target) = spawn_effect(effect);
    assert!(
        wait_for(&started, Duration::from_secs(5)),
        "the forked child never started"
    );
    std::thread::sleep(Duration::from_millis(20));
    target.interrupt_unsafe();

    let cause = match target.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("a failed body must not report success"),
    };
    assert!(cause.is_interrupted(), "the interrupt was not appended");
    assert_eq!(
        cause.failure(),
        Some(&"boom".to_owned()),
        "the body's own failure must survive the interrupt"
    );

    let child = child_slot
        .lock()
        .unwrap()
        .take()
        .expect("the effect forked a child");
    match child.join() {
        Exit::Failure(cause) => assert!(cause.is_interrupted()),
        Exit::Success(_) => panic!("the child was interrupted, so it cannot succeed"),
    }
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2523 `respects an enclosing uninterruptible region while awaiting
/// children`. One shape, two runs: the child parks until the test releases it.
/// Inside the region the await waits it out, so the step after the await runs;
/// outside one the await gives up at once and that step never runs. Both
/// fibers end interrupted, so the difference is only where the wait stopped.
#[test]
fn await_all_children_respects_an_enclosing_uninterruptible_region() {
    for masked in [true, false] {
        let started = flag();
        let released = flag();
        let after_await = flag();
        let child_started = started.clone();
        let child_released = released.clone();
        let after = after_await.clone();

        let wrapped = Effect::suspend(move || {
            Effect::fork_child(until_released(
                child_released.clone(),
                child_started.clone(),
            ))
        })
        .flat_map(|_child| Effect::<u8, String>::succeed(1));

        let effect = wrapped.await_all_children().flat_map(move |value| {
            let after = after.clone();
            Effect::<u8, String>::sync(move || {
                after.store(true, Ordering::SeqCst);
                value
            })
        });
        let effect = if masked { effect.uninterruptible() } else { effect };

        let (rt, target) = spawn_effect(effect);
        assert!(
            wait_for(&started, Duration::from_secs(5)),
            "masked={masked}: the forked child never started"
        );
        target.interrupt_unsafe();
        // The child ignores interruption, and both runs owe it a wait: the
        // region waits inside the await, the plain run waits at completion.
        assert!(
            !target.is_done(),
            "masked={masked}: the fiber ended before the awaited child did"
        );

        released.store(true, Ordering::SeqCst);
        let cause = match target.join() {
            Exit::Failure(cause) => cause,
            Exit::Success(_) => panic!("masked={masked}: a parked region must end interrupted"),
        };
        assert!(cause.is_interrupted(), "masked={masked}: no interrupt in the exit");
        assert_eq!(
            is_set(&after_await),
            masked,
            "masked={masked}: the step after the await ran when it should not have, or vice versa"
        );
        assert!(rt.shutdown().is_empty(), "masked={masked}: a fiber outlived the test");
    }
}

/// :2556 `does not await children forked outside the wrapped effect`.
/// The wrapped effect runs on a fiber of its own with no children, so its
/// await returns while the parent's parked child is still alive. A port that
/// awaited the room rather than its own children would block here.
#[test]
fn await_all_children_ignores_a_child_forked_outside_it() {
    let returned = flag();
    let released = flag();

    let rt = Runtime::new();
    let (tx, rx) = mpsc::channel();
    let child_released = released.clone();
    let returned_flag = returned.clone();

    // The parent parks until shutdown interrupts it; joining it here would
    // wait on a fiber only `rt.shutdown()` can end.
    let _parent = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            let preexisting = ctx.fork_child(
                move |_| -> Exit<(), String> {
                    while !child_released.load(Ordering::SeqCst) {
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                    Exit::succeed(())
                },
                ForkOpts::immediate(),
            );
            let effect = Effect::succeed(1u8).await_all_children().flat_map(move |value| {
                let returned = returned_flag.clone();
                Effect::<u8, String>::sync(move || {
                    returned.store(true, Ordering::SeqCst);
                    value
                })
            });
            let handle = ctx.spawn_effect(effect);
            let dummy = ctx.fork_child(
                |_| -> Exit<(), String> { Exit::succeed(()) },
                ForkOpts::immediate(),
            );
            dummy.join();
            let _ = tx.send((handle, preexisting));
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::succeed(())
        },
        ForkOpts::immediate(),
    );

    let (handle, preexisting) = rx.recv().expect("the parent hands its fibers over");
    assert!(
        wait_for(&returned, Duration::from_secs(5)),
        "the await waited for a child it did not fork"
    );
    assert!(
        !preexisting.is_done(),
        "the outside child ended before the await returned"
    );
    assert_eq!(handle.join(), Exit::Success(1));

    released.store(true, Ordering::SeqCst);
    preexisting.join();
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2568 `does not await preexisting children`.
/// Same fiber, flat shape: a child forked before the awaited effect is not
/// the awaited effect's child.
#[test]
fn await_all_children_ignores_a_preexisting_child() {
    let child_started = flag();
    let released = flag();
    let returned = flag();
    let child_slot: Arc<Mutex<Option<FiberHandle<(), String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&child_slot);
    let child_released = released.clone();
    let started = child_started.clone();
    let returned_flag = returned.clone();

    let effect = Effect::suspend(move || {
        Effect::fork_child(until_released(child_released.clone(), started.clone()))
    })
    .flat_map(move |child| {
        *slot.lock().unwrap() = Some(child);
        Effect::<u8, String>::succeed(1)
    })
    .flat_map(move |_| {
        let returned = returned_flag.clone();
        Effect::succeed(1u8)
            .await_all_children()
            .flat_map(move |value| {
                let returned = returned.clone();
                Effect::<u8, String>::sync(move || {
                    returned.store(true, Ordering::SeqCst);
                    value
                })
            })
    });

    let (rt, target) = spawn_effect(effect);
    // The child parks until `released`, so the flag can only be set if the
    // await returned without waiting for it. A port that awaited every child
    // in the registry blocks here and fails loudly instead of passing.
    assert!(
        wait_for(&returned, Duration::from_secs(5)),
        "the await waited for a child forked before it"
    );
    let child = child_slot.lock().unwrap().take().expect("a child was forked");

    released.store(true, Ordering::SeqCst);
    assert_eq!(target.join(), Exit::Success(1));
    child.join();
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2576 `does not await preexisting children in the same fiber`.
/// Nested shape: the inner await waits for the child it forked and not for the
/// one the enclosing effect forked before it.
#[test]
fn await_all_children_ignores_a_preexisting_child_in_the_same_fiber() {
    let outer_started = flag();
    let inner_started = flag();
    let outer_released = flag();
    let inner_released = flag();
    let nested_returned = flag();

    let outer_slot: Arc<Mutex<Option<FiberHandle<(), String>>>> = Arc::new(Mutex::new(None));
    let inner_slot: Arc<Mutex<Option<FiberHandle<(), String>>>> = Arc::new(Mutex::new(None));

    let a_started = outer_started.clone();
    let a_released = outer_released.clone();
    let b_started = inner_started.clone();
    let b_released = inner_released.clone();
    let a_slot = Arc::clone(&outer_slot);
    let b_slot = Arc::clone(&inner_slot);
    let returned = nested_returned.clone();

    let effect = Effect::suspend(move || {
        Effect::fork_child(until_released(a_released.clone(), a_started.clone()))
    })
    .flat_map(move |outer| {
        *a_slot.lock().unwrap() = Some(outer);
        Effect::suspend(move || {
            Effect::fork_child(until_released(b_released.clone(), b_started.clone()))
        })
        .flat_map(move |inner| {
            *b_slot.lock().unwrap() = Some(inner);
            Effect::<u8, String>::succeed(2)
        })
        .await_all_children()
    })
    .flat_map(move |value| {
        let returned = returned.clone();
        Effect::<u8, String>::sync(move || {
            returned.store(true, Ordering::SeqCst);
            value
        })
    });

    let (rt, target) = spawn_effect(effect);
    assert!(
        wait_for(&outer_started, Duration::from_secs(5)),
        "the enclosing child never started"
    );
    assert!(
        wait_for(&inner_started, Duration::from_secs(5)),
        "the nested child never started"
    );

    // Release only the nested effect's child: the await is owed that one, and
    // nothing else.
    inner_released.store(true, Ordering::SeqCst);
    assert!(
        wait_for(&nested_returned, Duration::from_secs(5)),
        "the nested await waited for a child forked before it"
    );

    let outer = outer_slot.lock().unwrap().take().expect("a child was forked");
    assert!(
        !outer.is_done(),
        "the enclosing child ended before the nested await returned"
    );

    outer_released.store(true, Ordering::SeqCst);
    assert_eq!(target.join(), Exit::Success(2));
    outer.join();
    let inner = inner_slot.lock().unwrap().take().expect("a child was forked");
    inner.join();
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

// ---------------------------------------------------------------------------
// Effect.withFiber at the pin (Effect.test.ts describe-group "withFiber").
// The port's handle carries identity and the interrupt flag, not the pin's
// mutable `fiber.context`.
// ---------------------------------------------------------------------------

#[test]
fn with_fiber_reports_the_fiber_executing_the_effect() {
    // The effect runs as a spawned fiber's body, so the id `with_fiber` hands
    // it must be that fiber's own id: not the caller's, not a stand-in.
    let rt = Runtime::new();
    let seen = Arc::new(AtomicU64::new(0));
    let body_seen = seen.clone();
    let effect: Effect<u64, String> = Effect::with_fiber(move |f| {
        let id = f.id();
        let seen = body_seen.clone();
        Effect::<u64, String>::sync(move || {
            seen.store(id.0, Ordering::SeqCst);
            7
        })
    });
    let handle = rt.spawn_effect(effect);
    let fiber_id = handle.id().0;
    assert_eq!(handle.join(), Exit::Success(7));
    assert_eq!(seen.load(Ordering::SeqCst), fiber_id);
}

#[test]
fn with_fiber_sees_the_same_fiber_when_nested() {
    // Nested with_fiber must report one fiber, and it must be the fiber the
    // effect actually runs on (`ctx.spawn_effect`'s child).
    let rt = Runtime::new();
    let parent = rt.fork_detach_with(
        move |ctx| -> Exit<bool, String> {
            let inner: Effect<bool, String> = Effect::with_fiber(|outer| {
                let outer_id = outer.id();
                Effect::with_fiber(move |inner| {
                    let same = inner.id() == outer_id;
                    Effect::<bool, String>::succeed(same)
                })
            });
            match ctx.spawn_effect(inner).join() {
                Exit::Success(same) => Exit::succeed(same),
                Exit::Failure(cause) => Exit::Failure(cause),
            }
        },
        ForkOpts::immediate(),
    );
    assert_eq!(parent.join(), Exit::Success(true));
}

#[test]
fn with_fiber_observes_cooperative_interruption() {
    // The handle sees the same flag the supervisor sets. `spawn_effect`
    // children start deferred, so the parent drives their latch the way a
    // join would (a dummy join drains pending starts) and only interrupts
    // once the body has signalled it is inside the loop: no pre-start race.
    let rt = Runtime::new();
    let started = flag();
    let saw = flag();
    let test_saw = saw.clone();
    let parent = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            let body_started = started.clone();
            let body_saw = saw.clone();
            let target: Effect<(), String> = Effect::with_fiber(move |f| {
                let f = f.clone();
                let started = body_started.clone();
                let saw = body_saw.clone();
                Effect::<(), String>::sync(move || {
                    started.store(true, Ordering::SeqCst);
                    while !f.is_interrupted() {
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                    saw.store(true, Ordering::SeqCst);
                })
            });
            let handle = ctx.spawn_effect(target);
            let dummy = ctx.fork_child(
                |_| -> Exit<(), String> { Exit::succeed(()) },
                ForkOpts::immediate(),
            );
            dummy.join();
            while !is_set(&started) {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            handle.interrupt();
            match handle.join() {
                Exit::Failure(cause) if cause.is_interrupted() => Exit::succeed(()),
                Exit::Success(()) => Exit::Failure(Cause::die(Defect::new(
                    "the interrupted fiber reported success",
                ))),
                Exit::Failure(cause) => Exit::Failure(cause),
            }
        },
        ForkOpts::immediate(),
    );
    assert_eq!(parent.join(), Exit::Success(()));
    assert!(
        is_set(&test_saw),
        "the loop must end by observing the handle's flag"
    );
}

#[test]
fn with_fiber_self_interrupt_turns_success_into_an_interrupt() {
    // `fiber.interrupt` from inside: the body's own step still runs to
    // completion, then the supervisor's boundary reports the interrupt.
    let rt = Runtime::new();
    let produced = flag();
    let body_produced = produced.clone();
    let effect: Effect<u64, String> = Effect::with_fiber(move |f| {
        let f = f.clone();
        let produced = body_produced.clone();
        Effect::<u64, String>::sync(move || {
            f.interrupt();
            produced.store(true, Ordering::SeqCst);
            7
        })
    });
    let exit = rt.spawn_effect(effect).join();
    assert!(is_set(&produced));
    assert!(matches!(exit, Exit::Failure(_)));
}

#[test]
fn with_fiber_outside_a_fiber_is_a_defect_and_never_calls_the_body() {
    let called = flag();
    let body_called = called.clone();
    let effect: Effect<u64, String> = Effect::with_fiber(move |_f| {
        body_called.store(true, Ordering::SeqCst);
        Effect::<u64, String>::succeed(1)
    });
    match Runtime::run(effect) {
        Exit::Failure(cause) => {
            assert!(
                cause.failure().is_none(),
                "missing context is a defect, never a typed failure"
            );
            assert_eq!(cause.defects().len(), 1);
            assert!(cause.defects()[0].message.contains("outside a fiber"));
        }
        Exit::Success(value) => panic!("expected a defect, got a success of {value}"),
    }
    assert!(
        !is_set(&called),
        "a run with no fiber must refuse before calling the body"
    );
}
