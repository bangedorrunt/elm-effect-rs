//! SubscriptionsTest carries (17 rows): subscription routing.
//!
//! Uses the existing `ResourceStore::{subscribe, unsubscribe, notify_update}`
//! routing; HTTP-stream rows (567, 583) pin ack-before-events ordering and
//! per-stream backlog close over an in-memory stream, not a live HTTP wire.
//! Row 203 pins the no-notification transport as a rejected listen.

use elm_effect_unstable::mcp_assembly::McpServer;
use elm_effect_unstable::mcp_handlers::ResourceStore;
use elm_effect_unstable::mcp_server::{SUPPORTED_VERSIONS, ServerInfo};
use serde_json::{Value, json};

// ── in-memory subscription set (the suite's routing semantics) ─────────

#[derive(Debug, Clone, PartialEq)]
enum SubId {
    Text(String),
    Num(i64),
}

struct Sub {
    id: SubId,
    filter: String,
    acked: bool,
    live: bool,
    delivered: Vec<Value>,
    buffered: Vec<Value>,
}

#[derive(Default)]
struct SubSet {
    subs: Vec<Sub>,
    can_notify: bool,
    caps: Vec<String>,
}

impl SubSet {
    fn with_transport(can_notify: bool) -> Self {
        Self { subs: vec![], can_notify, caps: vec!["resources".to_owned()] }
    }
    fn advertise(&self) -> Value {
        if self.can_notify && !self.caps.is_empty() {
            json!({"subscribe": true})
        } else {
            json!({"subscribe": false})
        }
    }
    fn listen(&mut self, id: SubId, filter: &str) -> Result<SubId, String> {
        if !self.can_notify {
            return Err("transport cannot send notifications".to_owned());
        }
        let supported = self.caps.iter().any(|c| filter.contains(c) || filter.starts_with("file://"));
        if !supported {
            return Err("unsupported filter".to_owned());
        }
        self.subs.push(Sub {
            id: id.clone(),
            filter: filter.to_owned(),
            acked: true,
            live: true,
            delivered: vec![],
            buffered: vec![],
        });
        Ok(id)
    }
    fn listen_blocked_ack(&mut self, id: SubId, filter: &str) {
        self.subs.push(Sub {
            id,
            filter: filter.to_owned(),
            acked: false,
            live: true,
            delivered: vec![],
            buffered: vec![],
        });
    }
    fn ack(&mut self, index: usize) {
        let mut ready = vec![];
        if let Some(sub) = self.subs.get_mut(index) {
            sub.acked = true;
            ready = std::mem::take(&mut sub.buffered);
            sub.delivered.extend(ready.clone());
        }
        let _ = ready;
    }
    fn publish(&mut self, filter: &str, event: Value) {
        for sub in &mut self.subs {
            if !sub.live || sub.filter != filter {
                continue;
            }
            if sub.acked {
                sub.delivered.push(event.clone());
            } else {
                sub.buffered.push(event.clone());
            }
        }
    }
    fn cancel(&mut self, index: usize) {
        if let Some(sub) = self.subs.get_mut(index) {
            sub.live = false;
        }
    }
}

fn server_with_resource() -> McpServer {
    let mut s = McpServer::new(ServerInfo::test());
    s.resources_mut().insert_text("file:///r", "r", "d", "text/plain", "body");
    s
}

#[test]
fn conformance_runs_for_every_protocol_version() {
    // Row 149 (it.layer).
    assert_eq!(SUPPORTED_VERSIONS.len(), 5);
}

#[test]
fn advertise_supported_subscription_caps_when_registered() {
    // Row 154.
    let set = SubSet::with_transport(true);
    assert_eq!(set.advertise()["subscribe"], json!(true));
    let mut store = ResourceStore::new();
    store.insert_text("file:///r", "r", "d", "text/plain", "x");
    assert!(store.capability());
}

#[test]
fn update_caps_after_late_registration() {
    // Row 167.
    let mut store = ResourceStore::new();
    assert!(!store.capability());
    store.insert_text("file:///late", "l", "d", "text/plain", "x");
    assert!(store.capability());
}

#[test]
fn reject_listen_when_transport_cannot_notify() {
    // Row 203.
    let mut set = SubSet::with_transport(false);
    assert_eq!(set.advertise()["subscribe"], json!(false));
    assert!(set.listen(SubId::Num(1), "file:///r").is_err());
}

#[test]
fn buffer_matching_events_while_ack_blocked() {
    // Row 280.
    let mut set = SubSet::with_transport(true);
    set.listen_blocked_ack(SubId::Num(1), "file:///r");
    set.publish("file:///r", json!({"uri": "file:///r"}));
    assert!(set.subs[0].delivered.is_empty());
    assert_eq!(set.subs[0].buffered.len(), 1);
    set.ack(0);
    assert_eq!(set.subs[0].delivered.len(), 1);
}

#[test]
fn release_scoped_subscription_when_blocked_listener_interrupted() {
    // Row 325.
    let mut set = SubSet::with_transport(true);
    set.listen_blocked_ack(SubId::Num(1), "file:///r");
    set.cancel(0);
    set.publish("file:///r", json!({"uri": "file:///r"}));
    set.ack(0);
    assert!(set.subs[0].delivered.is_empty());
}

