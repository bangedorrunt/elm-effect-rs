//! hi5 tail slice: interruptible semaphore takes.
//! Ports packages/effect/test/PartitionedSemaphore.test.ts interruption
//! rows, left uncovered by the area beads. Follows the TestClock sleep
//! precedent: blocking waits observe interruption on a 1ms poll quantum.
#![cfg(feature = "tokio")]

use elm_effect::PartitionedSemaphore;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Row 161: an interruption before the take starts holds nothing.
#[test]
fn interrupted_before_take_holds_no_permit() {
    let sem = PartitionedSemaphore::new(1, 1);
    assert!(!sem.acquire_interruptible(0, &|| true));
    assert_eq!(sem.available(0), 1, "nothing taken, nothing to restore");
    assert_eq!(
        sem.with_permits_interruptible(0, 1, &|| true, || "ran"),
        None
    );
    assert_eq!(sem.available(0), 1);
}

// Row 93: a take satisfied immediately still restores its permit when the
// interruption lands before the body runs.
#[test]
fn satisfied_take_restores_on_interrupt_before_body() {
    use std::sync::atomic::AtomicUsize;
    let sem = PartitionedSemaphore::new(1, 1);
    let calls = AtomicUsize::new(0);
    let out = sem.with_permits_interruptible(
        0,
        1,
        &|| calls.fetch_add(1, Ordering::SeqCst) == 1,
        || "ran",
    );
    assert_eq!(out, None, "interrupted after the take, before the body");
    assert_eq!(sem.available(0), 1, "the satisfied permit is restored");
}

// Row 75: a partially satisfied waiter (one of two permits taken) releases
// everything when the wait for the rest is interrupted.
#[test]
fn partial_take_releases_on_interrupt() {
    let sem = Arc::new(PartitionedSemaphore::new(1, 1));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_sem = sem.clone();
    let worker = std::thread::spawn(move || {
        worker_sem.with_permits_interruptible(0, 2, &|| worker_stop.load(Ordering::SeqCst), || "ran")
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    stop.store(true, Ordering::SeqCst);
    assert_eq!(worker.join().expect("worker ends"), None);
    assert_eq!(sem.available(0), 1, "partial take released its permit");
}

// Success path: a full take runs the body and restores everything after.
#[test]
fn full_take_restores_after_body() {
    let sem = PartitionedSemaphore::new(1, 2);
    let out = sem.with_permits_interruptible(0, 2, &|| false, || "ran");
    assert_eq!(out, Some("ran"));
    assert_eq!(sem.available(0), 2);
}
