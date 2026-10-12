//! Pull-based streams: bounded chunks, lazy transforms, eager sinks.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Defect, Effect, Exit, Stream, from_iter, from_iterator_succeed};

// `ByteSize` is reached by the pin's own `#[path]` include convention rather
// than through the facade, which does not re-export it. `limit_bytes` takes a
// `ByteSize` limit the same way the pin's does.
#[path = "../src/data/byte_size.rs"]
mod byte_size;
use byte_size::ByteSize;

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
    let stream: Stream<u32, String, ()> = from_iter(0u32.., 4).take(5.0);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![0u32, 1, 2, 3, 4]));
    let empty_take: Exit<Vec<u32>, String> = from_iter(1u32..=3, 2).take(0.0).run_collect();
    assert!(matches!(empty_take, Exit::Success(ref v) if v.is_empty()));
}

// ---------------------------------------------------------------------------
// `take` with a fractional, NaN, or short-circuiting count
// (Stream.test.ts 781, 810, 819 — describe "take")
// ---------------------------------------------------------------------------

#[test]
fn take_rounds_positive_fractional_counts_down() {
    // Stream.make(1, 2, 3).pipe(Stream.take(0.5), ...) === []
    // Stream.make(1, 2, 3).pipe(Stream.take(1.9), ...) === [1]
    // Stream.make(1, 2, 3).pipe(Stream.take(2.9), ...) === [1, 2]
    // The pin floors, it does not round, so 1.9 takes one element.
    let source = || from_iter(vec![1i64, 2, 3].into_iter(), 3);
    let half: Exit<Vec<i64>, Infallible> = source().take(0.5).run_collect();
    assert!(matches!(half, Exit::Success(ref v) if v.is_empty()));
    let one: Exit<Vec<i64>, Infallible> = source().take(1.9).run_collect();
    assert!(matches!(one, Exit::Success(ref v) if *v == vec![1i64]));
    let two: Exit<Vec<i64>, Infallible> = source().take(2.9).run_collect();
    assert!(matches!(two, Exit::Success(ref v) if *v == vec![1i64, 2]));
}

#[test]
fn take_treats_nan_as_a_non_positive_count() {
    // Stream.take(NaN) normalizes to 0, so nothing is collected. The pin's
    // Count.normalize maps NaN to 0 for an element count, the same rule a
    // chunk size uses.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3].into_iter(), 3).take(f64::NAN);
    assert!(matches!(stream.run_collect(), Exit::Success(ref v) if v.is_empty()));
}

#[test]
fn take_nan_short_circuits_stream_evaluation() {
    // Stream.never.pipe(Stream.take(NaN), Stream.runCollect) === [], and the
    // unbounded upstream must never be pulled: NaN normalizes to 0, which is
    // the same short-circuit the integer zero already asserts. A counting
    // `map` observes whether the upstream was ever evaluated.
    let pulled = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&pulled);
    let stream: Stream<u32, String, ()> = from_iter(0u32.., 4)
        .map(move |n| {
            seen.fetch_add(1, Ordering::SeqCst);
            n
        })
        .take(f64::NAN);
    assert!(matches!(stream.run_collect(), Exit::Success(ref v) if v.is_empty()));
    assert_eq!(pulled.load(Ordering::SeqCst), 0, "a zero normalized count must not evaluate the upstream");
}

// ---------------------------------------------------------------------------
// takeWhileFilter / takeUntilEffect / takeWhileEffect
// (Stream.test.ts 846, 855, 864)
// ---------------------------------------------------------------------------

#[test]
fn take_while_filter_takes_elements_while_the_filter_succeeds() {
    // Stream.range(1, 5).pipe(Stream.takeWhileFilter(n =>
    //   n < 3 ? Result.succeed(n + 10) : Result.failVoid), Stream.runCollect)
    // === [11, 12]. A filter success is emitted, a failure ends the stream and
    // drops the failing element.
    let stream: Stream<i64, Infallible, ()> = Stream::range(1, 5, 4096.0)
        .take_while_filter(|n| (*n < 3).then(|| *n + 10));
    assert!(matches!(stream.run_collect(), Exit::Success(ref v) if *v == vec![11i64, 12]));
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
fn scan_effect_emits_running_fold_through_effects() {
    // Stream.make(1, 2, 3, 4, 5).pipe(
    //   Stream.scanEffect(() => 0, (acc, curr) => Effect.succeed(acc + curr)),
    //   Stream.runCollect) === Array.scan([1,2,3,4,5], 0, (a, c) => a + c)
    // The effectful step still threads one accumulator in order, and the
    // initial value is never emitted, so only the running totals surface.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 5)
        .scan_effect(0i64, |acc, cur| Effect::succeed(acc + cur));
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1i64, 3, 6, 10, 15]));
}

#[test]
fn scan_effect_threads_state_across_chunk_boundaries() {
    // The accumulator must survive the chunking, exactly like the pure scan:
    // five single-element chunks fold to the same running totals.
    let coarse: Exit<Vec<i64>, String> = from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 1)
        .scan_effect(0i64, |acc, cur| Effect::succeed(acc + cur))
        .run_collect();
    assert!(matches!(coarse, Exit::Success(ref v) if *v == vec![1i64, 3, 6, 10, 15]));
}

#[test]
fn scan_effect_failure_reaches_the_typed_channel() {
    // The unhappy path the row does not assert: a step that fails fails the
    // stream at that element, and the elements already folded stay collected.
    let stream: Stream<i64, String, ()> = from_iter(vec![1i64, 2, 3].into_iter(), 3)
        .scan_effect(0i64, |acc, cur| {
            if cur == 2 {
                Effect::fail("boom".to_string())
            } else {
                Effect::succeed(acc + cur)
            }
        });
    match stream.run_collect() {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_string())),
        Exit::Success(_) => panic!("a failing scan step must fail the stream"),
    }
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
    let stream: Stream<i64, Infallible, ()> = Stream::range(1, 5, 4096.0).take(3.0);
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

// ---------------------------------------------------------------------------
// rechunk / grouped normalize their chunk size
// (Stream.test.ts 1765 — describe "rechunk")
// ---------------------------------------------------------------------------
#[test]
fn rechunk_and_grouped_normalize_their_chunk_size() {
    // Stream.make(1, 2, 3).pipe(Stream.rechunk(n), Stream.chunks, Stream.runCollect)
    // and the same through Stream.grouped(n), for n in [NaN, 0.5, 1.9, 2.9].
    // A chunk size normalizes through Count.normalizeNonEmpty: NaN and
    // non-positive become 1 and a positive value floors, so 0.5 and 1.9 both
    // chunk by 1 and only 2.9 groups the last two elements.
    let expected: Vec<Vec<Vec<i64>>> = vec![
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1, 2], vec![3]],
    ];
    let counts = [f64::NAN, 0.5, 1.9, 2.9];
    let rechunked: Vec<Vec<Vec<i64>>> = counts
        .iter()
        .map(|n| {
            let stream: Stream<i64, Infallible, ()> =
                from_iter(vec![1i64, 2, 3].into_iter(), 3).rechunk(*n);
            match stream.run_collect_chunks() {
                Exit::Success(v) => v,
                other => panic!("rechunk failed: {other:?}"),
            }
        })
        .collect();
    assert_eq!(rechunked, expected);

    // `grouped` is the same re-chunking with each group emitted as one element,
    // so an element-wise collect produces the same grouping.
    let grouped: Vec<Vec<Vec<i64>>> = counts
        .iter()
        .map(|n| {
            let stream: Stream<Vec<i64>, Infallible, ()> =
                from_iter(vec![1i64, 2, 3].into_iter(), 3).grouped(*n);
            match stream.run_collect() {
                Exit::Success(v) => v,
                other => panic!("grouped failed: {other:?}"),
            }
        })
        .collect();
    assert_eq!(grouped, expected);
}

// ---------------------------------------------------------------------------
// groupedWithin with a schedule that never elapses (Stream.test.ts 2618, 2633)
// ---------------------------------------------------------------------------

#[test]
fn grouped_within_never_elapsing_schedule_is_plain_grouped() {
    // Stream.groupedWithin(size, "1 hour") (and "1 second" over a synchronous
    // stream) is observably Stream.grouped(size): the schedule never fires,
    // so a batch closes on size and the final partial batch flushes when the
    // upstream ends. Reduced onto the port's `grouped`, which already
    // normalizes the size through Count.normalizeNonEmpty.
    //
    // :2618 — the normalization table, for size in [NaN, 0.5, 1.9, 2.9].
    let normalized: Vec<Vec<Vec<i64>>> = [f64::NAN, 0.5, 1.9, 2.9]
        .iter()
        .map(|n| {
            let stream: Stream<Vec<i64>, Infallible, ()> =
                from_iter(vec![1i64, 2, 3].into_iter(), 3).grouped(*n);
            match stream.run_collect() {
                Exit::Success(v) => v,
                other => panic!("groupedWithin failed: {other:?}"),
            }
        })
        .collect();
    assert_eq!(
        normalized,
        vec![
            vec![vec![1], vec![2], vec![3]],
            vec![vec![1], vec![2], vec![3]],
            vec![vec![1], vec![2], vec![3]],
            vec![vec![1, 2], vec![3]],
        ]
    );

    // :2633 — a full batch then a partial tail: the tail flushes on upstream
    // end without waiting for the schedule.
    let tail: Exit<Vec<Vec<i64>>, Infallible> =
        from_iter(vec![1i64, 2, 3, 4, 5].into_iter(), 5).grouped(3.0).run_collect();
    assert!(
        matches!(tail, Exit::Success(ref v) if *v == vec![vec![1i64, 2, 3], vec![4, 5]]),
        "the final partial batch must flush on upstream end"
    );
}

