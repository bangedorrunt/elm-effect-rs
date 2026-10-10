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

// Row 196: an interrupted waiter that had just been resumed must leave its
// permit behind for the next waiter on the same partition. The pin drives the
// pause through a custom fiber Scheduler; here the same claim is observed on
// the portable cancellation story (the interrupt flag), the way rows 161/93/75
// are already ported in this file.
#[test]
fn interrupted_resumed_waiter_leaves_the_permit_for_the_next() {
    let sem = Arc::new(PartitionedSemaphore::new(1, 1));
    assert!(
        sem.acquire_interruptible(0, &|| false),
        "the holder takes the only permit"
    );
    assert_eq!(sem.available(0), 0);

    // Waiter one parks on the empty partition, then is interrupted while the
    // holder hands the permit back.
    let stop = Arc::new(AtomicBool::new(false));
    let waiter_stop = stop.clone();
    let waiter_sem = sem.clone();
    let waiter = std::thread::spawn(move || {
        waiter_sem.acquire_interruptible(0, &|| waiter_stop.load(Ordering::SeqCst))
    });
    std::thread::sleep(std::time::Duration::from_millis(20));
    stop.store(true, Ordering::SeqCst);
    sem.release(0);
    assert!(
        !waiter.join().expect("waiter ends"),
        "the interrupted waiter holds nothing"
    );
    assert_eq!(sem.available(0), 1, "its permit is still in the pool");

    // The row's claim: the next same-partition waiter still gets it. The
    // wait is bounded so a leaked permit fails this assertion instead of
    // parking the suite forever.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    assert!(
        sem.acquire_interruptible(0, &|| std::time::Instant::now() >= deadline),
        "the next waiter on the same key takes the permit"
    );
    assert_eq!(sem.available(0), 0);

    // The `other` half of the row: a different key is a different pool, so an
    // interrupted waiter on one key never reaches into another's permits.
    let two = PartitionedSemaphore::new(2, 1);
    assert!(two.acquire_interruptible(0, &|| false));
    assert!(
        two.acquire_interruptible(1, &|| false),
        "the other key's permit is untouched"
    );
}

// Success path: a full take runs the body and restores everything after.
#[test]
fn full_take_restores_after_body() {
    let sem = PartitionedSemaphore::new(1, 2);
    let out = sem.with_permits_interruptible(0, 2, &|| false, || "ran");
    assert_eq!(out, Some("ran"));
    assert_eq!(sem.available(0), 2);
}
