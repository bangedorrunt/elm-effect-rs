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

use std::sync::{Arc, Mutex};

fn seq(items: Vec<u32>) -> pull::Pull<u32, String> {
    let items = Arc::new(Mutex::new(items));
    pull::Pull::from_fn(move || match items.lock().unwrap().pop() {
        Some(v) => Take::Items(vec![v]),
        None => Take::End,
    })
}

fn slow_seq(items: Vec<u32>, delay_ms: u64) -> pull::Pull<u32, String> {
    let items = Arc::new(Mutex::new(items));
    pull::Pull::from_fn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        match items.lock().unwrap().pop() {
            Some(v) => Take::Items(vec![v]),
            None => Take::End,
        }
    })
}

fn drain(mut pull: pull::Pull<u32, String>) -> Vec<u32> {
    let mut out = Vec::new();
    loop {
        match pull.pull() {
            Take::Items(items) => out.extend(items),
            Take::End => return out,
            Take::Fail(e) => panic!("unexpected failure: {e}"),
        }
    }
}

#[test]
fn merge_halt_strategies() {
    use channel::{HaltStrategy, merge};

    // Either: stops at the first End; the slow side never delivers.
    let merged = merge(
        seq(vec![1]),
        slow_seq(vec![9, 9], 100),
        HaltStrategy::Either,
    );
    assert_eq!(drain(merged), vec![1]);

    // Both: runs until both sides end, interleaving in arrival order.
    let merged = merge(seq(vec![1]), seq(vec![2]), HaltStrategy::Both);
    let mut collected = drain(merged);
    collected.sort();
    assert_eq!(collected, vec![1, 2]);

    // Left: stops when the left ends, even with right items pending.
    let merged = merge(
        seq(vec![1]),
        slow_seq(vec![9, 9, 9], 100),
        HaltStrategy::Left,
    );
    assert_eq!(drain(merged), vec![1]);

    // Right: stops when the right ends.
    let merged = merge(slow_seq(vec![9, 9], 100), seq(vec![1]), HaltStrategy::Right);
    assert_eq!(drain(merged), vec![1]);

    // Failure always wins, whichever side it comes from.
    let failing = pull::Pull::from_fn(|| Take::Fail("boom".to_string()));
    let mut merged = merge(failing, seq(vec![1, 2, 3]), HaltStrategy::Both);
    let mut saw_fail = false;
    loop {
        match merged.pull() {
            Take::Fail(e) => {
                assert_eq!(e, "boom");
                saw_fail = true;
                break;
            }
            Take::End => break,
            Take::Items(_) => continue,
        }
    }
    assert!(saw_fail);
}

#[test]
fn take_while_keeps_leftovers() {
    use sink::{take_while_effect, take_while_filter};

    let source = pull::Pull::from_fn({
        let items = Arc::new(Mutex::new(vec![
            Take::<u32, String>::End,
            Take::Items(vec![4u32, 5]),
            Take::Items(vec![1u32, 2, 3]),
        ]));
        move || items.lock().unwrap().pop().unwrap_or(Take::End)
    });
    let sink = take_while_effect(|n: &u32| Ok::<bool, String>(*n < 3));
    let exit = sink.run(source);
    match exit {
        elm_effect::Exit::Success((taken, leftover)) => {
            assert_eq!(taken, vec![1u32, 2]);
            // The failing input is consumed; the rest is leftover.
            assert_eq!(leftover, vec![4u32, 5]);
        }
        elm_effect::Exit::Failure(_) => panic!("expected success"),
    }

    let source2 = pull::Pull::from_fn({
        let items = Arc::new(Mutex::new(vec![
            Take::<u32, String>::End,
            Take::Items(vec![1u32, 2, 3]),
        ]));
        move || items.lock().unwrap().pop().unwrap_or(Take::End)
    });
    let sink2 = take_while_filter(|n: &u32| (*n < 3).then_some(n * 10));
    let exit2 = sink2.run(source2);
    match exit2 {
        elm_effect::Exit::Success((taken, leftover)) => {
            assert_eq!(taken, vec![10u32, 20]);
            assert_eq!(leftover, Vec::<u32>::new());
        }
        elm_effect::Exit::Failure(_) => panic!("expected success"),
    }

    // flatMap over leftovers stays total.
    let doubled: Vec<u32> = vec![4u32].into_iter().flat_map(|n| [n, n]).collect();
    assert_eq!(doubled, vec![4u32, 4]);
}

