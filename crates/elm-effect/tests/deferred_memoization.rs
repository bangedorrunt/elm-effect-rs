//! `Deferred.complete` with an effect body: the memoization observable, which
//! is that a caller's body runs exactly once no matter how many times complete
//! is called.

use elm_effect::{Deferred, Exit};

#[test]
fn complete_runs_its_body_exactly_once() {
    // Deferred.test.ts:18 — the first complete runs the body and wins; the
    // second does not run it and reports false; both awaits read the same
    // recorded value.
    let runs = std::cell::Cell::new(0u32);
    let body = || {
        runs.set(runs.get() + 1);
        runs.get()
    };
    let deferred: Deferred<u32, ()> = Deferred::new();

    assert!(deferred.complete_with(body), "the first completion wins");
    assert!(!deferred.complete_with(body), "a second one is ignored");
    assert_eq!(runs.get(), 1, "the body ran once");

    assert_eq!(deferred.poll(), Some(Exit::Success(1)));
    assert_eq!(deferred.wait(), 1);
    assert_eq!(deferred.wait(), 1, "the outcome is memoized");
    assert_eq!(runs.get(), 1, "waiting does not re-run the body");
    assert!(deferred.is_done());
}

#[test]
fn a_panicking_body_leaves_the_deferred_usable() {
    // The body runs under the cell lock; a panic used to poison it, so every
    // later poll/wait panicked. The slot is untouched on a panic, so recovery
    // is the right answer.
    let deferred: Deferred<u32, String> = Deferred::new();
    let boom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        deferred.complete_with(|| panic!("boom"));
    }));
    assert!(boom.is_err(), "the body panic must propagate");
    assert!(!deferred.is_done(), "nothing was recorded");
    assert!(deferred.complete(7));
    assert_eq!(deferred.wait(), 7);
}
