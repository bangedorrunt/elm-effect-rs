//! Workflow module fence: stored-and-replayed activities, durable clock /
//! deferred / queue, compensation on failure and cancel only. No collector,
//! no transport: everything here runs in-process.

use elm_effect_unstable::workflow::{
    Activity, ActivityStore, CompScope, DurableClock, DurableDeferred, DurableQueue, compensate,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

#[test]
fn activity_result_is_stored_and_replayed() {
    let runs = counter();
    let runs_body = runs.clone();
    let store = ActivityStore::new();
    let activity = Activity::make("key-1", move || {
        runs_body.fetch_add(1, Ordering::SeqCst);
        Ok::<i32, String>(7)
    });
    assert_eq!(store.execute(&activity), Ok(7));
    assert_eq!(store.execute(&activity), Ok(7));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(store.runs("key-1"), 1);
}

#[test]
fn activity_retry_replays_until_budget() {
    let runs = counter();
    let runs_body = runs.clone();
    let store = ActivityStore::new();
    let activity = Activity::make("flaky", move || {
        let n = runs_body.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err("flaky".to_owned())
        } else {
            Ok(9)
        }
    });
    assert_eq!(store.retry(&activity, 3), Ok(9));
    assert_eq!(runs.load(Ordering::SeqCst), 3);
    let always = Activity::make("down", || Err::<i32, String>("down".to_owned()));
    assert_eq!(store.retry(&always, 2), Err("down".to_owned()));
}

#[test]
fn durable_clock_waits_and_advances() {
    let clock = DurableClock::new(1_000);
    assert_eq!(clock.now_ms(), 1_000);
    clock.advance(500);
    assert_eq!(clock.now_ms(), 1_500);
    assert!(clock.wait_until(1_500, &|| false));
    assert!(!clock.wait_until(9_999, &|| true));
    let moved = Arc::new(DurableClock::new(0));
    let moved_body = moved.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        moved_body.advance(100);
    });
    assert!(moved.wait_until(100, &|| false));
}

#[test]
fn durable_deferred_completes_once_and_waits() {
    let cell = Arc::new(DurableDeferred::new());
    assert!(!cell.is_done());
    let waiter = cell.clone();
    let handle = std::thread::spawn(move || waiter.await_value(&|| false).expect("completes"));
    std::thread::sleep(Duration::from_millis(20));
    assert!(cell.complete(41));
    assert!(!cell.complete(42));
    assert_eq!(handle.join().expect("thread"), 41);
    assert!(cell.is_done());
    assert_eq!(cell.await_value(&|| true), Some(41));
}

#[test]
fn durable_queue_fifo_and_shutdown() {
    let queue = Arc::new(DurableQueue::new());
    assert!(queue.is_empty());
    queue.offer(1);
    queue.offer(2);
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.take(&|| false), Some(1));
    assert_eq!(queue.take(&|| false), Some(2));
    assert!(queue.is_empty());
    let waiter = queue.clone();
    let handle = std::thread::spawn(move || waiter.take(&|| false));
    std::thread::sleep(Duration::from_millis(20));
    queue.offer(7);
    assert_eq!(handle.join().expect("thread"), Some(7));
    queue.shutdown();
    assert!(queue.is_shutdown());
    assert_eq!(queue.take(&|| false), None);
}

#[test]
fn compensation_fires_on_failure_and_cancel_only() {
    let undone = counter();
    // Failure: compensations run LIFO.
    let u1 = undone.clone();
    let u2 = undone.clone();
    let out = compensate(
        &|| false,
        || "cancelled".to_owned(),
        |scope: &mut CompScope| {
            scope.compensate(move || {
                u1.fetch_add(1, Ordering::SeqCst);
            });
            scope.compensate(move || {
                u2.fetch_add(10, Ordering::SeqCst);
            });
            Err::<i32, String>("boom".to_owned())
        },
    );
    assert_eq!(out, Err("boom".to_owned()));
    assert_eq!(undone.load(Ordering::SeqCst), 11);
    // Cancel: compensations run, outcome maps through on_cancel.
    let undone2 = counter();
    let u3 = undone2.clone();
    let out = compensate(
        &|| true,
        || "cancelled".to_owned(),
        |scope: &mut CompScope| {
            scope.compensate(move || {
                u3.fetch_add(1, Ordering::SeqCst);
            });
            Ok::<i32, String>(1)
        },
    );
    assert_eq!(out, Err("cancelled".to_owned()));
    assert_eq!(undone2.load(Ordering::SeqCst), 1);
    // Success: nothing runs.
    let undone3 = counter();
    let u4 = undone3.clone();
    let out = compensate(
        &|| false,
        || "cancelled".to_owned(),
        |scope: &mut CompScope| {
            scope.compensate(move || {
                u4.fetch_add(1, Ordering::SeqCst);
            });
            Ok::<i32, String>(2)
        },
    );
    assert_eq!(out, Ok(2));
    assert_eq!(undone3.load(Ordering::SeqCst), 0);
}