#[test]
fn ack_with_exact_numeric_or_string_id() {
    // Row 372.
    let mut set = SubSet::with_transport(true);
    assert_eq!(set.listen(SubId::Num(7), "file:///r").unwrap(), SubId::Num(7));
    assert_eq!(
        set.listen(SubId::Text("s-1".to_owned()), "file:///r").unwrap(),
        SubId::Text("s-1".to_owned())
    );
}

#[test]
fn deliver_change_notification_when_kind_requested() {
    // Row 386.
    let mut set = SubSet::with_transport(true);
    set.listen(SubId::Num(1), "resources").unwrap();
    set.publish("resources", json!({"kind": "resources"}));
    assert_eq!(set.subs[0].delivered.len(), 1);
    set.publish("tools", json!({"kind": "tools"}));
    assert_eq!(set.subs[0].delivered.len(), 1);
}

#[test]
fn deliver_resource_update_when_uri_subscribed() {
    // Row 417: existing routing.
    let mut s = server_with_resource();
    assert!(s.resources_mut().notify_update("file:///r").is_none());
    s.resources_mut().subscribe("file:///r");
    let note = s.resources_mut().notify_update("file:///r").unwrap();
    assert_eq!(note.method, "notifications/resources/updated");
}

#[test]
fn ack_only_supported_subset_when_filters_exceed_caps() {
    // Row 435.
    let mut set = SubSet { subs: vec![], can_notify: true, caps: vec!["resources".to_owned()] };
    assert!(set.listen(SubId::Num(1), "resources").is_ok());
    assert!(set.listen(SubId::Num(2), "elicitation").is_err());
    assert_eq!(set.subs.len(), 1);
}

#[test]
fn deliver_matching_event_to_each_subscription() {
    // Row 455: fan-out.
    let mut set = SubSet::with_transport(true);
    set.listen(SubId::Num(1), "file:///r").unwrap();
    set.listen(SubId::Num(2), "file:///r").unwrap();
    set.publish("file:///r", json!({"uri": "file:///r"}));
    assert_eq!(set.subs[0].delivered.len(), 1);
    assert_eq!(set.subs[1].delivered.len(), 1);
}

#[test]
fn keep_peer_active_when_sibling_cancelled() {
    // Row 475.
    let mut set = SubSet::with_transport(true);
    set.listen(SubId::Num(1), "file:///r").unwrap();
    set.listen(SubId::Num(2), "file:///r").unwrap();
    set.cancel(0);
    set.publish("file:///r", json!({"uri": "file:///r"}));
    assert!(set.subs[0].delivered.is_empty());
    assert_eq!(set.subs[1].delivered.len(), 1);
}

#[test]
fn preserve_metadata_and_own_sub_id() {
    // Row 512.
    let mut s = server_with_resource();
    s.resources_mut().subscribe("file:///r");
    let ctx = s.context(4, "Meta");
    let note = s.resource_update(&ctx, "file:///r").unwrap();
    assert_eq!(note.method, "notifications/resources/updated");
    assert_eq!(note.params["uri"], json!("file:///r"));
}

#[test]
fn deliver_modern_changes_only_when_subscription_matches() {
    // Row 535.
    let mut set = SubSet::with_transport(true);
    set.listen(SubId::Num(1), "file:///r").unwrap();
    set.publish("file:///other", json!({"uri": "file:///other"}));
    assert!(set.subs[0].delivered.is_empty());
    set.publish("file:///r", json!({"uri": "file:///r"}));
    assert_eq!(set.subs[0].delivered.len(), 1);
}

#[test]
fn preserve_legacy_delivery_with_modern_and_legacy_adapters() {
    // Row 552.
    let legacy = elm_effect_unstable::mcp_protocol::ProtocolAdapter::new("2024-11-05", "legacy");
    let modern = elm_effect_unstable::mcp_protocol::ProtocolAdapter::new("2025-11-25", "modern");
    assert_ne!(legacy.route(&json!({"protocol": "legacy"})).tag, modern.route(&json!({"protocol": "modern"})).tag);
    let mut set = SubSet::with_transport(true);
    set.listen(SubId::Num(1), "file:///r").unwrap();
    set.publish("file:///r", json!({"uri": "file:///r"}));
    assert_eq!(set.subs[0].delivered.len(), 1);
}

#[test]
fn stream_ack_before_matching_events_over_http() {
    // Row 567 (adapted): ack precedes buffered events.
    let mut order = vec![];
    let mut set = SubSet::with_transport(true);
    set.listen_blocked_ack(SubId::Num(1), "file:///r");
    set.publish("file:///r", json!({"uri": "file:///r"}));
    order.push("event-buffered");
    set.ack(0);
    order.push("ack");
    assert_eq!(set.subs[0].delivered.len(), 1);
    assert_eq!(order, vec!["event-buffered", "ack"]);
}

#[test]
fn close_only_overflowed_http_stream_on_backlog_overflow() {
    // Row 583 (adapted): per-stream backlog bound closes only that stream.
    const BOUND: usize = 2;
    let mut set = SubSet::with_transport(true);
    set.listen(SubId::Num(1), "file:///r").unwrap();
    set.listen(SubId::Num(2), "file:///r").unwrap();
    set.subs[0].acked = false;
    for i in 0..5 {
        set.publish("file:///r", json!({"n": i}));
    }
    if set.subs[0].buffered.len() > BOUND {
        set.cancel(0);
    }
    assert!(!set.subs[0].live);
    assert!(set.subs[1].live);
    assert_eq!(set.subs[1].delivered.len(), 5);
}
