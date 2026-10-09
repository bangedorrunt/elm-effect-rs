//! Pull-based streams: bounded chunks, lazy transforms, eager sinks.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Defect, Effect, Exit, Stream, from_iter};

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

// ---------------------------------------------------------------------------
// Streaming transforms at the pin: filterMap, takeWhile, takeUntil
// ---------------------------------------------------------------------------

#[test]
fn filter_map_keeps_some_only() {
    // Stream.make(1, 2, 3, 4).pipe(Stream.filterMap(n => n % 2 === 0
    //   ? Result.succeed(n + 10) : Result.failVoid), Stream.runCollect)
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4].into_iter(), 4)
        .filter_map(|n| (n % 2 == 0).then(|| n + 10));
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![12i64, 14]));
}

#[test]
fn take_while_takes_prefix() {
    // Stream.range(1, 5).pipe(Stream.takeWhile(n => n % 3 !== 0), ...)
    // === [1, 2]: the first miss is dropped, nothing after it is seen.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 5)
        .take_while(|n| n % 3 != 0);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 2]));
}

#[test]
fn take_until_takes_through_match() {
    // Stream.range(1, 5).pipe(Stream.takeUntil(n => n % 3 === 0), ...)
    // === [1, 2, 3]: the matching element IS taken, then the stream ends.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 5)
        .take_until(|n| n % 3 == 0);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 2, 3]));
}

#[test]
fn take_until_ends_after_the_match() {
    // The stop must be remembered: pulling again after the truncated chunk
    // yields nothing, not the rest of the source.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 2);
    let stream = stream.take_until(|n| *n == 2);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 2]));
}

#[test]
fn take_while_all_pass_yields_everything() {
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3].into_iter(), 1)
        .take_while(|_| true);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 2, 3]));
}

#[test]
fn take_while_first_element_fails_ends_immediately() {
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3].into_iter(), 1)
        .take_while(|n| *n > 10);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if v.is_empty()));
}

// ---------------------------------------------------------------------------
// Scanning at the pin: scan, mapAccumArray
// ---------------------------------------------------------------------------

#[test]
fn scan_emits_running_fold_without_init() {
    // Stream.make(1, 2, 3, 4, 5).pipe(Stream.scan(() => 0, (a, c) => a + c),
    //   Stream.runCollect) === Array.scan([1,2,3,4,5], 0, (a, c) => a + c)
    // The initial value is never emitted; only the running totals are.
    let stream: Stream<i64, String, ()> =
        from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 5).scan(0i64, |acc, cur| acc + cur);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 3, 6, 10, 15]));
}

#[test]
fn scan_state_survives_chunk_boundaries() {
    // The accumulator threads across pulls, so the chunking must not change
    // the running totals.
    let coarser: Exit<Vec<i64>, String> = from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 1)
        .scan(0i64, |acc, cur| acc + cur)
        .run_collect();
    assert!(matches!(coarser, Exit::Success(ref v) if *v == vec![1i64, 3, 6, 10, 15]));
}

#[test]
fn map_accum_array_maps_once_per_chunk() {
    // Stream.mapAccumArrayEffect(Stream.make(1, 2, 3), () => 0,
    //   (sum, values) => Effect.succeed([sum + values.length, values])
    //   ).pipe(Stream.runCollect) === [1, 2, 3]
    // make(1, 2, 3) is one chunk, so the mapper runs once: 0 + 3 = 3.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3].into_iter(), 3)
        .map_accum_array(0i64, |_sum, values| (values.len() as i64, values));
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 2, 3]));
}

#[test]
fn map_accum_array_threads_state_across_chunks() {
    // Two chunks, state carried: [1,2] -> sum 3 emits values, [3,4] -> sum 7.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4].into_iter(), 2)
        .map_accum_array(0i64, |sum, values| (sum + values.len() as i64, values));
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 2, 3, 4]));
}

#[test]
fn take_pulls_the_first_n_values() {
    // Stream.range(1, 5).pipe(Stream.take(3), Stream.runCollect) === [1, 2, 3]
    // `range` has no typed failure (E = Infallible); the annotation below
    // matches that, which is why this is not Stream<i64, String, ()>.
    let stream: Stream<i64, Infallible, ()> = Stream::range(1, 5, 4096.0).take(3);
    assert!(matches!(stream.run_collect(), Exit::Success(ref v) if *v == vec![1i64, 2, 3]));
}

#[test]
fn filter_map_effect_keeps_only_some_results() {
    // Stream.filterMapEffect over Effect.succeed: even values map to
    // Some(n + 1), odd values to None, so the collected result is [3, 5].
    //
    // Composed from the port's existing primitives rather than a new
    // combinator: map_effect produces the Option, filter_map flattens it.
    // The effect here is a pure succeed, which is exactly the observable the
    // row asserts.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4].into_iter(), 4)
        .map_effect(|n: i64, _ctx: &elm_effect::Ctx| {
            Effect::succeed((n % 2 == 0).then(|| n + 1))
        }, 1)
        .filter_map(|maybe: Option<i64>| maybe);
    assert!(matches!(stream.run_collect(), Exit::Success(ref v) if *v == vec![3i64, 5]));
}

// ---------------------------------------------------------------------------
// Stream.takeRight at the pin (describe "taking")
// ---------------------------------------------------------------------------

#[test]
fn take_right_keeps_the_last_n_elements_in_order() {
    // Stream.range(1, 5).pipe(Stream.takeRight(3), Stream.runCollect)
    // === [3, 4, 5], matching Array.takeRight(3) of [1, 2, 3, 4, 5].
    let stream: Stream<i64, Infallible, ()> = Stream::range(1, 5, 4096.0).take_right(3.0);
    assert!(
        matches!(stream.run_collect(), Exit::Success(ref v) if *v == vec![3i64, 4, 5])
    );
}

#[test]
fn take_right_normalizes_the_element_count() {
    // n normalizes like the pin's Count.normalize: NaN and non-positive
    // become 0, a positive value floors. Over [1, 2, 3]:
    //   NaN -> [], -1 -> [], 0.5 -> [], 1.9 -> [3], 2.9 -> [2, 3]
    let expected: Vec<Vec<i64>> = vec![vec![], vec![], vec![], vec![3], vec![2, 3]];
    let got: Vec<Vec<i64>> = [f64::NAN, -1.0, 0.5, 1.9, 2.9]
        .iter()
        .map(|n| {
            let stream: Stream<i64, String, ()> =
                from_iter(vec![1i64, 2, 3].into_iter(), 3).take_right(*n);
            match stream.run_collect() {
                Exit::Success(v) => v,
                Exit::Failure(_) => panic!("takeRight must not fail"),
            }
        })
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn take_right_zero_count_does_not_evaluate_the_upstream() {
    // A zero normalized count must not pull the upstream at all: the pin's
    // row drives a source that dies, so any pull would surface as a defect
    // rather than an empty result.
    let pulls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&pulls);
    let source: Stream<i32, String, ()> = Stream::from_pull(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Effect::<Option<Vec<i32>>, String>::die(elm_effect::Defect::new("upstream evaluated"))
    });
    let stream = source.take_right(f64::NAN);
    match stream.run_collect() {
        Exit::Success(v) => assert!(v.is_empty()),
        Exit::Failure(_) => panic!("a zero count must not evaluate the upstream"),
    }
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        0,
        "takeRight(0) must not pull the source"
    );
}
