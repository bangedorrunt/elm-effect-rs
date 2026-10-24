//! Effect-level fan-out: ordered results, bounded concurrency, fail-fast
//! sibling interruption, ambient services, and parent interruption.

#![cfg(feature = "tokio")]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use elm_effect::{Concurrency, Effect, Exit, ManagedRuntime, Runtime, ServiceTag, use_service};

#[test]
fn all_fans_out_and_keeps_input_order() {
    // The barrier only clears if all three run at once; collecting in input
    // order proves the combinator re-orders by index, not by finish time.
    let barrier = Arc::new(Barrier::new(3));
    let effects: Vec<Effect<usize, String>> = (0..3)
        .map(|index| {
            let barrier = Arc::clone(&barrier);
            Effect::sync(move || {
                barrier.wait();
                index
            })
        })
        .collect();
    assert_eq!(
        Runtime::run(Effect::all(effects, Concurrency::Unbounded)),
        Exit::Success(vec![0, 1, 2])
    );
}

#[test]
fn bounded_fan_out_keeps_at_most_n_alive() {
    // Each pair meets at the barrier, so the peak is exactly 2 when the
    // bound is 2, and the batch still returns every result in order.
    let barrier = Arc::new(Barrier::new(2));
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let effects: Vec<Effect<usize, String>> = (0..6)
        .map(|index| {
            let barrier = Arc::clone(&barrier);
            let current = Arc::clone(&current);
            let peak = Arc::clone(&peak);
            Effect::sync(move || {
                let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                barrier.wait();
                current.fetch_sub(1, Ordering::SeqCst);
                index
            })
        })
        .collect();
    assert_eq!(
        Runtime::run(Effect::all(effects, Concurrency::Bounded(2))),
        Exit::Success((0..6).collect())
    );
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "bounded fan-out ran more than its bound"
    );
}

#[test]
fn first_failure_interrupts_and_releases_siblings() {
    let released = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&released);
    let slow = Effect::<u32, String>::never().ensuring(move || {
        flag.store(true, Ordering::SeqCst);
    });
    let failing = Effect::<u32, String>::fail("boom".to_string());
    let exit = Runtime::run(Effect::all(vec![slow, failing], Concurrency::Unbounded));
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_string())),
        Exit::Success(_) => panic!("a failed child must fail the fan-out"),
    }
    assert!(
        released.load(Ordering::SeqCst),
        "the interrupted sibling's release ran"
    );
}

#[test]
fn children_read_services_provided_around_the_fan_out() {
    struct Counter;
    impl ServiceTag for Counter {
        type Service = u32;
        const ID: &'static str = "test/fan-out-counter";
    }
    let effects: Vec<Effect<u32, std::convert::Infallible, Counter>> =
        (0..3).map(|_| use_service::<Counter>()).collect();
    let program = Effect::all(effects, Concurrency::Unbounded).provide::<Counter, ()>(7);
    assert_eq!(Runtime::run(program), Exit::Success(vec![7, 7, 7]));
}

#[test]
fn all_discard_drops_values_and_keeps_failures() {
    let ok: Vec<Effect<u32, String>> = vec![Effect::succeed(1), Effect::succeed(2)];
    assert_eq!(
        Runtime::run(Effect::all_discard(ok, Concurrency::Sequential)),
        Exit::Success(())
    );
    let bad: Vec<Effect<u32, String>> = vec![Effect::succeed(1), Effect::fail("no".to_string())];
    match Runtime::run(Effect::all_discard(bad, Concurrency::Unbounded)) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"no".to_string())),
        Exit::Success(()) => panic!("a failed child must fail all_discard"),
    }
}

#[test]
fn interrupting_the_parent_stops_the_children_and_runs_their_releases() {
    let started = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let child = |started: Arc<AtomicUsize>, released: Arc<AtomicUsize>| {
        let start = Arc::clone(&started);
        let release = Arc::clone(&released);
        Effect::<(), String>::sync(move || {
            start.fetch_add(1, Ordering::SeqCst);
        })
        .flat_map(|()| Effect::<u32, String>::never())
        .ensuring(move || {
            release.fetch_add(1, Ordering::SeqCst);
        })
    };
    let program = Effect::all(
        vec![
            child(Arc::clone(&started), Arc::clone(&released)),
            child(Arc::clone(&started), Arc::clone(&released)),
        ],
        Concurrency::Unbounded,
    );
    // `run_fork` starts immediately; `Runtime::spawn_effect` would wait for
    // a join, and a join here would block before the interrupt can land.
    let rt = ManagedRuntime::new();
    let handle = rt.run_fork(program);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while started.load(Ordering::SeqCst) < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the fan-out children never started"
        );
        std::thread::park_timeout(std::time::Duration::from_millis(5));
    }
    handle.interrupt();
    assert!(
        matches!(handle.join(), Exit::Failure(_)),
        "an interrupted fan-out must exit interrupted"
    );
    assert_eq!(
        released.load(Ordering::SeqCst),
        2,
        "both children released on the parent's interrupt"
    );
    rt.dispose();
}
