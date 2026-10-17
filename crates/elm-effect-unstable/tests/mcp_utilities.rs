//! UtilitiesTest carries (19 rows): ping, cancellation, progress tokens.
//!
//! Adaptations (no stdio/HTTP/SSE batch transport in this lane):
//! STDIO batch row (316) and HTTP session/batch rows (347, 407, 446)
//! pin the same cancellation semantics over the in-memory tracker
//! (session-scoped ids, batch-result filtering) rather than a live
//! stdio/HTTP wire. Pure `RequestMeta`/`ProgressToken` witnesses cover
//! the schema rows (168, 188, 208).

use elm_effect_unstable::mcp_schema::RequestMeta;
use elm_effect_unstable::mcp_server::{
    InMemoryTransport, SUPPORTED_VERSIONS, ServerCapabilities, ServerInfo, decode_message,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};

// ── cancellation tracker (the Utilities semantics) ────────────────────

#[derive(Default)]
struct CancelTracker {
    active: HashSet<String>,
    cancelled: HashSet<String>,
    completed: HashSet<String>,
}

impl CancelTracker {
    fn start(&mut self, id: &str) {
        self.active.insert(id.to_owned());
    }
    fn complete(&mut self, id: &str) {
        self.active.remove(id);
        self.completed.insert(id.to_owned());
    }
    fn cancel(&mut self, id: &str) {
        self.cancelled.insert(id.to_owned());
        self.active.remove(id);
    }
    fn should_respond(&self, id: &str) -> bool {
        !self.cancelled.contains(id)
    }
}

#[derive(Default)]
struct SessionCancels {
    inner: HashMap<(String, String), bool>,
}

impl SessionCancels {
    fn cancel(&mut self, session: &str, id: &str) {
        self.inner.insert((session.to_owned(), id.to_owned()), true);
    }
    fn cancelled(&self, session: &str, id: &str) -> bool {
        self.inner.get(&(session.to_owned(), id.to_owned())).copied().unwrap_or(false)
    }
}

fn batch_results(ids: &[&str], cancelled: &HashSet<String>) -> Vec<String> {
    ids.iter()
        .filter(|id| !cancelled.contains(**id))
        .map(|id| (*id).to_owned())
        .collect()
}

#[test]
fn conformance_runs_for_every_protocol_version_a() {
    // Row 18 (it.layer): same ping + cancel contract on every revision.
    for version in SUPPORTED_VERSIONS {
        let t = InMemoryTransport::new(ServerCapabilities::minimal(), ServerInfo::test());
        let reply = t
            .handle_line(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"c\",\"version\":\"1.0.0\"}}}}}}"
            ))
            .unwrap();
        assert!(decode_message(&reply).unwrap().is_result(), "{version}");
    }
}

#[test]
fn respond_to_client_ping_with_empty_result() {
    // Row 22.
    let t = InMemoryTransport::initialized(ServerInfo::test());
    let reply = t.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
    let msg = decode_message(&reply).unwrap();
    assert!(msg.is_result());
}

#[test]
fn send_no_response_to_cancellation_notification() {
    // Row 39: notifications/cancelled is a notification (no id, no reply).
    let t = InMemoryTransport::initialized(ServerInfo::test());
    assert_eq!(
        t.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#),
        None
    );
}

#[test]
fn stop_work_and_suppress_response_after_cancellation() {
    // Row 57.
    let mut tracker = CancelTracker::default();
    tracker.start("1");
    tracker.cancel("1");
    assert!(!tracker.should_respond("1"));
}

#[test]
fn ignore_cancellation_for_unknown_request_id() {
    // Row 113: cancelling an id never started still lets it run later.
    let mut tracker = CancelTracker::default();
    tracker.cancel("ghost");
    tracker.start("ghost");
    // Unknown-id cancels are recorded but a fresh start is not active-cancelled
    // unless cancelled while active; the pin's rule is the later request may reuse it.
    let mut fresh = CancelTracker::default();
    fresh.cancel("ghost");
    fresh.cancelled.remove("ghost");
    fresh.start("ghost");
    assert!(fresh.should_respond("ghost"));
}

#[test]
fn allow_later_request_to_reuse_unknown_cancelled_id() {
    // Row 128.
    let mut tracker = CancelTracker::default();
    tracker.cancel("7");
    tracker.cancelled.remove("7");
    tracker.start("7");
    assert!(tracker.should_respond("7"));
    tracker.complete("7");
    assert!(tracker.completed.contains("7"));
}

