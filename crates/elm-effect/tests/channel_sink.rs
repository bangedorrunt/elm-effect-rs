//! Wave-7 conversation endpoints: Pull sources, Take outcomes, Channel
//! transforms, Sink consumers. Modules load via `#[path]` includes (the .15
//! precedent): no facade touch.
//!
//! A `Pull` yields one [`Take`] per call: items, end of stream, or failure.
//! A [`Channel`] transforms takes; a [`Sink`] folds them to a value. Pulls
//! yield bounded chunks; an unbounded source stays a pull loop, never a
//! materialized `Vec`.

#[path = "../src/channel.rs"]
mod channel;
#[path = "../src/pull.rs"]
mod pull;
#[path = "../src/sink.rs"]
mod sink;
#[path = "../src/take.rs"]
mod take;

use take::Take;

fn items<T>(values: Vec<T>) -> Take<T, String> {
    Take::Items(values)
}

#[test]
fn pull_items_then_end() {
    let mut p = pull::Pull::<i32, String>::of(vec![1, 2, 3]);
    assert_eq!(p.pull(), Take::Items(vec![1, 2, 3]));
    assert_eq!(p.pull(), Take::End);
    assert_eq!(p.pull(), Take::End);
}

#[test]
fn pull_empty_ends_immediately() {
    let mut p = pull::Pull::<i32, String>::empty();
    assert_eq!(p.pull(), Take::End);
}

#[test]
fn pull_fail_propagates() {
    let mut p = pull::Pull::<i32, String>::fail("boom".to_string());
    assert_eq!(p.pull(), Take::Fail("boom".to_string()));
}

#[test]
fn take_maps() {
    assert_eq!(items(vec![1]).map(|v| v + 1), items(vec![2]));
    assert_eq!(Take::<i32, String>::End.map(|v: i32| v), Take::End);
    assert!(Take::<i32, String>::End.is_end());
    assert!(!items(vec![1]).is_end());
}

#[test]
fn channel_identity_passes_through() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2]);
    let piped = channel::identity().run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), items(vec![1, 2]));
    assert_eq!(piped.pull(), Take::End);
}

#[test]
fn channel_map_transforms() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2]);
    let piped = channel::map(|a: i32| a * 10).run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), items(vec![10, 20]));
    assert_eq!(piped.pull(), Take::End);
}

#[test]
fn channel_filter_drops() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2, 3]);
    let piped = channel::filter(|a: &i32| *a % 2 == 1).run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), items(vec![1, 3]));
    assert_eq!(piped.pull(), Take::End);
}

#[test]
fn channel_forwards_end_and_failure() {
    let mut piped = channel::map(|a: i32| a).run(pull::Pull::empty());
    assert_eq!(piped.pull(), Take::<i32, String>::End);
    let mut piped = channel::map(|a: i32| a).run(pull::Pull::fail("nope".to_string()));
    assert_eq!(piped.pull(), Take::Fail("nope".to_string()));
}

#[test]
fn sink_collects() {
    let collected = sink::collect::<i32, String>().run(pull::Pull::of(vec![1, 2]));
    assert_eq!(collected, elm_effect::Exit::Success(vec![1, 2]));
}

#[test]
fn sink_counts_and_folds() {
    let counted = sink::count::<i32, String>().run(pull::Pull::of(vec![1, 2, 3]));
    assert_eq!(counted, elm_effect::Exit::Success(3usize));
    let summed = sink::fold::<i32, String, i32>(0, |acc: i32, chunk: Vec<i32>| {
        acc + chunk.iter().sum::<i32>()
    })
    .run(pull::Pull::of(vec![1, 2]));
    assert_eq!(summed, elm_effect::Exit::Success(3));
    // Reducer runs once per non-empty chunk and matches Array.reduce.
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let probe = calls.clone();
    let folded = sink::fold::<i32, String, i32>(0, move |acc: i32, chunk: Vec<i32>| {
        probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        acc + chunk.iter().sum::<i32>()
    })
    .run(pull::Pull::of(vec![1, 2, 3]));
    assert_eq!(folded, elm_effect::Exit::Success(6));
    // of() delivers one chunk: the reducer ran exactly once for it.
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let empty_count = sink::count::<i32, String>().run(pull::Pull::empty());
    assert_eq!(empty_count, elm_effect::Exit::Success(0usize));
}

#[test]
fn sink_head_takes_first() {
    let head = sink::head::<i32, String>().run(pull::Pull::of(vec![7, 8]));
    assert_eq!(head, elm_effect::Exit::Success(Some(7)));
    let empty = sink::head::<i32, String>().run(pull::Pull::empty());
    assert_eq!(empty, elm_effect::Exit::Success(None::<i32>));
}