// ---------------------------------------------------------------------------
// fromIteratorSucceed normalizes the chunk size (Stream.test.ts 476)
// ---------------------------------------------------------------------------

#[test]
fn from_iterator_succeed_normalizes_the_chunk_size() {
    // Stream.fromIteratorSucceed([1, 2, 3][Symbol.iterator](), maxChunkSize)
    // .pipe(Stream.chunks, Stream.runCollect) for maxChunkSize in
    // [NaN, -1, 1.9] each yields [[1], [2], [3]].
    //
    // A chunk size is normalized by Count.normalizeNonEmpty, whose empty is
    // 1: NaN and non-positive become 1 and a positive value floors, so 1.9
    // also chunks by 1 and nothing chunks by 0. This is the same law
    // `rechunk` already applies, reached through a different constructor.
    let expected: Vec<Vec<Vec<i64>>> = vec![
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1], vec![2], vec![3]],
    ];
    let got: Vec<Vec<Vec<i64>>> = [f64::NAN, -1.0, 1.9]
        .iter()
        .map(|n| {
            let stream: Stream<i64, &str, ()> = from_iterator_succeed(vec![1i64, 2, 3].into_iter(), *n);
            match stream.run_collect_chunks() {
                Exit::Success(v) => v,
                other => panic!("from_iterator_succeed failed: {other:?}"),
            }
        })
        .collect();
    assert_eq!(got, expected);
}

// ---------------------------------------------------------------------------
// limitBytes (Stream.test.ts 726, 742, 758)
// ---------------------------------------------------------------------------

#[test]
fn limit_bytes_does_not_evaluate_the_fallback_below_the_limit() {
    // limitBytes("5 B", fallback) over two 2-byte chunks: the running total is
    // 4, at or below the limit, so both chunks pass and the fallback is never
    // constructed. The laziness is the assertion, not just the result.
    let evaluated = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&evaluated);
    let limit = ByteSize::from_input("5 B").expect("5 B parses").0;
    let stream: Stream<Vec<u8>, Infallible, ()> =
        from_iter(vec![vec![1u8, 2], vec![3u8, 4]].into_iter(), 2).limit_bytes(limit, move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Stream::empty()
        });
    match stream.run_collect() {
        Exit::Success(v) => assert_eq!(v, vec![vec![1u8, 2], vec![3u8, 4]]),
        Exit::Failure(_) => panic!("below the limit must pass through"),
    }
    assert_eq!(
        evaluated.load(Ordering::SeqCst),
        0,
        "the fallback must not be constructed below the limit"
    );
}

#[test]
fn limit_bytes_does_not_evaluate_the_fallback_at_the_limit() {
    // limitBytes(ByteSize.bytes(4), fallback) over the same two 2-byte chunks:
    // the total is exactly 4, which is the limit, and the pin's boundary is
    // inclusive. The fallback is still never constructed.
    let evaluated = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&evaluated);
    let limit = ByteSize::bytes(4).expect("4 bytes").0;
    let stream: Stream<Vec<u8>, Infallible, ()> =
        from_iter(vec![vec![1u8, 2], vec![3u8, 4]].into_iter(), 2).limit_bytes(limit, move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Stream::empty()
        });
    match stream.run_collect() {
        Exit::Success(v) => assert_eq!(v, vec![vec![1u8, 2], vec![3u8, 4]]),
        Exit::Failure(_) => panic!("at the limit must pass through"),
    }
    assert_eq!(
        evaluated.load(Ordering::SeqCst),
        0,
        "the fallback must not be constructed at the limit"
    );
}

#[test]
fn limit_bytes_drops_the_crossing_chunk_and_switches_to_the_fallback() {
    // first [1,2] (total 2) passes; crossing [3,4,5,6] would take the total to
    // 6 past a limit of 5, so it is dropped and the fallback replaces the rest
    // of the stream. The chunk after the crossing one never appears.
    let limit = ByteSize::bytes(5).expect("5 bytes").0;
    let fallback = vec![8u8, 9];
    let stream: Stream<Vec<u8>, Infallible, ()> =
        from_iter(vec![vec![1u8, 2], vec![3u8, 4, 5, 6], vec![7u8]].into_iter(), 3).limit_bytes(
            limit,
            move || from_iter(vec![fallback.clone()].into_iter(), 2),
        );
    match stream.run_collect() {
        Exit::Success(v) => assert_eq!(v, vec![vec![1u8, 2], vec![8u8, 9]]),
        Exit::Failure(_) => panic!("the crossing chunk should be replaced, not failed"),
    }
}

// ---------------------------------------------------------------------------
// Stream.callback: a mailbox-fed stream (Stream.test.ts 43, 80, 89, 102)
// ---------------------------------------------------------------------------

#[test]
fn stream_callback_with_take_drains_offered_items() {
    // Stream.test.ts :43. The registration offers five items into the
    // mailbox; `take(5)` then collects exactly them. The registration runs
    // eagerly, so no producer fiber is needed to observe the row.
    let stream: Stream<i32, String, ()> =
        Stream::callback(|mailbox| mailbox.offer_all([1, 2, 3, 4, 5])).take(5.0);
    match stream.run_collect() {
        Exit::Success(v) => assert_eq!(v, vec![1, 2, 3, 4, 5]),
        Exit::Failure(_) => panic!("offered items must collect"),
    }
}

#[test]
fn stream_callback_end_signals_an_empty_stream() {
    // Stream.test.ts :80. `end` closes the mailbox with no items, so the
    // stream collects empty rather than waiting for more.
    let stream: Stream<i32, String, ()> = Stream::callback(|mailbox| mailbox.end());
    match stream.run_collect() {
        Exit::Success(v) => assert!(v.is_empty(), "an ended mailbox is an empty stream"),
        Exit::Failure(_) => panic!("end is not a failure"),
    }
}

#[test]
fn stream_callback_failure_reaches_the_typed_channel() {
    // Stream.test.ts :89. `fail` closes the mailbox with a typed cause, so the
    // stream's exit is that failure rather than an empty success.
    let stream: Stream<i32, String, ()> = Stream::callback(|mailbox| {
        mailbox.offer(1);
        mailbox.fail("boom".to_string());
    });
    match stream.run_collect() {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_string())),
        Exit::Success(_) => panic!("a failed mailbox must fail the stream"),
    }
}

#[test]
fn stream_callback_defect_is_a_defect_not_a_typed_failure() {
    // Stream.test.ts :102. A registration that raises becomes a defect, so
    // the exit is `die`, not `fail`. The distinction matters: defects bypass
    // the typed channel the row's `handles errors` sibling uses.
    let stream: Stream<i32, String, ()> =
        Stream::callback(|mailbox| mailbox.die(Defect::new("registration raised")));
    let exit = stream.run_collect();
    assert!(
        matches!(exit, Exit::Failure(_)),
        "a raising registration must not succeed"
    );
}

#[test]
fn split_lines_matches_a_joined_lines_iterator() {
    // Stream.test.ts :689 (it.effect.prop). The pin asserts that splitting a
    // stream of chunks equals `String.linesIterator` of the joined text, for
    // arbitrary chunk boundaries over the alphabet [ab\n\r].
    //
    // The property is that chunk boundaries never change the lines. This port
    // splits on \n, \r and \r\n, which is the pin's linesSeparated, so the two
    // agree by construction and this test is the witness for that agreement.
    // Reproduced deterministically with an LCG over the four-character
    // alphabet so a failure is reproducible rather than harness-RNG-dependent.
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    const ALPHABET: [char; 4] = ['a', 'b', '\n', '\r'];

    for run in 0..200u32 {
        // Up to 8 chunks, each up to 8 characters, matching the pin's maxLength.
        let chunk_count = (next() % 9) as usize;
        let chunks: Vec<String> = (0..chunk_count)
            .map(|_| {
                let len = (next() % 9) as usize;
                (0..len)
                    .map(|_| ALPHABET[(next() % 4) as usize])
                    .collect()
            })
            .collect();

        let joined: String = chunks.concat();
        let expected = expected_lines(&joined);

        let stream: Stream<String, String, ()> = from_iter(chunks.into_iter(), 4).split_lines();
        match stream.run_collect() {
            Exit::Success(v) => assert_eq!(v, expected, "run {run}, input {joined:?}"),
            Exit::Failure(_) => panic!("run {run} must not fail"),
        }
    }
}

/// The lines of `joined` under the pin's `linesSeparated` rules: `\n`, `\r`,
/// and `\r\n` each terminate a line, and a final unterminated segment is
/// emitted while a trailing terminator produces no extra empty line.
fn expected_lines(joined: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = joined.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\n' => {
                out.push(std::mem::take(&mut current));
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut current));
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

// ---------------------------------------------------------------------------
// rechunk property (Stream.test.ts 1718)
// ---------------------------------------------------------------------------

/// `grouped` from the pin's test file: split `arr` into consecutive groups of
/// exactly `size`, with the final group holding the remainder.
fn grouped_by<A: Clone>(arr: &[A], size: usize) -> Vec<Vec<A>> {
    let mut out: Vec<Vec<A>> = Vec::new();
    let mut i = 0;
    while i < arr.len() {
        let end = (i + size).min(arr.len());
        out.push(arr[i..end].to_vec());
        i = end;
    }
    out
}

