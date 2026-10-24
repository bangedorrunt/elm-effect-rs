//! Supervisor isolation, detach shutdown, and interrupt back-pressure.
//! Bodies cooperate via [`Ctx`](elm_effect::Ctx).
#![cfg(feature = "tokio")]

use elm_effect::Duration as EffectDuration;
use elm_effect::{
    Body, Cause, Clock as ClockTag, ClockImpl, Concurrency, Ctx, Defect, Deferred, Effect, Exit,
    FiberHandle, FiberId, ForkOpts, Runtime, SleepOutcome, SystemClock, TestClock, all_in,
    for_each_in,
};
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
// awaitAllChildren in Effect 4 (Effect.test.ts describe "awaitAllChildren")
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
    // observable claim of Effect.awaitAllChildren in this crate (a parent
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
// Effect 4 forks children *inside* an effect; this crate could not until
// `Effect::fork_child` carried the spawn entry on the ambient fiber, so these
// cases were missing for want of a fork, not for want of an await.
// ---------------------------------------------------------------------------

/// Effect 4's `Deferred.await(latch)`: park until the fiber is interrupted,
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
/// assertion, which is the loud failure these cases need.
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

    let effect =
        Effect::suspend(move || Effect::fork_child(until_interrupted(child_started.clone())))
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
        let effect = if masked {
            effect.uninterruptible()
        } else {
            effect
        };

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
        assert!(
            cause.is_interrupted(),
            "masked={masked}: no interrupt in the exit"
        );
        assert_eq!(
            is_set(&after_await),
            masked,
            "masked={masked}: the step after the await ran when it should not have, or vice versa"
        );
        assert!(
            rt.shutdown().is_empty(),
            "masked={masked}: a fiber outlived the test"
        );
    }
}

/// :2556 `does not await children forked outside the wrapped effect`.
/// The wrapped effect runs on a fiber of its own with no children, so its
/// await returns while the parent's parked child is still alive. A crate that
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
            let effect = Effect::succeed(1u8)
                .await_all_children()
                .flat_map(move |value| {
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
    // await returned without waiting for it. A crate that awaited every child
    // in the registry blocks here and fails loudly instead of passing.
    assert!(
        wait_for(&returned, Duration::from_secs(5)),
        "the await waited for a child forked before it"
    );
    let child = child_slot
        .lock()
        .unwrap()
        .take()
        .expect("a child was forked");

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

    let outer = outer_slot
        .lock()
        .unwrap()
        .take()
        .expect("a child was forked");
    assert!(
        !outer.is_done(),
        "the enclosing child ended before the nested await returned"
    );

    outer_released.store(true, Ordering::SeqCst);
    assert_eq!(target.join(), Exit::Success(2));
    outer.join();
    let inner = inner_slot
        .lock()
        .unwrap()
        .take()
        .expect("a child was forked");
    inner.join();
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

// ---------------------------------------------------------------------------
// Effect.withFiber in Effect 4 (Effect.test.ts describe-group "withFiber").
// The crate's handle carries identity and the interrupt flag, not Effect 4's
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

/// Same handover as [`spawn_effect`], for the shapes these cases need: the
/// forking parent parks, so only the witness can end the effect's fiber.
///
/// The returned handle is only handed over once the effect's first step has
/// run. These cases interrupt the instant they have the handle, and without that
/// handshake the interrupt can land before the fiber body starts: the fiber is
/// then pre-empted rather than interrupted mid-chain, and a case that counts what
/// the chain did reads zero. Effect 4 never races here, because its interrupt is
/// issued from a fiber that only resumes after the child has started.
fn spawn_effect_any<A, E>(effect: Effect<A, E>) -> (Runtime, FiberHandle<A, E>)
where
    A: Send + 'static,
    E: Send + 'static,
{
    let rt = Runtime::new();
    let (tx, rx) = mpsc::channel();
    let began = flag();
    let announce = began.clone();
    let effect = Effect::<(), E>::sync(move || announce.store(true, Ordering::SeqCst))
        .flat_map(move |_| effect);
    let _parent = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            let handle = ctx.spawn_effect(effect);
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
    assert!(
        wait_for(&began, Duration::from_secs(5)),
        "the effect's first step never ran"
    );
    (rt, handle)
}

/// Wait until exactly `expected` workers are parked on the virtual clock. The
/// crate's sleepers register when they reach the sleep, so this is also how a
/// test knows a launched item actually started.
fn wait_for_sleepers(clock: &TestClock, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while clock.pending_sleepers() != expected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        clock.pending_sleepers(),
        expected,
        "the batch never parked where the case expects"
    );
}

/// Wait until `done` records `expected` items. Better than a sleeper count
/// for "this item finished": a removed sleeper and a newly parked one look the
/// same for a moment, which is exactly when a second adjust would be too early.
fn wait_for_done(done: &Arc<Mutex<Vec<u8>>>, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while done.lock().unwrap().len() < expected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        done.lock().unwrap().len() >= expected,
        "the batch never reached {expected} finished items"
    );
}