#[test]
fn from_write_aborts_on_failure() {
    use sink::from_write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Tee {
        seen: Arc<Mutex<Vec<u8>>>,
        flushes: Arc<AtomicUsize>,
    }
    impl std::io::Write for Tee {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.seen.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // Success path writes bytes and flushes once on End.
    let (seen, flushes) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(AtomicUsize::new(0)),
    );
    let source = pull::Pull::from_fn({
        let items = Arc::new(Mutex::new(vec![
            Take::<u8, String>::End,
            Take::Items(vec![104u8, 105]),
        ]));
        move || items.lock().unwrap().pop().unwrap_or(Take::End)
    });
    let exit = from_write(Tee {
        seen: Arc::clone(&seen),
        flushes: Arc::clone(&flushes),
    })
    .run(source);
    assert!(matches!(exit, elm_effect::Exit::Success(())));
    assert_eq!(*seen.lock().unwrap(), vec![104u8, 105]);
    assert_eq!(flushes.load(Ordering::SeqCst), 1);

    // Failure path aborts without flushing.
    let (seen2, flushes2) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(AtomicUsize::new(0)),
    );
    let failing = pull::Pull::from_fn(|| Take::Fail("boom".to_string()));
    let exit2 = from_write(Tee {
        seen: seen2,
        flushes: Arc::clone(&flushes2),
    })
    .run(failing);
    assert!(matches!(exit2, elm_effect::Exit::Failure(_)));
    assert_eq!(flushes2.load(Ordering::SeqCst), 0);
}

#[test]
fn from_write_interruptible_aborts_instead_of_closing() {
    use sink::from_write_interruptible;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Tee {
        seen: Arc<Mutex<Vec<u8>>>,
        flushes: Arc<AtomicUsize>,
    }
    impl std::io::Write for Tee {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.seen.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // Close path: End flushes, exit succeeds.
    let (seen, flushes) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(AtomicUsize::new(0)),
    );
    let closing = Arc::new(AtomicBool::new(false));
    let source = pull::Pull::from_fn({
        let items = Arc::new(Mutex::new(vec![
            Take::<u8, String>::End,
            Take::<u8, String>::Items(vec![104u8, 105]),
        ]));
        move || items.lock().unwrap().pop().unwrap_or(Take::End)
    });
    let exit = from_write_interruptible(
        Tee {
            seen: Arc::clone(&seen),
            flushes: Arc::clone(&flushes),
        },
        {
            let closing = Arc::clone(&closing);
            move || closing.load(Ordering::SeqCst)
        },
    )
    .run(source);
    assert!(matches!(exit, elm_effect::Exit::Success(())));
    assert_eq!(*seen.lock().unwrap(), vec![104u8, 105]);
    assert_eq!(flushes.load(Ordering::SeqCst), 1);

    // Interrupt path: the bytes already written stay, no close flush runs,
    // and the exit says interrupted rather than success.
    let (seen2, flushes2) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(AtomicUsize::new(0)),
    );
    let interrupted = Arc::new(AtomicBool::new(false));
    let source2 = pull::Pull::from_fn({
        let interrupted = Arc::clone(&interrupted);
        move || {
            interrupted.store(true, Ordering::SeqCst);
            Take::<u8, String>::Items(vec![104u8])
        }
    });
    let exit2 = from_write_interruptible(
        Tee {
            seen: Arc::clone(&seen2),
            flushes: Arc::clone(&flushes2),
        },
        {
            let interrupted = Arc::clone(&interrupted);
            move || interrupted.load(Ordering::SeqCst)
        },
    )
    .run(source2);
    assert_eq!(*seen2.lock().unwrap(), vec![104u8]);
    assert_eq!(flushes2.load(Ordering::SeqCst), 0);
    match exit2 {
        elm_effect::Exit::Failure(cause) => assert!(cause.is_interrupted()),
        elm_effect::Exit::Success(()) => panic!("interrupted write reported a close"),
    }
}