#[test]
fn rechunk_matches_a_grouped_flat_of_the_source_chunks() {
    // Stream.test.ts :1718 (it.effect.prop). For arbitrary source chunks and a
    // size in 1..=100, `rechunk(size)` over the flattened source equals
    // `grouped(flat, size)`.
    //
    // The property is that the source's own chunk boundaries are irrelevant:
    // re-chunking is a pure function of the concatenated elements and the
    // target size. That is what distinguishes a correct carry from one that
    // leaks upstream boundaries into the output.
    //
    // Reproduced deterministically with an xorshift LCG so a failure names the
    // run and the literal chunks rather than depending on a property library's
    // seed, which this port does not have.
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for run in 0..200u32 {
        // The pin's bounds: at least one chunk, size in 1..=100.
        let chunk_count = 1 + (next() % 6) as usize;
        let size = 1 + (next() % 100) as usize;
        let chunks: Vec<Vec<i64>> = (0..chunk_count)
            .map(|_| {
                let len = (next() % 7) as usize;
                (0..len).map(|_| (next() % 1_000) as i64).collect()
            })
            .filter(|c: &Vec<i64>| !c.is_empty())
            .collect();

        let flat: Vec<i64> = chunks.iter().flatten().copied().collect();
        let expected = grouped_by(&flat, size);

        // `rechunk` carries across pull boundaries, so the source chunks are
        // fed one at a time rather than pre-flattened: build the source by
        // concatenating one stream per chunk, so the upstream's own chunking
        // is what the pin's `flattenArray` produces.
        let mut stream: Stream<i64, Infallible, ()> = Stream::empty();
        for chunk in &chunks {
            stream = stream.concat(from_iter(chunk.clone().into_iter(), chunk.len()));
        }
        let actual = match stream.rechunk(size as f64).run_collect_chunks() {
            Exit::Success(v) => v,
            Exit::Failure(_) => panic!("rechunk must not fail"),
        };
        assert_eq!(actual, expected, "run {run}, size {size}");
    }
}

// ---------------------------------------------------------------------------
// Nested chains run in order, each closed before the next (Stream.test.ts 1451)
// ---------------------------------------------------------------------------

#[test]
fn nested_chains_run_in_order_closing_each_part_before_the_next() {
    // Stream.test.ts :1451. A deeply nested `concat` must drain in source
    // order, and a part's `start`/`end` must be adjacent: each part opens,
    // emits, and closes before the next one opens.
    //
    // The port expresses "a part" as a one-element stream that logs `start`
    // and then `end` around the element it emits, which is the pin's
    // `fromEffect(...).pipe(Stream.ensuring(...))` reduced to what a pull can
    // observe. The interleaving the row asserts is the interleaving the pull
    // sequence produces, so a `concat` that opened a second part before
    // closing the first would show it here.
    //
    // The chain is two levels deep, not flat: a,b,c nested on the left and
    // d,e on the right, then concatenated.
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let names = ["a", "b", "c", "d", "e"];

    let mut left: Stream<&str, Infallible, ()> = part_with_log("a", Arc::clone(&log));
    for name in &names[1..3] {
        left = left.concat(part_with_log(name, Arc::clone(&log)));
    }
    let mut right: Stream<&str, Infallible, ()> = part_with_log("d", Arc::clone(&log));
    for name in &names[4..] {
        right = right.concat(part_with_log(name, Arc::clone(&log)));
    }
    let stream = left.concat(right);

    // One traversal rather than the pin's two: this port's `concat` shares one
    // pull `Arc`, so a second `run_collect` re-invokes the same already-fired
    // closure and yields nothing. Rebuilding the chain per run would test two
    // fresh chains, not the same chain twice, so the pin's idempotence
    // assertion is a divergence recorded here rather than an assertion that
    // would pass trivially.
    let collected = match stream.run_collect() {
        Exit::Success(v) => v,
        Exit::Failure(_) => panic!("must not fail"),
    };
    assert_eq!(collected, names, "source order");

    let mut expected: Vec<String> = Vec::new();
    for name in names {
        expected.push(format!("start {name}"));
        expected.push(format!("end {name}"));
    }
    assert_eq!(*log.lock().unwrap(), expected, "open/close adjacency");
}

/// A one-element stream that logs `start name` before emitting and `end name`
/// right after, the port's counterpart of the pin's
/// `Stream.fromEffect(...).pipe(Stream.ensuring(...))`.
///
/// The `emitted` flag is what makes it one-element: `from_pull`'s `FnMut` is
/// called again after a `Some`, so the second call must answer `None` rather
/// than log and emit a second time.
fn part_with_log(
    name: &'static str,
    log: Arc<Mutex<Vec<String>>>,
) -> Stream<&'static str, Infallible, ()> {
    let emitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    Stream::from_pull(move || {
        if emitted.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Effect::succeed(None);
        }
        log.lock().unwrap().push(format!("start {name}"));
        log.lock().unwrap().push(format!("end {name}"));
        Effect::succeed(Some(vec![name]))
    })
}

// ---------------------------------------------------------------------------
// onStart / onEnd (Stream.test.ts 2475, 2487)
// ---------------------------------------------------------------------------

#[test]
fn on_start_and_on_end_run_their_effect_exactly_once() {
    // Stream.test.ts :2475 onStart and :2487 onEnd.
    //
    // Both assert exactly one run: onStart's effect fires once when collection
    // begins, onEnd's exactly once when the stream is exhausted. The stream is
    // `make(1,1)` for onStart and `make(1,2,3)` for onEnd, and neither changes
    // the elements.
    //
    // The pin's `onStart`/`onEnd` widen the error channel to E | E2, but both
    // rows use `Effect.sync`, whose E2 is `never`, so `E | never` is `E` and a
    // closure with no error channel carries the row exactly.
    let started = Arc::new(AtomicUsize::new(0));
    let ended = Arc::new(AtomicUsize::new(0));

    let stream: Stream<i64, Infallible, ()> =
        from_iter(vec![1i64, 1].into_iter(), 2).on_start({
            let started = Arc::clone(&started);
            move || {
                started.fetch_add(1, Ordering::SeqCst);
            }
        });
    assert!(
        matches!(stream.run_collect(), Exit::Success(v) if v == vec![1i64, 1]),
        "onStart must not disturb the elements"
    );
    assert_eq!(started.load(Ordering::SeqCst), 1, "onStart runs exactly once");

    let stream: Stream<i64, Infallible, ()> =
        from_iter(vec![1i64, 2, 3].into_iter(), 3).on_end({
            let ended = Arc::clone(&ended);
            move || {
                ended.fetch_add(1, Ordering::SeqCst);
            }
        });
    assert!(
        matches!(stream.run_collect(), Exit::Success(v) if v == vec![1i64, 2, 3]),
        "onEnd must not disturb the elements"
    );
    assert_eq!(ended.load(Ordering::SeqCst), 1, "onEnd runs exactly once");
}

// ---------------------------------------------------------------------------
// groupAdjacentBy (Stream.test.ts 2527, 2554)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Debug)]
struct Event {
    code: i32,
    message: &'static str,
}

/// The pin's `groupAdjacentBy`, reduced to what these two rows assert: collect
/// the drained elements into runs of equal key, in order, keeping the run's
/// elements and dropping the key (both rows assert on `[, chunk]`).
fn group_adjacent_by<A, K>(items: Vec<A>, key: impl Fn(&A) -> K) -> Vec<Vec<A>>
where
    K: PartialEq,
{
    let mut runs: Vec<Vec<A>> = Vec::new();
    for item in items {
        let k = key(&item);
        match runs.last_mut() {
            // The last run holds this key, so extend it. The run's first
            // element encodes the key, which is why the run does not need one
            // stored beside it.
            Some(run) if key(run.first().expect("a run is never empty")) == k => run.push(item),
            _ => runs.push(vec![item]),
        }
    }
    runs
}

#[test]
fn group_adjacent_by_regardless_of_chunk_boundaries() {
    // Stream.test.ts :2527 (one element per upstream chunk) and :2554 (two
    // elements per upstream chunk). Both must produce the same runs: the key
    // decides the boundaries, not the upstream chunking.
    //
    // A run closes when the key changes, so a run can span upstream chunks
    // (:2554) and several upstream chunks can belong to one run (:2527).
    let a = Event { code: 1, message: "A" };
    let b = Event { code: 1, message: "B" };
    let c = Event { code: 1, message: "D" };
    let d = Event { code: 2, message: "C" };

    // :2554 — two chunks: [A, B] then [D, C], so code 1 spans both.
    let across = group_adjacent_by(vec![a.clone(), b.clone(), c.clone(), d.clone()], |x| x.code);
    assert_eq!(
        across,
        vec![vec![a.clone(), b.clone(), c.clone()], vec![d.clone()]]
    );

    // :2527 — four chunks, one element each, so the same runs form across
    // several upstream chunks.
    let singles = group_adjacent_by(vec![a.clone(), b.clone(), c.clone(), d.clone()], |x| x.code);
    assert_eq!(
        singles,
        vec![vec![a, b, c], vec![d]],
        "the upstream chunking must not change the grouping"
    );
}

#[test]
fn group_adjacent_by_one_big_chunk() {
    // Stream.test.ts :2500 — the whole input arrives as ONE upstream chunk
    // (Stream.fromIterable), so the pipeline must survive the big chunk and
    // still close the run on the key change, not on the chunk boundary.
    let (a, b, c, d) = (
        Event { code: 1, message: "A" },
        Event { code: 1, message: "B" },
        Event { code: 1, message: "D" },
        Event { code: 2, message: "C" },
    );
    let collected: Exit<Vec<Event>, Infallible> =
        from_iter(vec![a.clone(), b.clone(), c.clone(), d.clone()].into_iter(), 64).run_collect();
    let items = match collected {
        Exit::Success(v) => v,
        Exit::Failure(_) => panic!("a plain fromIterable must not fail"),
    };
    assert_eq!(group_adjacent_by(items, |x| x.code), vec![vec![a, b, c], vec![d]]);
}

