//! `packages/effect/test/workers/Worker.test.ts` run-lifecycle witnesses.

use elm_effect_unstable::workers::{WorkerErrorReason, WorkerRun, is_worker_error};

fn exit_error() -> elm_effect_unstable::workers::WorkerError {
    elm_effect_unstable::workers::WorkerError::new(WorkerErrorReason::receive(
        "The worker has exited with code: 1",
    ))
}

#[test]
fn run_fails_when_worker_exits_before_ready() {
    // Worker.test.ts:30 — exit arrives before the ready message.
    let mut run = WorkerRun::new();
    run.signal_exit(exit_error());
    match run.poll() {
        elm_effect_unstable::workers::RunPoll::Failed(err) => assert!(is_worker_error(&err)),
        _ => panic!("exit before ready must fail the run"),
    }
}

#[test]
fn run_fails_when_worker_exits_while_waiting_for_ready() {
    // Worker.test.ts:40 — run is pending, then the worker exits.
    let mut run = WorkerRun::new();
    assert!(run.poll().is_pending());
    run.signal_exit(exit_error());
    let outcome = run.poll();
    assert!(outcome.is_failure());
}

#[test]
fn run_is_interruptible_while_waiting_for_ready() {
    // Worker.test.ts:58 — run is pending, then interrupted.
    let mut run = WorkerRun::new();
    assert!(run.poll().is_pending());
    run.interrupt();
    let outcome = run.poll();
    assert!(outcome.is_interrupted());
}

#[test]
fn run_still_fails_on_worker_exit_after_ready() {
    // Worker.test.ts:73 — ready first, then the worker exits.
    let mut run = WorkerRun::new();
    run.signal_ready();
    assert!(run.poll().is_pending());
    run.signal_exit(exit_error());
    let outcome = run.poll();
    assert!(outcome.is_failure());
}
