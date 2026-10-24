//! Interrupt wake latency, measured end to end.
//!
//! The fiber parks in `wait_interrupted`; `interrupt` must wake it through
//! `request_interrupt`'s notification, not through the old 10 ms cooperative
//! quantum. Averaging keeps the witness from turning into a scheduler-noise
//! test while still failing if a poll comes back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use elm_effect::{Exit, ForkOpts, Runtime};

#[test]
fn an_interrupt_wakes_a_parked_fiber_before_the_quantum() {
    let runtime = Runtime::new();
    let mut samples = Vec::new();

    for _ in 0..20 {
        let (ready_tx, ready_rx) = mpsc::channel();
        let parked = Arc::new(AtomicBool::new(false));
        let body_parked = Arc::clone(&parked);
        let handle = runtime.fork_detach_with(
            move |ctx| {
                ready_tx.send(()).unwrap();
                body_parked.store(true, Ordering::SeqCst);
                ctx.fiber().wait_interrupted();
                Exit::<(), String>::succeed(())
            },
            ForkOpts::immediate(),
        );
        ready_rx.recv().unwrap();
        while !parked.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        // Give the body the last few instructions into the wait. A request
        // that lands before the park is consumed by the flag, which would
        // measure the wrong path.
        std::thread::sleep(Duration::from_millis(1));

        let started = Instant::now();
        handle.interrupt();
        let latency = started.elapsed();
        samples.push(latency);
        let exit = handle.join();
        assert!(
            matches!(exit, Exit::Failure(_)),
            "a body woken by an interrupt ends interrupted, not success: {exit:?}"
        );
    }

    let total: Duration = samples.iter().sum();
    let average = total / samples.len() as u32;
    println!(
        "interrupt-to-wake average {average:?}, worst {:?}",
        samples.iter().max().unwrap()
    );
    assert!(
        average < Duration::from_millis(3),
        "average wake latency {average:?} looks like a poll; samples: {samples:?}"
    );
}