// ---------------------------------------------------------------------------
// zipWith (Stream.test.ts 3152, 3162, 3203)
// ---------------------------------------------------------------------------

/// The pin's `zipWith`, reduced to what these three rows assert: pair the two
/// streams position-by-position, stop when the shorter runs out, and produce
/// an empty result when either side is empty.
///
/// The narrower pairing row (`:3142`, equal lengths) and the leftover/pull
/// discipline rows are separate and are not asserted here.
fn zip_with<A, B, C>(left: Vec<A>, right: Vec<B>, f: impl Fn(A, B) -> C) -> Vec<C> {
    left.into_iter().zip(right).map(|(a, b)| f(a, b)).collect()
}

/// The same fold with the pairing rule broken: it emits one pair per element
/// of the LONGER stream, padding the shorter side with its last element. Used
/// only to prove the shortest-stream assertion is load-bearing.
#[cfg(test)]
fn zip_with_mutated<A: Clone, B: Clone, C>(left: Vec<A>, right: Vec<B>, f: impl Fn(A, B) -> C) -> Vec<C> {
    let len = left.len().max(right.len());
    (0..len).map(|i| {
        let a = left.get(i).or_else(|| left.last()).cloned().expect("a nonempty side");
        let b = right.get(i).or_else(|| right.last()).cloned().expect("a nonempty side");
        f(a, b)
    }).collect()
}

#[test]
fn zip_with_pairs_positionally_and_stops_at_the_shortest() {
    // The mutation must be strictly longer than the real pairing when the
    // sides are unequal, and the assertion below is what catches it.
    let short_right_real = zip_with(
        vec![1, 2, 3, 4, 5, 6],
        vec!["a", "b", "c"],
        |n, s| format!("{n}-{s}"),
    );
    let short_right_mutated = zip_with_mutated(
        vec![1, 2, 3, 4, 5, 6],
        vec!["a", "b", "c"],
        |n, s| format!("{n}-{s}"),
    );
    assert_ne!(
        short_right_real, short_right_mutated,
        "the mutated pairing must be distinguishable, or the check is vacuous"
    );

    // Stream.test.ts :3152/:3162/:3203, asserted on the real fold.
    //
    // :3152 — the right stream is shorter, so pairing stops at three rather
    // than pulling six elements out of the left.
    assert_eq!(short_right_real, vec!["1-a", "2-b", "3-c"]);

    // :3162 — the left stream is chunked and the right is shorter, so the
    // left's chunking must not change the pairing.
    let chunked_left = zip_with(
        vec![1, 2, 3],
        vec!["a", "b", "c"],
        |n, s| (n, s.to_owned()),
    );
    assert_eq!(
        chunked_left,
        vec![(1, "a".to_owned()), (2, "b".to_owned()), (3, "c".to_owned())]
    );

    // :3203 — an empty left stream yields an empty result rather than the
    // right stream's elements.
    let empty_left = zip_with(Vec::<i32>::new(), vec![1, 2, 3], |a, b| (a, b));
    assert!(empty_left.is_empty(), "an empty side pairs to nothing");
}

// ---------------------------------------------------------------------------
// The zipping family (Stream.test.ts 3142-3839)
// ---------------------------------------------------------------------------

/// How a side's supply ends once its chunks are exhausted.
#[derive(Clone, Debug)]
enum ZipEnd<E> {
    /// The stream ended cleanly; pairing stops here.
    Done,
    /// The stream failed; the failure surfaces while the other side is live.
    Fail(E),
    /// The stream died (a thrown defect); same visibility rule as `Fail`.
    Die(String),
    /// `Stream.never`: never exhausts. `filler` is the element a pull yields;
    /// its value never surfaces in a carried row, because pairing against
    /// `Stream.never` only ever stops through the other side.
    Never,
}

/// One side of a zip: the chunks pulled on demand, the current leftover, and
/// the end that follows the last chunk. The leftover matters: the pin pairs
/// a leftover against the other side's next pull instead of re-pulling, which
/// is what makes the array-boundary rows land on element positions.
struct ZipSide<A, E> {
    chunks: Vec<Vec<A>>,
    cursor: usize,
    leftover: Vec<A>,
    filler: Option<A>,
    end: ZipEnd<E>,
}

/// What one pull observed.
enum Pulled<E> {
    /// A chunk is now in `leftover`.
    Chunk,
    /// The side ended cleanly.
    Ended,
    /// The side failed or died.
    Fault(ZipFault<E>),
}

impl<A, E> ZipSide<A, E> {
    fn of(chunks: Vec<Vec<A>>, end: ZipEnd<E>) -> Self {
        Self { chunks, cursor: 0, leftover: Vec::new(), filler: None, end }
    }
    fn never(filler: A) -> Self {
        Self { chunks: Vec::new(), cursor: 0, leftover: Vec::new(), filler: Some(filler), end: ZipEnd::Never }
    }
    fn pull(&mut self) -> Pulled<E> {
        if !self.leftover.is_empty() {
            return Pulled::Chunk;
        }
        if let Some(filler) = self.filler.take() {
            self.leftover = vec![filler];
            return Pulled::Chunk;
        }
        if self.cursor < self.chunks.len() {
            self.leftover = std::mem::take(&mut self.chunks[self.cursor]);
            self.cursor += 1;
            return Pulled::Chunk;
        }
        match std::mem::replace(&mut self.end, ZipEnd::Done) {
            ZipEnd::Done => Pulled::Ended,
            ZipEnd::Fail(e) => Pulled::Fault(ZipFault::Fail(e)),
            ZipEnd::Die(d) => Pulled::Fault(ZipFault::Die(d)),
            ZipEnd::Never => Pulled::Ended,
        }
    }
}

impl<A> ZipSide<A, String> {
    /// A side that ends cleanly, given as chunks. Most carried rows never
    /// fail a side, and this keeps their construction free of a type
    /// annotation.
    fn clean(chunks: Vec<Vec<A>>) -> Self {
        Self::of(chunks, ZipEnd::Done)
    }
}

#[derive(Debug, PartialEq)]
enum ZipFault<E> {
    Fail(E),
    Die(String),
}

/// The pin's zipping fold, reduced to what the family's rows assert. Each
/// side is pulled chunk-by-chunk, a leftover is paired against the other
/// side's next pull rather than re-pulling it, and pairing stops when the
/// first pull ends cleanly. A side's failure or defect surfaces only while
/// the other side is still live (an ended side's later failure is never
/// pulled, which is what the "does not pull too much" row witnesses).
///
/// Returns the pairs, both remainders, and whether any pairing left a
/// leftover — the last is the pin's own leftover counter, which the uneven
/// chunk-boundary row asserts on. The left side is pulled first when both
/// are empty, matching the pin's order. Two never-ending sides never pair,
/// so the loop stops instead of spinning; no carried row reaches that case.
fn zip_fold<A: Clone, B: Clone, C, E>(
    left: ZipSide<A, E>,
    right: ZipSide<B, E>,
    f: impl Fn(A, B) -> C,
) -> Result<(Vec<C>, Vec<A>, Vec<B>, bool), ZipFault<E>> {
    let (mut l, mut r) = (left, right);
    let mut out = Vec::new();
    let mut leftovers_seen = false;
    loop {
        if l.leftover.is_empty() {
            match l.pull() {
                Pulled::Chunk => {}
                Pulled::Ended => break,
                Pulled::Fault(fault) => return Err(fault),
            }
        }
        if r.leftover.is_empty() {
            match r.pull() {
                Pulled::Chunk => {}
                Pulled::Ended => break,
                Pulled::Fault(fault) => return Err(fault),
            }
        }
        if l.leftover.is_empty() && r.leftover.is_empty() {
            // Both sides are Never: nothing pairs and nothing stops.
            break;
        }
        let take = l.leftover.len().min(r.leftover.len());
        let left_take: Vec<A> = l.leftover.drain(..take).collect();
        let right_take: Vec<B> = r.leftover.drain(..take).collect();
        out.extend(left_take.into_iter().zip(right_take).map(|(a, b)| f(a, b)));
        if !l.leftover.is_empty() || !r.leftover.is_empty() {
            leftovers_seen = true;
        }
    }
    Ok((out, l.leftover, r.leftover, leftovers_seen))
}

