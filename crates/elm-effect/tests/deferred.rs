//! `Deferred` outcomes.
//!
//! A `Deferred` completes exactly once: success, failure, defect, or
//! interruption. It carries an `Exit` rather than a bare value, so `poll` can
//! tell a success from a failure. These tests cover the four outcomes and the
//! accessors that distinguish them.

use elm_effect::{Cause, Deferred, Exit, ForkOpts, Runtime};
use std::convert::Infallible;
use std::sync::Arc;

fn succeed() -> Deferred<u32, Infallible> {
    Deferred::new()
}

fn failing() -> Deferred<u32, String> {
    Deferred::new()
}

#[test]
fn poll_returns_none_while_incomplete() {
    assert_eq!(succeed().poll(), None);
}

#[test]
fn complete_records_the_success() {
    let cell = succeed();
    assert!(cell.complete(42));
    assert_eq!(cell.poll(), Some(Exit::Success(42)));
    assert_eq!(cell.wait(), 42, "wait returns the value on success");
}

#[test]
fn complete_memoizes_the_outcome() {
    // Exactly once: the first completion wins and later ones are ignored.
    let cell = succeed();
    assert!(cell.complete(1));
    assert!(!cell.complete(2), "a second completion must be refused");
    assert_eq!(cell.poll(), Some(Exit::Success(1)));
}

#[test]
fn fail_records_the_failure() {
    // The distinction a bare-value Deferred cannot express.
    let cell = failing();
    cell.complete_exit(Exit::Failure(Cause::fail("boom".to_string())));
    match cell.poll() {
        Some(Exit::Failure(cause)) => assert_eq!(cause.failure(), Some(&"boom".to_string())),
        other => panic!("expected a recorded failure, got {other:?}"),
    }
}

#[test]
fn interrupt_records_the_interruption() {
    // An interrupt is not a failure: it must stay distinguishable.
    let cell = failing();
    cell.complete_exit(Exit::Failure(Cause::interrupt(None)));
    match cell.poll() {
        Some(Exit::Failure(cause)) => assert!(cause.is_interrupted(), "recorded as an interrupt"),
        other => panic!("expected a recorded interrupt, got {other:?}"),
    }
    // And it must not read as a typed failure.
    assert_eq!(
        cell.poll().and_then(|e| match e {
            Exit::Failure(cause) => cause.failure().cloned(),
            Exit::Success(_) => None,
        }),
        None
    );
}

#[test]
fn is_done_is_true_only_once_completed() {
    let cell = succeed();
    assert!(!cell.is_done(), "empty means not done");
    cell.complete(7);
    assert!(cell.is_done());
}

#[test]
fn wait_exit_returns_the_whole_outcome() {
    let cell: Deferred<u32, String> = Deferred::new();
    cell.complete_exit(Exit::Failure(Cause::fail("boom".to_string())));
    match cell.wait_exit() {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"boom".to_string())),
        Exit::Success(value) => panic!("expected the recorded failure, got {value}"),
    }
}

#[test]
fn wait_exit_ends_when_the_waiting_fiber_is_interrupted() {
    let cell: Arc<Deferred<u32, String>> = Arc::new(Deferred::new());
    let started: Arc<Deferred<(), String>> = Arc::new(Deferred::new());
    let runtime = Runtime::new();
    let handle = {
        let cell = Arc::clone(&cell);
        let started = Arc::clone(&started);
        runtime.fork_detach_with(
            move |_ctx| {
                started.complete(());
                match cell.wait_exit() {
                    Exit::Failure(cause) if cause.is_interrupted() => Exit::succeed(()),
                    other => Exit::fail(format!("expected an interrupt, got {other:?}")),
                }
            },
            ForkOpts::immediate(),
        )
    };

    assert_eq!(started.wait_exit(), Exit::Success(()));
    handle.interrupt();
    match handle.join() {
        Exit::Failure(cause) => assert!(cause.is_interrupted(), "the wait must end interrupted"),
        Exit::Success(()) => panic!("the fiber must not succeed after an interrupt"),
    }
}

#[test]
fn wait_blocks_until_completed() {
    let cell: Arc<Deferred<u32, Infallible>> = Arc::new(Deferred::new());
    let waiter = {
        let done = Arc::clone(&cell);
        std::thread::spawn(move || done.wait())
    };
    // The waiter must still be parked when we complete: this is the
    // handoff property, not just the value assertion.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(cell.complete(99));
    assert_eq!(waiter.join().expect("waiter"), 99);
}
