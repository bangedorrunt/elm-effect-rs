//! Semaphore.test.ts rows the port's permit pools carry.
//!
//! The pin's `PartitionedSemaphore` shares one permit pool across keys with
//! round-robin partial grants; the port's types give each key its own pool.
//! Only the rows whose outcome does not depend on that difference are here:
//! one key at a time, sequential runs, zero permits, and the all-or-nothing
//! single-pool `Semaphore` API.
#![cfg(feature = "tokio")]

#[path = "../src/partitioned_semaphore.rs"]
mod partitioned_semaphore;

use elm_effect::{PartitionedSemaphore, Semaphore};
use partitioned_semaphore::PartitionedSemaphore as GuardedSemaphore;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn wait_until(cond: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// :7 `basic single partition operation`: four tasks of two permits on a pool
/// of four admit exactly two at a time, and the others follow once permits
/// come back.
#[test]
fn two_of_four_tasks_run_until_permits_return() {
    let sem = Arc::new(PartitionedSemaphore::new(1, 4));
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let sem = Arc::clone(&sem);
            let running = Arc::clone(&running);
            let peak = Arc::clone(&peak);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            std::thread::spawn(move || {
                sem.with_permits_interruptible(0, 2, &|| false, || {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    entered.fetch_add(1, Ordering::SeqCst);
                    while !release.load(Ordering::SeqCst) {
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                    running.fetch_sub(1, Ordering::SeqCst);
                });
            })
        })
        .collect();

    wait_until(
        || entered.load(Ordering::SeqCst) == 2,
        "two tasks hold the pool",
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "four permits admit exactly two tasks of two"
    );
    assert_eq!(entered.load(Ordering::SeqCst), 2, "the rest are parked");

    release.store(true, Ordering::SeqCst);
    for handle in handles {
        handle.join().expect("worker");
    }
    assert_eq!(entered.load(Ordering::SeqCst), 4, "every task ran");
    assert_eq!(sem.available(0), 4);
}

/// :128 `single permit operations`: one permit serializes the tasks of a
/// partition, and all of them run in order.
#[test]
fn one_permit_serializes_partition_tasks() {
    let sem = PartitionedSemaphore::new(1, 1);
    let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
    for n in 0..3 {
        let ran = Arc::clone(&ran);
        let out = sem.with_permits_interruptible(
            0,
            1,
            &|| false,
            move || {
                ran.lock().unwrap().push(n);
                n
            },
        );
        assert_eq!(out, Some(n));
    }
    assert_eq!(*ran.lock().unwrap(), vec![0, 1, 2]);
    assert_eq!(sem.available(0), 1, "every permit came back");
}

