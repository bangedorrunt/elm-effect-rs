//! `Atom.fn` and `Atom.pull` at the pin.
//!
//! A callable atom describes work and accepts its outcome; it never
//! interprets. `call` hands back the `Effect` and an id, the application's
//! runtime edge runs it, and `resolve` writes the `Exit` back under that id.
//! That keeps this crate free of a second runtime and makes the state machine
//! testable without one.

use std::sync::Arc;

use elm_effect::Cause;
use elm_effect::effect::Effect;
use elm_effect::pull::Pull;
use elm_effect::take::Take;
use elm_effect_unstable::reactivity::{AsyncResult, Registry, fn_atom, pull_atom};

fn inline() -> Arc<Registry> {
    Registry::make()
}

#[test]
fn a_fn_atom_starts_initial_and_answers_after_a_call() {
    // Atom.test.ts :1428 — `Atom.fn` reads as initial until it is called,
    // then carries the effect's success.
    let counter = fn_atom(|value: i64| Effect::<i64, String>::succeed(value * 2));
    let registry = inline();
    assert!(registry.get(counter.atom()).is_initial());

    let call = counter.call(&registry, 21);
    let _: Effect<i64, String> = call.effect;
    assert!(
        registry.get(counter.atom()).is_waiting(),
        "a call in flight is waiting"
    );

    counter.resolve(&registry, call.id, elm_effect::Exit::Success(42));
    match registry.get(counter.atom()) {
        AsyncResult::Success { value, waiting, .. } => {
            assert_eq!(value, 42);
            assert!(!waiting);
        }
        other => panic!("expected the call's success, got {other:?}"),
    }
}

