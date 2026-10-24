//! Interruptible partitioned takes: a blocking wait observes interruption on
//! a 1 ms poll quantum, the same bound the test clock uses.
#![cfg(feature = "tokio")]

use elm_effect::PartitionedSemaphore;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn wait_until(cond: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

// Case 161: withPermitsIfAvailable takes the permit and then never reaches its
// body. Effect 4 interrupts the fiber between the take and the installed
// cleanup; here the same window is a body that unwinds, so the hold has to
// restore the permit on the way out.
#[test]
fn interrupted_before_take_holds_no_permit() {
    let sem = PartitionedSemaphore::<&str>::new(1);
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sem.with_permits_if_available(1, || -> () { panic!("interrupted before the body") })
    }));
    assert!(body.is_err(), "the body was interrupted");
    assert_eq!(sem.available(), 1, "the taken permit came back");
    assert_eq!(sem.waiting(), 0, "and left no parked charge behind");
}

// Case 93: a take satisfied immediately still restores its permit when the
// interruption lands after the take, before the body runs.
#[test]
fn satisfied_take_restores_on_interrupt_before_body() {
    use std::sync::atomic::AtomicUsize;
    let sem = PartitionedSemaphore::new(1);
    let calls = AtomicUsize::new(0);
    let out = sem.with_permits_interruptible(
        "k",
        1,
        // The take itself succeeds without consulting the flag; the check at
        // the with-permits boundary is the one that must see the interrupt.
        &|| calls.fetch_add(1, Ordering::SeqCst) == 0,
        || "ran",
    );
    assert_eq!(out, None, "interrupted after the take, before the body");
    assert_eq!(sem.available(), 1, "the satisfied permit is restored");
}

// Case 75: a partially satisfied waiter releases everything it had absorbed.
// The holder keeps 3 of 4 permits, the waiter asks for 3 on another key, so
// one released permit is granted to it before the interrupt lands.
#[test]
fn partial_take_releases_on_interrupt() {
    let sem = Arc::new(PartitionedSemaphore::new(4));
    sem.take("a", 3);
    assert_eq!(sem.available(), 1);

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_sem = sem.clone();
    let worker = std::thread::spawn(move || {
        worker_sem.take_interruptible("b", 3, &|| worker_stop.load(Ordering::SeqCst))
    });
    wait_until(|| sem.waiting() == 2, "the waiter parks on its shortfall");
    assert_eq!(sem.available(), 0, "the waiter absorbed the free permit");

    sem.release(1);
    assert_eq!(
        sem.available(),
        0,
        "the release went to the waiter, not the pool"
    );
    assert_eq!(sem.waiting(), 1, "one permit short of its three");

    stop.store(true, Ordering::SeqCst);
    assert_eq!(worker.join().expect("worker ends"), None);
    assert_eq!(
        sem.available(),
        2,
        "the interrupted waiter gave back the permit it had absorbed and the one it was granted"
    );
    sem.release(3);
    assert_eq!(sem.available(), 4);
}

// Case 196: an interrupted waiter that had just been resumed leaves its permit
// behind for the next waiter. Effect 4 drives the pause through a custom fiber
// Scheduler; here the interrupt is observed at the moment the holder hands the
// permit over, so the waiter either drops its charge or restores the grant,
// and either way the permit stays out of the body and in the pool.
#[test]
fn interrupted_resumed_waiter_leaves_the_permit_for_the_next() {
    let sem = Arc::new(PartitionedSemaphore::new(1));
    sem.take("holder", 1);

    let stop = Arc::new(AtomicBool::new(false));
    let (waiter_sem, waiter_stop) = (sem.clone(), stop.clone());
    let waiter = std::thread::spawn(move || {
        waiter_sem.with_permits_interruptible(
            "same",
            1,
            &|| waiter_stop.load(Ordering::SeqCst),
            || "ran",
        )
    });
    wait_until(|| sem.waiting() == 1, "the waiter parks");

    stop.store(true, Ordering::SeqCst);
    sem.release(1);
    assert_eq!(
        waiter.join().expect("waiter ends"),
        None,
        "the resumed waiter was interrupted before its body"
    );
    assert_eq!(sem.available(), 1, "its permit is still in the pool");

    // The case's claim: the next waiter on the same key still gets it. The
    // wait is bounded so a leaked permit fails this assertion instead of
    // parking the suite forever.
    let deadline = Instant::now() + Duration::from_secs(2);
    assert_eq!(
        sem.take_interruptible("same", 1, &|| Instant::now() >= deadline),
        Some(()),
        "the next waiter on the same key takes the permit"
    );
    assert_eq!(sem.available(), 0);
    sem.release(1);
}

// Success path: a full take runs the body and restores everything after.
#[test]
fn full_take_restores_after_body() {
    let sem = PartitionedSemaphore::new(2);
    let out = sem.with_permits_interruptible("k", 2, &|| false, || "ran");
    assert_eq!(out, Some("ran"));
    assert_eq!(sem.available(), 2);
}
