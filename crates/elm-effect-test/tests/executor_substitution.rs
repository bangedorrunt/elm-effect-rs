//! Executor substitution: fan-out runs on the injected executor.
//!
//! Before `Runtime::with_executor`, every concurrency combinator hardcoded
//! the Tokio pool, so virtual-time tests could not cover fan-out, race, or
//! timeout paths. These tests pin the injection point: an inline recording
//! executor drives `all` with no threads at all, and cloned deterministic
//! handles share one queue pair.

use elm_effect::{Concurrency, Executor, Exit, Runtime};
use elm_effect_test::DeterministicExecutor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct InlineExecutor {
    spawns: Arc<AtomicUsize>,
}

impl Executor for InlineExecutor {
    fn spawn_boxed(&self, _fut: elm_effect::BoxFuture<'static, ()>) {
        panic!("inline executor drives blocking fibers only");
    }
    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send + 'static>) {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        f();
    }
}

#[test]
fn fanout_uses_the_injected_executor() {
    let spawns = Arc::new(AtomicUsize::new(0));
    let rt = Runtime::with_executor(InlineExecutor {
        spawns: Arc::clone(&spawns),
    });
    let bodies: Vec<elm_effect::Body<u32, String>> = (0..3)
        .map(|i| {
            let step: Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<u32, String> + Send> =
                Box::new(move |_| Exit::Success(i));
            step as elm_effect::Body<u32, String>
        })
        .collect();
    match elm_effect::all(&rt, bodies, Concurrency::Unbounded) {
        Exit::Success(values) => assert_eq!(values, vec![0, 1, 2]),
        other => panic!("expected success, got {other:?}"),
    }
    assert_eq!(
        spawns.load(Ordering::SeqCst),
        3,
        "every body via the injected executor"
    );
}

#[test]
fn deterministic_handles_share_one_queue_pair() {
    let submitter = DeterministicExecutor::new();
    let driver = submitter.clone();
    submitter.spawn_blocking(Box::new(|| {}));
    assert_eq!(driver.pending(), 1, "clone sees the shared queue");
    assert_eq!(driver.run_until_idle(), 1);
    assert_eq!(submitter.pending(), 0, "drain is visible to both handles");
}
