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
    // Base 40ms scaled by the pin's jitter band, [0.8, 1.2).
    assert!((32..48).contains(&d1.as_millis()));
}

/// The pin's `runDelays`: drive `decide` over each input, recording the delay,
/// with a trailing zero where the schedule completes.
fn run_delays<In, A>(schedule: &Schedule<In, A>, inputs: &[In]) -> Vec<u64>
where
    In: Send + Sync + 'static,
    A: Send + 'static,
{
    let mut out = Vec::new();
    for (attempt, input) in inputs.iter().enumerate() {
        match schedule.decide(attempt as u64, input) {
            Some((_, delay)) => out.push(delay.as_millis()),
            None => {
                out.push(0);
                break;
            }
        }
    }
    out
}

#[test]
fn schedule_constructors_and_combining_match_the_pin() {
    // Schedule.test.ts:500, :510 — spaced and fixed hold a constant delay.
    assert_eq!(
        run_delays(&Schedule::<(), ()>::spaced(Duration::from_secs(1)), &[(); 5]),
        vec![1000; 5]
    );
    assert_eq!(
        run_delays(&Schedule::<(), ()>::fixed(Duration::from_secs(1)), &[(); 5]),
        vec![1000; 5]
    );

    // :21 — max waits the slowest member.
    let slowest = Schedule::<(), ()>::fixed(Duration::from_secs(5))
        .max(Schedule::exponential(Duration::from_secs(5)))
        .max(Schedule::spaced(Duration::from_secs(10)));
    assert_eq!(run_delays(&slowest, &[(); 3]), vec![10_000, 10_000, 20_000]);

    // :51 — min waits the fastest member.
    let fastest = Schedule::<(), ()>::fixed(Duration::from_secs(5))
        .min(Schedule::exponential(Duration::from_secs(5)))
        .min(Schedule::spaced(Duration::from_secs(10)));
    assert_eq!(run_delays(&fastest, &[(); 3]), vec![5_000, 5_000, 5_000]);

    // :476 — jittered stays inside 80%-120% of the base delay.
    let jittered = Schedule::<(), ()>::spaced(Duration::from_secs(1)).jitter(Random::with_seed(11));
    for attempt in 0..20 {
        let (_, delay) = jittered.decide(attempt, &()).unwrap();
        assert!(
            (800..=1200).contains(&delay.as_millis()),
            "attempt {attempt} left the band: {}ms",
            delay.as_millis()
        );
    }
}

#[test]
fn schedule_once_and_up_to_match_the_pin() {
    // :379 — once continues on the first step, then completes.
    let once = Schedule::<&'static str, ()>::once();
    assert_eq!(once.decide(0, &"first").map(|(_, d)| d.as_millis()), Some(0));
    assert!(once.decide(1, &"second").is_none());

    // :405 — upTo({ times: 2 }) lets two through and echoes the input.
    let capped = Schedule::<&'static str, &'static str>::identity().up_to_times(2);
    assert_eq!(capped.decide(0, &"a").map(|(out, _)| out), Some("a"));
    assert_eq!(capped.decide(1, &"b").map(|(out, _)| out), Some("b"));
    assert!(capped.decide(2, &"c").is_none());

    // :393 — repeat(once) executes the effect exactly twice.
    let runs = Arc::new(AtomicUsize::new(0));
    let exit = Runtime::run(repeat(
        {
            let runs = Arc::clone(&runs);
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                Effect::<u32, String, ()>::succeed(7)
            }
        },
        Schedule::<u32, ()>::once(),
    ));
    assert!(matches!(exit, Exit::Success(7)));
    assert_eq!(runs.load(Ordering::SeqCst), 2);
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
