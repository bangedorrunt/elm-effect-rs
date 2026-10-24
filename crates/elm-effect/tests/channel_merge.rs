//! `merge_with_deadline`: a stalled producer fails with the caller's error
//! instead of hanging the consumer past every timeout.

#[allow(dead_code, clippy::type_complexity)]
#[path = "../src/channel.rs"]
mod channel;
#[allow(dead_code, clippy::type_complexity)]
#[path = "../src/pull.rs"]
mod pull;
#[allow(dead_code, clippy::type_complexity)]
#[path = "../src/take.rs"]
mod take;

use channel::{HaltStrategy, merge, merge_with_deadline};
use pull::Pull;
// The `#[path]` includes reach `Cause`/`Defect` through `super`, so this
// crate root has to name them.
use elm_effect::{Cause, Defect};
use take::Take;

fn seq(items: Vec<u32>) -> Pull<u32, String> {
    let items = std::sync::Arc::new(std::sync::Mutex::new(items));
    Pull::from_fn(move || match items.lock().unwrap().pop() {
        Some(v) => Take::Items(vec![v]),
        None => Take::End,
    })
}

/// A producer that never answers, but can be released: the halt hook wakes
/// the condvar, so a merge that gives up does not strand the thread forever.
fn stalled() -> Pull<u32, String> {
    use std::sync::{Arc, Condvar, Mutex};
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let wake = {
        let gate = Arc::clone(&gate);
        move || {
            let (lock, condvar) = &*gate;
            *lock.lock().unwrap() = true;
            condvar.notify_all();
        }
    };
    Pull::from_fn(move || {
        let (lock, condvar) = &*gate;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = condvar.wait(open).unwrap();
        }
        Take::End
    })
    .on_halt(wake)
}

#[test]
fn merge_deadline_fails_with_caller_error_on_stall() {
    let mut merged = merge_with_deadline(
        seq(vec![1]),
        stalled(),
        HaltStrategy::Both,
        std::time::Duration::from_millis(50),
        || "timed-out".to_string(),
    );
    // Fast side items still arrive before the deadline bites.
    let mut saw_fail = false;
    loop {
        match merged.pull() {
            Take::Items(_) => continue,
            Take::Fail(cause) => {
                assert_eq!(cause.failure(), Some(&"timed-out".to_string()));
                saw_fail = true;
                break;
            }
            Take::End => break,
        }
    }
    assert!(saw_fail, "a stalled side must fail, not hang or truncate");
}

#[test]
fn dropping_a_merge_wakes_a_blocked_producer() {
    // Regression: a merge spawned a thread per side and only checked its
    // `halted` flag between pulls, so a producer blocked inside its pull
    // outlived the consumer. A source that registers a halt hook must be
    // released when the merged pull is dropped.
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let exited = Arc::new(AtomicBool::new(false));
    let wake = {
        let gate = Arc::clone(&gate);
        move || {
            let (lock, condvar) = &*gate;
            *lock.lock().unwrap() = true;
            condvar.notify_all();
        }
    };
    let blocked = {
        let gate = Arc::clone(&gate);
        let entered = Arc::clone(&entered);
        let exited = Arc::clone(&exited);
        Pull::from_fn(move || {
            let (lock, condvar) = &*gate;
            let mut open = lock.lock().unwrap();
            entered.store(true, Ordering::SeqCst);
            while !*open {
                open = condvar.wait(open).unwrap();
            }
            exited.store(true, Ordering::SeqCst);
            Take::<u32, String>::End
        })
        .on_halt(wake)
    };
    let mut merged = merge(blocked, seq(vec![1]), HaltStrategy::Both);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !entered.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        entered.load(Ordering::SeqCst),
        "the producer never reached its blocking pull"
    );
    loop {
        if let Take::Items(_) = merged.pull() {
            break;
        }
    }
    drop(merged);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !exited.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        exited.load(Ordering::SeqCst),
        "the halted producer never woke"
    );
}

#[test]
fn merge_deadline_leaves_fast_merges_untouched() {
    let mut merged = merge_with_deadline(
        seq(vec![1]),
        seq(vec![2]),
        HaltStrategy::Both,
        std::time::Duration::from_secs(30),
        || "timed-out".to_string(),
    );
    let mut out = Vec::new();
    loop {
        match merged.pull() {
            Take::Items(items) => out.extend(items),
            Take::End => break,
            Take::Fail(cause) => panic!("fast merge must not fail: {cause}"),
        }
    }
    out.sort();
    assert_eq!(out, vec![1, 2]);
}
