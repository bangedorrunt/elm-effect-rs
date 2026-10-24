//! Semaphore.test.ts cases on Effect 4's `PartitionedSemaphore`: one shared
//! permit pool, waiters grouped by key, released permits handed out in
//! round-robin order. The single-pool `Semaphore` cases live here too, because
//! Effect 4's `Semaphore` suite drives the same partitioned type.
#![cfg(feature = "tokio")]

use elm_effect::{PartitionedSemaphore, Semaphore};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

fn wait_until(cond: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// Holds bodies inside the pool until the test lets them finish, so "who got a
/// permit" is observed without sleep-based guessing.
struct Latch {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Latch {
    fn new() -> Self {
        Self {
            open: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }

    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

const QUANTUM: Duration = Duration::from_millis(50);

/// :7 `basic single partition operation`: four tasks of two permits on a pool
/// of four admit exactly two at a time, and the others follow once permits
/// come back.
#[test]
fn two_of_four_tasks_run_until_permits_return() {
    let sem = Arc::new(PartitionedSemaphore::new(4));
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let latch = Arc::new(Latch::new());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (sem, running, peak, entered, latch) = (
                sem.clone(),
                running.clone(),
                peak.clone(),
                entered.clone(),
                latch.clone(),
            );
            std::thread::spawn(move || {
                sem.with_permits("partition-1", 2, || {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    entered.fetch_add(1, Ordering::SeqCst);
                    latch.wait();
                    running.fetch_sub(1, Ordering::SeqCst);
                });
            })
        })
        .collect();

    wait_until(
        || entered.load(Ordering::SeqCst) == 2,
        "two tasks hold the pool",
    );
    std::thread::sleep(QUANTUM);
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "four permits admit exactly two tasks of two"
    );
    assert_eq!(entered.load(Ordering::SeqCst), 2, "the rest are parked");

    latch.open();
    for handle in handles {
        handle.join().expect("worker");
    }
    assert_eq!(entered.load(Ordering::SeqCst), 4, "every task ran");
    assert_eq!(sem.available(), 4);
}

/// :33 `multiple partitions share total permits`: the two keys draw from the
/// same four permits, so the third task of two waits even though its own key
/// has asked for nothing else.
#[test]
fn multiple_partitions_share_total_permits() {
    let sem = Arc::new(PartitionedSemaphore::new(4));
    let entered = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let latch = Arc::new(Latch::new());

    let mut handles = Vec::new();
    for (key, label) in [
        ("partition-1", "p1-task1"),
        ("partition-2", "p2-task1"),
        ("partition-1", "p1-task2"),
    ] {
        let (worker_sem, worker_entered, worker_latch) =
            (sem.clone(), entered.clone(), latch.clone());
        handles.push(std::thread::spawn(move || {
            worker_sem.with_permits(key, 2, || {
                worker_entered.lock().unwrap().push(label);
                worker_latch.wait();
            });
        }));
        if label == "p2-task1" {
            // Both four-permit tasks are in before the third asks.
            wait_until(|| sem.available() == 0, "the first two hold the pool");
        }
    }

    std::thread::sleep(QUANTUM);
    assert_eq!(
        entered.lock().unwrap().len(),
        2,
        "two tasks of two fill a pool of four; the third waits"
    );

    latch.open();
    for handle in handles {
        handle.join().expect("worker");
    }
    assert_eq!(entered.lock().unwrap().len(), 3, "every task ran");
    assert_eq!(sem.available(), 4);
}

/// :68 `round-robin fairness across partitions`: when both keys have waiters,
/// two released permits go one to each key, never both to the busier key.
#[test]
fn released_permits_rotate_across_partitions() {
    let sem = Arc::new(PartitionedSemaphore::new(2));
    let entered = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let latch = Arc::new(Latch::new());

    // The initial holder takes the whole pool and keeps it until the waiters
    // are parked.
    let holder_latch = Arc::new(Latch::new());
    let (holder_sem, holder_hold) = (sem.clone(), holder_latch.clone());
    let holder = std::thread::spawn(move || {
        holder_sem.with_permits("partition-1", 2, || holder_hold.wait())
    });
    wait_until(|| sem.available() == 0, "the pool is held");

    let mut waiters = Vec::new();
    for (key, label) in [
        ("partition-1", "p1-task1"),
        ("partition-1", "p1-task2"),
        ("partition-1", "p1-task3"),
        ("partition-2", "p2-task1"),
        ("partition-2", "p2-task2"),
        ("partition-2", "p2-task3"),
    ] {
        let (worker_sem, worker_entered, worker_latch) =
            (sem.clone(), entered.clone(), latch.clone());
        waiters.push(std::thread::spawn(move || {
            worker_sem.with_permits(key, 1, || {
                worker_entered.lock().unwrap().push(label);
                worker_latch.wait();
            });
        }));
        // One permit of shortfall appears per parked waiter, in spawn order.
        let parked = match label {
            "p1-task1" => 1,
            "p1-task2" => 2,
            "p1-task3" => 3,
            "p2-task1" => 4,
            "p2-task2" => 5,
            _ => 6,
        };
        wait_until(|| sem.waiting() == parked, "the waiter parks");
    }

    // The two permits come back together: one to each key, in rotation.
    holder_latch.open();
    holder.join().expect("holder");
    wait_until(
        || entered.lock().unwrap().len() == 2,
        "exactly the two grants resume",
    );
    std::thread::sleep(QUANTUM);
    let mut resumed = entered.lock().unwrap().clone();
    resumed.sort();
    assert_eq!(
        resumed,
        vec!["p1-task1", "p2-task1"],
        "the pool alternates between keys instead of finishing one key first"
    );

    latch.open();
    for waiter in waiters {
        waiter.join().expect("waiter");
    }
    assert_eq!(entered.lock().unwrap().len(), 6, "every waiter ran");
    assert_eq!(sem.available(), 2);
}

/// :519 `exact permit match`: a take of exactly the pool size is granted, and
/// a single permit behind it waits for the whole pool to come back.
#[test]
fn an_exact_match_take_lets_nothing_else_in() {
    let sem = Arc::new(PartitionedSemaphore::new(4));
    let entered = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let latch = Arc::new(Latch::new());

    let (s1, e1, l1) = (sem.clone(), entered.clone(), latch.clone());
    let exact = std::thread::spawn(move || {
        s1.with_permits("partition-1", 4, || {
            e1.lock().unwrap().push("exact-match");
            l1.wait();
        })
    });
    wait_until(|| sem.available() == 0, "the exact take holds the pool");

    let (s2, e2) = (sem.clone(), entered.clone());
    let waiting = std::thread::spawn(move || {
        s2.with_permits("partition-2", 1, || {
            e2.lock().unwrap().push("waiting");
        })
    });

    std::thread::sleep(QUANTUM);
    assert_eq!(
        *entered.lock().unwrap(),
        vec!["exact-match"],
        "nothing is left for the one-permit task"
    );

    latch.open();
    exact.join().expect("exact match");
    waiting.join().expect("waiter");
    assert_eq!(*entered.lock().unwrap(), vec!["exact-match", "waiting"]);
    assert_eq!(sem.available(), 4);
}

/// :214 `interruption with partial permit acquisition releases all taken
/// permits`: a waiter that absorbed one permit and was parked for two more
/// gives both back, so another key can take one and the full pool returns
/// once the holder is done.
#[test]
fn interrupted_partial_grant_releases_every_taken_permit() {
    let sem = Arc::new(PartitionedSemaphore::new(3));
    let latch = Arc::new(Latch::new());
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let (s1, l1, log1) = (sem.clone(), latch.clone(), log.clone());
    let holder = std::thread::spawn(move || {
        s1.with_permits("partition-1", 2, || {
            log1.lock().unwrap().push("first");
            l1.wait();
        })
    });
    wait_until(|| sem.available() == 1, "the holder leaves one permit");

    let stop = Arc::new(AtomicBool::new(false));
    let (s2, stop2) = (sem.clone(), stop.clone());
    let waiter = std::thread::spawn(move || {
        s2.take_interruptible("partition-2", 3, &|| stop2.load(Ordering::SeqCst))
    });
    wait_until(|| sem.waiting() == 2, "the waiter parked on its shortfall");
    assert_eq!(sem.available(), 0, "it absorbed the last free permit");

    stop.store(true, Ordering::SeqCst);
    assert_eq!(waiter.join().expect("waiter ends"), None);
    assert_eq!(
        sem.available(),
        1,
        "the absorbed permit is back while the holder keeps its two"
    );

    let (s3, log3) = (sem.clone(), log.clone());
    s3.with_permits("partition-3", 1, || {
        log3.lock().unwrap().push("can-acquire-one")
    });

    latch.open();
    holder.join().expect("holder");
    assert_eq!(sem.available(), 3, "the whole pool is back");

    let (s4, log4) = (sem.clone(), log.clone());
    s4.with_permits("partition-4", 3, || log4.lock().unwrap().push("all-three"));
    assert_eq!(
        *log.lock().unwrap(),
        vec!["first", "can-acquire-one", "all-three"]
    );
}

/// :549 `many partitions`: ten one-permit tasks over five keys still hold only
/// three at a time, and every task runs.
#[test]
fn ten_tasks_over_five_partitions_hold_three() {
    let sem = Arc::new(PartitionedSemaphore::new(3));
    let latch = Arc::new(Latch::new());
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let (sem, latch, running, peak, entered) = (
                sem.clone(),
                latch.clone(),
                running.clone(),
                peak.clone(),
                entered.clone(),
            );
            std::thread::spawn(move || {
                sem.with_permits(format!("partition-{}", i % 5), 1, || {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    entered.fetch_add(1, Ordering::SeqCst);
                    latch.wait();
                    running.fetch_sub(1, Ordering::SeqCst);
                });
            })
        })
        .collect();

    wait_until(
        || entered.load(Ordering::SeqCst) == 3,
        "three tasks fill the pool",
    );
    std::thread::sleep(QUANTUM);
    assert_eq!(
        entered.load(Ordering::SeqCst),
        3,
        "a pool of three admits three tasks however many keys ask"
    );

    latch.open();
    for handle in handles {
        handle.join().expect("worker");
    }
    assert_eq!(entered.load(Ordering::SeqCst), 10, "every task ran");
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    assert_eq!(sem.available(), 3);
}