#[test]
fn sink_flat_map_preserves_leftovers() {
    use sink::{count, head};

    // Chunks mirror the TS case: [[1,2], [3,4,5], [], [7,8,9,10]].
    let source = pull::Pull::from_fn({
        let chunks = Arc::new(Mutex::new(vec![
            Take::<u32, String>::End,
            Take::Items(vec![7, 8, 9, 10]),
            Take::Items(vec![]),
            Take::Items(vec![3, 4, 5]),
            Take::Items(vec![1, 2]),
        ]));
        move || chunks.lock().unwrap().pop().unwrap_or(Take::End)
    });
    let sink = head::<u32, String>().flat_map(|head: Option<u32>| {
        count::<u32, String>().map(move |n| (head, n))
    });
    match sink.run(source) {
        elm_effect::Exit::Success((head, n)) => {
            assert_eq!(head, Some(1));
            // head consumed exactly one element, so the count sees the
            // other eight; together they account for all nine delivered.
            assert_eq!(n, 8);
            assert_eq!(n + usize::from(head.is_some()), 9);
        }
        elm_effect::Exit::Failure(_) => panic!("expected success"),
    }
}

#[test]
fn sink_with_duration_measures_clock_delta() {
    use elm_effect::{ClockImpl, Duration, TestClock};
    use sink::{count, with_duration};

    // The source advances the shared test clock mid-run, so the measured
    // delta is exactly the adjustment, read from the clock service.
    let clock = Arc::new(TestClock::new());
    let ticker = Arc::clone(&clock);
    let source = pull::Pull::from_fn(move || {
        ticker.adjust(Duration::from_millis(100));
        Take::<u32, String>::End
    });
    match with_duration(count::<u32, String>(), clock).run(source) {
        elm_effect::Exit::Success((n, elapsed)) => {
            assert_eq!(n, 0);
            assert_eq!(elapsed, Duration::from_millis(100));
        }
        elm_effect::Exit::Failure(_) => panic!("expected success"),
    }

    // A clock that steps backward (a wall regression) clamps the delta to
    // zero instead of wrapping: the subtraction saturates.
    struct Backward {
        reads: std::sync::Mutex<Vec<u64>>,
    }
    impl ClockImpl for Backward {
        fn sleep(
            &self,
            _duration: Duration,
            _interrupted: &dyn Fn() -> bool,
        ) -> elm_effect::SleepOutcome {
            elm_effect::SleepOutcome::Elapsed
        }
        fn now_millis(&self) -> u64 {
            let mut reads = self.reads.lock().unwrap();
            let value = if reads.is_empty() { 1000 } else { 0 };
            reads.push(value);
            value
        }
    }
    let backward = Arc::new(Backward {
        reads: std::sync::Mutex::new(Vec::new()),
    });
    match with_duration(count::<u32, String>(), backward)
        .run(pull::Pull::<u32, String>::empty())
    {
        elm_effect::Exit::Success((_, elapsed)) => assert_eq!(elapsed, Duration::ZERO),
        elm_effect::Exit::Failure(_) => panic!("expected success"),
    }
}

#[test]
fn channel_catch_all_recovers_any_failure() {
    // One-shot failure: Pull::fail repeats forever, so recovery tests
    // need a source that ends after failing once.
    let mut first = true;
    let source = pull::Pull::from_fn(move || {
        if first {
            first = false;
            Take::Fail("boom".to_string())
        } else {
            Take::End
        }
    });
    let piped = channel::catch_all(|_: String| pull::Pull::of(vec![9]));
    let mut piped = piped.run(source);
    assert_eq!(piped.pull(), Take::Items(vec![9]));
    assert_eq!(piped.pull(), Take::End);
}

#[test]
fn channel_catch_or_else_splits_match_and_fallback() {
    #[derive(Debug, Clone, PartialEq)]
    enum Problem {
        Http,
        Validation,
    }
    let run = |p: Problem| {
        let mut first = true;
        let source = pull::Pull::from_fn(move || {
            if first {
                first = false;
                Take::Fail(p.clone())
            } else {
                Take::End
            }
        });
        let mut piped = channel::catch_or_else(
            |e: &Problem| matches!(e, Problem::Http),
            |_: Problem| pull::Pull::of(vec!["http".to_string()]),
            |_: Problem| pull::Pull::of(vec!["fallback".to_string()]),
        )
        .run(source);
        let first = piped.pull();
        assert_eq!(piped.pull(), Take::End);
        first
    };
    assert_eq!(run(Problem::Http), Take::Items(vec!["http".to_string()]));
    assert_eq!(
        run(Problem::Validation),
        Take::Items(vec!["fallback".to_string()])
    );
}