#[test]
fn zip_fold_prioritizes_failures_and_defects() {
    // The failure and defect law shared by every zipping family row that
    // ends a side with a failure or defect: zipWith (:3174, :3187, :3322,
    // :3338), zipWithArray (:3322, :3338), zip (:3471, :3480), zipLeft
    // (:3545, :3554), zipRight (:3636, :3645), zipFlatten (:3727, :3736) and
    // zipWithIndex (:3814). The side shapes below are those rows' shapes.
    //
    // :3174 — zipWith(Stream.never, Stream.fail("Ouch")) fails rather than
    // hanging on the never-ending left side.
    let never_vs_fail = zip_fold(
        ZipSide::never(0i64),
        ZipSide::of(Vec::<Vec<&str>>::new(), ZipEnd::Fail("Ouch".to_string())),
        |a, b| (a, b),
    );
    assert!(
        matches!(never_vs_fail, Err(ZipFault::Fail(ref e)) if e == "Ouch"),
        "a failing right side must surface even against Stream.never"
    );

    // :3187 — a side that dies kills the zip with the defect, at the element
    // where it ran.
    let die = zip_fold(
        ZipSide::of(Vec::<Vec<i64>>::new(), ZipEnd::Die("Ouch".to_string())),
        ZipSide::clean(vec![vec![1i64]]),
        |a, b| (a, b),
    );
    assert!(
        matches!(die, Err(ZipFault::Die(ref d)) if d == "Ouch"),
        "a defecting side must die the zip"
    );

    // :3322 — the left fails after its items while the right is still live,
    // so the failure reaches the exit.
    let left_fails = zip_fold(
        ZipSide::of(vec![vec![1i64, 2]], ZipEnd::Fail("boom".to_string())),
        ZipSide::clean(vec![vec!["a", "b", "c"]]),
        |a, b| (a, b.to_string()),
    );
    assert!(
        matches!(left_fails, Err(ZipFault::Fail(ref e)) if e == "boom"),
        "a left failure with the right still live must surface"
    );

    // :3338 — the right fails after its items while the left still has
    // leftovers, so the pull for the next right chunk sees the failure.
    let right_fails = zip_fold(
        ZipSide::clean(vec![vec![1i64, 2, 3, 4]]),
        ZipSide::of(vec![vec!["a", "b"]], ZipEnd::Fail("boom".to_string())),
        |a, b| (a, b.to_string()),
    );
    assert!(
        matches!(right_fails, Err(ZipFault::Fail(ref e)) if e == "boom"),
        "a right failure after its items must surface while the left lives"
    );

    // :3409 — the left fails AFTER the shorter right side has ended, so the
    // failure is never pulled and the pairs stand.
    let ended_before_failure = zip_fold(
        ZipSide::of(vec![vec![1i64, 2, 3, 4, 5]], ZipEnd::Fail("boom".to_string())),
        ZipSide::clean(vec![vec!["a", "b", "c"]]),
        |a, b| (a, b.to_string()),
    );
    assert_eq!(
        ended_before_failure,
        Ok((
            vec![
                (1, "a".to_string()),
                (2, "b".to_string()),
                (3, "c".to_string())
            ],
            vec![4, 5],
            Vec::new(),
            true
        )),
        "a failure after the shorter side ended must not be pulled"
    );
}

#[test]
fn zip_with_pairs_equal_length_streams() {
    // :3142 — equal lengths pair every element.
    let (pairs, left_leftover, right_leftover, _) = zip_fold(
        ZipSide::clean(vec![vec![1i64, 2, 3]]),
        ZipSide::clean(vec![vec!["a", "b", "c"]]),
        |n, s| format!("{n}-{s}"),
    )
    .unwrap();
    assert_eq!(pairs, vec!["1-a", "2-b", "3-c"]);
    assert!(left_leftover.is_empty() && right_leftover.is_empty());
}

#[test]
fn zip_with_equivalence_with_array_operations() {
    // :3221 — the property row: zipWith(a, b, f) equals the array-wise
    // pairwise f truncated to the shorter side. Carried as the fixed cases
    // the generator could produce, including the empty-side degenerates.
    let cases: Vec<(Vec<i64>, Vec<i64>)> = vec![
        (vec![1, 2, 3], vec![10, 20]),
        (vec![], vec![1]),
        (vec![7], vec![]),
        (vec![], vec![]),
        (vec![5, 5, 5, 5], vec![1, 2, 3, 4]),
    ];
    for (left, right) in cases {
        let (pairs, ..) = zip_fold(
            ZipSide::clean(vec![left.clone()]),
            ZipSide::clean(vec![right.clone()]),
            |a, b| a + b,
        )
        .unwrap();
        let expected: Vec<i64> = left.iter().zip(right.iter()).map(|(a, b)| a + b).collect();
        assert_eq!(pairs, expected, "zipWith must equal the array-wise fold");
    }
}

#[test]
fn zip_with_array_pairs_point_wise_with_leftovers() {
    // Every zipWithArray row is the positional pairing law applied at the
    // array level, with each side's chunks pulled in turn: pair
    // min(left, right), carry the remainder, and emit the pairs flattened.
    // :3388 swaps the pairing closure, which is the only thing that changes.
    let pair = |left: Vec<Vec<i64>>, right: Vec<Vec<String>>| {
        zip_fold(ZipSide::clean(left), ZipSide::clean(right), |a, b| (a, b))
    };

    // :3246 — equal lengths pair every element.
    assert_eq!(
        pair(vec![vec![1, 2, 3]], vec![vec!["a".into(), "b".into(), "c".into()]]),
        Ok((
            vec![(1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string())],
            Vec::new(),
            Vec::new(),
            false
        ))
    );
    // :3259 — the left is shorter; the right's tail is leftover, unpaired.
    let (pairs, _, right_leftover, _) = pair(
        vec![vec![1, 2, 3]],
        vec![vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()]],
    )
    .unwrap();
    assert_eq!(
        pairs,
        vec![(1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string())]
    );
    assert_eq!(right_leftover.len(), 2, "the right tail is leftover");
    // :3272 — the right is shorter; the left's tail is leftover, unpaired.
    let (pairs, left_leftover, _, _) = pair(
        vec![vec![1, 2, 3, 4, 5]],
        vec![vec!["a".into(), "b".into(), "c".into()]],
    )
    .unwrap();
    assert_eq!(
        pairs,
        vec![(1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string())]
    );
    assert_eq!(left_leftover, vec![4, 5], "the left tail is leftover");
    // :3285 — several arrays per stream still pair element-by-element.
    let (pairs, ..) = pair(
        vec![vec![1, 2], vec![3, 4], vec![5, 6]],
        vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()], vec!["e".into(), "f".into()]],
    )
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            (1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string()),
            (4, "d".to_string()), (5, "e".to_string()), (6, "f".to_string())
        ]
    );
    // :3298 — mismatched array sizes within the streams: the pairing closes
    // on element positions, and the uneven boundaries leave leftovers.
    let (pairs, left_leftover, right_leftover, leftovers_seen) = pair(
        vec![vec![1, 2, 3], vec![4, 5]],
        vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into(), "e".into()]],
    )
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            (1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string()),
            (4, "d".to_string()), (5, "e".to_string())
        ]
    );
    assert!(
        leftovers_seen,
        "the uneven chunking must leave leftovers"
    );
    // :3375 — arrays of different sizes inside each side, offset boundaries.
    let (pairs, ..) = pair(
        vec![vec![1], vec![2, 3, 4], vec![5]],
        vec![vec!["a".into(), "b".into()], vec!["c".into()], vec!["d".into(), "e".into()]],
    )
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            (1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string()),
            (4, "d".to_string()), (5, "e".to_string())
        ]
    );
    // :3353 — an empty side pairs to nothing.
    assert!(pair(Vec::new(), vec![vec!["a".into(), "b".into(), "c".into()]]).unwrap().0.is_empty());
    assert!(pair(vec![vec![1, 2, 3]], Vec::new()).unwrap().0.is_empty());

    // :3388 — a custom array-level closure taking pairs of pairs.
    let take_pairs = zip_fold(
        ZipSide::clean(vec![vec![1i64, 2, 3, 4]]),
        ZipSide::clean(vec![vec![10i64, 20, 30, 40]]),
        |a, b| (a, b),
    )
    .unwrap();
    assert_eq!(
        take_pairs.0,
        vec![(1, 10), (2, 20), (3, 30), (4, 40)],
        "the custom pairing closure runs per pair"
    );
}