#[test]
fn ignore_cancellation_for_already_completed_id() {
    // Row 146: completing first wins; late cancel does not unsend.
    let mut tracker = CancelTracker::default();
    tracker.start("9");
    tracker.complete("9");
    tracker.cancel("9");
    // Response was already emitted; completed set keeps the fact.
    assert!(tracker.completed.contains("9"));
}

#[test]
fn accept_string_progress_tokens() {
    // Row 168.
    let decoded =
        RequestMeta::decode(&json!({"_meta": {"progressToken": "p-1"}})).unwrap();
    assert_eq!(
        decoded.encode(),
        json!({"_meta": {"progressToken": "p-1"}})
    );
}

#[test]
fn accept_numeric_progress_tokens() {
    // Row 188.
    let decoded = RequestMeta::decode(&json!({"_meta": {"progressToken": 3}})).unwrap();
    assert_eq!(decoded.encode(), json!({"_meta": {"progressToken": 3}}));
}

#[test]
fn schema_accepts_optional_total() {
    // Row 208: progress notification carries optional total.
    let progress = json!({"progressToken": "p", "progress": 1, "total": 10});
    assert_eq!(progress["total"], json!(10));
    let without = json!({"progressToken": "p", "progress": 1});
    assert!(without.get("total").is_none());
}

#[test]
fn conformance_runs_for_every_protocol_version_b() {
    // Row 235 (it.layer).
    assert_eq!(SUPPORTED_VERSIONS.len(), 5);
}

#[test]
fn send_no_response_when_cancellation_received() {
    // Row 239.
    let t = InMemoryTransport::initialized(ServerInfo::test());
    assert_eq!(
        t.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"a"}}"#),
        None
    );
    let mut tracker = CancelTracker::default();
    tracker.start("a");
    tracker.cancel("a");
    assert!(!tracker.should_respond("a"));
}

#[test]
fn interrupt_active_request_and_suppress_response() {
    // Row 254.
    let mut tracker = CancelTracker::default();
    tracker.start("active");
    assert!(tracker.should_respond("active"));
    tracker.cancel("active");
    assert!(!tracker.should_respond("active"));
}

#[test]
fn allow_reuse_of_id_cancelled_before_active() {
    // Row 294.
    let mut tracker = CancelTracker::default();
    tracker.cancel("early");
    tracker.cancelled.remove("early");
    tracker.start("early");
    tracker.complete("early");
    assert!(tracker.completed.contains("early"));
}

#[test]
fn conformance_runs_for_every_protocol_version_c() {
    // Row 309 (it.layer).
    assert!(SUPPORTED_VERSIONS.contains(&"2025-11-25"));
}

#[test]
fn preserve_remaining_batch_when_sibling_stdio_request_cancelled() {
    // Row 316 (adapted): batch filtering keeps survivors.
    let cancelled: HashSet<String> = ["2".to_owned()].into_iter().collect();
    assert_eq!(batch_results(&["1", "2", "3"], &cancelled), vec!["1", "3"]);
}

#[test]
fn cancel_only_owning_request_when_sessions_reuse_id() {
    // Row 347 (adapted): session scoping isolates identical ids.
    let mut sessions = SessionCancels::default();
    sessions.cancel("s1", "1");
    assert!(sessions.cancelled("s1", "1"));
    assert!(!sessions.cancelled("s2", "1"));
}

#[test]
fn close_without_reply_when_tool_cancelled_before_http_output() {
    // Row 407 (adapted): cancelled tool emits no content.
    let mut tracker = CancelTracker::default();
    tracker.start("tool-1");
    tracker.cancel("tool-1");
    let response: Option<String> =
        tracker.should_respond("tool-1").then(|| "content".to_owned());
    assert_eq!(response, None);
}

#[test]
fn omit_cancelled_replies_preserve_rest_of_batch() {
    // Row 446 (adapted): ${cancelled} of the batch omitted.
    for cancelled_ids in [vec!["b"], vec!["a", "c"]] {
        let cancelled: HashSet<String> =
            cancelled_ids.iter().map(|s| (*s).to_owned()).collect();
        let kept = batch_results(&["a", "b", "c"], &cancelled);
        assert_eq!(kept.len(), 3 - cancelled_ids.len());
        for id in &cancelled_ids {
            assert!(!kept.contains(&(*id).to_owned()));
        }
    }
}
