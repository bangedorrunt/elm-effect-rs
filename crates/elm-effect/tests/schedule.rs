//! Schedules as values; retry repeats typed failures only.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{Defect, Duration, Effect, Exit, Random, Runtime, Schedule, repeat, retry};

#[test]
fn recurs_counts_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = {
        let calls = Arc::clone(&calls);
        move || {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 3 {
                Effect::<u32, String, ()>::fail("flaky".to_string())
            } else {
                Effect::succeed(n as u32)
            }
        }
    };
    let exit = Runtime::run(retry(make, Schedule::recurs(2)));
    assert!(matches!(exit, Exit::Success(3)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // max stops when every member stops: recurs(1) ends it after 1 retry.
    let calls = Arc::new(AtomicUsize::new(0));
    let policy = Schedule::recurs(1).max(Schedule::recurs(3));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::fail("x".to_string())
            }
        },
        policy,
    ));
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // min continues while any member continues: recurs(3) carries to 3 retries.
    let calls = Arc::new(AtomicUsize::new(0));
    let policy = Schedule::recurs(1).min(Schedule::recurs(3));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::fail("x".to_string())
            }
        },
        policy,
    ));
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[test]
fn max_min_choose_delays() {
    let long = Schedule::spaced(Duration::from_millis(10))
        .max(Schedule::spaced(Duration::from_millis(30)));
    let (_, delay) = long.decide(0, &"e".to_string()).unwrap();
    assert_eq!(delay.as_millis(), 30);

    let short = Schedule::spaced(Duration::from_millis(10))
        .min(Schedule::spaced(Duration::from_millis(30)));
    let (_, delay) = short.decide(0, &"e".to_string()).unwrap();
    assert_eq!(delay.as_millis(), 10);
}

#[test]
fn jitter_is_deterministic_per_seed() {
    let policy =
        Schedule::<String, ()>::exponential(Duration::from_millis(20)).jitter(Random::with_seed(7));
    let (_, d1) = policy.decide(1, &"e".to_string()).unwrap();
    let again =
        Schedule::<String, ()>::exponential(Duration::from_millis(20)).jitter(Random::with_seed(7));
    let (_, d2) = again.decide(1, &"e".to_string()).unwrap();
    assert_eq!(d1.as_millis(), d2.as_millis());
    // Base 40ms scaled by [0.5, 1.5).
    assert!((20..60).contains(&d1.as_millis()));
}

#[test]
fn retry_never_retries_die_or_interrupt() {
    // Defects pass through on first sight.
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::die(Defect::new("kaput"))
            }
        },
        Schedule::recurs(5),
    ));
    match exit {
        Exit::Failure(c) => assert_eq!(c.defects().len(), 1),
        Exit::Success(_) => panic!("expected failure"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Interrupts pass through on first sight.
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::interrupt()
            }
        },
        Schedule::recurs(5),
    ));
    match exit {
        Exit::Failure(c) => assert!(c.is_interrupted()),
        Exit::Success(_) => panic!("expected failure"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Typed failures do retry.
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(retry(
        {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::fail("nope".to_string())
            }
        },
        Schedule::recurs(2),
    ));
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn repeat_reruns_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(repeat(
        {
            let calls = Arc::clone(&calls);
            move || {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                Effect::<u32, String, ()>::succeed(n as u32)
            }
        },
        Schedule::recurs(2),
    ));
    assert!(matches!(exit, Exit::Success(3)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // A failure stops a repeat immediately.
    let exit = Runtime::run(repeat(
        || Effect::<u32, String, ()>::fail("stop".to_string()),
        Schedule::recurs(5),
    ));
    assert!(matches!(exit, Exit::Failure(_)));
}