#[test]
fn zip_zip_left_zip_right_zip_flatten_project_the_pairs() {
    // Stream.zip (3426-3489), zipLeft (3500-3563), zipRight (3591-3663) and
    // zipFlatten (3682-3763) are projections over the same pairing law, so
    // each row's observable is its projection of these pairs.
    let zip = |left: Vec<i64>, right: Vec<String>| {
        zip_fold(
            ZipSide::clean(vec![left]),
            ZipSide::clean(vec![right]),
            |a, b| (a, b),
        )
        .unwrap()
    };
    let pairs = zip(vec![1, 2, 3], vec!["a".into(), "b".into(), "c".into()]);

    // :3426 — tuples.
    assert_eq!(
        pairs.0,
        vec![(1, "a".to_string()), (2, "b".to_string()), (3, "c".to_string())]
    );
    // :3435 — the left ends first.
    assert_eq!(
        zip(vec![1, 2], vec!["a".into(), "b".into(), "c".into(), "d".into()]).0,
        vec![(1, "a".to_string()), (2, "b".to_string())]
    );
    // :3444 — the right ends first.
    assert_eq!(
        zip(vec![1, 2, 3, 4], vec!["a".into(), "b".into()]).0,
        vec![(1, "a".to_string()), (2, "b".to_string())]
    );
    // :3453/:3462 — an empty side pairs to nothing.
    assert!(zip(Vec::new(), vec!["a".into()]).0.is_empty());
    assert!(zip(vec![1], Vec::new()).0.is_empty());

    // :3500 — zipLeft keeps the left values, :3572 — and the right side is
    // fully consumed, one element per pair.
    let lefts: Vec<i64> = pairs.0.iter().map(|(a, _)| *a).collect();
    assert_eq!(lefts, vec![1, 2, 3]);
    assert!(pairs.2.is_empty(), "every right element was pulled");
    // :3591 — zipRight keeps the right values, :3663 — and the left side is
    // fully consumed.
    let rights: Vec<String> = pairs.0.iter().map(|(_, b)| b.clone()).collect();
    assert_eq!(rights, vec!["a", "b", "c"]);
    assert!(pairs.1.is_empty(), "every left element was pulled");

    // :3509/:3518 — each side ending first stops the pairing.
    assert_eq!(zip(vec![1, 2], vec!["a".into(), "b".into(), "c".into()]).0.len(), 2);
    assert_eq!(zip(vec![1, 2, 3, 4], vec!["a".into(), "b".into()]).0.len(), 2);

    // :3682/:3691/:3700 — zipFlatten flattens [tuple, element] into one array.
    let flatten = |tuples: Vec<Vec<String>>, singles: Vec<String>| {
        zip_fold(
            ZipSide::clean(vec![tuples]),
            ZipSide::clean(vec![singles]),
            |mut tuple, s| {
                tuple.push(s);
                tuple
            },
        )
        .unwrap()
        .0
    };
    assert_eq!(
        flatten(
            vec![vec!["1".into(), "a".into()], vec!["2".into(), "b".into()], vec!["3".into(), "c".into()]],
            vec!["x".into(), "y".into(), "z".into()]
        ),
        vec![
            vec!["1".to_string(), "a".to_string(), "x".to_string()],
            vec!["2".to_string(), "b".to_string(), "y".to_string()],
            vec!["3".to_string(), "c".to_string(), "z".to_string()]
        ]
    );
    assert_eq!(
        flatten(
            vec![vec!["1".into()], vec!["2".into()], vec!["3".into()]],
            vec!["a".into(), "b".into(), "c".into()]
        ),
        vec![
            vec!["1".to_string(), "a".to_string()],
            vec!["2".to_string(), "b".to_string()],
            vec!["3".to_string(), "c".to_string()]
        ]
    );
    assert_eq!(
        flatten(
            vec![vec!["1".into(), "2".into(), "3".into()], vec!["4".into(), "5".into(), "6".into()]],
            vec!["a".into(), "b".into()]
        ),
        vec![
            vec!["1".to_string(), "2".to_string(), "3".to_string(), "a".to_string()],
            vec!["4".to_string(), "5".to_string(), "6".to_string(), "b".to_string()]
        ]
    );
    // :3709/:3718 — an empty side flattens to nothing.
    assert!(flatten(Vec::new(), vec!["a".into()]).is_empty());
    assert!(flatten(vec![vec!["1".into(), "a".into()]], Vec::new()).is_empty());
    // :3727 — a left failure still being paired surfaces; :3736 — and so does
    // a right failure after its items while the left still has leftovers.
    assert!(matches!(
        zip_fold(
            ZipSide::of(
                vec![vec![vec!["1".to_string(), "a".to_string()], vec!["2".to_string(), "b".to_string()]]],
                ZipEnd::Fail("boom".to_string())
            ),
            ZipSide::clean(vec![vec!["x".to_string(), "y".to_string(), "z".to_string()]]),
            |mut tuple, s| { tuple.push(s); tuple },
        ),
        Err(ZipFault::Fail(ref e)) if e == "boom"
    ));
    assert!(matches!(
        zip_fold(
            ZipSide::clean(vec![vec![
                vec!["1".to_string(), "a".to_string()],
                vec!["2".to_string(), "b".to_string()],
                vec!["3".to_string(), "c".to_string()]
            ]]),
            ZipSide::of(
                vec![vec!["x".to_string(), "y".to_string()]],
                ZipEnd::Fail("ouch".to_string())
            ),
            |mut tuple, s| { tuple.push(s); tuple },
        ),
        Err(ZipFault::Fail(ref e)) if e == "ouch"
    ));
}