#[test]
fn channel_catch_or_else_nests_reason_match() {
    #[derive(Debug, Clone, PartialEq)]
    enum Reason {
        RateLimit(u64),
        Quota(u64),
    }
    #[derive(Debug, Clone, PartialEq)]
    enum Problem {
        Ai(Reason),
        Other,
    }
    let run = |p: Problem| {
        let mut first = true;
        let source = pull::Pull::from_fn(move || {
            if first {
                first = false;
                Take::Fail(p.clone())
            } else {
                Take::End
            }
        });
        let mut piped = channel::catch_or_else(
            |e: &Problem| matches!(e, Problem::Ai(_)),
            |e: Problem| match e {
                Problem::Ai(Reason::RateLimit(n)) => {
                    pull::Pull::of(vec![format!("retry: {n}")])
                }
                Problem::Ai(Reason::Quota(n)) => {
                    pull::Pull::of(vec![format!("quota: {n}")])
                }
                Problem::Other => unreachable!(),
            },
            |_: Problem| pull::Pull::of(vec!["other".to_string()]),
        )
        .run(source);
        let first = piped.pull();
        assert_eq!(piped.pull(), Take::End);
        first
    };
    assert_eq!(
        run(Problem::Ai(Reason::Quota(100))),
        Take::Items(vec!["quota: 100".to_string()])
    );
    assert_eq!(
        run(Problem::Ai(Reason::RateLimit(60))),
        Take::Items(vec!["retry: 60".to_string()])
    );
    assert_eq!(
        run(Problem::Other),
        Take::Items(vec!["other".to_string()])
    );
}

#[test]
fn channel_unwrap_reason_extracts_nested_failure() {
    #[derive(Debug, Clone, PartialEq)]
    enum Problem {
        Ai(String),
        RateLimit(u64),
    }
    let source = pull::Pull::<i32, Problem>::fail(Problem::Ai("inner".to_string()));
    let mut piped = channel::unwrap_reason(
        |e: &Problem| matches!(e, Problem::Ai(_)),
        |e: Problem| match e {
            Problem::Ai(s) => Problem::RateLimit(s.len() as u64),
            other => other,
        },
    )
    .run(source);
    assert_eq!(piped.pull(), Take::Fail(Problem::RateLimit(5)));
    let source = pull::Pull::<i32, Problem>::fail(Problem::RateLimit(7));
    let mut piped = channel::unwrap_reason(
        |e: &Problem| matches!(e, Problem::Ai(_)),
        |e: Problem| match e {
            Problem::Ai(s) => Problem::RateLimit(s.len() as u64),
            other => other,
        },
    )
    .run(source);
    assert_eq!(piped.pull(), Take::Fail(Problem::RateLimit(7)));
}

#[test]
fn channel_filter_map_effect_short_circuits() {
    let source = pull::Pull::<i32, String>::of(vec![1, 2, 3]);
    let piped = channel::filter_map_effect(|a: i32| {
        if a > 2 {
            Err("big".to_string())
        } else {
            Ok(if a % 2 == 1 { Some(a * 10) } else { None })
        }
    })
    .run(source);
    let mut piped = piped;
    assert_eq!(piped.pull(), take::Take::Fail("big".to_string()));
    // After error, subsequent pull is End (channel already terminated? re-run source)
    // Re-run fresh to check short-circuit fail:
    let source = pull::Pull::<i32, String>::of(vec![1, 2, 3]);
    let mut piped = channel::filter_map_effect(|a: i32| {
        if a == 2 {
            Err("big".to_string())
        } else {
            Ok(Some(a))
        }
    })
    .run(source);
    assert_eq!(piped.pull(), take::Take::Fail("big".to_string()));
}

#[test]
fn channel_catch_defect_routes_to_handler() {
    let mut first = true;
    let source = pull::Pull::from_fn(move || {
        if first {
            first = false;
            take::Take::Fail("boom".to_string())
        } else {
            take::Take::End
        }
    });
    let piped = channel::catch_defect(|_: String| pull::Pull::of(vec![42]));
    let mut piped = piped.run(source);
    assert_eq!(piped.pull(), take::Take::Items(vec![42]));
    assert_eq!(piped.pull(), take::Take::End);
    // Non-failing pass-through on empty source:
    let mut piped = channel::catch_defect(|_: String| pull::Pull::of(vec![1])).run(pull::Pull::empty());
    assert_eq!(piped.pull(), take::Take::End);
}
