//! `packages/effect/test/workers/WorkerError.test.ts` witnesses.

use elm_effect_unstable::workers::{
    BufferedWorker, WorkerError, WorkerErrorReason, is_worker_error,
};

#[test]
fn delegates_message_from_reason() {
    // WorkerError.test.ts:16
    let error = WorkerError::new(WorkerErrorReason::send("Failed to send message to worker"));
    assert_eq!(error.message(), "Failed to send message to worker");
    assert_eq!(error.reason.tag(), "WorkerSendError");
}

#[test]
fn worker_error_delegates_message_and_cause_from_reason() {
    // WorkerError.test.ts:27
    let cause = serde_json::json!({"name":"Error","message":"boom"});
    let reason = WorkerErrorReason::spawn("Failed to spawn worker").with_cause(cause.clone());
    assert_eq!(reason.message(), "Failed to spawn worker");
    assert_eq!(reason.cause(), Some(&cause));
}

#[test]
fn matches_wrapper_only() {
    // WorkerError.test.ts:40
    let wrapper = WorkerError::new(WorkerErrorReason::receive("Failed to receive message"));
    assert!(is_worker_error(&wrapper));
    assert!(!is_worker_error(&WorkerErrorReason::receive(
        "Failed to receive message"
    )));
    let plain = std::io::Error::new(std::io::ErrorKind::Other, "regular error");
    assert!(!is_worker_error(&plain));
    struct Decoy {
        _tag: &'static str,
    }
    let decoy = Decoy {
        _tag: "WorkerError",
    };
    assert!(!is_worker_error(&decoy));
}

#[test]
fn worker_spawn_error_roundtrip() {
    // WorkerError.test.ts:55
    let reason = WorkerErrorReason::spawn("Failed to spawn worker");
    let encoded = reason.encode();
    let decoded = WorkerErrorReason::decode(&encoded).unwrap();
    assert_eq!(decoded.tag(), "WorkerSpawnError");
    assert_eq!(decoded.message(), "Failed to spawn worker");
}

#[test]
fn worker_error_reason_union_roundtrip() {
    // WorkerError.test.ts:66
    let reason = WorkerErrorReason::unknown("Worker crashed unexpectedly");
    let encoded = reason.encode();
    let decoded = WorkerErrorReason::decode(&encoded).unwrap();
    assert_eq!(decoded.tag(), "WorkerUnknownError");
    assert_eq!(decoded.message(), "Worker crashed unexpectedly");
}

#[test]
fn worker_error_roundtrip() {
    // WorkerError.test.ts:77
    let error = WorkerError::new(WorkerErrorReason::send("Failed to send message"));
    let encoded = error.encode();
    let decoded = WorkerError::decode(&encoded).unwrap();
    assert_eq!(decoded.tag(), "WorkerError");
    assert_eq!(decoded.reason.tag(), "WorkerSendError");
    assert_eq!(decoded.message(), "Failed to send message");
}

#[test]
fn buffered_send_reports_post_failure_as_send_error() {
    // WorkerError.test.ts:93 reports postMessage failures as WorkerSendError.
    // Send buffers before ready; the ready flush calls postMessage, which
    // throws, so the run fails with WorkerSendError and no panic (no Die).
    let mut worker = BufferedWorker::new(|_| Err("post failed".to_owned()));
    worker.send(b"buffered".to_vec()).unwrap();
    let err = worker.mark_ready_and_flush().unwrap_err();
    assert!(is_worker_error(&err));
    assert_eq!(err.reason.tag(), "WorkerSendError");
    assert_eq!(err.message(), "Failed to send message to worker");
}