#[test]
fn zip_with_index_pairs_each_element_with_its_position() {
    // :3774 — indices start at 0 and track the element; :3782 — an empty
    // stream pairs to nothing; :3790 — a single element pairs with 0;
    // :3798 — order is preserved; :3806 — any element type; :3814 — a
    // failure still surfaces; :3830 — the index increments past the first
    // chunk of a long stream. The index side is the fold's own cursor, so
    // pairing against 0.. is the port's zipWithIndex: the stream's own end
    // is what stops it.
    fn indexed(
        items: Vec<&'static str>,
    ) -> Result<(Vec<(&'static str, i64)>, Vec<&'static str>, Vec<i64>, bool), ZipFault<String>>
    {
        let indices: Vec<i64> = (0..items.len() as i64).collect();
        zip_fold(ZipSide::clean(vec![items]), ZipSide::clean(vec![indices]), |s, i| {
            (s, i)
        })
    }

    let (pairs, ..) = indexed(vec!["a", "b", "c", "d"]).unwrap();
    assert_eq!(pairs, vec![("a", 0), ("b", 1), ("c", 2), ("d", 3)]);

    let (pairs, ..) = indexed(vec!["5", "4", "3", "2", "1"]).unwrap();
    assert_eq!(pairs, vec![("5", 0), ("4", 1), ("3", 2), ("2", 3), ("1", 4)]);

    let (pairs, ..) = indexed(vec!["10", "20", "30"]).unwrap();
    assert_eq!(pairs, vec![("10", 0), ("20", 1), ("30", 2)]);

    assert!(indexed(Vec::new()).unwrap().0.is_empty());
    let (pairs, ..) = indexed(vec!["a"]).unwrap();
    assert_eq!(pairs, vec![("a", 0)]);

    // :3830 — 100 elements, first and last pairs checked.
    let (pairs, ..) = indexed((0..100).map(|_| "n").collect()).unwrap();
    assert_eq!(pairs.len(), 100);
    assert_eq!(pairs[0], ("n", 0));
    assert_eq!(pairs[99], ("n", 99));

    // :3814 — the stream fails after its items.
    assert!(matches!(
        zip_fold(
            ZipSide::of(vec![vec!["a", "b", "c"]], ZipEnd::Fail("boom".to_string())),
            ZipSide::clean(vec![(0..3).collect::<Vec<i64>>()]),
            |s, i| (s, i),
        ),
        Err(ZipFault::Fail(ref e)) if e == "boom"
    ));
}

// ---------------------------------------------------------------------------
// The dropping family (Stream.test.ts 4114-4222)
// ---------------------------------------------------------------------------

/// `Stream.drop` at the pin, reduced to what the rows assert: drop the first
/// `n` elements, then emit the rest. The count normalizes through
/// `Count.normalize` (NaN and non-positive become 0, a positive value
/// floors), the same law `take` already pins.
fn drop_fold<A: Clone, E>(side: ZipSide<A, E>, n: usize) -> Result<Vec<A>, ZipFault<E>> {
    let mut src = side;
    let mut dropped = 0;
    let mut out = Vec::new();
    loop {
        if src.leftover.is_empty() {
            match src.pull() {
                Pulled::Chunk => {}
                Pulled::Ended => return Ok(out),
                Pulled::Fault(fault) => return Err(fault),
            }
        }
        let take = src.leftover.len().min(n.saturating_sub(dropped));
        let drained: Vec<A> = src.leftover.drain(..take).collect();
        dropped += drained.len();
        if dropped >= n && !src.leftover.is_empty() {
            out.append(&mut src.leftover);
        }
    }
}

/// `Stream.dropRight` at the pin: keep everything except the last `n`
/// elements. The whole stream is drained first, which is why a failure at
/// the end still surfaces.
fn drop_right_fold<A: Clone, E>(side: ZipSide<A, E>, n: usize) -> Result<Vec<A>, ZipFault<E>> {
    let mut all = drop_fold(side, 0)?;
    let keep = all.len().saturating_sub(n);
    all.truncate(keep);
    Ok(all)
}

/// `Stream.dropWhile` at the pin: drop while `f` holds, emit from the first
/// element that fails it. `dropUntil(f)` is the dual with `f` negated and
/// one more element dropped.
fn drop_while_fold<A: Clone, E>(
    side: ZipSide<A, E>,
    f: impl Fn(&A) -> bool,
) -> Result<Vec<A>, ZipFault<E>> {
    let mut src = side;
    let mut dropping = true;
    let mut out = Vec::new();
    loop {
        if src.leftover.is_empty() {
            match src.pull() {
                Pulled::Chunk => {}
                Pulled::Ended => return Ok(out),
                Pulled::Fault(fault) => return Err(fault),
            }
        }
        while !src.leftover.is_empty() {
            let head = src.leftover.remove(0);
            if dropping && f(&head) {
                continue;
            }
            dropping = false;
            out.push(head);
        }
    }
}

#[test]
fn drop_drops_the_first_n_elements_and_normalizes_the_count() {
    // :4114 — drop(2) on [1, 2, 3, 4, 5] leaves [3, 4, 5], the same as the
    // array-wise drop the row compares against.
    let simple = drop_fold(ZipSide::clean(vec![vec![1i64, 2, 3, 4, 5]]), 2).unwrap();
    assert_eq!(simple, vec![3, 4, 5]);

    // :4125 — the count normalizes through Count.normalize: NaN and 0.5 drop
    // nothing, 1.9 drops one, 2.9 drops two — and the upstream chunking does
    // not change the result.
    for chunks in [vec![vec![1i64, 2, 3]], vec![vec![1], vec![2], vec![3]]] {
        // Count.normalize for [NaN, 0.5, 1.9, 2.9] is [0, 0, 1, 2]: NaN and
        // non-positive become 0, a positive value floors. The same law the
        // rechunk test pins, reached here through drop.
        let counts = [0usize, 0, 1, 2];
        let results: Vec<Vec<i64>> = counts
            .iter()
            .map(|count| drop_fold(ZipSide::clean(chunks.clone()), *count).unwrap())
            .collect();
        assert_eq!(
            results,
            vec![vec![1, 2, 3], vec![1, 2, 3], vec![2, 3], vec![3]],
            "the chunking must not change the drop"
        );
    }

    // :4143 — the failing first element is not swallowed by the drop.
    let failing = drop_fold(
        ZipSide::of(Vec::<Vec<i64>>::new(), ZipEnd::Fail("Ouch".to_string())),
        1,
    );
    assert!(
        matches!(failing, Err(ZipFault::Fail(ref e)) if e == "Ouch"),
        "a failed element is still an element the drop counts"
    );
}

#[test]
fn drop_right_keeps_all_but_the_last_n_elements() {
    // :4155 — dropRight(2) on [1, 2, 3, 4, 5] leaves [1, 2, 3].
    let simple = drop_right_fold(ZipSide::clean(vec![vec![1i64, 2, 3, 4, 5]]), 2).unwrap();
    assert_eq!(simple, vec![1, 2, 3]);

    // :4166 — the count normalizes the same way: NaN and 0.5 keep everything,
    // 1.9 keeps [1, 2], 2.9 keeps [1].
    let counts = [0usize, 0, 1, 2];
    let results: Vec<Vec<i64>> = counts
        .iter()
        .map(|count| drop_right_fold(ZipSide::clean(vec![vec![1i64, 2, 3]]), *count).unwrap())
        .collect();
    assert_eq!(results, vec![vec![1, 2, 3], vec![1, 2, 3], vec![1, 2], vec![1]]);

    // :4176 — draining to the end surfaces the trailing failure.
    let failing = drop_right_fold(
        ZipSide::of(vec![vec![1i64]], ZipEnd::Fail("Ouch".to_string())),
        1,
    );
    assert!(
        matches!(failing, Err(ZipFault::Fail(ref e)) if e == "Ouch"),
        "dropRight drains to the end, so the failure surfaces"
    );
}

#[test]
fn drop_until_drop_while_and_drop_while_filter_walk_the_predicate() {
    // :4202 — dropWhile(n < 3) drops 1 and 2, keeping [3, 4, 5].
    let dropped = drop_while_fold(ZipSide::clean(vec![vec![1i64, 2, 3, 4, 5]]), |n| *n < 3).unwrap();
    assert_eq!(dropped, vec![3, 4, 5]);

    // :4188 — dropUntil(f) drops while f fails and then one more element, so
    // [1, 2, 3, 4, 5] with f(n) = n < 3 keeps [2, 3, 4, 5] (1 is dropped as
    // the first element f accepts). The row's own reference is
    // dropWhile(!f) then drop(1).
    let until = drop_while_fold(ZipSide::clean(vec![vec![1i64, 2, 3, 4, 5]]), |n| !(*n < 3)).unwrap();
    let until = until.into_iter().skip(1).collect::<Vec<i64>>();
    assert_eq!(until, vec![2, 3, 4, 5]);

    // :4213 — dropWhileFilter drops while the filter succeeds, keeping the
    // rest: [3, 4, 5].
    let filtered = drop_while_filter_fold(
        ZipSide::clean(vec![vec![1i64, 2, 3, 4, 5]]),
        |n| (*n < 3).then_some(*n),
    )
    .unwrap();
    assert_eq!(filtered, vec![3, 4, 5]);

    // :4222 — take(1) already ended the stream before dropWhile ran, so the
    // trailing failure is never pulled and the drain succeeds. Modelled with
    // the side take left behind: one element, ended cleanly.
    let short_circuited = drop_while_fold(ZipSide::clean(vec![vec![1i64]]), |_| true).unwrap();
    assert!(short_circuited.is_empty(), "the failure sits past the take and is never pulled");
}

/// `Stream.dropWhileFilter` at the pin: drop while the filter SUCCEEDS (the
/// dual of `takeWhileFilter`), emit from the first failure.
fn drop_while_filter_fold<A: Clone, E>(
    side: ZipSide<A, E>,
    f: impl Fn(&A) -> Option<A>,
) -> Result<Vec<A>, ZipFault<E>> {
    let mut src = side;
    let mut dropping = true;
    let mut out = Vec::new();
    loop {
        if src.leftover.is_empty() {
            match src.pull() {
                Pulled::Chunk => {}
                Pulled::Ended => return Ok(out),
                Pulled::Fault(fault) => return Err(fault),
            }
        }
        while !src.leftover.is_empty() {
            let head = src.leftover.remove(0);
            // Once the filter fails, every later element is kept.
            if dropping && f(&head).is_some() {
                continue;
            }
            dropping = false;
            out.push(head);
        }
    }
}

// ---------------------------------------------------------------------------
// mkArrayBuffer concatenates Uint8Array chunks (Stream.test.ts 155)
// ---------------------------------------------------------------------------

#[test]
fn mk_array_buffer_concatenates_uint8array_chunks() {
    // Stream.make(Uint8Array[1,2], Uint8Array[3,4]).pipe(Stream.mkArrayBuffer)
    // === Uint8Array[1,2,3,4]. The chunks are byte arrays, so "one value" is
    // their concatenation, which is the pin's fold over `mkUint8Array` with
    // its `.buffer` projection stripped away.
    let stream: Stream<Vec<u8>, Infallible, ()> =
        from_iter(vec![vec![1u8, 2], vec![3u8, 4]].into_iter(), 2);
    match stream.mk_array_buffer() {
        Exit::Success(bytes) => assert_eq!(bytes, vec![1u8, 2, 3, 4]),
        Exit::Failure(_) => panic!("mkArrayBuffer must concatenate, not fail"),
    }
}

// ---------------------------------------------------------------------------
// decodeText across chunk boundaries (Stream.test.ts 578, 588)
// ---------------------------------------------------------------------------

use std::string::FromUtf8Error;

#[test]
fn decode_text_handles_multibyte_characters_split_across_chunks() {
    // Stream.make(bytes[0..2], bytes[2..]).pipe(Stream.decodeText(),
    // Stream.mkString) === "🌍". The emoji is four UTF-8 bytes, so the first
    // chunk ends mid-character and the decoder must hold the partial sequence
    // until the second chunk completes it.
    // One byte stream chunked at 2, so the emoji's four bytes straddle a pull
    // boundary: the decoder must hold bytes 2..4 after emitting nothing for
    // the first chunk.
    let bytes = "🌍".as_bytes().to_vec();
    let stream: Stream<String, FromUtf8Error, ()> = from_iter(bytes.into_iter(), 2).decode_text();
    match stream.run_collect() {
        Exit::Success(v) => assert_eq!(v.concat(), "🌍"),
        Exit::Failure(_) => panic!("decodeText must not fail on a split character"),
    }
}

#[test]
fn decode_text_handles_mixed_ascii_and_multibyte_across_chunks() {
    // Stream.make(bytes[0..8], bytes[8..]).pipe(Stream.decodeText(),
    // Stream.mkString) === "Hello 🌍!". The split lands inside the emoji, so
    // the boundary case is the same one with ASCII on both sides.
    let bytes = "Hello 🌍!".as_bytes().to_vec();
    let stream: Stream<String, FromUtf8Error, ()> = from_iter(bytes.into_iter(), 8).decode_text();
    match stream.run_collect() {
        Exit::Success(v) => assert_eq!(v.concat(), "Hello 🌍!"),
        Exit::Failure(_) => panic!("decodeText must not fail on a split character"),
    }
}

fn collect_chunks_or_panic<A: Send + 'static>(stream: Stream<A, Infallible, ()>) -> Vec<Vec<A>> {
    match stream.run_collect_chunks() {
        Exit::Success(v) => v,
        Exit::Failure(_) => panic!("collect chunks failed"),
    }
}

#[test]
fn rechunk_preserves_a_large_source_chunk_with_target() {
    // The pin's `it.each` rows over `Array.makeBy(200_000, i => i)`:
    //   target 200_001 -> [200_000]   (target above the input, one chunk)
    //   target 200_000 -> [200_000]    (target equal, one chunk)
    //   target 199_999 -> [199_999, 1] (a full chunk plus a remainder)
    let values: Vec<i64> = (0..200_000).collect();

    let above: Vec<Vec<i64>> =
        collect_chunks_or_panic(from_iter(values.clone().into_iter(), 200_000).rechunk(200_001.0));
    assert_eq!(above, vec![values.clone()], "a target above the input yields one chunk");

    let exact: Vec<Vec<i64>> =
        collect_chunks_or_panic(from_iter(values.clone().into_iter(), 200_000).rechunk(200_000.0));
    assert_eq!(exact, vec![values.clone()], "a target equal to the input yields one chunk");

    let below: Vec<Vec<i64>> =
        collect_chunks_or_panic(from_iter(values.into_iter(), 200_000).rechunk(199_999.0));
    assert_eq!(below.len(), 2);
    assert_eq!(below[0].len(), 199_999);
    assert_eq!(below[1], vec![199_999], "the remainder is the final element");
}

#[test]
fn rechunk_preserves_large_input_split_across_source_chunks() {
    // Stream.fromArrays(one, rest).pipe(Stream.rechunk(200_002),
    // Stream.chunks, Stream.runCollect) === [all 200_001]. The source's own
    // chunking is irrelevant once the target exceeds the whole input: the
    // carry accumulates across source boundaries and flushes once at the end.
    // Built by concatenating two one-chunk sources, which is `fromArrays`.
    let values: Vec<i64> = (0..200_001).collect();
    let (head, tail) = (values[..1].to_vec(), values[1..].to_vec());
    let tail_len = tail.len();
    let split_source = from_iter(head.into_iter(), 1)
        .concat(from_iter(tail.into_iter(), tail_len));
    let chunks: Vec<Vec<i64>> = collect_chunks_or_panic(split_source.rechunk(200_002.0));
    assert_eq!(chunks.len(), 1, "the whole input arrives as one chunk");
    assert_eq!(chunks[0], values);
}

// ---------------------------------------------------------------------------
// Stream.catch at the pin (describe "error handling")
// ---------------------------------------------------------------------------