/// :580 `partial permit allocation`: a two-permit task that finds one permit
/// free parks for the second instead of running with half a grant, and runs
/// as soon as the pool refills.
#[test]
fn a_waiting_partition_runs_once_the_pool_refills() {
    let sem = Arc::new(PartitionedSemaphore::new(3));
    let latch = Arc::new(Latch::new());
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let (s1, l1, log1) = (sem.clone(), latch.clone(), log.clone());
    let first = std::thread::spawn(move || {
        s1.with_permits("partition-1", 2, || {
            log1.lock().unwrap().push("first");
            l1.wait();
        })
    });
    wait_until(|| sem.available() == 1, "the first task holds two");

    let second_entered = Arc::new(AtomicBool::new(false));
    let (s2, entered2) = (sem.clone(), second_entered.clone());
    let second = std::thread::spawn(move || {
        s2.with_permits("partition-2", 2, || entered2.store(true, Ordering::SeqCst))
    });
    wait_until(|| sem.waiting() == 1, "the second task parks one short");
    std::thread::sleep(QUANTUM);
    assert_eq!(sem.available(), 0, "it absorbed the free permit");
    assert!(
        !second_entered.load(Ordering::SeqCst),
        "half a grant does not run the body"
    );

    latch.open();
    first.join().expect("first");
    second.join().expect("second");
    assert!(
        second_entered.load(Ordering::SeqCst),
        "the refilled pool lets the second task in"
    );
    assert_eq!(sem.available(), 3);
}