/// :157 `different permit sizes on same partition`: with the pool large
/// enough for the sum, 3 + 1 + 1 run together rather than queueing.
#[test]
fn different_permit_sizes_all_run() {
    let sem = Arc::new(PartitionedSemaphore::new(1, 5));
    let ran = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = [3usize, 1, 1]
        .into_iter()
        .map(|need| {
            let sem = Arc::clone(&sem);
            let ran = Arc::clone(&ran);
            std::thread::spawn(move || {
                sem.with_permits_interruptible(0, need, &|| false, || {
                    ran.fetch_add(1, Ordering::SeqCst);
                })
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().expect("worker"), Some(()));
    }
    assert_eq!(ran.load(Ordering::SeqCst), 3);
    assert_eq!(sem.available(0), 5);
}

/// :115 `requesting more permits than total returns never`: a need above the
/// pool is not a wait that can end, so the taker stays parked.
#[test]
fn a_need_above_the_pool_stays_parked() {
    let sem = Arc::new(PartitionedSemaphore::new(1, 4));
    let done = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let worker = {
        let sem = Arc::clone(&sem);
        let done = Arc::clone(&done);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let out = sem.with_permits_interruptible(0, 5, &|| stop.load(Ordering::SeqCst), || {});
            assert_eq!(out, None, "the only way out is the interrupt");
            done.store(true, Ordering::SeqCst);
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !done.load(Ordering::SeqCst),
        "a take larger than the pool must never complete"
    );
    // The waiter accumulates as permits free (the pin's partial acquisition):
    // it holds all four and parks on the fifth, which the pool can never give.
    assert_eq!(sem.available(0), 0, "the parked take holds its partial grant");
    stop.store(true, Ordering::SeqCst);
    worker.join().expect("the interrupted worker leaves");
    assert_eq!(sem.available(0), 4, "the interrupt restores the partial grant");
}

/// :613 `zero permits requested`: nothing is taken and the body runs.
#[test]
fn zero_permits_runs_the_body() {
    let sem = PartitionedSemaphore::new(1, 2);
    let executed = AtomicBool::new(false);
    let out = sem.with_permits_interruptible(0, 0, &|| false, || {
        executed.store(true, Ordering::SeqCst);
    });
    assert_eq!(out, Some(()));
    assert!(executed.load(Ordering::SeqCst));
    assert_eq!(sem.available(0), 2, "a zero take never debits the pool");
}

/// :627 `sequential tasks in same partition`: three runs of the full pool
/// complete one after another in call order.
#[test]
fn sequential_full_pool_tasks_keep_their_order() {
    let sem = PartitionedSemaphore::new(1, 2);
    let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
    for n in 1..=3 {
        let ran = Arc::clone(&ran);
        let out = sem.with_permits_interruptible(0, 2, &|| false, move || {
            ran.lock().unwrap().push(format!("task-{n}"));
        });
        assert_eq!(out, Some(()));
    }
    assert_eq!(
        *ran.lock().unwrap(),
        vec!["task-1", "task-2", "task-3"]
    );
}

/// :189 `interruption releases permits`: a holder that unwinds releases what
/// it held, so the next taker gets the whole pool (`Guard` on drop is the
/// port's cancellation story).
#[test]
fn an_unwinding_holder_releases_its_permits() {
    let sem = Arc::new(GuardedSemaphore::new(2));
    let holder = {
        let sem = Arc::clone(&sem);
        std::thread::spawn(move || {
            let _held = sem.acquire("partition-1", 2);
            panic!("interrupted while holding");
        })
    };
    assert!(holder.join().is_err(), "the holder unwinds");
    wait_until(|| sem.available("partition-1") == 2, "permits return");
    sem.with_permits("partition-1", 2, || {});
    assert_eq!(sem.available("partition-1"), 2);
}

/// :308 `withPermits interruption does not leak permits`: a waiter that is
/// interrupted before its take completes holds nothing and never runs.
#[test]
fn an_interrupted_waiter_leaks_nothing() {
    let sem = Arc::new(Semaphore::new(1));
    let stop = Arc::new(AtomicBool::new(false));
    let acquired = Arc::new(AtomicBool::new(false));

    let waiter = {
        let sem = Arc::clone(&sem);
        let stop = Arc::clone(&stop);
        let acquired = Arc::clone(&acquired);
        std::thread::spawn(move || {
            sem.with_permits_interruptible(2, &|| stop.load(Ordering::SeqCst), || {
                acquired.store(true, Ordering::SeqCst);
            })
        })
    };

    std::thread::sleep(Duration::from_millis(20));
    stop.store(true, Ordering::SeqCst);
    assert_eq!(waiter.join().expect("waiter"), None);
    assert!(!acquired.load(Ordering::SeqCst), "the body must not run");

    assert!(
        sem.take_if_available(1),
        "the pool still has its permit after the interrupted take"
    );
    sem.release(1);
    sem.with_permits(1, || {});
    assert_eq!(sem.available(), 1);
}

/// :458 `takeIfAvailable acquires permits when they are available`: an exact
/// take succeeds once, the next reports unavailable, and releasing answers
/// the count that was out on loan.
#[test]
fn take_if_available_takes_exactly_once() {
    let sem = Semaphore::new(2);
    assert!(sem.take_if_available(2));
    assert!(!sem.take_if_available(1), "nothing is left to take");
    assert_eq!(sem.release(2), 2, "both permits were out on loan");
    assert_eq!(sem.available(), 2);
}

/// :472 `takeIfAvailable returns immediately when permits are unavailable`.
#[test]
fn take_if_available_does_not_wait() {
    let sem = Semaphore::new(1);
    sem.take(1);
    let started = Instant::now();
    assert!(!sem.take_if_available(1));
    assert!(
        started.elapsed() < Duration::from_millis(50),
        "a refused take must not park"
    );
    sem.release(1);
    assert!(sem.take_if_available(1));
}
