//! i4t slice: TxDeferred fallible-gate cells.
//! Ports packages/effect/test/TxDeferred.test.ts rows left open after the
//! infallible complete/poll face. Follows tx_core.rs (atomically composes;
//! first completion wins; every waiter unblocks).
#![cfg(feature = "tokio")]

use elm_effect::{TxDeferred, atomically, is_tx_deferred};
use std::sync::Arc;

// Row 36: poll reports the failure after fail.
#[test]
fn deferred_poll_failure_after_fail() {
    let gate = TxDeferred::<i32, String>::new();
    assert_eq!(gate.poll_outcome(), None);
    assert!(gate.fail("boom".to_string()));
    assert_eq!(gate.poll_outcome(), Some(Err("boom".to_string())));
    assert_eq!(gate.poll(), None, "value view stays empty on error");
}

// Rows 77/84: succeed wins once; a second completion loses.
#[test]
fn deferred_succeed_first_wins() {
    let gate = TxDeferred::<i32, String>::new();
    assert!(gate.succeed(1));
    assert!(!gate.succeed(2));
    assert_eq!(gate.poll_outcome(), Some(Ok(1)));
}

// Rows 92/99: fail wins once; a later succeed loses.
#[test]
fn deferred_fail_first_wins() {
    let gate = TxDeferred::<i32, String>::new();
    assert!(gate.fail("boom".to_string()));
    assert!(!gate.succeed(1));
    assert_eq!(gate.poll_outcome(), Some(Err("boom".to_string())));
}

// Rows 107/116: done behaves like succeed/fail by outcome.
#[test]
fn deferred_done_behaves_like_succeed_and_fail() {
    let ok_gate = TxDeferred::<i32, String>::new();
    assert!(ok_gate.done(Ok(7)));
    assert!(!ok_gate.done(Err("late".to_string())));
    assert_eq!(ok_gate.poll_outcome(), Some(Ok(7)));
    let err_gate = TxDeferred::<i32, String>::new();
    assert!(err_gate.done(Err("boom".to_string())));
    assert!(!err_gate.done(Ok(7)));
    assert_eq!(err_gate.poll_outcome(), Some(Err("boom".to_string())));
}

// Rows 50/58: await reports the value or the error.
#[test]
fn deferred_await_value_and_error() {
    let gate = TxDeferred::<i32, String>::new();
    gate.succeed(3);
    assert_eq!(gate.await_outcome(), Ok(3));
    let gate = TxDeferred::<i32, String>::new();
    gate.fail("boom".to_string());
    assert_eq!(gate.await_outcome(), Err("boom".to_string()));
}

// Row 66: await blocks until another fiber completes the gate.
#[test]
fn deferred_await_blocks_until_completed() {
    let gate = Arc::new(TxDeferred::<i32, String>::new());
    let waiter = gate.clone();
    let fiber = std::thread::spawn(move || waiter.await_outcome());
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(gate.poll_outcome(), None, "still pending before complete");
    assert!(gate.succeed(9));
    assert_eq!(fiber.join().expect("waiter ends"), Ok(9));
}

// Row 139: await observes a completion made earlier in the same transaction.
#[test]
fn deferred_await_inside_same_transaction() {
    let gate = TxDeferred::<i32, String>::new();
    let seen = atomically(|| {
        gate.succeed(5);
        gate.await_outcome()
    });
    assert_eq!(seen, Ok(5));
}

// Row 151: two deferreds complete atomically together.
#[test]
fn deferred_two_gates_complete_atomically() {
    let first = TxDeferred::<i32, String>::new();
    let second = TxDeferred::<i32, String>::new();
    atomically(|| {
        first.succeed(1);
        second.fail("boom".to_string());
    });
    assert_eq!(first.await_outcome(), Ok(1));
    assert_eq!(second.await_outcome(), Err("boom".to_string()));
}

// Row 169: every awaiting fiber unblocks on the single completion.
#[test]
fn deferred_all_waiters_unblock() {
    let gate = Arc::new(TxDeferred::<i32, String>::new());
    let mut fibers = Vec::new();
    for _ in 0..4 {
        let waiter = gate.clone();
        fibers.push(std::thread::spawn(move || waiter.await_outcome()));
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(gate.succeed(1));
    for fiber in fibers {
        assert_eq!(fiber.join().expect("waiter ends"), Ok(1));
    }
}

// Row 186: a succeed/fail race leaves exactly one winner.
#[test]
fn deferred_succeed_fail_race_first_wins() {
    for _ in 0..50 {
        let gate = Arc::new(TxDeferred::<i32, String>::new());
        let racer = gate.clone();
        let fiber = std::thread::spawn(move || racer.fail("boom".to_string()));
        let main_won = gate.succeed(1);
        let fiber_won = fiber.join().expect("racer ends");
        assert_ne!(main_won, fiber_won, "exactly one winner");
        let outcome = gate.await_outcome();
        assert_eq!(outcome.is_ok(), main_won, "await agrees with the winner");
    }
}

#[test]
fn tx_deferred_guard_identifies_deferreds() {
    // TxDeferred.test.ts:127 — the guard matches deferred instances.
    assert!(is_tx_deferred(&TxDeferred::<i32, String>::new()));
}