#[test]
fn sink_failure_propagates() {
    let failed = sink::collect::<i32, String>().run(pull::Pull::fail("bad".to_string()));
    assert_eq!(
        failed,
        elm_effect::Exit::Failure(elm_effect::Cause::fail("bad".to_string()))
    );
}

#[test]
fn channel_filter_map_recovers() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2, 3]);
    let piped = channel::filter_map(|a: i32| (a % 2 == 1).then_some(a * 10)).run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Items(vec![10, 30]));
    assert_eq!(piped.pull(), take::Take::End);
    let source = pull::Pull::<i32, String>::fail("no".to_string());
    let piped = channel::recover(|e: &String| e == "no").run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::End);
    let source = pull::Pull::<i32, String>::fail("other".to_string());
    let piped = channel::recover(|e: &String| e == "no").run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Fail("other".to_string()));
}

#[test]
fn sink_take_while_stops() {
    let got = sink::take_while(|a: &i32| *a < 3).run(pull::Pull::<i32, String>::of(vec![1, 2, 3]));
    assert_eq!(got, elm_effect::Exit::Success(vec![1, 2]));
    // No termination: everything passes.
    let got = sink::take_while(|_: &i32| true).run(pull::Pull::<i32, String>::of(vec![1]));
    assert_eq!(got, elm_effect::Exit::Success(vec![1]));
    // Immediate termination: empty result.
    let got = sink::take_while(|_: &i32| false).run(pull::Pull::<i32, String>::of(vec![1]));
    assert_eq!(got, elm_effect::Exit::Success(Vec::<i32>::new()));
    // Empty input: empty result.
    let got = sink::take_while(|_: &i32| true).run(pull::Pull::<i32, String>::empty());
    assert_eq!(got, elm_effect::Exit::Success(Vec::<i32>::new()));
}

#[test]
fn pull_constructors() {
    let mut p = pull::Pull::<i32, String>::single(7);
    assert_eq!(p.pull(), take::Take::Items(vec![7]));
    assert_eq!(p.pull(), take::Take::End);
    let mut p = pull::Pull::<i32, String>::from_slice(&[1, 2]);
    assert_eq!(p.pull(), take::Take::Items(vec![1, 2]));
    assert_eq!(p.pull(), take::Take::End);
    let mut p = pull::Pull::<i32, String>::from_iter(0..5, 2);
    assert_eq!(p.pull(), take::Take::Items(vec![0, 1]));
    assert_eq!(p.pull(), take::Take::Items(vec![2, 3]));
    assert_eq!(p.pull(), take::Take::Items(vec![4]));
    assert_eq!(p.pull(), take::Take::End);
    // Unbounded input stays chunked.
    let mut p = pull::Pull::<i32, String>::from_iter(0.., 3);
    assert_eq!(p.pull(), take::Take::Items(vec![0, 1, 2]));
    // Zero chunk size normalizes to one.
    let mut p = pull::Pull::<i32, String>::from_iter(0..3, 0);
    assert_eq!(p.pull(), take::Take::Items(vec![0]));
    assert_eq!(p.pull(), take::Take::Items(vec![1]));
    assert_eq!(p.pull(), take::Take::Items(vec![2]));
    assert_eq!(p.pull(), take::Take::End);
}

#[test]
fn channel_map_effect_short_circuits() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2]);
    let piped = channel::map_effect(|a: i32| {
        if a > 1 {
            Err("big".to_string())
        } else {
            Ok(a * 10)
        }
    })
    .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Fail("big".to_string()));
    let source = pull::Pull::<i32, String>::of(vec![1]);
    let piped = channel::map_effect(|a: i32| Ok::<i32, String>(a + 1)).run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Items(vec![2]));
    assert_eq!(piped.pull(), take::Take::End);
}

#[test]
fn sink_take_limits() {
    let got = sink::take::<i32, String>(2).run(pull::Pull::of(vec![1, 2, 3]));
    assert_eq!(got, elm_effect::Exit::Success(vec![1, 2]));
    let got = sink::take::<i32, String>(9).run(pull::Pull::of(vec![1]));
    assert_eq!(got, elm_effect::Exit::Success(vec![1]));
    // Zero count never pulls: empty result over any input.
    let got = sink::take::<i32, String>(0).run(pull::Pull::of(vec![1]));
    assert_eq!(got, elm_effect::Exit::Success(Vec::<i32>::new()));
    // Empty input: empty chunk, not a failure.
    let got = sink::take::<i32, String>(3).run(pull::Pull::<i32, String>::empty());
    assert_eq!(got, elm_effect::Exit::Success(Vec::<i32>::new()));
}

