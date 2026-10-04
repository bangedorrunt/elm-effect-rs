//! Pull-based streams: bounded chunks, lazy transforms, eager sinks.

use elm_effect::{Effect, Exit, Stream, from_iter};

#[test]
fn run_fold_sums_chunks() {
    let stream = from_iter(1u32..=100, 7);
    let exit = stream.run_fold(0u32, |acc, n| acc + n);
    assert!(matches!(exit, Exit::Success(5050)));
    let normalized = from_iter(1u32..=3, 0).run_collect();
    assert!(matches!(normalized, Exit::Success(ref v) if *v == vec![1u32, 2, 3]));
}

#[test]
fn map_filter_transform_lazily() {
    let stream = from_iter(1u32..=10, 3)
        .map(|n| n * 2)
        .filter(|n| n % 3 == 0);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![6u32, 12, 18]));
}

#[test]
fn take_terminates_infinite_input() {
    // Never materializes the unbounded input: only five pulls run.
    let stream = from_iter(0u32.., 4).take(5);
    let exit = stream.run_collect();
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![0u32, 1, 2, 3, 4]));
    let empty_take = from_iter(1u32..=3, 2).take(0).run_collect();
    assert!(matches!(empty_take, Exit::Success(ref v) if v.is_empty()));
}

#[test]
fn empty_and_concat() {
    let first = from_iter(vec![1u32, 2].into_iter(), 10);
    let second = from_iter(vec![3u32].into_iter(), 10);
    let exit = first
        .concat(second)
        .concat(Stream::<u32, std::convert::Infallible, ()>::empty())
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
