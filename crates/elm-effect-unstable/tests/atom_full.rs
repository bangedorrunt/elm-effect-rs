//! `reactivity/Atom.ts` at the pin, beyond the lifecycle core: asynchronous
//! atoms, batching, families, and the `AsyncResult` states they read.
//!
//! Each test names the pin's own case as `Atom.test.ts :LINE`. The port's
//! asynchronous atoms settle on the scheduler a registry was built with; the
//! inline one settles on the read that started the fetch, and the deferred one
//! lets a test observe the waiting state before it drains.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::cause::Cause;
use elm_effect_unstable::reactivity::{
    AsyncResult, DeferredScheduler, Registry, TaskScheduler, async_atom, family, map, map_result,
    with_fallback,
};

fn deferred_registry() -> (Arc<Registry>, Arc<DeferredScheduler>) {
    let scheduler = Arc::new(DeferredScheduler::new());
    let registry = Registry::with_scheduler(scheduler.clone() as Arc<dyn TaskScheduler>);
    (registry, scheduler)
}

#[test]
fn async_atom_starts_initial_and_settles_on_the_scheduler() {
    // Atom.test.ts :372 — a fresh asynchronous atom reads as an initial
    // result while its work runs, and as a success once it answers.
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let value = async_atom(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        Ok::<i64, String>(42)
    });
    let (registry, scheduler) = deferred_registry();

    let first = registry.get(&value);
    assert!(first.is_initial(), "nothing has answered yet");
    assert!(first.is_waiting(), "the first run is in flight");
    assert_eq!(scheduler.pending(), 1);

    scheduler.run();
    match registry.get(&value) {
        AsyncResult::Success { value, .. } => assert_eq!(value, 42),
        other => panic!("expected a success, got {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one fetch, not one per read");
}