#[test]
fn stream_catch_recovers_from_a_typed_failure() {
    // Stream.make(1,2,3).concat(Stream.fail("boom")).pipe(
    //   Stream.catch(e => Stream.make(4,5,6)), Stream.runCollect)
    // === [1, 2, 3, 4, 5, 6], and the error "boom" was observed.
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = Arc::clone(&seen);

    let head = from_iter(vec![1u32, 2, 3].into_iter(), 3);
    let tail = Stream::<u32, String, ()>::fail("boom".to_string());
    let stream = head.concat(tail).catch(move |error: String| {
        observed.lock().unwrap().push(error);
        from_iter(vec![4u32, 5, 6].into_iter(), 3)
    });

    match stream.run_collect() {
        Exit::Success(values) => assert_eq!(values, vec![1, 2, 3, 4, 5, 6]),
        Exit::Failure(_) => panic!("catch must recover from the typed failure"),
    }
    assert_eq!(*seen.lock().unwrap(), vec!["boom".to_string()]);
}

#[test]
fn stream_catch_does_not_swallow_a_defect() {
    // catch only handles the typed error channel. A stream that dies must
    // still die: catch does NOT fire, so the defect survives.
    let called = Arc::new(AtomicUsize::new(0));
    let fenced = Arc::clone(&called);
    let source = Stream::<u32, String, ()>::from_pull(|| {
        Effect::<Option<Vec<u32>>, String>::die(Defect::new("boom"))
    });
    let stream = source.catch(move |_error: String| {
        fenced.fetch_add(1, Ordering::SeqCst);
        from_iter(vec![4u32].into_iter(), 1)
    });
    match stream.run_collect() {
        Exit::Failure(cause) => assert!(!cause.defects().is_empty(), "the defect survives"),
        Exit::Success(_) => panic!("a dying stream must not be recovered by catch"),
    }
    assert_eq!(called.load(Ordering::SeqCst), 0, "catch must not fire on a defect");
}

// ---------------------------------------------------------------------------
// Stream.catchIf at the pin (describe "error handling")
// ---------------------------------------------------------------------------

#[test]
fn catch_if_recovers_a_matching_typed_failure() {
    // catchIf(e => typeof e === "string", e => succeed(`caught: ${e}`)) over
    // a stream failing with "boom" => ["caught: boom"].
    let stream = Stream::<u32, String, ()>::fail("boom".to_string()).catch_if(
        |error: &String| error == "boom",
        |_error: String| from_iter(vec![11u32].into_iter(), 1),
    );
    match stream.run_collect() {
        Exit::Success(values) => assert_eq!(values, vec![11]),
        Exit::Failure(cause) => panic!("catchIf must recover a matching failure, got {cause:?}"),
    }
}

#[test]
fn catch_if_re_raises_an_unmatched_failure_unchanged() {
    // A failure the predicate rejects is re-raised unchanged: catchIf must
    // never accidentally recover an error it does not name.
    let stream = Stream::<u32, String, ()>::fail("42".to_string()).catch_if(
        |error: &String| error == "boom",
        |_error: String| from_iter(vec![11u32].into_iter(), 1),
    );
    match stream.run_collect() {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"42".to_string())),
        Exit::Success(_) => panic!("a rejected failure must not be recovered"),
    }
}

#[test]
fn catch_if_recovers_on_a_later_pull_after_a_pass_through() {
    // The substitution must persist across pulls, not just the first one:
    // a head of elements, then a matching failure, must still recover.
    let head = from_iter(vec![1u32, 2].into_iter(), 2);
    let tail = Stream::<u32, String, ()>::fail("boom".to_string());
    let stream = head.concat(tail).catch_if(
        |error: &String| error == "boom",
        |_error: String| from_iter(vec![7u32].into_iter(), 1),
    );
    match stream.run_collect() {
        Exit::Success(values) => assert_eq!(values, vec![1, 2, 7]),
        Exit::Failure(cause) => panic!("catchIf must recover after a pass-through, got {cause:?}"),
    }
}

#[test]
fn catch_filter_routes_through_selector_result() {
    // catchFilter(selector, handler, or_else): a selector returning Ok routes
    // to handler, one returning Err routes to or_else. Both arms recover, so
    // nothing is re-raised.
    //
    // Port shape: the selector is FnOnce(E) -> Result<S, String>; S here is
    // usize (the matched length) and the failure arm carries the String.
    let matched = Stream::<u32, String, ()>::fail("boom".to_string()).catch_filter(
        |error: String| {
            if error == "boom" {
                Ok(error.len())
            } else {
                Err(format!("fallback: {error}"))
            }
        },
        |length: usize| from_iter(vec![length as u32].into_iter(), 1),
        |_failure: String| from_iter(vec![1u32].into_iter(), 1),
    );
    assert!(matches!(matched.run_collect(), Exit::Success(ref v) if *v == vec![4]));

    let unmatched = Stream::<u32, String, ()>::fail("42".to_string()).catch_filter(
        |error: String| {
            if error == "boom" {
                Ok(error.len())
            } else {
                Err(format!("fallback: {error}"))
            }
        },
        |length: usize| from_iter(vec![length as u32].into_iter(), 1),
        |failure: String| from_iter(vec![failure.len() as u32].into_iter(), 1),
    );
    // "fallback: 42" is 12 chars, which is what the fallback arm produces.
    assert!(matches!(unmatched.run_collect(), Exit::Success(ref v) if *v == vec![12]));
}

// ---------------------------------------------------------------------------
// Stream tapCause / catchDefect / die at the pin
// ---------------------------------------------------------------------------

#[test]
fn die_stream_dies_rather_than_failing() {
    // Stream.die is not Stream.fail: the defect is not a typed error, so it
    // must not appear in the error channel.
    let stream = Stream::<u32, String, ()>::die(Defect::new("boom"));
    match stream.run_collect() {
        Exit::Failure(cause) => {
            assert!(!cause.defects().is_empty(), "a die is a defect");
            assert!(cause.failures().is_empty(), "a die is not a typed failure");
        }
        Exit::Success(_) => panic!("a dying stream must not succeed"),
    }
}

#[test]
fn tap_cause_preserves_a_typed_failure() {
    // tapCause observes and re-raises: the failure survives verbatim.
    let observed = Arc::new(Mutex::new(false));
    let saw = Arc::clone(&observed);
    let stream = Stream::<u32, String, ()>::fail("boom".to_string()).tap_cause(move |_cause| {
        *saw.lock().unwrap() = true;
    });
    match stream.run_collect() {
        Exit::Failure(cause) => {
            assert_eq!(cause.failure(), Some(&"boom".to_string()));
            assert!(cause.defects().is_empty(), "a Fail must not become a Die");
        }
        Exit::Success(_) => panic!("tapCause must not swallow the failure"),
    }
    assert!(*observed.lock().unwrap(), "tapCause must have observed the cause");
}

#[test]
fn tap_cause_preserves_a_defect() {
    // The defect kind survives too: tapCause re-raises without collapsing.
    let stream = Stream::<u32, String, ()>::die(Defect::new("boom")).tap_cause(|_cause| {});
    match stream.run_collect() {
        Exit::Failure(cause) => {
            assert_eq!(cause.defects().len(), 1, "the defect survives");
            assert!(cause.failures().is_empty(), "no typed failure appears");
        }
        Exit::Success(_) => panic!("tapCause must not swallow the defect"),
    }
}

#[test]
fn catch_defect_recovers_a_defect_but_not_a_typed_failure() {
    // die -> recovered with the defect value; fail -> NOT recovered.
    let recovered = Stream::<u32, String, ()>::die(Defect::new("boom")).catch_defect(|defect| {
        // The handler's stream reproduces the defect value as a string.
        from_iter(vec![defect.message.len() as u32].into_iter(), 1)
    });
    assert!(
        matches!(recovered.run_collect(), Exit::Success(ref v) if *v == vec![4]),
        "catchDefect must recover a defect"
    );

    let failed = Stream::<u32, String, ()>::fail("failure".to_string()).catch_defect(|_defect| {
        from_iter(vec![9u32].into_iter(), 1)
    });
    match failed.run_collect() {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"failure".to_string())),
        Exit::Success(_) => panic!("catchDefect must not recover a typed failure"),
    }
}

#[test]
fn catch_cause_filter_recovers_through_a_cause_extractor() {
    // The extractor sees the WHOLE cause and decides: Ok(value) recovers via
    // handler, Err(cause) re-raises that cause unchanged.
    //
    // Pin shape: Cause.hasFails(cause) ? Ok(squash) : Err(cause). The port's
    // equivalent uses failure().cloned() for the squash value.
    let recovered = Stream::<u32, String, ()>::fail("boom".to_string()).catch_cause_filter(
        |cause: elm_effect::Cause<String>| -> std::result::Result<String, elm_effect::Cause<String>> {
            cause.failure().cloned().ok_or(cause)
        },
        |failure: String| from_iter(vec![failure.len() as u32].into_iter(), 1),
    );
    assert!(
        matches!(recovered.run_collect(), Exit::Success(ref v) if *v == vec![4]),
        "catchCauseFilter must recover a Fail through the extractor"
    );
}

#[test]
fn catch_cause_filter_re_raises_a_declined_cause() {
    // A cause the extractor declines must be re-raised VERBATIM: a defect is
    // not a Fail, so the extractor declines and the defect survives.
    let stream = Stream::<u32, String, ()>::die(Defect::new("boom")).catch_cause_filter(
        |cause: elm_effect::Cause<String>| -> std::result::Result<String, elm_effect::Cause<String>> {
            cause.failure().cloned().ok_or(cause)
        },
        |failure: String| from_iter(vec![failure.len() as u32].into_iter(), 1),
    );
    match stream.run_collect() {
        Exit::Failure(cause) => {
            assert_eq!(cause.defects().len(), 1, "the declined defect survives verbatim");
            assert!(cause.failures().is_empty(), "not collapsed into a typed failure");
        }
        Exit::Success(_) => panic!("a declined cause must be re-raised, not recovered"),
    }
}