#[test]
fn a_fn_atom_records_a_typed_failure() {
    // Atom.test.ts :1437 — an effect that fails leaves a Failure holding the
    // typed error.
    let failing = fn_atom(|value: i64| {
        if value < 0 {
            Effect::<i64, String>::fail("negative".to_string())
        } else {
            Effect::<i64, String>::succeed(value)
        }
    });
    let registry = inline();
    let call = failing.call(&registry, -1);
    failing.resolve(
        &registry,
        call.id,
        elm_effect::Exit::Failure(Cause::fail("negative".to_string())),
    );
    match registry.get(failing.atom()) {
        AsyncResult::Failure { cause, .. } => {
            assert_eq!(cause.find_error().map(String::as_str), Some("negative"));
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn a_failure_carries_the_previous_success() {
    // Atom.test.ts :1476 — a later failing call keeps the last good value.
    let calls = fn_atom(|value: i64| Effect::<i64, String>::succeed(value));
    let registry = inline();
    let first = calls.call(&registry, 5);
    calls.resolve(&registry, first.id, elm_effect::Exit::Success(5));

    let second = calls.call(&registry, 6);
    calls.resolve(
        &registry,
        second.id,
        elm_effect::Exit::Failure(Cause::fail("late".to_string())),
    );
    match registry.get(calls.atom()) {
        AsyncResult::Failure {
            previous_success, ..
        } => assert_eq!(previous_success.expect("the success is carried").value, 5),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn the_latest_call_wins_when_two_runs_overlap() {
    // Atom.test.ts :455 — concurrent calls keep the newest answer; an older
    // run that settles later is discarded.
    let calls = fn_atom(|value: i64| Effect::<i64, String>::succeed(value));
    let registry = inline();
    assert!(registry.get(calls.atom()).is_initial());

    let first = calls.call(&registry, 1);
    let second = calls.call(&registry, 2);
    calls.resolve(&registry, first.id, elm_effect::Exit::Success(1));
    assert!(
        registry.get(calls.atom()).is_waiting(),
        "the superseded answer changes nothing"
    );
    calls.resolve(&registry, second.id, elm_effect::Exit::Success(2));
    match registry.get(calls.atom()) {
        AsyncResult::Success { value, .. } => assert_eq!(value, 2, "the later call wins"),
        other => panic!("expected a success, got {other:?}"),
    }
}

#[test]
fn reset_returns_the_atom_to_initial() {
    // Atom.test.ts :2850's Reset control: writing `Reset` clears the argument
    // and the result.
    let calls = fn_atom(|value: i64| Effect::<i64, String>::succeed(value));
    let registry = inline();
    let call = calls.call(&registry, 7);
    calls.resolve(&registry, call.id, elm_effect::Exit::Success(7));
    assert!(registry.get(calls.atom()).is_success());
    calls.reset(&registry);
    assert!(registry.get(calls.atom()).is_initial());
}

#[test]
fn interrupt_exposes_an_interruption_failure() {
    // Atom.test.ts :2850 — the `Interrupt` control cancels the run and leaves
    // an interrupted failure behind.
    let calls = fn_atom(|value: i64| Effect::<i64, String>::succeed(value));
    let registry = inline();
    let call = calls.call(&registry, 1);
    calls.interrupt(&registry);
    calls.resolve(&registry, call.id, elm_effect::Exit::Success(1));
    match registry.get(calls.atom()) {
        AsyncResult::Failure { cause, .. } => {
            assert!(cause.is_interrupted(), "the answer after Interrupt is dropped");
        }
        other => panic!("expected an interruption, got {other:?}"),
    }
}

#[test]
fn a_pull_atom_takes_the_whole_source_and_reports_done() {
    // Atom.test.ts :913 — pulling a finite source yields its items, and the
    // pull that sees `End` reports done.
    let pulled = pull_atom(Pull::<i64, String>::of(vec![1i64, 2, 3]), true);
    let registry = inline();
    pulled.pull(&registry);
    match registry.get(pulled.atom()) {
        AsyncResult::Success { value, .. } => {
            assert_eq!(value.items, vec![1, 2, 3]);
            assert!(!value.done, "the chunk arrives before the End is seen");
        }
        other => panic!("expected the pulled chunk, got {other:?}"),
    }
    pulled.pull(&registry);
    match registry.get(pulled.atom()) {
        AsyncResult::Success { value, .. } => {
            assert!(value.done);
            assert_eq!(value.items, vec![1, 2, 3], "End adds nothing");
        }
        other => panic!("expected the end chunk, got {other:?}"),
    }
    assert!(pulled.is_done());
}

#[test]
fn a_pull_atom_accumulates_across_chunks() {
    // Atom.test.ts :1023 — each pull takes the next chunk; accumulation keeps
    // the earlier items in the result.
    let mut step = 0;
    let source = Pull::<i64, String>::from_fn(move || -> Take<i64, String> {
        step += 1;
        match step {
            1 => Take::Items(vec![1i64]),
            2 => Take::Items(vec![2i64]),
            _ => Take::End,
        }
    });
    let pulled = pull_atom(source, true);
    let registry = inline();

    pulled.pull(&registry);
    match registry.get(pulled.atom()) {
        AsyncResult::Success { value, .. } => {
            assert_eq!(value.items, vec![1]);
            assert!(!value.done);
        }
        other => panic!("expected the first chunk, got {other:?}"),
    }

    pulled.pull(&registry);
    match registry.get(pulled.atom()) {
        AsyncResult::Success { value, .. } => {
            assert_eq!(value.items, vec![1, 2]);
            assert!(!value.done);
        }
        other => panic!("expected the accumulated chunk, got {other:?}"),
    }

    pulled.pull(&registry);
    match registry.get(pulled.atom()) {
        AsyncResult::Success { value, .. } => {
            assert!(value.done);
            assert_eq!(value.items, vec![1, 2], "End adds nothing");
        }
        other => panic!("expected the end chunk, got {other:?}"),
    }
    assert!(pulled.is_done());
}

#[test]
fn without_accumulation_each_pull_replaces_the_chunk() {
    // Atom.test.ts :1023's `disableAccumulation`: the result holds only the
    // chunk just pulled.
    let mut step = 0;
    let source = Pull::<i64, String>::from_fn(move || -> Take<i64, String> {
        step += 1;
        match step {
            1 => Take::Items(vec![1i64, 2]),
            _ => Take::Items(vec![3i64]),
        }
    });
    let pulled = pull_atom(source, false);
    let registry = inline();
    pulled.pull(&registry);
    pulled.pull(&registry);
    match registry.get(pulled.atom()) {
        AsyncResult::Success { value, .. } => assert_eq!(value.items, vec![3]),
        other => panic!("expected the newest chunk, got {other:?}"),
    }
}

#[test]
fn a_fn_atom_write_inside_a_batch_notifies_once_at_commit() {
    // Atom.test.ts :1277 — a call and its resolution inside one batch reach a
    // listener once, with the committed result.
    let calls = fn_atom(|value: i64| Effect::<i64, String>::succeed(value));
    let registry = inline();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let _sub = registry.subscribe(calls.atom(), move |value: AsyncResult<i64, String>| {
        log.lock().unwrap().push(value);
    });

    registry.batch(|| {
        let call = calls.call(&registry, 4);
        calls.resolve(&registry, call.id, elm_effect::Exit::Success(9));
    });

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one notification for the batch");
    assert_eq!(seen[0].value(), Some(&9), "and it holds the committed value");
}