#[test]
fn async_atom_records_a_typed_failure() {
    // Atom.test.ts :1476 — a failed run is a Failure holding the cause, with
    // no previous success on the first attempt.
    let value = async_atom(|| Err::<i64, String>("boom".to_string()));
    let registry = Registry::make();
    match registry.get(&value) {
        AsyncResult::Failure {
            cause,
            previous_success,
            waiting,
        } => {
            assert_eq!(cause.find_error().map(String::as_str), Some("boom"));
            assert!(previous_success.is_none());
            assert!(!waiting);
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn async_atom_carries_the_previous_success_through_a_failure() {
    // Atom.test.ts :1476 — after a success, a failing refresh keeps the last
    // good value as `previousSuccess`.
    let attempt = Arc::new(AtomicUsize::new(0));
    let counted = attempt.clone();
    let value = async_atom(move || {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok::<i64, String>(7)
        } else {
            Err("late failure".to_string())
        }
    });
    let registry = Registry::make();
    assert!(matches!(registry.get(&value), AsyncResult::Success { .. }));

    registry.refresh(&value);
    match registry.get(&value) {
        AsyncResult::Failure {
            previous_success, ..
        } => {
            let previous = previous_success.expect("the success is carried");
            assert_eq!(previous.value, 7);
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn refresh_marks_a_success_waiting_and_discards_a_superseded_run() {
    // Atom.test.ts :544 — `Atom.refresh` re-runs the work; the value stays
    // visible but waiting, and only the newest run's answer is committed.
    let attempt = Arc::new(AtomicUsize::new(0));
    let counted = attempt.clone();
    let value = async_atom(move || {
        let n = counted.fetch_add(1, Ordering::SeqCst);
        Ok::<i64, String>(n as i64)
    });
    let (registry, scheduler) = deferred_registry();
    assert!(registry.get(&value).is_initial(), "the read starts the work");
    scheduler.run();
    assert!(registry.get(&value).is_success());

    registry.refresh(&value);
    let waiting = registry.get(&value);
    assert!(waiting.is_waiting(), "a refresh shows the old value waiting");
    assert_eq!(waiting.value(), Some(&0), "the old value is still there");

    scheduler.run();
    match registry.get(&value) {
        AsyncResult::Success { value, waiting, .. } => {
            assert_eq!(value, 1, "the newest run's answer wins");
            assert!(!waiting);
        }
        other => panic!("expected a success, got {other:?}"),
    }
}

#[test]
fn a_write_supersedes_an_in_flight_fetch() {
    // Atom.test.ts :2866 — writing an atom while its work runs leaves the
    // written value in place; the older answer is not committed.
    let value = async_atom(|| Ok::<i64, String>(1));
    let (registry, scheduler) = deferred_registry();
    assert!(registry.get(&value).is_initial());
    registry.set(&value, AsyncResult::success(99));
    scheduler.run();
    match registry.get(&value) {
        AsyncResult::Success { value, .. } => assert_eq!(value, 99),
        other => panic!("the write wins, got {other:?}"),
    }
}

#[test]
fn map_result_maps_only_the_success_value() {
    // Atom.test.ts :515 — `Atom.mapResult` maps a successful result and
    // leaves the other states alone.
    let source = async_atom(|| Ok::<i64, String>(3));
    let doubled = map_result(&source, |n: &i64| n * 2);
    let registry = Registry::make();
    match registry.get(&doubled) {
        AsyncResult::Success { value, .. } => assert_eq!(value, 6),
        other => panic!("expected a mapped success, got {other:?}"),
    }
}

#[test]
fn map_result_leaves_a_failure_failure() {
    // Atom.test.ts :528 — the double-map case: a failure passes through
    // untouched, including its carried previous success.
    let source = async_atom(|| Err::<i64, String>("nope".to_string()));
    let doubled = map_result(&source, |n: &i64| n * 2);
    let registry = Registry::make();
    match registry.get(&doubled) {
        AsyncResult::Failure { cause, .. } => {
            assert_eq!(cause.find_error().map(String::as_str), Some("nope"));
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn with_fallback_reads_the_fallback_until_the_primary_answers() {
    // Atom.test.ts :1446 — a fallback atom reads as a success while the
    // primary is initial, then switches to the primary's own success.
    let source = async_atom(|| Ok::<i64, String>(5));
    let fallback = with_fallback(&source, 0);
    let (registry, scheduler) = deferred_registry();

    let before = registry.get(&fallback);
    assert_eq!(before.value(), Some(&0), "the fallback stands in");
    assert!(before.is_success());

    scheduler.run();
    assert_eq!(registry.get(&fallback).value(), Some(&5));
}

#[test]
fn batching_defers_listener_calls_until_the_batch_ends() {
    // Atom.test.ts :1100 — two writes inside `Atom.batch` reach a listener
    // once, after the batch commits.
    let left = elm_effect_unstable::reactivity::Atom::make(0i64);
    let right = elm_effect_unstable::reactivity::Atom::make(0i64);
    let sum = map(&left, |n: &i64| *n);
    let registry = Registry::make();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    let _sub = registry.subscribe(&sum, move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });

    registry.batch(|| {
        registry.set(&left, 1);
        registry.set(&right, 2);
        assert_eq!(seen.load(Ordering::SeqCst), 0, "nothing during the batch");
    });
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "one notification for the batch"
    );
}

#[test]
fn nested_batches_notify_once() {
    // Atom.test.ts :1177 — the outermost batch commits; an inner one does
    // not fire early.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64);
    let registry = Registry::make();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    let _sub = registry.subscribe(&atom, move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    registry.batch(|| {
        registry.set(&atom, 1);
        registry.batch(|| {
            registry.set(&atom, 2);
        });
        assert_eq!(seen.load(Ordering::SeqCst), 0);
    });
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert_eq!(registry.get(&atom), 2);
}

#[test]
fn a_batch_listener_reads_the_committed_value() {
    // Atom.test.ts :1198 — the listener sees the final state, not the value
    // it was written to mid-batch.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64);
    let registry = Registry::make();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    let _sub = registry.subscribe(&atom, move |value: i64| {
        log.lock().unwrap().push(value);
    });
    registry.batch(|| {
        registry.set(&atom, 1);
        registry.set(&atom, 2);
    });
    assert_eq!(*seen.lock().unwrap(), vec![2]);
}

#[test]
fn modify_returns_the_first_half_and_writes_the_second() {
    // Atom.test.ts :1100's write path: `registry.modify` writes the second
    // tuple half and hands back the first.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64);
    let registry = Registry::make();
    let previous = registry.modify(&atom, |current: i64| (current, current + 5));
    assert_eq!(previous, 0);
    assert_eq!(registry.get(&atom), 5);
}

#[test]
fn update_writes_the_mapped_value() {
    // `registry.update` is `modify` without a return value.
    let atom = elm_effect_unstable::reactivity::Atom::make(2i64);
    let registry = Registry::make();
    registry.update(&atom, |current: i64| current * 3);
    assert_eq!(registry.get(&atom), 6);
}

#[test]
fn a_mounted_atom_survives_the_disposal_pass_and_reverts_after_cancel() {
    // Atom.test.ts :73 — a subscription keeps the node mounted; cancel
    // releases it, and the next pass restores the initial value.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64);
    let registry = Registry::make();
    let sub = registry.mount(&atom);
    registry.set(&atom, 5);
    registry.tick();
    assert_eq!(registry.get(&atom), 5, "mounted nodes are kept");
    sub.cancel();
    registry.tick();
    assert_eq!(registry.get(&atom), 0);
}

#[test]
fn get_result_answers_none_until_the_work_settles() {
    // Atom.test.ts :2900 — the result read waits for a settled state; before
    // that there is nothing to give.
    let value = async_atom(|| Ok::<i64, String>(8));
    let (registry, scheduler) = deferred_registry();
    assert!(registry.get_result(&value).is_none());
    scheduler.run();
    assert_eq!(registry.get_result(&value), Some(Ok(8)));
}

#[test]
fn get_result_hands_back_the_failure_cause() {
    // Atom.test.ts :1523 — a failed result surfaces its cause.
    let value = async_atom(|| Err::<i64, String>("bad".to_string()));
    let registry = Registry::make();
    match registry.get_result(&value) {
        Some(Err(cause)) => assert_eq!(cause.find_error().map(String::as_str), Some("bad")),
        other => panic!("expected the failure cause, got {other:?}"),
    }
}

#[test]
fn family_memoizes_one_atom_per_key() {
    // Atom.test.ts :1048 — a family hands out the same atom for the same
    // key, and different atoms for different keys.
    let users = family(|id: &u32| elm_effect_unstable::reactivity::Atom::make(*id as i64));
    let first = users.get(&1);
    let again = users.get(&1);
    let other = users.get(&2);
    assert_eq!(first.label(), again.label(), "the same handle is reused");
    let registry = Registry::make();
    registry.set(&first, 100);
    assert_eq!(registry.get(&again), 100);
    assert_eq!(registry.get(&other), 2, "keys do not share a node");
}

#[test]
fn a_label_is_carried_for_debugging() {
    // Atom.test.ts :1094 — `Atom.withLabel` tags the atom; the registry does
    // not change its behavior.
    let atom = elm_effect_unstable::reactivity::Atom::make(1i64).with_label("counter");
    assert_eq!(atom.label().as_deref(), Some("counter"));
    let registry = Registry::make();
    assert_eq!(registry.get(&atom), 1);
}

#[test]
fn an_initial_value_overrides_what_a_fresh_node_starts_from() {
    // Atom.test.ts :1294 — `Atom.initialValue` seeds the node; the atom's
    // own initial value is what a later release reverts to.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64).initial_value(9);
    let registry = Registry::make();
    assert_eq!(registry.get(&atom), 9);
    registry.tick();
    assert_eq!(registry.get(&atom), 9, "a release re-seeds the override");
}

#[test]
fn idle_ttl_holds_an_unlistened_node_until_it_expires() {
    // Atom.test.ts :1326 — `idleTTL` keeps an unused atom alive for a while,
    // so a remount does not lose its value.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64)
        .idle_ttl(std::time::Duration::from_secs(60));
    let registry = Registry::make();
    let sub = registry.mount(&atom);
    registry.set(&atom, 4);
    sub.cancel();
    registry.tick();
    assert_eq!(registry.get(&atom), 4, "the TTL has not run out");
}

#[test]
fn auto_dispose_undoes_keep_alive() {
    // Atom.test.ts :53/:62 — `Atom.autoDispose` returns a kept atom to the
    // releasable set.
    let atom = elm_effect_unstable::reactivity::Atom::make(0i64)
        .keep_alive()
        .auto_dispose();
    let registry = Registry::make();
    registry.set(&atom, 3);
    registry.tick();
    assert_eq!(registry.get(&atom), 0);
}

#[test]
fn set_lazy_false_rebuilds_a_derived_atom_without_a_read() {
    // Atom.test.ts :1656 — `Atom.setLazy(false)` makes the derived atom
    // rebuild as soon as its source changes.
    let builds = Arc::new(AtomicUsize::new(0));
    let counted = builds.clone();
    let source = elm_effect_unstable::reactivity::Atom::make(1i64);
    let derived = map(&source, move |n: &i64| {
        counted.fetch_add(1, Ordering::SeqCst);
        n * 10
    })
    .set_lazy(false);
    let registry = Registry::make();
    assert_eq!(registry.get(&derived), 10);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    registry.set(&source, 2);
    assert_eq!(
        builds.load(Ordering::SeqCst),
        2,
        "the rebuild happened at the write"
    );
    assert_eq!(registry.get(&derived), 20);
}

#[test]
fn async_results_map_and_carry_their_waiting_flag() {
    // reactivity/AsyncResult.ts — `map` keeps the state, `waiting` marks it,
    // and a failure maps its carried success too.
    let success: AsyncResult<i64, String> = AsyncResult::success(2).mark_waiting();
    assert!(success.is_waiting());
    let mapped = success.map(|n| n * 3);
    assert_eq!(mapped.value(), Some(&6));
    assert!(mapped.is_waiting());

    let failure = AsyncResult::<i64, String>::failure_with(
        Cause::fail("e".to_string()),
        Some(AsyncResult::success_state(4)),
    );
    let mapped = failure.map(|n| n + 1);
    assert_eq!(mapped.value(), Some(&5), "the carried success is mapped");
    assert_eq!(mapped.error().map(String::as_str), Some("e"));
}

#[test]
fn async_result_all_stops_at_the_first_non_success() {
    // reactivity/AsyncResult.ts — `all` collects successes, reports waiting
    // when any input waits, and returns the first failure.
    let all = AsyncResult::<i64, String>::all(&[
        AsyncResult::success(1),
        AsyncResult::success_waiting(2),
    ]);
    assert_eq!(all.value(), Some(&vec![1, 2]));
    assert!(all.is_waiting());

    let stopped = AsyncResult::<i64, String>::all(&[
        AsyncResult::success(1),
        AsyncResult::initial(),
    ]);
    assert!(stopped.is_initial());

    let failed = AsyncResult::<i64, String>::all(&[
        AsyncResult::success(1),
        AsyncResult::fail("no".to_string()),
    ]);
    assert!(failed.is_failure());
    assert_eq!(failed.error().map(String::as_str), Some("no"));
}

#[test]
fn match_with_waiting_gives_the_waiting_branch_priority() {
    // reactivity/AsyncResult.ts — `matchWithWaiting` renders one waiting
    // branch whatever the variant holds.
    let waiting = AsyncResult::<i64, String>::success_waiting(1);
    let rendered = waiting.match_with_waiting(
        |_| "waiting",
        |_| "success",
        |_| "error",
        || "defect",
    );
    assert_eq!(rendered, "waiting");

    let failed = AsyncResult::<i64, String>::fail("bad".to_string());
    let rendered = failed.match_with_waiting(
        |_| "waiting".to_string(),
        |_| "success".to_string(),
        |error| error.clone(),
        || "defect".to_string(),
    );
    assert_eq!(rendered, "bad");
}

#[test]
fn a_non_typed_failure_renders_as_a_defect() {
    // reactivity/AsyncResult.ts — a defect cause has no typed error, so the
    // defect branch answers.
    let defect = AsyncResult::<i64, String>::failure(Cause::die(
        elm_effect::cause::Defect::new("exploded"),
    ));
    let rendered = defect.match_with_waiting(
        |_| "waiting",
        |_| "success",
        |_| "error",
        || "defect",
    );
    assert_eq!(rendered, "defect");
}

#[test]
fn registries_do_not_share_derived_values() {
    // Atom.test.ts :2737 — the same atom in two registries keeps two
    // independent values.
    let source = elm_effect_unstable::reactivity::Atom::make(1i64);
    let derived = map(&source, |n: &i64| n + 1);
    let first = Registry::make();
    let second = Registry::make();
    first.set(&source, 10);
    assert_eq!(first.get(&derived), 11);
    assert_eq!(second.get(&derived), 2);
}
