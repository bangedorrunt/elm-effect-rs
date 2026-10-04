//! Pull-based streams: bounded chunks, lazy transforms, eager sinks.

use std::sync::Arc;

use elm_effect::{Effect, Exit, Stream, from_iter};

#[test]
fn run_fold_sums_chunks() {
    let stream: Stream<u32, String, ()> = from_iter(1u32..=100, 7);
    let exit = stream.run_fold(0u32, |acc, n| acc + n);
    assert!(matches!(exit, Exit::Success(5050)));
    let normalized: Exit<Vec<u32>, String> = from_iter(1u32..=3, 0).run_collect();
    assert!(matches!(normalized, Exit::Success(ref v) if *v == vec![1u32, 2, 3]));
}

#[test]
fn map_filter_transform_lazily() {
    let stream: Stream<u32, String, ()> = from_iter(1u32..=10, 3)
        .map(|n| n * 2)
        .filter(|n| n % 3 == 0);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![6u32, 12, 18]));
}

#[test]
fn take_terminates_infinite_input() {
    // Never materializes the unbounded input: only five pulls run.
    let stream: Stream<u32, String, ()> = from_iter(0u32.., 4).take(5);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![0u32, 1, 2, 3, 4]));
    let empty_take: Exit<Vec<u32>, String> = from_iter(1u32..=3, 2).take(0).run_collect();
    assert!(matches!(empty_take, Exit::Success(ref v) if v.is_empty()));
}

#[test]
fn empty_and_concat() {
    let first: Stream<u32, String, ()> = from_iter(vec![1u32, 2].into_iter(), 10);
    let second: Stream<u32, String, ()> = from_iter(vec![3u32].into_iter(), 10);
    let exit = first
        .concat(second)
        .concat(Stream::<u32, String, ()>::empty())
        .run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1u32, 2, 3]));
}

#[test]
fn failure_propagates_from_pulls() {
    let stream = Stream::<u32, String, ()>::from_pull(|| {
        Effect::<Option<Vec<u32>>, String>::fail("boom".to_string())
    });
    let exit = stream.run_collect();
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_string())),
        Exit::Success(_) => panic!("expected failure"),
    }
}

#[test]
fn fanout_peak_in_flight() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let stream: Stream<u32, String, ()> = from_iter(0u32..10, 10).map_effect(
        {
            let (current, peak) = (Arc::clone(&current), Arc::clone(&peak));
            move |n: u32, _ctx: &elm_effect::Ctx| {
                let (current, peak) = (Arc::clone(&current), Arc::clone(&peak));
                Effect::sync(move || {
                    let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    current.fetch_sub(1, Ordering::SeqCst);
                    n * 2
                })
            }
        },
        3,
    );
    let exit = stream.run_collect();
    assert!(
        matches!(exit, Exit::Success(ref v) if *v == vec![0u32, 2, 4, 6, 8, 10, 12, 14, 16, 18])
    );
    assert!(peak.load(Ordering::SeqCst) <= 3);
}

#[test]
fn cancel_batch_releases_every_child() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let started = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let stream: Stream<u32, String, ()> = from_iter(0u32..5, 5).map_effect(
        {
            let (started, released) = (Arc::clone(&started), Arc::clone(&released));
            move |_n: u32, ctx: &elm_effect::Ctx| {
                let (started, released) = (Arc::clone(&started), Arc::clone(&released));
                ctx.acquire_release(
                    Effect::<(), String>::sync(move || {
                        started.fetch_add(1, Ordering::SeqCst);
                    }),
                    move |_| {
                        released.fetch_add(1, Ordering::SeqCst);
                        Effect::<(), String>::succeed(())
                    },
                )
                .expect("acquire cannot fail");
                while !ctx.is_interrupted() {
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
                Effect::<u32, String>::fail("cancelled".to_string())
            }
        },
        5,
    );
    let driver = stream.clone();
    let exit = std::thread::scope(|s| {
        let handle = s.spawn(move || driver.run_collect());
        while started.load(Ordering::SeqCst) < 5 {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        stream.cancel();
        handle.join().unwrap()
    });
    assert!(matches!(exit, Exit::Failure(_)));
    assert_eq!(started.load(Ordering::SeqCst), 5);
    assert_eq!(released.load(Ordering::SeqCst), 5);
}

#[test]
fn map_effect_smaller_bound_is_sequential() {
    let stream: Stream<u32, String, ()> = from_iter(0u32..5, 5)
        .map_effect(|n: u32, _ctx: &elm_effect::Ctx| Effect::succeed(n + 1), 1);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1u32, 2, 3, 4, 5]));
}
