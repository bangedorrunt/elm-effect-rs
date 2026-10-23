//! `merge_with_deadline`: a stalled producer fails with the caller's error
//! instead of hanging the consumer past every timeout.

#[path = "../src/channel.rs"]
mod channel;
#[path = "../src/pull.rs"]
mod pull;
#[path = "../src/take.rs"]
mod take;

use channel::{HaltStrategy, merge_with_deadline};
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

/// A producer that never answers.
fn stalled() -> Pull<u32, String> {
    Pull::from_fn(|| {
        std::thread::park();
        Take::End
    })
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
