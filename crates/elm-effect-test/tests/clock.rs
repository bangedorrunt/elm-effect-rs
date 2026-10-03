//! Law L8: time enters through the Clock service; tests substitute the world.
//!
//! All coordination flows through `TestClock::adjust` and fiber join: no test
//! sleeps wall-clock minutes. (A 1ms interrupt-poll quantum inside sleeps
//! bounds wall latency only; outcomes are virtual-time-deterministic. Since
//! virtual time never advances by itself, tests run the blocking `timeout`
//! on a worker thread, wait for both racers to park, then adjust.)

use elm_effect::{Cause, Clock, ClockImpl, Duration, Effect, Exit, ForkOpts, Scope, TestClock};
use elm_effect_test::effect_test;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

fn park_until_pending(clock: &Arc<TestClock>, n: usize) {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while clock.pending_sleepers() < n {
        assert!(
            Instant::now() < deadline,
            "sleepers never parked (deadlock guard)"
        );
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
}

effect_test! { fn adjust_before_sleep_is_scheduled_does_not_complete_it(rt, clock) {
    // Adjust FIRST: the sleep schedules against the new time, so its
    // deadline is still in the future and the fiber stays suspended.
    // (Adjusting does not fast-forward sleeps scheduled afterwards.)
    clock.adjust(Duration::from_minutes(5));
    assert_eq!(clock.pending_sleepers(), 0);
    let body_clock = clock.clone();
    let sleeper = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            match body_clock.sleep(Duration::from_minutes(1), &|| ctx.is_interrupted()) {
                elm_effect::SleepOutcome::Elapsed => Exit::<(), String>::succeed(()),
                elm_effect::SleepOutcome::Interrupted => {
                    Exit::Failure(Cause::interrupt(None))
                }
            }
        },
        ForkOpts::immediate(),
    );
    // Still suspended: no further adjust has happened. Joining now would
    // block, so observe suspension without joining.
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while clock.pending_sleepers() < 1 {
        assert!(Instant::now() < deadline, "sleeper never parked");
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!sleeper.is_done());
    // Advance past the deadline: now it completes, and unregisters.
    clock.adjust(Duration::from_minutes(1));
    assert_eq!(sleeper.join(), Exit::<(), String>::succeed(()));
    assert_eq!(clock.pending_sleepers(), 0);
}}

effect_test! { fn timeout_interrupts_and_releases_loser(rt, clock) {
    let released = Arc::new(AtomicBool::new(false));
    let worker_released = released.clone();
    let worker_clock = clock.clone();
    let worker_rt = rt;
    let shared: Arc<dyn elm_effect::ClockImpl> = clock.clone();
    // Blocking race on a worker thread; the main thread drives virtual time.
    let worker = std::thread::spawn(move || {
        elm_effect::timeout(
            &worker_rt,
            shared,
            Duration::from_minutes(1),
            move |ctx| -> Exit<(), String> {
                ctx.add_finalizer(move || worker_released.store(true, Ordering::SeqCst));
                match worker_clock.sleep(Duration::from_minutes(5), &|| ctx.is_interrupted()) {
                    elm_effect::SleepOutcome::Elapsed => Exit::<(), String>::succeed(()),
                    elm_effect::SleepOutcome::Interrupted => {
                        Exit::Failure(Cause::interrupt(None))
                    }
                }
            },
        )
    });
    // Both racers parked (body 5min, timeout 1min); advance past the timeout.
    park_until_pending(&clock, 2);
    clock.adjust(Duration::from_minutes(1));
    let exit = worker.join().expect("worker thread");
    // Timeout won: None, and the loser was interrupted AND released first.
    assert_eq!(exit, Exit::<Option<()>, String>::succeed(None));
    assert!(released.load(Ordering::SeqCst));
}}

effect_test! { fn test_clock_onboarding_sleep_timeout_adjust(rt, clock) {
    // The v4 onboarding scenario: sleep 5 minutes under a 1 minute timeout,
    // fork, adjust 1 minute. The sleep never completes (interrupted: the
    // exit is a failure), with NO real sleep.
    let started = Instant::now();
    let completed = Arc::new(AtomicBool::new(false));
    let worker_completed = completed.clone();
    let worker_clock = clock.clone();
    let worker_rt = rt;
    let shared: Arc<dyn elm_effect::ClockImpl> = clock.clone();
    let worker = std::thread::spawn(move || {
        elm_effect::timeout(
            &worker_rt,
            shared,
            Duration::from_minutes(1),
            move |ctx| -> Exit<&'static str, String> {
                match worker_clock.sleep(Duration::from_minutes(5), &|| ctx.is_interrupted()) {
                    elm_effect::SleepOutcome::Elapsed => {
                        worker_completed.store(true, Ordering::SeqCst);
                        Exit::succeed("slept")
                    }
                    elm_effect::SleepOutcome::Interrupted => {
                        Exit::Failure(Cause::interrupt(None))
                    }
                }
            },
        )
    });
    park_until_pending(&clock, 2);
    clock.adjust(Duration::from_minutes(1));
    let exit = worker.join().expect("worker thread");
    assert_eq!(exit, Exit::<Option<&'static str>, String>::succeed(None));
    assert!(!completed.load(Ordering::SeqCst));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "no wall-clock minutes may pass"
    );
}}

effect_test! { fn clock_service_resolves_test_clock(rt, clock) {
    // The Clock service provided by the layer fixture resolves TestClock.
    let layer = elm_effect_test::clock_layer(&clock);
    let scope = Scope::new(200);
    let e: Effect<u64, Infallible, Clock> =
        elm_effect::use_service::<Clock>().map(|c| c.now_millis());
    let provided = layer.provide(e, &scope).expect("closed layer provides");
    assert_eq!(
        rt.spawn_effect(provided).join(),
        Exit::<u64, Infallible>::succeed(0)
    );
    clock.adjust(Duration::from_minutes(2));
    let provided2 = layer
        .provide(
            elm_effect::use_service::<Clock>().map(|c| c.now_millis()),
            &scope,
        )
        .expect("closed layer provides");
    assert_eq!(
        rt.spawn_effect(provided2).join(),
        Exit::<u64, Infallible>::succeed(2 * 60 * 1000)
    );
}}