/// :128 `single permit operations`: one permit serializes the tasks of a
/// partition, and all of them run in order.
#[test]
fn one_permit_serializes_partition_tasks() {
    let sem = PartitionedSemaphore::new(1);
    let ran = Arc::new(Mutex::new(Vec::new()));
    for n in 0..3 {
        let ran = Arc::clone(&ran);
        let out = sem.with_permits_interruptible("p", 1, &|| false, move || {
            ran.lock().unwrap().push(n);
            n
        });
        assert_eq!(out, Some(n));
    }
    assert_eq!(*ran.lock().unwrap(), vec![0, 1, 2]);
    assert_eq!(sem.available(), 1, "every permit came back");
}

/// :157 `different permit sizes on same partition`: with the pool large
/// enough for the sum, 3 + 1 + 1 run together rather than queueing.
#[test]
fn different_permit_sizes_all_run() {
    let sem = Arc::new(PartitionedSemaphore::new(5));
    let ran = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = [3usize, 1, 1]
        .into_iter()
        .map(|need| {
            let (sem, ran) = (sem.clone(), ran.clone());
            std::thread::spawn(move || {
                sem.with_permits("partition-1", need, || {
                    ran.fetch_add(1, Ordering::SeqCst);
                })
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("worker");
    }
    assert_eq!(ran.load(Ordering::SeqCst), 3);
    assert_eq!(sem.available(), 5);
}

/// :115 `requesting more permits than total returns never`: a need above the
/// pool is not a wait that can end, and Effect 4 never grants it a part of the
/// pool either, so the taker stays parked and holds nothing.
#[test]
fn a_need_above_the_pool_stays_parked() {
    let sem = Arc::new(PartitionedSemaphore::new(4));
    let done = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let worker = {
        let (sem, done, stop) = (sem.clone(), done.clone(), stop.clone());
        std::thread::spawn(move || {
            let out =
                sem.with_permits_interruptible("p", 5, &|| stop.load(Ordering::SeqCst), || {});
            assert_eq!(out, None, "the only way out is the interrupt");
            done.store(true, Ordering::SeqCst);
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !done.load(Ordering::SeqCst),
        "a take larger than the pool must never complete"
    );
    assert_eq!(
        sem.available(),
        4,
        "an impossible take holds no partial grant"
    );
    stop.store(true, Ordering::SeqCst);
    worker.join().expect("the interrupted worker leaves");
    assert_eq!(sem.available(), 4, "the pool was never debited");
}

/// :613 `zero permits requested`: nothing is taken and the body runs.
#[test]
fn zero_permits_runs_the_body() {
    let sem = PartitionedSemaphore::new(2);
    let executed = AtomicBool::new(false);
    let out = sem.with_permits_interruptible("p", 0, &|| false, || {
        executed.store(true, Ordering::SeqCst);
    });
    assert_eq!(out, Some(()));
    assert!(executed.load(Ordering::SeqCst));
    assert_eq!(sem.available(), 2, "a zero take never debits the pool");
}

/// :627 `sequential tasks in same partition`: three runs of the full pool
/// complete one after another in call order.
#[test]
fn sequential_full_pool_tasks_keep_their_order() {
    let sem = PartitionedSemaphore::new(2);
    let ran = Arc::new(Mutex::new(Vec::new()));
    for n in 1..=3 {
        let ran = Arc::clone(&ran);
        let out = sem.with_permits_interruptible("p", 2, &|| false, move || {
            ran.lock().unwrap().push(format!("task-{n}"));
        });
        assert_eq!(out, Some(()));
    }
    assert_eq!(*ran.lock().unwrap(), vec!["task-1", "task-2", "task-3"]);
}

/// :189 `interruption releases permits`: a holder that unwinds releases what
/// it held, so the next taker gets the whole pool (the hold's drop is the
/// port's cancellation story).
#[test]
fn an_unwinding_holder_releases_its_permits() {
    let sem = Arc::new(PartitionedSemaphore::new(2));
    let holder = {
        let sem = Arc::clone(&sem);
        std::thread::spawn(move || {
            sem.with_permits("partition-1", 2, || panic!("interrupted while holding"))
        })
    };
    assert!(holder.join().is_err(), "the holder unwinds");
    wait_until(|| sem.available() == 2, "permits return");
    sem.with_permits("partition-1", 2, || {});
    assert_eq!(sem.available(), 2);
}

/// :308 `withPermits interruption does not leak permits`: a waiter that is
/// interrupted before its take completes holds nothing and never runs.
#[test]
fn an_interrupted_waiter_leaks_nothing() {
    let sem = Arc::new(Semaphore::new(1));
    let stop = Arc::new(AtomicBool::new(false));
    let acquired = Arc::new(AtomicBool::new(false));

    let waiter = {
        let (sem, stop, acquired) = (sem.clone(), stop.clone(), acquired.clone());
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

/// Regression: a shrink below the loaned count made `permits - taken`
/// underflow on the next take. The free count is a saturating difference.
#[test]
fn shrinking_below_loans_never_underflows() {
    let sem = Semaphore::new(2);
    assert_eq!(sem.take(2), 2);
    sem.resize(1);
    assert!(
        !sem.take_if_available(1),
        "nothing is free after the shrink"
    );
    assert_eq!(sem.available(), 0, "availability floors at zero");
    // The pool is now size 1 with 2 permits out on loan: returning one still
    // leaves the pool fully lent, so availability stays at zero.
    assert_eq!(sem.release(1), 1);
    assert_eq!(sem.available(), 0);
    assert!(!sem.take_if_available(1));
    assert_eq!(sem.release(1), 1);
    assert_eq!(sem.available(), 1);
    assert!(sem.take_if_available(1));
    sem.release(1);
}

/// Regression: a panic inside `with_permits` used to skip the release and
/// leak the permits out of the pool forever.
#[test]
fn with_permits_restores_on_panic() {
    let sem = Semaphore::new(1);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sem.with_permits(1, || panic!("guarded body"));
    }));
    assert!(panicked.is_err(), "the panic must still propagate");
    assert_eq!(sem.available(), 1, "the guard must release on unwind");
    assert!(sem.take_if_available(1));
    sem.release(1);
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
