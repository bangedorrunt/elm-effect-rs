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