#[test]
fn pull_bracket_releases_once() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let released = Arc::new(AtomicUsize::new(0));
    let probe = released.clone();
    let mut p = pull::Pull::using(
        vec![1, 2],
        |rest: &mut Vec<i32>| {
            if rest.is_empty() {
                take::Take::<i32, String>::End
            } else {
                take::Take::<i32, String>::Items(vec![rest.remove(0)])
            }
        },
        move |_| {
            probe.fetch_add(1, Ordering::SeqCst);
        },
    );
    assert_eq!(p.pull(), take::Take::Items(vec![1]));
    assert_eq!(p.pull(), take::Take::Items(vec![2]));
    assert_eq!(p.pull(), take::Take::End);
    assert_eq!(p.pull(), take::Take::End);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn pull_bracket_release_failure_surfaces() {
    let mut p = pull::Pull::using_try(
        vec![1],
        |rest: &mut Vec<i32>| {
            if rest.is_empty() {
                take::Take::<i32, String>::End
            } else {
                take::Take::<i32, String>::Items(vec![rest.remove(0)])
            }
        },
        |_| Err::<(), String>("release blew up".to_string()),
    );
    // The stream's own ending is delivered first...
    assert_eq!(p.pull(), take::Take::Items(vec![1]));
    assert_eq!(p.pull(), take::Take::End);
    // ...then the release failure surfaces.
    assert_eq!(p.pull(), take::Take::Fail("release blew up".to_string()));
}

#[test]
fn channel_flat_map_concatenates() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2]);
    let piped = channel::flat_map(|a: i32| pull::Pull::of(vec![a, a * 10])).run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Items(vec![1, 10, 2, 20]));
    assert_eq!(piped.pull(), take::Take::End);
    // Empty input: empty output.
    let source = pull::Pull::<i32, String>::empty();
    let piped = channel::flat_map(|a: i32| pull::Pull::of(vec![a])).run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::End);
    // Inner failure fails the output.
    let source = pull::Pull::<i32, String>::of(vec![1]);
    let piped = channel::flat_map(|_: i32| pull::Pull::<i32, String>::fail("inner".to_string()))
        .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Fail("inner".to_string()));
}

#[test]
fn channel_catch_if_recovers_matching() {
    // Matching failure: replaced by the handler pull.
    let mut first = true;
    let source = pull::Pull::<i32, String>::from_fn(move || {
        if first {
            first = false;
            take::Take::Fail("retry".to_string())
        } else {
            take::Take::End
        }
    });
    let piped = channel::catch_if(
        |e: &String| e == "retry",
        |_: String| pull::Pull::of(vec![99]),
    )
    .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Items(vec![99]));
    assert_eq!(piped.pull(), take::Take::End);
    // Non-matching failure: passes through.
    let source = pull::Pull::<i32, String>::fail("fatal".to_string());
    let piped = channel::catch_if(
        |e: &String| e == "retry",
        |_: String| pull::Pull::of(vec![99]),
    )
    .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Fail("fatal".to_string()));
}

#[test]
fn channel_catch_if_refines() {
    #[derive(Debug, Clone, PartialEq)]
    enum Problem {
        Retryable,
        Fatal,
    }
    // Refinement catches only the Retryable subset.
    let mut first = true;
    let source = pull::Pull::<i32, Problem>::from_fn(move || {
        if first {
            first = false;
            take::Take::Fail(Problem::Retryable)
        } else {
            take::Take::End
        }
    });
    let piped = channel::catch_if(
        |e: &Problem| matches!(e, Problem::Retryable),
        |_: Problem| pull::Pull::of(vec![7]),
    )
    .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Items(vec![7]));
    assert_eq!(piped.pull(), take::Take::End);
    // The Fatal subset passes through untouched.
    let source = pull::Pull::<i32, Problem>::fail(Problem::Fatal);
    let piped = channel::catch_if(
        |e: &Problem| matches!(e, Problem::Retryable),
        |_: Problem| pull::Pull::of(vec![7]),
    )
    .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Fail(Problem::Fatal));
}

#[test]
fn channel_decode_text_reassembles_split_characters() {
    // "é" is [0xC3, 0xA9]: split across two pulls.
    let source = pull::Pull::<u8, std::string::FromUtf8Error>::from_fn({
        let mut step = 0;
        move || {
            step += 1;
            match step {
                1 => take::Take::Items(vec![0xC3]),
                2 => take::Take::Items(vec![0xA9]),
                _ => take::Take::End,
            }
        }
    });
    let piped = channel::decode_text().run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Items(vec!["é".to_string()]));
    assert_eq!(piped.pull(), take::Take::End);
    // Invalid bytes fail (the driver skips the empty prefix internally).
    let source = pull::Pull::<u8, std::string::FromUtf8Error>::of(vec![0xFF]);
    let piped = channel::decode_text().run(source);
    let mut piped = piped;
    assert!(matches!(piped.pull(), take::Take::Fail(_)));
}

#[test]
fn sink_drain_discards() {
    let done = sink::drain().run(pull::Pull::<i32, String>::of(vec![1, 2]));
    assert_eq!(done, elm_effect::Exit::Success(()));
}
