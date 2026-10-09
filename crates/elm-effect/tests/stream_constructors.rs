//! `Stream.range` and `Stream.runForEachWhile` at the pin
//! (`packages/effect/test/Stream.test.ts`, describe "constructors" and the
//! `runForEachWhile` block).
//!
//! Chunk boundaries are observable: `Stream.chunks().runCollect()` in the
//! pin, `run_collect_chunks` here.

use std::convert::Infallible;

use elm_effect::{Exit, Stream, from_iter};

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

// ---------------------------------------------------------------------------
// splitLines at the pin (describe "encoding" / "splitLines")
// ---------------------------------------------------------------------------

fn lines_of(chunks: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for chunk in chunks {
        items.push((*chunk).to_string());
    }
    let stream: Stream<String, String, ()> =
        from_iter(items.into_iter(), chunks.len().max(1));
    let stream = stream.split_lines();
    match stream.run_collect() {
        Exit::Success(lines) => lines,
        Exit::Failure(_) => panic!("split_lines must not fail"),
    }
}

#[test]
fn split_lines_handles_an_empty_stream() {
    assert!(lines_of(&[]).is_empty());
}

#[test]
fn split_lines_handles_empty_chunks_and_repeated_line_feeds() {
    assert_eq!(lines_of(&["", "a\n\n", "", "b\n", ""]), vec!["a", "", "b"]);
}

#[test]
fn split_lines_handles_line_feeds_across_chunks() {
    // A terminator at the end of one chunk and text at the start of the next
    // are one break, not two.
    assert_eq!(lines_of(&["a\nb", "\nc\n", "d"]), vec!["a", "b", "c", "d"]);
}

#[test]
fn split_lines_handles_carriage_return_line_feed_pairs_across_chunks() {
    // "a\r" then "\nb\r" then "\nc": the CRLF spans the first boundary, so
    // "a" is emitted before the second chunk is read.
    assert_eq!(lines_of(&["a\r", "\nb\r", "\nc"]), vec!["a", "b", "c"]);
}

#[test]
fn split_lines_emits_a_carriage_return_terminated_line_before_pulling_upstream() {
    // A bare \r terminates the line inside the same chunk.
    assert_eq!(lines_of(&["a\rb\rc"]), vec!["a", "b", "c"]);
}

#[test]
fn split_lines_handles_standalone_carriage_returns_across_chunk_boundaries() {
    assert_eq!(lines_of(&["a\r", "b\r", "c"]), vec!["a", "b", "c"]);
}

#[test]
fn split_lines_emits_the_final_line_when_the_stream_does_not_end_with_a_newline() {
    // The unterminated tail is flushed as the last line on upstream end.
    assert_eq!(lines_of(&["a\nb\nc\n", "d\ne"]), vec!["a", "b", "c", "d", "e"]);
}

#[test]
fn split_lines_pin_doc_example() {
    // The pin's own example: "a\nb\r\n", "c\n" => ['a', 'b', 'c']
    assert_eq!(lines_of(&["a\nb\r\n", "c\n"]), vec!["a", "b", "c"]);
}