/// :709 `sequential interrupt`.
/// One item at a time: the two whose delay elapsed are done, the third is in
/// flight when the interrupt lands and never records itself.
#[test]
fn for_each_sequential_stops_at_the_interrupt() {
    let clock = Arc::new(TestClock::new());
    let done = Arc::new(Mutex::new(Vec::<u8>::new()));
    let rt = Runtime::new();

    let body_clock = clock.clone();
    let body_done = done.clone();
    let fiber = rt.fork_detach_with(
        move |ctx| -> Exit<Vec<u8>, String> {
            for_each_in(
                ctx,
                vec![1u8, 2, 3, 4, 5, 6],
                Concurrency::Sequential,
                move |i, c| {
                    let clock = Arc::clone(&body_clock);
                    let done = Arc::clone(&body_done);
                    match clock.sleep(EffectDuration::from_millis(300), &|| c.is_interrupted()) {
                        SleepOutcome::Elapsed => {
                            done.lock().unwrap().push(i);
                            Exit::Success(i)
                        }
                        SleepOutcome::Interrupted => Exit::Failure(Cause::interrupt(None)),
                    }
                },
            )
        },
        ForkOpts::immediate(),
    );

    wait_for_sleepers(&clock, 1);
    clock.adjust(EffectDuration::from_millis(300));
    wait_for_done(&done, 1);
    wait_for_sleepers(&clock, 1);
    clock.adjust(EffectDuration::from_millis(300));
    wait_for_done(&done, 2);
    wait_for_sleepers(&clock, 1);

    fiber.interrupt();
    let exit = fiber.join();

    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the batch ends interrupted"
    );
    let mut recorded = done.lock().unwrap().clone();
    recorded.sort_unstable();
    assert_eq!(
        recorded,
        vec![1u8, 2],
        "the third item was in flight and must not record"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :724 `unbounded interrupt`.
/// Every item starts at once, and none of them finishes before the interrupt,
/// so nothing is recorded.
#[test]
fn for_each_unbounded_interrupt_records_nothing() {
    let clock = Arc::new(TestClock::new());
    let done = Arc::new(Mutex::new(Vec::<u8>::new()));
    let rt = Runtime::new();

    let body_clock = clock.clone();
    let body_done = done.clone();
    let fiber = rt.fork_detach_with(
        move |ctx| -> Exit<Vec<u8>, String> {
            for_each_in(ctx, vec![1u8, 2, 3], Concurrency::Unbounded, move |i, c| {
                let clock = Arc::clone(&body_clock);
                let done = Arc::clone(&body_done);
                match clock.sleep(EffectDuration::from_millis(150), &|| c.is_interrupted()) {
                    SleepOutcome::Elapsed => {
                        done.lock().unwrap().push(i);
                        Exit::Success(i)
                    }
                    SleepOutcome::Interrupted => Exit::Failure(Cause::interrupt(None)),
                }
            })
        },
        ForkOpts::immediate(),
    );

    wait_for_sleepers(&clock, 3);
    clock.adjust(EffectDuration::from_millis(50));
    fiber.interrupt();
    let exit = fiber.join();

    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the batch ends interrupted"
    );
    assert!(
        done.lock().unwrap().is_empty(),
        "no worker may finish after the interrupt"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :739 `bounded interrupt`.
/// Two at a time: the first pair completes, the second pair is parked when
/// the interrupt lands, and nothing beyond the first pair is recorded.
#[test]
fn for_each_bounded_interrupt_stops_the_window() {
    let clock = Arc::new(TestClock::new());
    let done = Arc::new(Mutex::new(Vec::<u8>::new()));
    let rt = Runtime::new();

    let body_clock = clock.clone();
    let body_done = done.clone();
    let fiber = rt.fork_detach_with(
        move |ctx| -> Exit<Vec<u8>, String> {
            for_each_in(
                ctx,
                vec![1u8, 2, 3, 4, 5, 6],
                Concurrency::Bounded(2),
                move |i, c| {
                    let clock = Arc::clone(&body_clock);
                    let done = Arc::clone(&body_done);
                    match clock.sleep(EffectDuration::from_millis(200), &|| c.is_interrupted()) {
                        SleepOutcome::Elapsed => {
                            done.lock().unwrap().push(i);
                            Exit::Success(i)
                        }
                        SleepOutcome::Interrupted => Exit::Failure(Cause::interrupt(None)),
                    }
                },
            )
        },
        ForkOpts::immediate(),
    );

    wait_for_sleepers(&clock, 2);
    clock.adjust(EffectDuration::from_millis(200));
    wait_for_done(&done, 2);
    // The next window is parked when the interrupt lands.
    wait_for_sleepers(&clock, 2);

    fiber.interrupt();
    let exit = fiber.join();

    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the batch ends interrupted"
    );
    // One adjust wakes both workers, so their record order is arbitrary; the
    // case pins that only the first window's items finish.
    let mut recorded = done.lock().unwrap().clone();
    recorded.sort_unstable();
    assert_eq!(recorded, vec![1u8, 2], "only the first window may finish");
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :802 `unbounded fail`.
/// The failing item cancels the slower siblings in flight, so the items whose
/// delays had not elapsed never record themselves.
#[test]
fn for_each_unbounded_failure_cancels_the_slower_workers() {
    let clock = Arc::new(TestClock::new());
    let done = Arc::new(Mutex::new(Vec::<u8>::new()));
    let rt = Runtime::new();

    let body_clock = clock.clone();
    let body_done = done.clone();
    let fiber = rt.fork_detach_with(
        move |ctx| -> Exit<Vec<u8>, String> {
            for_each_in(
                ctx,
                vec![1u8, 2, 3, 4, 5],
                Concurrency::Unbounded,
                move |i, c| {
                    let clock = Arc::clone(&body_clock);
                    let done = Arc::clone(&body_done);
                    let wait = u64::from(i) * 100;
                    match clock.sleep(EffectDuration::from_millis(wait), &|| c.is_interrupted()) {
                        SleepOutcome::Elapsed => {
                            done.lock().unwrap().push(i);
                            if i == 3 {
                                Exit::<u8, String>::fail("error".to_owned())
                            } else {
                                Exit::Success(i)
                            }
                        }
                        SleepOutcome::Interrupted => Exit::Failure(Cause::interrupt(None)),
                    }
                },
            )
        },
        ForkOpts::immediate(),
    );

    wait_for_sleepers(&clock, 5);
    clock.adjust(EffectDuration::from_millis(300));

    let exit = fiber.join();
    match exit {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&"error".to_owned()),
            "the first failure wins"
        ),
        Exit::Success(values) => panic!("expected the batch to fail, got {values:?}"),
    }
    // The crate's TestClock wakes every due sleeper at once, so the three that
    // were due record in an arbitrary order. What the case pins is which items
    // record at all: 4 and 5 were still in flight and must not.
    let mut recorded = done.lock().unwrap().clone();
    recorded.sort_unstable();
    assert_eq!(
        recorded,
        vec![1u8, 2, 3],
        "the siblings still in flight never record"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :754 `preserves interruption when a child cleanup dies`.
/// The interrupt and the cleanup's own defect share one cause: the batch does
/// not trade one for the other.
#[test]
fn an_interrupt_preserves_a_child_cleanup_defect() {
    let started = Arc::new(Deferred::<(), String>::new());
    let rt = Runtime::new();
    let body_started = started.clone();

    let park = |ctx: &Ctx| {
        while !ctx.is_interrupted() {
            std::thread::sleep(Duration::from_millis(1));
        }
        Exit::Failure(Cause::interrupt(None))
    };
    let bodies: Vec<Body<(), String>> = vec![
        Box::new(move |ctx: &Ctx| {
            // A held resource whose release dies: the cleanup runs when the
            // interrupt closes the scope.
            let held = ctx.acquire_release(Effect::<(), String>::succeed(()), |_: ()| {
                Effect::<(), String>::die(Defect::new("cleanup defect"))
            });
            // The fiber's own scope is open here, so the release is registered
            // rather than run inline; an `Err` would mean it was dropped.
            assert!(held.is_ok(), "the release was not registered: {held:?}");
            body_started.complete(());
            park(ctx)
        }),
        Box::new(park),
    ];

    let fiber = rt.fork_detach_with(
        move |ctx| all_in(ctx, bodies, Concurrency::Bounded(2)),
        ForkOpts::immediate(),
    );
    started.wait();
    fiber.interrupt();

    let cause = match fiber.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("an interrupted batch must not succeed"),
    };
    assert!(cause.is_interrupted(), "the interrupt survives the cleanup");
    assert!(
        cause
            .defects()
            .iter()
            .any(|defect| defect.message.contains("cleanup defect")),
        "the cleanup defect rides along in the same cause: {cause:?}"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :772 `preserves a recorded sibling failure on external interruption`.
/// The sibling's typed failure was recorded first; the interrupt that lands
/// while its cleanup is running is appended, never substituted.
#[test]
fn an_interrupt_keeps_a_sibling_failure_that_was_already_recorded() {
    let started = Arc::new(Deferred::<(), String>::new());
    let cleanup_started = Arc::new(Deferred::<(), String>::new());
    let release_cleanup = Arc::new(Deferred::<(), String>::new());
    let rt = Runtime::new();

    let (body_started, body_cleanup, body_release) = (
        started.clone(),
        cleanup_started.clone(),
        release_cleanup.clone(),
    );
    let sibling_started = started.clone();
    let bodies: Vec<Body<(), String>> = vec![
        Box::new(move |ctx: &Ctx| {
            body_started.complete(());
            while !ctx.is_interrupted() {
                std::thread::sleep(Duration::from_millis(1));
            }
            body_cleanup.complete(());
            body_release.wait();
            Exit::Failure(Cause::interrupt(None))
        }),
        Box::new(move |_ctx: &Ctx| {
            while !sibling_started.is_done() {
                std::thread::sleep(Duration::from_millis(1));
            }
            Exit::fail("sibling failure".to_owned())
        }),
    ];

    let fiber = rt.fork_detach_with(
        move |ctx| all_in(ctx, bodies, Concurrency::Bounded(2)),
        ForkOpts::immediate(),
    );
    cleanup_started.wait();
    fiber.interrupt_unsafe();
    release_cleanup.complete(());

    let cause = match fiber.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("an interrupted batch must not succeed"),
    };
    assert!(cause.is_interrupted(), "the interrupt is part of the cause");
    assert_eq!(
        cause.failure(),
        Some(&"sibling failure".to_owned()),
        "the recorded failure keeps its reason"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :818 `interrupts started workers when the mapper throws during initial
/// pumping`. A body that panics is a defect, and the worker already running is
/// interrupted and waited out before the batch reports.
#[test]
fn a_mapper_defect_interrupts_the_workers_that_started() {
    let interrupt_started = Arc::new(Deferred::<(), String>::new());
    let interrupt_finished = Arc::new(Deferred::<(), String>::new());
    let rt = Runtime::new();

    let (body_started, body_finished) = (interrupt_started.clone(), interrupt_finished.clone());
    let bodies: Vec<Body<(), String>> = vec![
        Box::new(move |ctx: &Ctx| {
            while !ctx.is_interrupted() {
                std::thread::sleep(Duration::from_millis(1));
            }
            body_started.complete(());
            body_finished.wait();
            Exit::Failure(Cause::interrupt(None))
        }),
        Box::new(|_ctx: &Ctx| panic!("mapper defect")),
    ];

    let fiber = rt.fork_detach_with(
        move |ctx| all_in(ctx, bodies, Concurrency::Bounded(2)),
        ForkOpts::immediate(),
    );
    interrupt_started.wait();
    assert!(
        !fiber.is_done(),
        "the batch waits for the interrupted worker's cleanup"
    );
    interrupt_finished.complete(());

    let cause = match fiber.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("a panicking body must not succeed"),
    };
    assert!(
        cause
            .defects()
            .iter()
            .any(|defect| defect.message.contains("mapper defect")),
        "the mapper's panic is the cause"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :844 `preserves a sibling cleanup defect when the mapper throws`.
/// The mapper's defect comes first and the cleanup that dies while the batch
/// stops the sibling is appended to it, in that order.
#[test]
fn a_mapper_defect_keeps_a_sibling_cleanup_defect() {
    let cleaned_up = flag();
    let cleaned = cleaned_up.clone();

    let bodies: Vec<Body<(), String>> = vec![
        Box::new(move |ctx: &Ctx| {
            let _held = ctx.acquire_release(Effect::<(), String>::succeed(()), |_: ()| {
                Effect::<(), String>::die(Defect::new("cleanup defect"))
            });
            while !ctx.is_interrupted() {
                std::thread::sleep(Duration::from_millis(1));
            }
            cleaned.store(true, Ordering::SeqCst);
            Exit::Failure(Cause::interrupt(None))
        }),
        Box::new(|_ctx: &Ctx| panic!("mapper defect")),
    ];

    let rt = Runtime::new();
    let fiber = rt.fork_detach_with(
        move |ctx| all_in(ctx, bodies, Concurrency::Bounded(2)),
        ForkOpts::immediate(),
    );
    let cause = match fiber.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("a panicking body must not succeed"),
    };

    assert!(is_set(&cleaned_up), "the sibling's cleanup ran");
    let messages: Vec<String> = cause
        .defects()
        .iter()
        .map(|defect| defect.message.clone())
        .collect();
    assert_eq!(
        messages.len(),
        2,
        "both defects are in the cause: {messages:?}"
    );
    assert_eq!(messages[0], "mapper defect", "the mapper's defect leads");
    // A scope finalizer renders its own failure with a prefix, so the cleanup
    // defect is carried inside that rendering rather than bare.
    assert!(
        messages[1].contains("cleanup defect"),
        "the cleanup's defect follows: {messages:?}"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :871 `interrupts started workers when observer-driven refill throws`.
/// The throwing body is only launched once the first slot frees, and the
/// worker already running is interrupted and waited out before the report.
#[test]
fn a_refill_defect_interrupts_the_workers_that_started() {
    let release = Arc::new(Deferred::<(), String>::new());
    let interrupt_started = Arc::new(Deferred::<(), String>::new());
    let interrupt_finished = Arc::new(Deferred::<(), String>::new());

    let (body_release, body_started, body_finished) = (
        release.clone(),
        interrupt_started.clone(),
        interrupt_finished.clone(),
    );
    let bodies: Vec<Body<(), String>> = vec![
        Box::new(move |_ctx: &Ctx| {
            body_release.wait();
            Exit::Success(())
        }),
        Box::new(move |ctx: &Ctx| {
            while !ctx.is_interrupted() {
                std::thread::sleep(Duration::from_millis(1));
            }
            body_started.complete(());
            body_finished.wait();
            Exit::Failure(Cause::interrupt(None))
        }),
        Box::new(|_ctx: &Ctx| panic!("refill defect")),
    ];

    let rt = Runtime::new();
    let fiber = rt.fork_detach_with(
        move |ctx| all_in(ctx, bodies, Concurrency::Bounded(2)),
        ForkOpts::immediate(),
    );
    assert!(!fiber.is_done(), "the batch parks while a worker waits");

    release.complete(());
    interrupt_started.wait();
    assert!(
        !fiber.is_done(),
        "the batch waits for the interrupted worker's cleanup"
    );
    interrupt_finished.complete(());

    let cause = match fiber.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("a panicking body must not succeed"),
    };
    assert!(
        cause
            .defects()
            .iter()
            .any(|defect| defect.message.contains("refill defect")),
        "the refill's panic is the cause"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :1039 `concurrency interrupts started siblings on failure`.
/// The window is three: the failing item and the two beside it start, the
/// fourth never does, and the failure interrupts the pair that is running.
#[test]
fn all_concurrency_interrupts_started_siblings_on_failure() {
    let started = Arc::new(Mutex::new(Vec::<u8>::new()));
    let interrupted = Arc::new(Mutex::new(Vec::<u8>::new()));
    let rt = Runtime::new();

    let bodies: Vec<Body<u8, String>> = (1u8..=4)
        .map(|i| {
            let started = started.clone();
            let interrupted = interrupted.clone();
            let body: Body<u8, String> = Box::new(move |ctx: &Ctx| {
                if i == 2 {
                    std::thread::sleep(Duration::from_millis(100));
                    return Exit::<u8, String>::fail("boom".to_owned());
                }
                started.lock().unwrap().push(i);
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    if ctx.is_interrupted() {
                        interrupted.lock().unwrap().push(i);
                        return Exit::Failure(Cause::interrupt(None));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Exit::Success(i)
            });
            body
        })
        .collect();

    let fiber = rt.fork_detach_with(
        move |ctx| all_in(ctx, bodies, Concurrency::Bounded(3)),
        ForkOpts::immediate(),
    );
    let exit = fiber.join();

    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_owned())),
        Exit::Success(values) => panic!("expected the batch to fail, got {values:?}"),
    }
    // The two siblings start concurrently, so the order they record in is
    // arbitrary (a thread that gets the scheduler first wins the push). The case
    // pins which siblings started, never which got there first.
    let mut started = started.lock().unwrap().clone();
    started.sort_unstable();
    assert_eq!(started, vec![1u8, 3], "only the window starts");
    // Both interrupted siblings notice concurrently, so the order they record
    // in is arbitrary; the case pins which siblings were interrupted.
    let mut noticed = interrupted.lock().unwrap().clone();
    noticed.sort_unstable();
    assert_eq!(
        noticed,
        vec![1u8, 3],
        "the started siblings are interrupted"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :3352 `concurrent: true interrupts the other side on failure`.
/// A failing leg settles the pair and stops the leg that would never end.
#[test]
fn concurrent_zip_interrupts_the_other_side_on_failure() {
    let interrupted = flag();
    let flag = interrupted.clone();

    let left = Effect::<(), String>::never().on_interrupt(move || {
        flag.store(true, Ordering::SeqCst);
    });
    let right = Effect::<(), String>::sync(|| {
        std::thread::sleep(Duration::from_millis(10));
    })
    .flat_map(|_| Effect::<(), String>::fail("boom".to_owned()));

    let (rt, fiber) = spawn_effect_any(left.zip_concurrent(right));
    let exit = fiber.join();

    assert_eq!(exit, Exit::<((), ()), String>::fail("boom".to_owned()));
    assert!(
        is_set(&interrupted),
        "the endless leg is interrupted when its sibling fails"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :3405 `concurrent: true interrupts the other side on failure`, the
/// `zipWith` spelling. The pairing is the same race; the crate combines the
/// two values with `map` rather than a separate `zipWith` entry point.
#[test]
fn concurrent_zip_with_interrupts_the_other_side_on_failure() {
    let interrupted = flag();
    let flag = interrupted.clone();

    let left = Effect::<(), String>::sync(|| {
        std::thread::sleep(Duration::from_millis(10));
    })
    .flat_map(|_| Effect::<(), String>::fail("boom".to_owned()));
    let right = Effect::<(), String>::never().on_interrupt(move || {
        flag.store(true, Ordering::SeqCst);
    });

    let combined = left
        .zip_concurrent(right)
        .map(|(a, b)| format!("{a:?}{b:?}"));
    let (rt, fiber) = spawn_effect_any(combined);
    let exit = fiber.join();

    assert_eq!(exit, Exit::<String, String>::fail("boom".to_owned()));
    assert!(
        is_set(&interrupted),
        "the endless leg is interrupted when its sibling fails"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2013 `timeout a long computation with a failure`.
/// The deadline runs the caller's own exit — a defect here — and `sandbox` +
/// `flip` is how the case reads that defect back as a value.
#[test]
fn timeout_or_else_can_die_on_its_own_terms() {
    let effect = Effect::<bool, String>::sync(|| {
        std::thread::sleep(Duration::from_millis(200));
        true
    })
    .timeout_or_else(
        EffectDuration::from_millis(10),
        Effect::<bool, String>::die(Defect::new("boom")),
    )
    .sandbox()
    .flip()
    .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>);

    let (rt, fiber) = spawn_effect_any(effect);
    let exit = fiber.join();

    assert_eq!(
        exit,
        Exit::Success(Cause::<String>::die(Defect::new("boom"))),
        "the deadline's own defect is what the caller sees"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :1864 `a ready source value is not confused with a zero-duration timeout`.
/// The source is ready when the race starts, so the answer is its own value and
/// the fallback never runs.
#[test]
fn a_ready_source_beats_a_zero_duration_timeout() {
    let fallback_ran = flag();
    let ran = fallback_ran.clone();
    let effect = Effect::<Option<&'static str>, String>::succeed(None)
        .timeout_or_else(
            EffectDuration::ZERO,
            Effect::sync(move || {
                ran.store(true, Ordering::SeqCst);
                Some("unexpected fallback")
            }),
        )
        .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>);

    assert_eq!(
        Runtime::run(effect),
        Exit::<Option<&'static str>, String>::Success(None)
    );
    assert!(!is_set(&fallback_ran), "the fallback must not run");
}

/// :1878 `slow fallback cannot let the original source win after timeout`.
/// The deadline fires first and the source is interrupted at its park, so a
/// source that would have finished a few milliseconds later still loses.
#[test]
fn a_late_source_does_not_win_after_the_timeout() {
    let source_finished = flag();
    let finished = source_finished.clone();

    // The source is resumed a moment after the deadline has already fired: it
    // has lost, and its value must not become the answer.
    let (resume_tx, resume_rx) = mpsc::channel();
    let interrupted = flag();
    let source_interrupted = interrupted.clone();
    let source = Effect::<&'static str, String>::callback(move |resume| {
        let _ = resume_tx.send(resume);
    })
    .flat_map(move |value| {
        let finished = finished.clone();
        Effect::<&'static str, String>::sync(move || {
            finished.store(true, Ordering::SeqCst);
            value
        })
    })
    .on_interrupt(move || source_interrupted.store(true, Ordering::SeqCst));
    let fallback = Effect::<&'static str, String>::sync(|| {
        std::thread::sleep(Duration::from_millis(30));
        "fallback"
    });

    let effect = source
        .timeout_or_else(EffectDuration::from_millis(5), fallback)
        .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>);

    let (rt, fiber) = spawn_effect_any(effect);
    let resume = resume_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the source registered its resume handle");
    // Past the 5ms deadline, while the fallback is still sleeping.
    std::thread::sleep(Duration::from_millis(20));
    // Answering the source now changes nothing: the deadline already won.
    resume.succeed("source");
    let exit = fiber.join();

    assert_eq!(exit, Exit::Success("fallback"));
    assert!(
        !is_set(&source_finished),
        "the source lost the race, so its value is discarded"
    );
    assert!(
        is_set(&interrupted),
        "the source was interrupted, not resumed"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :1444 `is interruptible` (the `repeat` block).
/// A repeat that has no natural end still answers the deadline, and the answer
/// is the timeout's `None` rather than a hang.
#[test]
fn a_repeat_of_void_is_interruptible_by_its_deadline() {
    let clock = Arc::new(TestClock::new());
    let effect = Effect::<(), String>::forever(|| Effect::<(), String>::succeed(()))
        .timeout_option(EffectDuration::from_millis(50))
        .provide::<ClockTag, ()>(clock.clone() as Arc<dyn ClockImpl>);

    let (rt, fiber) = spawn_effect_any(effect);
    std::thread::sleep(Duration::from_millis(20));
    clock.adjust(EffectDuration::from_millis(50));

    assert_eq!(fiber.join(), Exit::Success(None));
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2028 `timeout repetition of uninterruptible effect`.
/// The repeat boundary is the delivery point: the body is uninterruptible,
/// but it cannot stop the deadline from ending the batch.
#[test]
fn a_timeout_still_ends_an_uninterruptible_repeat() {
    let clock = Arc::new(TestClock::new());
    let effect =
        Effect::<(), String>::forever(|| Effect::<(), String>::succeed(()).uninterruptible())
            .timeout_option(EffectDuration::from_millis(10))
            .provide::<ClockTag, ()>(clock.clone() as Arc<dyn ClockImpl>);

    let (rt, fiber) = spawn_effect_any(effect);
    // Let the deadline be taken from the virtual clock before it moves.
    std::thread::sleep(Duration::from_millis(20));
    clock.adjust(EffectDuration::from_millis(10));

    assert_eq!(fiber.join(), Exit::Success(None));
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2041 `timeout in uninterruptible region`.
/// The region's mask does not delay a body that finishes first, so the
/// timeout answers `Some` and completes long before its 20s deadline.
#[test]
fn timeout_inside_an_uninterruptible_region_still_completes() {
    let effect = Effect::<(), String>::succeed(())
        .timeout_option(EffectDuration::from_secs(20))
        .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>)
        .uninterruptible();

    let (rt, fiber) = spawn_effect_any(effect);
    assert_eq!(fiber.join(), Exit::Success(Some(())));
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :1265 `releases on interrupt`.
/// Effect 4 masks the acquisition, so an interrupt that lands while it is in
/// flight still lets it finish and still runs the release. A sync acquisition
/// is atomic here, which is this crate's version of that mask: the interrupt
/// lands inside the acquire's own step and the release runs when the scope
/// closes afterwards.
#[test]
fn acquire_release_still_releases_after_an_interrupt() {
    let released = flag();
    let rel = released.clone();
    let rt = Runtime::new();

    let fiber = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            let acquired = ctx.acquire_release(
                Effect::<u8, String>::sync(|| {
                    std::thread::sleep(Duration::from_millis(80));
                    7
                }),
                move |_value: u8| {
                    let rel = rel.clone();
                    Effect::<(), String>::sync(move || {
                        rel.store(true, Ordering::SeqCst);
                    })
                },
            );
            if let Err(cause) = acquired {
                return Exit::Failure(cause);
            }
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::Failure(Cause::interrupt(None))
        },
        ForkOpts::immediate(),
    );

    // Land the interrupt in the middle of the acquisition, not before it
    // starts: the case is about an acquire already in flight.
    std::thread::sleep(Duration::from_millis(20));
    fiber.interrupt();
    let _ = fiber.join();

    assert!(
        is_set(&released),
        "the scope closes on interrupt, and the release runs"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2149 `run of interruptible`.
/// `exit` inside an interruptible region sees the interrupt as a failure with
/// interrupts only, and the surrounding `uninterruptible` still runs to the
/// end of the chain.
#[test]
fn exit_inside_a_masked_region_observes_the_interrupt() {
    let recovered = flag();
    let record = recovered.clone();

    let effect = Effect::<(), String>::never()
        .interruptible()
        .exit()
        .flat_map(move |result| {
            let record = record.clone();
            Effect::<(), std::convert::Infallible>::sync(move || {
                if let Exit::Failure(cause) = result {
                    record.store(cause.has_interrupts_only(), Ordering::SeqCst);
                }
            })
        })
        .uninterruptible();

    let (rt, target) = spawn_effect_any(effect);
    target.interrupt();
    let _ = target.join();

    assert!(
        is_set(&recovered),
        "exit must observe the interrupt the region let through"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2167 `alternating interruptibility`.
/// Two interruptible windows inside one masked chain each observe the
/// interrupt, so the continuation after each window runs.
#[test]
fn alternating_interruptibility_runs_each_observed_window() {
    let seen = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let first_seen = seen.clone();
    let second_seen = seen.clone();
    // The interrupt must land while the fiber is parked in its FIRST window, as
    // it does in Effect 4: there the forked child reaches `never` before the
    // test fiber resumes to interrupt it. Without this handshake the interrupt
    // can arrive before the chain enters the window, and the first `exit` never
    // sees it (measured: only the outer window's continuation ran).
    let armed = flag();
    let announce_armed = armed.clone();

    let effect = Effect::<(), String>::sync(move || {
        announce_armed.store(true, Ordering::SeqCst);
    })
    .flat_map(|_| Effect::<(), String>::never())
    .interruptible()
    .exit()
    .flat_map(move |_| {
        let seen = first_seen.clone();
        Effect::<(), std::convert::Infallible>::sync(move || {
            seen.lock().unwrap().push("first");
        })
    })
    .uninterruptible()
    .interruptible()
    .exit()
    .flat_map(move |_| {
        let seen = second_seen.clone();
        Effect::<(), std::convert::Infallible>::sync(move || {
            seen.lock().unwrap().push("second");
        })
    })
    .uninterruptible();

    let (rt, target) = spawn_effect_any(effect);
    assert!(
        wait_for(&armed, Duration::from_secs(5)),
        "the first window never armed"
    );
    target.interrupt();
    let exit = target.join();

    assert_eq!(
        *seen.lock().unwrap(),
        vec!["first", "second"],
        "both interruptible windows must hand control back (exit: {exit:?})"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2337 `delivers a pending interrupt when restore re-enables
/// interruptibility`.
/// The interrupt is requested while masked, so it lands at the `restore`
/// boundary: the restored effect never runs and the exit carries the interrupt.
#[test]
fn restore_delivers_a_pending_interrupt_before_the_restored_effect_runs() {
    let masked = Arc::new(Deferred::<(), String>::new());
    let resume = Arc::new(Deferred::<(), String>::new());
    let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let body_masked = masked.clone();
    let body_resume = resume.clone();
    let body_events = events.clone();
    let effect = Effect::uninterruptible_mask(move |restore| {
        let inner_events = body_events.clone();
        Effect::<(), String>::sync(move || {
            body_masked.complete(());
        })
        .flat_map(move |_| {
            let resume = body_resume.clone();
            Effect::<(), String>::sync(move || resume.wait())
        })
        .flat_map(move |_| {
            let events = inner_events.clone();
            restore.apply(Effect::<(), String>::suspend(move || {
                events.lock().unwrap().push("restored");
                Effect::<(), String>::never()
            }))
        })
    });

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !masked.is_done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(masked.is_done(), "the masked region never announced itself");
    events.lock().unwrap().push("masked");

    target.interrupt_unsafe();
    events.lock().unwrap().push("interrupted");

    resume.complete(());
    events.lock().unwrap().push("resumed");

    let exit = target.join();
    assert_eq!(
        *events.lock().unwrap(),
        vec!["masked", "interrupted", "resumed"],
        "the restored effect must not run after the pending interrupt"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.has_interrupts()),
        "the fiber still ends interrupted"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2122 `interrupt of never is interrupted with cause`.
/// `never` ends only by interruption, and the exit it leaves carries the
/// interrupt as its cause: no typed failure, no defect.
#[test]
fn never_interrupted_reports_an_interrupt_cause() {
    let (rt, target) = spawn_effect_any(Effect::<(), String>::never());
    target.interrupt();

    let cause = match target.join() {
        Exit::Failure(cause) => cause,
        Exit::Success(_) => panic!("an interrupted never must not succeed"),
    };
    assert!(cause.is_interrupted(), "the interrupt is the cause");
    assert!(cause.failure().is_none(), "no typed failure was raised");
    assert!(cause.defects().is_empty(), "no defect was raised");
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2129 `catch + ensuring + interrupt`.
/// An interrupt is not a failure the handler sees, so `catchCause` stays
/// silent, while `ensuring` still runs on the way out.
#[test]
fn catch_cause_ignores_an_interrupt_and_ensuring_still_runs() {
    let caught = flag();
    let ensured = flag();
    let catch_flag = caught.clone();
    let ensuring_flag = ensured.clone();

    let effect = Effect::<(), String>::never()
        .catch_cause(move |_cause| {
            let flag = catch_flag.clone();
            Effect::<(), String>::sync(move || flag.store(true, Ordering::SeqCst))
        })
        .ensuring(move || {
            ensuring_flag.store(true, Ordering::SeqCst);
        });

    let (rt, target) = spawn_effect_any(effect);
    target.interrupt();
    let exit = target.join();

    assert!(
        !is_set(&caught),
        "catchCause must not observe an interrupt as a failure"
    );
    assert!(is_set(&ensured), "ensuring runs on the interrupt path");
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the interrupt still ends the fiber"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2962 `onExit - callback observes interrupt exit when the fiber is
/// interrupted`. The callback sees the failure exit with its interrupt cause,
/// and the effect still ends interrupted afterwards.
#[test]
fn on_exit_sees_the_interrupt_exit() {
    let observed = Arc::new(Mutex::new(None::<bool>));
    let slot = Arc::clone(&observed);

    let effect = Effect::<(), String>::never().on_exit(move |exit| {
        let interrupted = match exit {
            Exit::Failure(cause) => cause.is_interrupted(),
            Exit::Success(_) => false,
        };
        *slot.lock().unwrap() = Some(interrupted);
    });

    let (rt, target) = spawn_effect_any(effect);
    target.interrupt();
    let exit = target.join();

    assert_eq!(
        *observed.lock().unwrap(),
        Some(true),
        "onExit must see the interrupt exit"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the callback observes, it does not swallow"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2598 `is interrupted with parent`.
/// A child forked with `startImmediately` is running when the parent is
/// interrupted, so both the child's and the parent's finalizers run.
#[test]
fn fork_child_immediate_dies_with_its_parent() {
    let child = flag();
    let parent = flag();
    let (child_flag, parent_flag) = (Arc::clone(&child), Arc::clone(&parent));

    let effect = Effect::<(), String>::never()
        .on_interrupt(move || child_flag.store(true, Ordering::SeqCst))
        .fork_child_immediate()
        .flat_map(move |_child| {
            Effect::<(), String>::never()
                .on_interrupt(move || parent_flag.store(true, Ordering::SeqCst))
        });

    let (rt, outer) = spawn_effect_any(effect);
    std::thread::sleep(Duration::from_millis(30));
    outer.interrupt();
    let _ = outer.join();

    assert!(
        is_set(&child),
        "the forked child must be interrupted with its parent"
    );
    assert!(is_set(&parent), "the parent's own onInterrupt must run");
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2624 `is not interrupted with parent` (the `forkDaemon` block).
/// A detached child starts at once and keeps running after its parent is
/// interrupted, so only the parent's finalizer runs.
#[test]
fn fork_detach_outlives_its_parent() {
    let child = flag();
    let parent = flag();
    let (child_flag, parent_flag) = (Arc::clone(&child), Arc::clone(&parent));

    let effect = Effect::<(), String>::never()
        .on_interrupt(move || child_flag.store(true, Ordering::SeqCst))
        .fork_detach()
        .flat_map(move |_child| {
            Effect::<(), String>::never()
                .on_interrupt(move || parent_flag.store(true, Ordering::SeqCst))
        });

    let (rt, outer) = spawn_effect_any(effect);
    std::thread::sleep(Duration::from_millis(30));
    outer.interrupt();
    let _ = outer.join();
    // The daemon is untouched; give it a moment to prove it did not notice.
    std::thread::sleep(Duration::from_millis(50));

    assert!(is_set(&parent), "the parent's own onInterrupt must run");
    assert!(
        !is_set(&child),
        "a detached child must outlive its parent's interrupt"
    );
    let _ = rt.shutdown();
}

/// :2650 `is interrupted when scope is closed` (the `forkIn` block).
#[test]
fn fork_in_is_interrupted_when_its_scope_closes() {
    let interrupted = flag();
    let started = flag();
    let (child_flag, child_started) = (Arc::clone(&interrupted), Arc::clone(&started));

    let rt = Runtime::new();
    let scope = rt.create_scope();
    let child = Effect::<(), String>::sync(move || child_started.store(true, Ordering::SeqCst))
        .flat_map(|_| Effect::<(), String>::never())
        .on_interrupt(move || child_flag.store(true, Ordering::SeqCst));
    let effect = child
        .fork_in(Arc::clone(&scope))
        .flat_map(|_| Effect::<(), String>::never());

    let (_rt, outer) = spawn_effect_any(effect);
    assert!(
        wait_for(&started, Duration::from_secs(5)),
        "the scoped child never started"
    );
    scope.close();
    assert!(
        wait_for(&interrupted, Duration::from_secs(5)),
        "closing the scope must interrupt its member"
    );
    outer.interrupt_unsafe();
    let _ = outer.join();
}

/// :2668 `is interrupted when scope is closed` (the `forkScoped` block).
/// The ambient scope `Effect::scoped` installs is the one this child binds
/// to, and its close interrupts the child.
#[test]
fn fork_scoped_is_interrupted_when_the_ambient_scope_closes() {
    let interrupted = flag();
    let started = flag();
    let observed = Arc::clone(&started);
    let waited = Arc::clone(&started);
    let (child_flag, child_started) = (Arc::clone(&interrupted), Arc::clone(&started));

    let inner = Effect::<(), String>::sync(move || child_started.store(true, Ordering::SeqCst))
        .flat_map(|_| Effect::<(), String>::never())
        .on_interrupt(move || child_flag.store(true, Ordering::SeqCst))
        .fork_scoped()
        .flat_map(|_child| {
            // `scoped` closes the scope when this body returns, so the body
            // waits for the child to start; the close is then the interrupt.
            Effect::<(), String, _>::sync(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !waited.load(Ordering::SeqCst) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        });

    let rt = Runtime::new();
    assert_eq!(rt.spawn_effect(inner.scoped()).join(), Exit::Success(()));
    assert!(is_set(&observed), "the scoped child never started");
    assert!(
        wait_for(&interrupted, Duration::from_secs(5)),
        "closing the ambient scope must interrupt its member"
    );
}

/// :2382 `region after a restored section stays uninterruptible`.
/// The restored section runs interruptible, then the region parks masked: an
/// interrupt requested while it parks is deferred until the region completes,
/// and the events show the deferred order.
#[test]
fn region_after_a_restored_section_stays_uninterruptible() {
    let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let resume: Arc<Mutex<Option<elm_effect::Resume<(), String>>>> = Arc::new(Mutex::new(None));
    let parked = flag();

    let restored_events = Arc::clone(&events);
    let masked_events = Arc::clone(&events);
    let slot = Arc::clone(&resume);
    let parked_body = parked.clone();
    let effect = Effect::<(), String>::uninterruptible_mask(move |restore| {
        restore
            .apply(Effect::<(), String>::sync(move || {
                restored_events
                    .lock()
                    .unwrap()
                    .push("restored section completed")
            }))
            .flat_map(move |_| {
                let slot = Arc::clone(&slot);
                let parked = parked_body.clone();
                Effect::<(), String>::callback(move |answer| {
                    *slot.lock().unwrap() = Some(answer);
                    parked.store(true, Ordering::SeqCst);
                })
            })
            .flat_map(move |_| {
                masked_events
                    .lock()
                    .unwrap()
                    .push("masked region completed");
                Effect::succeed(())
            })
    });

    let (rt, fiber) = spawn_effect_any(effect);
    assert!(
        wait_for(&parked, Duration::from_secs(5)),
        "the masked region never parked"
    );
    events.lock().unwrap().push("after restore");
    // `interrupt()` waits for the region to finish, which it cannot do until
    // the resume below, so the request is the non-blocking one Effect 4's
    // `interruptUnsafe` answers.
    fiber.interrupt_unsafe();
    events.lock().unwrap().push("interrupted");
    assert!(
        !fiber.is_done(),
        "a masked region must defer the interrupt it was handed"
    );

    let answer = resume
        .lock()
        .unwrap()
        .take()
        .expect("the region parked with an answer slot");
    events.lock().unwrap().push("resumed");
    answer.succeed(());

    let exit = fiber.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the deferred interrupt still ends the fiber: {exit:?}"
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "restored section completed",
            "after restore",
            "interrupted",
            "resumed",
            "masked region completed"
        ]
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2284 `callback cleanup effect runs on interrupt`.
/// The crate's registration returns nothing, so the cleanup is the caller's
/// `onInterrupt` (the contract `Effect::callback` documents); the observable
/// is the same: the cleanup runs when the parked callback is interrupted.
#[test]
fn callback_cleanup_runs_on_interrupt() {
    let cleaned_up = flag();
    let body_flag = cleaned_up.clone();
    let effect = Effect::<(), String>::callback(|_answer| {})
        .on_interrupt(move || body_flag.store(true, Ordering::SeqCst));

    let (rt, fiber) = spawn_effect_any(effect);
    fiber.interrupt();
    let exit = fiber.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the interrupted callback must end interrupted: {exit:?}"
    );
    assert!(is_set(&cleaned_up), "the cleanup must run on the interrupt");
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2736 `onExit reports a thrown finalizer on success`.
/// A panicking finalizer is a defect, not an unwind: the exit the caller
/// reifies is `Exit.die("finalizer defect")`.
#[test]
fn on_exit_reports_a_thrown_finalizer_on_success() {
    let exit = Runtime::run(
        Effect::<i32, String>::succeed(1)
            .on_exit(|_| panic!("finalizer defect"))
            .exit(),
    );
    // `exit()` cannot fail, so the wrapper itself succeeds; the reified exit
    // is the one `exit()` answered.
    let Exit::Success(inner) = exit else {
        panic!("the wrapper must not fail")
    };
    let Exit::Failure(cause) = inner else {
        panic!("a panicking finalizer must not succeed")
    };
    assert!(cause.failure().is_none(), "a panic is never a typed error");
    assert_eq!(cause.defects().len(), 1);
    assert_eq!(cause.defects()[0].message, "finalizer defect");
}

/// Fiber.test.ts:205 `runs an async interrupt finalizer exactly once, in
/// order`. A self-interrupt ends the run at the next boundary: the finalizer
/// runs once, the code after it never runs, and an awaiter only resumes once
/// the run has settled. Effect 4's finalizer yields between its start and end;
/// this crate's finalizer is synchronous, so both halves are one callback.
#[test]
fn an_interrupt_finalizer_runs_once_before_the_awaiter_resumes() {
    let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let body_events = Arc::clone(&events);
    let finalizer_events = Arc::clone(&events);
    let tail_events = Arc::clone(&events);

    let effect: Effect<(), String> = Effect::with_fiber(move |fiber| {
        let fiber = fiber.clone();
        Effect::<(), String>::sync(move || {
            fiber.interrupt();
            body_events.lock().unwrap().push("acquired");
        })
        .on_interrupt(move || finalizer_events.lock().unwrap().push("finalizer"))
        .flat_map(move |_| {
            tail_events.lock().unwrap().push("unreachable");
            Effect::succeed(())
        })
    });

    let (rt, target) = spawn_effect_any(effect);
    let exit = target.join();
    events.lock().unwrap().push("awaited");

    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.has_interrupts()),
        "a self-interrupted run ends interrupted: {exit:?}"
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec!["acquired", "finalizer", "awaited"],
        "the finalizer runs once, before the awaiter, and the tail never runs"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2742 `an outer onInterrupt still sees the interruption after an inner
/// finalizer throws`. The reasons are `[Interrupt, Die]`, in that order, and
/// the outer finalizer runs even though the inner one panicked.
#[test]
fn on_interrupt_survives_an_inner_finalizer_panic() {
    let finalized = flag();
    let flag_for_callback = finalized.clone();
    let exit = Runtime::run(
        Effect::<i32, String>::interrupt()
            .on_exit(|_| panic!("finalizer defect"))
            .on_interrupt(move || flag_for_callback.store(true, Ordering::SeqCst))
            .exit(),
    );
    assert!(is_set(&finalized), "the outer onInterrupt must still run");
    let Exit::Success(inner) = exit else {
        panic!("the wrapper must not fail")
    };
    let Exit::Failure(cause) = inner else {
        panic!("an interrupt is not a success")
    };
    let tags: Vec<&str> = cause
        .reasons
        .iter()
        .map(|reason| match reason {
            elm_effect::Reason::Fail(_) => "Fail",
            elm_effect::Reason::Die(_) => "Die",
            elm_effect::Reason::Interrupt { .. } => "Interrupt",
        })
        .collect();
    assert_eq!(tags, vec!["Interrupt", "Die"]);
    match &cause.reasons[1] {
        elm_effect::Reason::Die(defect) => {
            assert_eq!(defect.message, "finalizer defect")
        }
        other => panic!("expected the finalizer's defect, got {other:?}"),
    }
}

/// :294 `callback can branch over sync/async`.
/// The registration may answer before the park (the sync branch) or from
/// another thread (the async one); both resolve the same callback.
#[test]
fn callback_answers_from_a_sync_or_async_registration() {
    let sync_answer = Runtime::run(Effect::<i32, String>::callback(|resume| {
        resume.succeed(1);
    }));
    assert_eq!(sync_answer, Exit::Success(1));

    let async_answer = Runtime::run(Effect::<i32, String>::callback(|resume| {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            resume.succeed(2);
        });
    }));
    assert_eq!(async_answer, Exit::Success(2));
}

/// :2115 `sync forever is interruptible`.
/// The repeat boundary answers an interrupt even though every iteration
/// succeeds at once.
#[test]
fn sync_forever_is_interruptible() {
    let (rt, fiber) = spawn_effect_any(Effect::<(), String>::forever(|| {
        Effect::<(), String>::succeed(())
    }));
    std::thread::sleep(Duration::from_millis(30));
    fiber.interrupt();
    let exit = fiber.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "forever must end interrupted: {exit:?}"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2246 `callback cannot resume on interrupt`.
/// The park ends on the interrupt even though its answer never arrives at that
/// point, and the fiber exits interrupted. The late answer is sent afterwards
/// to show it changes nothing (and does not crash a handle whose effect is
/// gone).
#[test]
fn a_callback_cannot_resume_after_the_interrupt() {
    let (tx, rx) = mpsc::channel();
    let effect = Effect::<String, String>::callback(move |resume| {
        // Keep the resume handle in the test, so the completion is ordered
        // after the interrupt by the test itself rather than by a sleep.
        let _ = tx.send(resume);
    });

    let (rt, target) = spawn_effect_any(effect);
    let resume = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the callback registers a resume handle");

    target.interrupt_unsafe();
    // No answer yet: the park has to end on the interrupt alone.
    let exit = target.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "an interrupted callback exits interrupted: {exit:?}"
    );
    // The answer that arrives afterwards is dropped, not a panic on a slot
    // whose effect is gone.
    resume.succeed("foo".to_owned());
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2297 `defers a pending interrupt until the masked region completes`.
/// The interrupt lands while the region is parked, the fiber is still alive
/// afterwards, and the region finishes its work before the interrupt is
/// delivered.
#[test]
fn a_masked_region_defers_a_pending_interrupt_until_it_completes() {
    let masked = Arc::new(Deferred::<(), String>::new());
    let resume = Arc::new(Deferred::<(), String>::new());
    let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let body_masked = masked.clone();
    let body_resume = resume.clone();
    let body_events = events.clone();
    let effect = Effect::uninterruptible_mask(move |_restore| {
        let completed_events = body_events.clone();
        Effect::<(), String>::sync(move || {
            body_masked.complete(());
        })
        .flat_map(move |_| {
            let resume = body_resume.clone();
            Effect::<(), String>::sync(move || resume.wait())
        })
        .flat_map(move |_| {
            let events = completed_events.clone();
            Effect::<(), String>::sync(move || {
                events.lock().unwrap().push("masked region completed")
            })
        })
    });

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !masked.is_done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(masked.is_done(), "the masked region never announced itself");
    events.lock().unwrap().push("masked");

    target.interrupt_unsafe();
    events.lock().unwrap().push("interrupted");
    assert!(
        !target.is_done(),
        "a masked region must outlive the interrupt that lands inside it"
    );

    resume.complete(());
    events.lock().unwrap().push("resumed");

    let exit = target.join();
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "masked",
            "interrupted",
            "resumed",
            "masked region completed"
        ],
        "the region's work runs after the interrupt was requested"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "and the fiber still ends interrupted"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2232 `async can be uninterruptible`.
/// An interrupt that arrives while the masked region is parked on a sleep must
/// not cut the park short: the sleep still runs to its deadline, and the region
/// hands the pending interrupt on only once it ends.
#[test]
fn a_masked_sleep_runs_to_its_deadline_before_the_interrupt_lands() {
    let started = flag();
    let body_started = started.clone();
    let recorded = flag();
    let body_recorded = recorded.clone();

    type NeedsClock = elm_effect::Cons<ClockTag, ()>;
    let effect =
        Effect::<(), String, NeedsClock>::sync(move || body_started.store(true, Ordering::SeqCst))
            .flat_map(|_| Effect::sleep(EffectDuration::from_millis(40)))
            .flat_map(move |_| {
                let recorded = body_recorded.clone();
                Effect::<(), String, NeedsClock>::sync(move || {
                    recorded.store(true, Ordering::SeqCst)
                })
            })
            .uninterruptible()
            .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>);

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_set(&started) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(is_set(&started), "the masked region never started");

    let interrupt_at = Instant::now();
    target.interrupt();
    let exit = target.join();
    let elapsed = interrupt_at.elapsed();

    assert!(
        is_set(&recorded),
        "the sync after the sleep must run despite the interrupt"
    );
    assert!(
        elapsed >= Duration::from_millis(30),
        "the park was cut short after {elapsed:?}, so the mask did not hold"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the interrupt still ends the fiber once the region is done"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :2260 `closing scope is uninterruptible`.
/// The same masked region, one fact further: the park's own finalizer and the
/// step after it both run, so the region closes cleanly before the interrupt is
/// delivered.
#[test]
fn a_masked_region_closes_its_scope_before_the_interrupt_returns() {
    let sleeping = flag();
    let body_sleeping = sleeping.clone();
    let closed = flag();
    let cleanup_closed = closed.clone();

    type NeedsClock = elm_effect::Cons<ClockTag, ()>;
    let effect =
        Effect::<(), String, NeedsClock>::sync(move || body_sleeping.store(true, Ordering::SeqCst))
            .flat_map(|_| Effect::sleep(EffectDuration::from_millis(40)))
            .ensuring(move || cleanup_closed.store(true, Ordering::SeqCst))
            .uninterruptible()
            .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>);

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_set(&sleeping) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(is_set(&sleeping), "the masked region never started");

    target.interrupt();
    let exit = target.join();

    assert!(
        is_set(&closed),
        "the region's finalizer must run before the interrupt is delivered"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the fiber still ends interrupted"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :1972 `external interruption stops a pending fallback`.
/// The source's finalizer has already run when the deadline wins, the fallback
/// is pending, and an interrupt from outside stops it too: both cleanups run and
/// the exit carries the interrupt.
#[test]
fn an_external_interrupt_stops_a_pending_fallback() {
    let source_stopped = flag();
    let cleanup_source = source_stopped.clone();
    let fallback_started = flag();
    let body_started = fallback_started.clone();
    let fallback_stopped = flag();
    let cleanup_fallback = fallback_stopped.clone();
    let calls = Arc::new(AtomicU64::new(0));
    let fallback_calls = calls.clone();

    let source = Effect::<(), String>::never().ensuring(move || {
        cleanup_source.store(true, Ordering::SeqCst);
    });
    // The fallback parks after announcing itself, so the test interrupts a
    // pending fallback rather than a finished one.
    let fallback = Effect::<(), String>::sync(move || {
        fallback_calls.fetch_add(1, Ordering::SeqCst);
        body_started.store(true, Ordering::SeqCst);
    })
    .flat_map(|_| Effect::<(), String>::never())
    .ensuring(move || cleanup_fallback.store(true, Ordering::SeqCst));

    // A non-zero deadline, so the source has parked and hung its finalizer
    // before the deadline wins (see :1813 for why zero is not used here).
    let effect = source
        .timeout_or_else(EffectDuration::from_millis(50), fallback)
        .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>);

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_set(&fallback_started) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        is_set(&fallback_started),
        "the deadline never handed over to the fallback"
    );

    target.interrupt();
    let exit = target.join();

    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the interrupt is what the fiber ends with: {exit:?}"
    );
    assert!(
        is_set(&source_stopped),
        "the source's cleanup ran when the deadline won"
    );
    assert!(
        is_set(&fallback_stopped),
        "the pending fallback's cleanup ran when the interrupt stopped it"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the fallback is built and run once"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// :1813 `${form}: factory and body follow cleanup and replay`.
/// The source's finalizer runs before the fallback, so the fallback observes a
/// stopped source; a second run of the same program runs the factory and the
/// body again, in the same order. The crate's fallback is an exit builder rather
/// than an effect builder, so the factory/body split is two closures here.
#[test]
fn a_timeout_fallback_runs_after_cleanup_and_again_on_the_next_run() {
    let stopped = flag();
    let factories = Arc::new(AtomicU64::new(0));
    let bodies = Arc::new(AtomicU64::new(0));
    let order = Arc::new(Mutex::new(Vec::<String>::new()));

    let program = |stopped: Arc<AtomicBool>,
                   factories: Arc<AtomicU64>,
                   bodies: Arc<AtomicU64>,
                   order: Arc<Mutex<Vec<String>>>| {
        let source_cleanup = stopped.clone();
        let order_cleanup = order.clone();
        let source = Effect::<bool, String>::never().ensuring(move || {
            source_cleanup.store(true, Ordering::SeqCst);
            order_cleanup.lock().unwrap().push("stopped".to_owned());
        });
        // A non-zero deadline, so the source has parked in `never` and hung its
        // finalizer before the deadline can win. The case is about cleanup
        // ordering and replay, not about what a zero-length deadline may skip.
        let fallback = Effect::<bool, String>::sync(move || {
            let seen = stopped.load(Ordering::SeqCst);
            factories.fetch_add(1, Ordering::SeqCst);
            order.lock().unwrap().push(format!("factory:{seen}"));
            let seen = stopped.load(Ordering::SeqCst);
            bodies.fetch_add(1, Ordering::SeqCst);
            order.lock().unwrap().push(format!("body:{seen}"));
            true
        });
        source
            .timeout_or_else(EffectDuration::from_millis(50), fallback)
            .provide::<ClockTag, ()>(Arc::new(SystemClock) as Arc<dyn ClockImpl>)
    };

    let before = (
        factories.load(Ordering::SeqCst),
        bodies.load(Ordering::SeqCst),
    );
    // A fiber, not `Runtime::run`: the deadline races a child of the running
    // fiber, so the program needs one.
    let (rt_first, first_run) = spawn_effect_any(program(
        stopped.clone(),
        factories.clone(),
        bodies.clone(),
        order.clone(),
    ));
    let first = first_run.join();
    let first_stopped = stopped.load(Ordering::SeqCst);
    stopped.store(false, Ordering::SeqCst);
    let (rt_second, second_run) = spawn_effect_any(program(
        stopped.clone(),
        factories.clone(),
        bodies.clone(),
        order.clone(),
    ));
    let second = second_run.join();

    assert_eq!(before, (0, 0), "building the program runs no factory");
    assert_eq!(first, Exit::Success(true));
    assert_eq!(second, Exit::Success(true));
    assert!(rt_first.shutdown().is_empty(), "a fiber outlived the run");
    assert!(rt_second.shutdown().is_empty(), "a fiber outlived the run");
    assert!(first_stopped, "the source's finalizer ran on the first run");
    assert!(stopped.load(Ordering::SeqCst), "and again on the second");
    assert_eq!(
        (
            factories.load(Ordering::SeqCst),
            bodies.load(Ordering::SeqCst)
        ),
        (2, 2),
        "one factory and one body per run"
    );
    assert_eq!(
        *order.lock().unwrap(),
        vec![
            "stopped",
            "factory:true",
            "body:true",
            "stopped",
            "factory:true",
            "body:true",
        ],
        "cleanup precedes the fallback on every run"
    );
}

/// :2687 `works` (do notation).
/// Rust has no do-notation: Effect 4's `bindTo`/`let`/`bind`/`tap` chain is the
/// same threading of bindings through `flat_map`, and the final tap sees the
/// pair Effect 4's record holds.
#[test]
fn a_flat_map_chain_threads_every_do_notation_binding() {
    let seen = Arc::new(Mutex::new(None::<(i32, String)>));
    let slot = seen.clone();

    let effect = Effect::<i32, String>::succeed(1)
        .flat_map(Effect::<i32, String>::succeed)
        .flat_map(|a| {
            let next = a + 1;
            Effect::<i32, String>::succeed(next).map(move |b| (a, b))
        })
        .flat_map(|(a, b)| {
            let text = b.to_string();
            Effect::<String, String>::succeed(text).map(move |b| (a, b))
        })
        .flat_map(move |pair| {
            let slot = slot.clone();
            Effect::<(), String>::sync(move || *slot.lock().unwrap() = Some(pair))
        });

    assert_eq!(Runtime::run(effect), Exit::Success(()));
    assert_eq!(
        *seen.lock().unwrap(),
        Some((1, "2".to_owned())),
        "both bindings reach the last step unchanged"
    );
}

#[test]
fn the_static_edge_gives_with_fiber_a_real_fiber() {
    // `Runtime::run` used to interpret on the caller's thread, so this was a
    // defect. The edge now runs the description on a fiber like the managed
    // edge does; the body must run and see that fiber's id.
    let called = flag();
    let body_called = called.clone();
    let effect: Effect<u64, String> = Effect::with_fiber(move |fiber| {
        body_called.store(true, Ordering::SeqCst);
        assert!(fiber.id().0 > 0, "the edge hands out a minted fiber id");
        Effect::<u64, String>::succeed(1)
    });
    assert_eq!(Runtime::run(effect), Exit::Success(1));
    assert!(
        is_set(&called),
        "the with-fiber body ran at the static edge"
    );
}

/// :1422 `${name} interrupts the loser when interrupted while starting`.
/// The left leg interrupts the fiber running the race. Both policies must stop
/// that leg, and its interrupt finalizer runs before the race returns.
#[test]
fn a_race_interrupts_its_loser_when_the_parent_is_interrupted_while_starting() {
    for race_first in [false, true] {
        let interrupted = flag();
        let loser_seen = interrupted.clone();
        let effect = Effect::<(), String>::with_fiber(move |parent| {
            let parent = parent.clone();
            let left = Effect::<(), String>::sync(move || parent.interrupt())
                .flat_map(|_| Effect::<(), String>::never())
                .on_interrupt(move || loser_seen.store(true, Ordering::SeqCst));
            let right = Effect::<(), String>::succeed(());
            if race_first {
                left.race_first(right)
            } else {
                left.race(right)
            }
        });

        let (rt, target) = spawn_effect_any(effect);
        let _ = target.join();

        assert!(
            is_set(&interrupted),
            "the losing leg's cleanup must run (race_first = {race_first})"
        );
        assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
    }
}

/// `Effect.acquireUseRelease`: the release runs whatever the use did, and the
/// acquire's own failure skips it.
#[test]
fn acquire_use_release_releases_on_success_and_on_a_failing_use() {
    for (name, use_fails) in [("success", false), ("failing use", true)] {
        let released = flag();
        let flag_for_release = released.clone();
        let acquire = Effect::<u32, String>::sync(|| 7);
        let effect = acquire.acquire_use_release(
            move |resource| {
                assert_eq!(resource, 7, "the use sees the acquired resource");
                Effect::<u32, String>::suspend(move || {
                    if use_fails {
                        Effect::fail("use failed".to_string())
                    } else {
                        Effect::succeed(resource * 2)
                    }
                })
            },
            move |resource, _| {
                let released = flag_for_release.clone();
                Effect::<(), String>::sync(move || {
                    assert_eq!(resource, 7, "the release sees the resource too");
                    released.store(true, Ordering::SeqCst);
                })
            },
        );

        let exit = Runtime::run(effect);
        assert!(is_set(&released), "release must run on {name}");
        if use_fails {
            assert_eq!(exit, Exit::Failure(Cause::fail("use failed".to_string())));
        } else {
            assert_eq!(exit, Exit::Success(14));
        }
    }

    // A failed acquire never reaches the use, so there is nothing to release.
    let released = flag();
    let flag_for_release = released.clone();
    let exit = Runtime::run(
        Effect::<u32, String>::fail("no resource".to_string()).acquire_use_release(
            |_| Effect::<u32, String>::succeed(1),
            move |_, _| {
                let released = flag_for_release.clone();
                Effect::<(), String>::sync(move || released.store(true, Ordering::SeqCst))
            },
        ),
    );
    assert_eq!(exit, Exit::Failure(Cause::fail("no resource".to_string())));
    assert!(
        !is_set(&released),
        "a failed acquire has nothing to release"
    );
}

/// Both halves can fail, and Effect 4 keeps both (`Cause.combine`).
#[test]
fn acquire_use_release_combines_use_and_release_failures() {
    let exit = Runtime::run(Effect::<u32, String>::succeed(1).acquire_use_release(
        |_| Effect::<u32, String>::fail("use failed".to_string()),
        |_, _| Effect::<(), String>::fail("release failed".to_string()),
    ));
    let Exit::Failure(cause) = exit else {
        panic!("both halves failing is a failure: {exit:?}")
    };
    let failures: Vec<&str> = cause.failures().into_iter().map(String::as_str).collect();
    assert_eq!(failures, vec!["use failed", "release failed"], "{cause:?}");
}

/// Effect.test.ts:2922 `rethrown caught error in release`. A dying release on
/// an otherwise successful use is the fiber's failure, and nothing erases it.
#[test]
fn acquire_use_release_surfaces_a_failing_release() {
    let exit = Runtime::run(Effect::<u32, String>::succeed(1).acquire_use_release(
        |_| Effect::<u32, String>::succeed(2),
        |_, _| Effect::<(), String>::die(Defect::new("release boom")),
    ));
    let Exit::Failure(cause) = exit else {
        panic!("a dying release must fail the fiber: {exit:?}")
    };
    assert!(cause.failure().is_none(), "a die is not a typed failure");
    assert_eq!(cause.defects().len(), 1, "the release's defect survives");
}

/// Effect.test.ts:342 `acquireUseRelease runs release when use throws`.
/// A panicking use is a defect, the release still runs exactly once, and it
/// sees that defect in the exit it is handed.
#[test]
fn acquire_use_release_releases_after_a_panicking_use() {
    let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release_count = releases.clone();
    let release_saw_defect = flag();
    let saw_defect = release_saw_defect.clone();
    let exit = Runtime::run(Effect::<u32, String>::succeed(1).acquire_use_release(
        |_| -> Effect<u32, String> { panic!("use blew up") },
        move |resource, use_exit| {
            let release_count = release_count.clone();
            let saw_defect = saw_defect.clone();
            // Read the exit here: the effect it builds must be 'static, so
            // it cannot carry the borrow.
            let carried_defect =
                matches!(use_exit, Exit::Failure(cause) if !cause.defects().is_empty());
            Effect::<(), String>::sync(move || {
                assert_eq!(resource, 1, "the release sees the resource");
                release_count.fetch_add(1, Ordering::SeqCst);
                if carried_defect {
                    saw_defect.store(true, Ordering::SeqCst);
                }
            })
        },
    ));
    assert_eq!(releases.load(Ordering::SeqCst), 1, "release runs once");
    assert!(is_set(&release_saw_defect), "the release sees the defect");
    assert!(
        matches!(&exit, Exit::Failure(cause) if !cause.defects().is_empty()),
        "a panicking use is a defect: {exit:?}"
    );
}

/// Effect.test.ts:315 `acquireUseRelease interrupt`.
/// The interrupt lands while the masked acquire is still running: the acquire
/// completes, the use is skipped, the release still runs, and it is the
/// release that sees the interrupt in its exit.
#[test]
fn acquire_use_release_skips_the_use_when_the_acquire_was_interrupted() {
    let acquired = flag();
    let used = flag();
    let released = flag();
    let release_saw_interrupt = flag();
    let started = flag();

    let acquire_started = started.clone();
    let acquire_done = acquired.clone();
    let slow_acquire = Arc::new(Deferred::<u32, String>::new());
    let acquire_resume = slow_acquire.clone();
    let use_flag = used.clone();
    let release_flag = released.clone();
    let release_interrupt = release_saw_interrupt.clone();

    let effect = Effect::<u32, String>::sync(move || {
        acquire_started.store(true, Ordering::SeqCst);
        let value = acquire_resume.wait();
        acquire_done.store(true, Ordering::SeqCst);
        value
    })
    .acquire_use_release(
        move |resource| {
            let used = use_flag.clone();
            Effect::<u32, String>::sync(move || {
                used.store(true, Ordering::SeqCst);
                resource
            })
        },
        move |resource, use_exit| {
            let released = release_flag.clone();
            let saw_interrupt = release_interrupt.clone();
            let interrupted = matches!(use_exit, Exit::Failure(cause) if cause.is_interrupted());
            Effect::<(), String>::sync(move || {
                assert_eq!(resource, 123, "the release sees the resource");
                if interrupted {
                    saw_interrupt.store(true, Ordering::SeqCst);
                }
                released.store(true, Ordering::SeqCst);
            })
        },
    );

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_set(&started) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(is_set(&started), "the acquire never started");

    target.interrupt_unsafe();
    slow_acquire.complete(123);
    let exit = target.join();

    assert!(is_set(&acquired), "the masked acquire completes");
    assert!(!is_set(&used), "the use is skipped after the interrupt");
    assert!(is_set(&released), "the release still runs");
    assert!(
        is_set(&release_saw_interrupt),
        "the release sees the interrupt"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the fiber ends interrupted: {exit:?}"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// Effect.test.ts:2209 `acquireUseRelease release runs when use is
/// interrupted`. The use never answers, the interrupt ends it, and the release
/// runs anyway with that interrupt in its exit.
#[test]
fn acquire_use_release_releases_when_the_use_is_interrupted() {
    let released = flag();
    let release_saw_interrupt = flag();
    let release_flag = released.clone();
    let release_interrupt = release_saw_interrupt.clone();

    let effect = Effect::<u32, String>::succeed(123).acquire_use_release(
        |_| Effect::<u32, String>::never(),
        move |resource, use_exit| {
            let released = release_flag.clone();
            let saw_interrupt = release_interrupt.clone();
            let interrupted = matches!(use_exit, Exit::Failure(cause) if cause.is_interrupted());
            Effect::<(), String>::sync(move || {
                assert_eq!(resource, 123);
                if interrupted {
                    saw_interrupt.store(true, Ordering::SeqCst);
                }
                released.store(true, Ordering::SeqCst);
            })
        },
    );

    let (rt, target) = spawn_effect_any(effect);
    target.interrupt_unsafe();
    let exit = target.join();

    assert!(is_set(&released), "release runs after an interrupted use");
    assert!(
        is_set(&release_saw_interrupt),
        "the release sees the interrupt"
    );
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the fiber ends interrupted: {exit:?}"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}

/// Effect.test.ts:370 `interruption during acquireUseRelease`.
/// The interrupt lands while the use is parked; the mask holds it off, the
/// use still finishes, the release still runs, and the fiber ends interrupted.
#[test]
fn acquire_use_release_still_releases_when_interrupted_inside_the_region() {
    let acquired = flag();
    let released = flag();
    let acquire_flag = acquired.clone();
    let release_flag = released.clone();
    let resume = Arc::new(Deferred::<u32, String>::new());

    let use_resume = resume.clone();
    let effect = Effect::<u32, String>::sync(move || {
        acquire_flag.store(true, Ordering::SeqCst);
        123
    })
    .acquire_use_release(
        move |resource| {
            Effect::<u32, String>::sync(move || {
                assert_eq!(resource, 123, "the use sees the acquired resource");
                use_resume.wait()
            })
        },
        move |_, _| {
            let released = release_flag.clone();
            Effect::<(), String>::sync(move || released.store(true, Ordering::SeqCst))
        },
    )
    .uninterruptible();

    let (rt, target) = spawn_effect_any(effect);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_set(&acquired) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(is_set(&acquired), "the region never acquired");

    target.interrupt_unsafe();
    assert!(
        !target.is_done(),
        "the masked use must outlive the interrupt that lands inside it"
    );
    assert!(
        !is_set(&released),
        "the release waits for the use, not for the interrupt"
    );

    resume.complete(123);
    let exit = target.join();

    assert!(is_set(&released), "the release runs at the region's end");
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.is_interrupted()),
        "the deferred interrupt is delivered at the region's end: {exit:?}"
    );
    assert!(rt.shutdown().is_empty(), "a fiber outlived the test");
}
