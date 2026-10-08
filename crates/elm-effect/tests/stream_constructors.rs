//! `Stream.range` and `Stream.runForEachWhile` at the pin
//! (`packages/effect/test/Stream.test.ts`, describe "constructors" and the
//! `runForEachWhile` block).
//!
//! Chunk boundaries are observable: `Stream.chunks().runCollect()` in the
//! pin, `run_collect_chunks` here.

use std::convert::Infallible;

use elm_effect::{Exit, Stream};

#[test]
fn range_min_less_than_max() {
    // Stream.range(1, 3).pipe(Stream.runCollect) === [1, 2, 3]
    let s: Stream<i64, Infallible, ()> = Stream::range(1, 3, 4096.0);
    assert!(matches!(s.run_collect(), Exit::Success(v) if v == vec![1, 2, 3]));
}

#[test]
fn range_zero_chunk_size() {
    // Stream.range(1, 3, 0).pipe(Stream.take(4), Stream.runCollect) === [1, 2, 3]
    // A zero chunk size normalizes to 1, so the range is unchanged.
    let s: Stream<i64, Infallible, ()> = Stream::range(1, 3, 0.0).take(4);
    assert!(matches!(s.run_collect(), Exit::Success(v) if v == vec![1, 2, 3]));
}

#[test]
fn range_normalizes_chunk_size() {
    // Stream.range(1, 5, chunkSize).pipe(Stream.chunks, Stream.runCollect)
    // over [NaN, 0, 2.9]: NaN and 0 both normalize to 1; 2.9 floors to 2.
    let expected: Vec<Vec<Vec<i64>>> = vec![
        vec![vec![1], vec![2], vec![3], vec![4], vec![5]],
        vec![vec![1], vec![2], vec![3], vec![4], vec![5]],
        vec![vec![1, 2], vec![3, 4], vec![5]],
    ];
    let got: Vec<Vec<Vec<i64>>> = [f64::NAN, 0.0, 2.9]
        .iter()
        .map(|size| match Stream::range(1, 5, *size).run_collect_chunks() {
            Exit::Success(chunks) => chunks,
            Exit::Failure(_) => panic!("range must not fail"),
        })
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn range_min_greater_than_max() {
    // Stream.range(4, 3).pipe(Stream.runCollect) === []
    let s: Stream<i64, Infallible, ()> = Stream::range(4, 3, 4096.0);
    assert!(matches!(s.run_collect(), Exit::Success(v) if v.is_empty()));
}

#[test]
fn range_min_equal_to_max() {
    // Stream.range(3, 3).pipe(Stream.runCollect) === [3]
    let s: Stream<i64, Infallible, ()> = Stream::range(3, 3, 4096.0);
    assert!(matches!(s.run_collect(), Exit::Success(v) if v == vec![3]));
}

#[test]
fn run_for_each_while_continues_across_chunks() {
    // fromArrays([1, 2], [3, 4]): the predicate always returns true, so every
    // element is seen including across the chunk boundary.
    let seen = std::cell::RefCell::new(Vec::<i64>::new());
    let exit = Stream::range(1, 4, 2.0).run_for_each_while(|n| {
        seen.borrow_mut().push(n);
        true
    });
    assert!(matches!(exit, Exit::Success(())));
    assert_eq!(*seen.borrow(), vec![1, 2, 3, 4]);
}

#[test]
fn run_for_each_while_stops_on_false() {
    // fromArrays([1, 2, 3], [4, 5]) with `n < 3`: the element that returned
    // false is visited, nothing after it is.
    let seen = std::cell::RefCell::new(Vec::<i64>::new());
    let exit = Stream::range(1, 5, 3.0).run_for_each_while(|n| {
        seen.borrow_mut().push(n);
        n < 3
    });
    assert!(matches!(exit, Exit::Success(())));
    assert_eq!(*seen.borrow(), vec![1, 2, 3]);
}
