//! Cluster rest: sharding config, runner storage, resource map.

#[path = "../src/cluster.rs"]
#[allow(dead_code)]
mod cluster;

use cluster::{
    ClusterError, Delivery, MemoryRunnerStorage, PlainJsonCodec, ReplyRegistry, ResourceMap,
    RunnerCodec, RunnerHealth, RunnerReply, RunnerRequest, RunnerTransport, Runners, ShardingConfig,
    StoredReply, WaiterFault,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn sharding_config_defaults_and_validation() {
    let defaults = ShardingConfig::defaults().validated().unwrap();
    assert_eq!(defaults.runner_residency_secs, 300);
    assert_eq!(defaults.storage_batch, 100);
    assert_eq!(defaults.listen_address(), None);

    let bad_residency = ShardingConfig {
        runner_residency_secs: 0,
        ..ShardingConfig::defaults()
    };
    assert!(bad_residency.validated().is_err());

    let bad_batch = ShardingConfig {
        storage_batch: 0,
        ..ShardingConfig::defaults()
    };
    assert!(bad_batch.validated().is_err());
}

#[test]
fn sharding_config_listen_address_needs_host() {
    let no_host = ShardingConfig {
        listen_host: None,
        listen_port: Some(8080),
        ..ShardingConfig::defaults()
    };
    assert_eq!(no_host.listen_address(), None);

    let with_host = ShardingConfig {
        listen_host: Some("0.0.0.0".to_string()),
        listen_port: Some(8080),
        ..ShardingConfig::defaults()
    };
    assert_eq!(with_host.listen_address(), Some("0.0.0.0:8080".to_string()));
}

#[test]
fn runner_storage_tracks_health_in_memory() {
    let mut s = MemoryRunnerStorage::new();
    assert_eq!(s.health("r1"), None);
    s.register("r1");
    assert_eq!(s.health("r1"), Some(RunnerHealth::Healthy));
}

#[test]
fn runner_storage_acquires_one_shot_iterables() {
    let mut s = MemoryRunnerStorage::new();
    s.acquire_all(vec!["a".to_string(), "b".to_string()].into_iter());
    assert_eq!(s.health("a"), Some(RunnerHealth::Healthy));
    assert_eq!(s.health("b"), Some(RunnerHealth::Healthy));
}

#[test]
fn resource_map_closes_failed_lookup_scopes() {
    let mut m: ResourceMap<String> = ResourceMap::new();
    let out: Result<&String, &str> = m.get_or_load("k", "scope-1", || Err("boom"));
    assert!(out.is_err());
    assert_eq!(m.closed_scopes, vec!["scope-1".to_string()]);

    let ok: Result<&String, &str> = m.get_or_load("k", "scope-2", || Ok("v".to_string()));
    assert_eq!(ok.unwrap(), "v");
    assert_eq!(m.closed_scopes.len(), 1);
}

#[test]
fn replies_for_returns_unacked_chunks_and_exits() {
    let mut reg = ReplyRegistry::new();
    assert!(reg.replies_for("r1").is_empty());
    reg.store(
        "r1",
        StoredReply::Chunk {
            payload: vec![1],
            acked: false,
        },
    );
    reg.store("r1", StoredReply::Exit { payload: vec![2] });
    assert_eq!(reg.replies_for("r1").len(), 2);
    assert!(reg.acknowledge("r1"));
    assert_eq!(
        reg.replies_for("r1"),
        vec![StoredReply::Exit { payload: vec![2] }]
    );
    assert!(!reg.acknowledge("r1"));
}

#[test]
fn parked_waiter_releases_when_reply_arrives() {
    let mut reg = ReplyRegistry::new();
    reg.register_waiter("r1", "shard-a");
    assert!(reg.is_parked("r1"));
    assert_eq!(reg.poll_waiter("r1"), None);
    reg.store("r1", StoredReply::Exit { payload: vec![9] });
    assert_eq!(reg.poll_waiter("r1"), Some("r1".to_string()));
    assert!(!reg.is_parked("r1"));
}

#[test]
fn unregister_shard_fails_parked_waiters_on_rebalance() {
    let mut reg = ReplyRegistry::new();
    reg.register_waiter("r1", "shard-a");
    reg.register_waiter("r2", "shard-b");
    let released = reg.unregister_shard("shard-a", false);
    assert_eq!(released, vec![("r1".to_string(), WaiterFault::NotAssigned)]);
    assert!(reg.is_parked("r2"));
}

#[test]
fn unregister_shard_abandons_parked_waiters_on_shutdown() {
    let mut reg = ReplyRegistry::new();
    reg.register_waiter("r1", "shard-a");
    let released = reg.unregister_shard("shard-a", true);
    assert_eq!(released, vec![("r1".to_string(), WaiterFault::Abandoned)]);
    assert!(reg.unregister_shard("shard-a", true).is_empty());
}

#[test]
fn reset_requests_preserves_replies_and_completion() {
    use cluster::MessageStorage;
    use cluster::{EntityAddress, StoredRequest};
    let mut storage = MessageStorage::new();
    let mut reg = ReplyRegistry::new();
    let addr = EntityAddress::new("T", "1");
    storage
        .save(StoredRequest {
            id: "stream".to_string(),
            address: addr.clone(),
            claimed_by: None,
            complete: false,
        })
        .unwrap();
    storage
        .save(StoredRequest {
            id: "done".to_string(),
            address: addr.clone(),
            claimed_by: None,
            complete: false,
        })
        .unwrap();
    reg.store(
        "stream",
        StoredReply::Chunk {
            payload: vec![1, 2, 3],
            acked: false,
        },
    );
    reg.store("done", StoredReply::Exit { payload: vec![4] });
    storage.complete("done");
    assert_eq!(reg.replies_for("stream").len(), 1);
    storage.reset_requests(&["stream", "done"]);
    assert_eq!(reg.replies_for("stream").len(), 1);
    assert_eq!(reg.replies_for("done").len(), 1);
    assert!(storage.unprocessed_messages(&[addr], None).is_ok());
}

#[test]
fn runner_endpoint_preserves_leading_slash() {
    use cluster::RunnerAddress;
    let addr = RunnerAddress::make("localhost", 8080);
    assert_eq!(
        addr.endpoint("http", "/runner"),
        "http://localhost:8080/runner"
    );
    assert_eq!(addr.endpoint("ws", "/runner"), "ws://localhost:8080/runner");
}

#[test]
fn runner_endpoint_adds_missing_leading_slash() {
    use cluster::RunnerAddress;
    let addr = RunnerAddress::make("localhost", 8080);
    assert_eq!(
        addr.endpoint("http", "runner"),
        "http://localhost:8080/runner"
    );
    assert_eq!(
        addr.endpoint("https", "runner"),
        "https://localhost:8080/runner"
    );
}

#[test]
fn runner_endpoint_brackets_bare_ipv6() {
    use cluster::RunnerAddress;
    let addr = RunnerAddress::make("::1", 8080);
    assert_eq!(addr.endpoint("http", "/runner"), "http://[::1]:8080/runner");
}

#[test]
fn unencodable_reply_stores_malformed_defect() {
    use cluster::{ReplyRegistry, StoredReply};
    let mut reg = ReplyRegistry::new();
    reg.register_waiter("bad-1", "shard-a");
    let stored = reg.store_encoded("bad-1", || Err("not json encodable".to_string()));
    assert_eq!(
        stored,
        StoredReply::Defect {
            reason: "MalformedMessage: not json encodable".to_string()
        }
    );
    assert_eq!(reg.replies_for("bad-1"), vec![stored]);
    assert_eq!(reg.poll_waiter("bad-1"), Some("bad-1".to_string()));
}

#[test]
fn failed_reply_serialization_isolates_one_request() {
    use cluster::{ReplyRegistry, StoredReply};
    let mut reg = ReplyRegistry::new();
    reg.register_waiter("bad-1", "shard-a");
    reg.register_waiter("good-1", "shard-a");
    reg.store_encoded("bad-1", || Err("not json encodable".to_string()));
    reg.store_encoded("good-1", || Ok(vec![7, 8]));
    assert!(matches!(
        reg.replies_for("bad-1")[0],
        StoredReply::Defect { .. }
    ));
    assert_eq!(
        reg.replies_for("good-1"),
        vec![StoredReply::Exit {
            payload: vec![7, 8]
        }]
    );
    assert_eq!(reg.poll_waiter("good-1"), Some("good-1".to_string()));
}

#[test]
fn stream_reply_ends_with_serialization_defect() {
    use cluster::{ReplyRegistry, StoredReply};
    let mut reg = ReplyRegistry::new();
    reg.store_encoded_chunk("s1", || Ok(vec![1, 2]));
    let tail = reg.store_encoded_chunk("s1", || Err("not json encodable".to_string()));
    assert!(matches!(tail, StoredReply::Defect { .. }));
    let stored = reg.replies_for("s1");
    assert_eq!(stored.len(), 2);
    assert!(matches!(stored.last(), Some(StoredReply::Defect { .. })));
}

#[test]
fn entity_activation_scope_per_entity_closes_with_client() {
    use cluster::EntityClients;
    use std::sync::{Arc, Mutex};
    let closed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let clients = EntityClients::new();
        for id in ["one", "two"] {
            let scope = clients.activate(id);
            assert_eq!(scope.entity_id(), id);
            let sink = Arc::clone(&closed);
            let owned = id.to_string();
            scope.add_finalizer(move || sink.lock().unwrap().push(owned));
        }
        let activations = clients.activations();
        assert_eq!(activations.len(), 2);
        assert!(!Arc::ptr_eq(&activations[0], &activations[1]));
        assert!(activations.iter().all(|s| !s.is_closed()));
        assert!(closed.lock().unwrap().is_empty());
    }
    let mut seen = closed.lock().unwrap().clone();
    seen.sort();
    assert_eq!(seen, vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn entity_scope_closes_finalizers_newest_first_once() {
    use cluster::EntityScope;
    let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let scope = EntityScope::new("counter");
    for step in ["first", "second"] {
        let order = std::sync::Arc::clone(&order);
        let name = step.to_string();
        scope.add_finalizer(move || order.lock().unwrap().push(name));
    }
    scope.close();
    scope.close();
    assert!(scope.is_closed());
    assert_eq!(*order.lock().unwrap(), vec!["second", "first"]);
}

#[test]
fn entity_scope_rejects_finalizers_after_close() {
    use cluster::EntityScope;
    let scope = EntityScope::new("counter");
    scope.close();
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&ran);
    scope.add_finalizer(move || flag.store(true, std::sync::atomic::Ordering::SeqCst));
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn entity_clients_close_activations_on_demand() {
    use cluster::EntityClients;
    let clients = EntityClients::new();
    clients.activate("a");
    clients.activate("b");
    let activations = clients.activations();
    clients.close();
    assert!(activations.iter().all(|scope| scope.is_closed()));
    clients.close();
}

// --- RunnerServer.test.ts rows ---

// RunnerServer.test.ts:136 — a successful runner stream completes: the value
// arrives as a chunk, the client ack hides it, and the terminal exit remains.
#[test]
fn runner_stream_completes_with_exit_after_ack() {
    let mut registry = ReplyRegistry::new();
    registry.succeed_stream("req", &[1]);
    assert_eq!(entity_values(&registry, "req"), vec![1]);
    assert!(registry.is_complete("req"));
    assert!(registry.acknowledge("req"));
    assert!(entity_values(&registry, "req").is_empty());
    assert_eq!(
        registry.replies_for("req"),
        vec![StoredReply::Exit { payload: Vec::new() }]
    );
    assert!(registry.is_complete("req"));
}

// RunnerServer.test.ts:380 — the serialization codec fills the entity payload
// and reply holes: the payload survives the codec round trip and the envelope
// carries it already encoded.
#[test]
fn runner_codec_fills_payload_and_reply_holes() {
    let codec = CountingCodec::new("codecForJsonString");
    let calls = Arc::clone(&codec.calls);
    let runners = Runners::new(
        Box::new(codec),
        Box::new(StaticTransport {
            outcome: Delivery::Delivered,
        }),
    );
    let mut request = RunnerRequest::new("1", "Double", r#"{"id":21}"#, false);
    let mut received = None;
    runners
        .send_local(&mut request, true, &mut |incoming| received = Some(incoming))
        .expect("a local send does not fail");
    let incoming = received.expect("the local send delivered");
    assert_eq!(incoming.payload, r#"{"id":21}"#);
    assert_eq!(request.encoded_by.as_deref(), Some("codecForJsonString"));
    let wire = runners.encode_request(&request);
    assert!(
        wire.starts_with('"') && wire.ends_with('"'),
        "the outer payload is a JSON string: {wire}"
    );
    let back = runners.decode_reply(&wire);
    assert!(
        back.contains(r#""payload":"{\"id\":21}""#),
        "the entity payload is carried, not re-encoded: {back}"
    );
    assert!(calls.load(Ordering::SeqCst) >= 3, "the codec filled the holes");
}

// --- Entity server (Entity.test.ts rows) ---

fn entity_values(registry: &ReplyRegistry, request_id: &str) -> Vec<i64> {
    registry
        .replies_for(request_id)
        .into_iter()
        .filter_map(|reply| match reply {
            StoredReply::Chunk { payload, .. } => Some(i64::from_le_bytes(
                payload.try_into().expect("a chunk carries one i64"),
            )),
            _ => None,
        })
        .collect()
}

fn entity_stream_ended(registry: &ReplyRegistry, request_id: &str) -> bool {
    registry
        .replies_for(request_id)
        .iter()
        .any(|reply| matches!(reply, StoredReply::Exit { .. }))
}

// --- Runners.makeRpc (Runners.test.ts rows) ---

/// The pin's `codecForJsonString`: every payload becomes a JSON string, so a
/// round trip through it is visible in the wire text. Counts its calls,
/// which is how the pin's row proves the codec ran.
struct CountingCodec {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}

impl CountingCodec {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RunnerCodec for CountingCodec {
    fn name(&self) -> &str {
        self.name
    }

    fn encode(&self, text: &str) -> String {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Rust's debug quoting is the JSON string form for these payloads.
        format!("{text:?}")
    }

    fn decode(&self, text: &str) -> String {
        self.calls.fetch_add(1, Ordering::SeqCst);
        PlainJsonCodec.decode(text)
    }
}

/// A transport that always answers the same way.
struct StaticTransport {
    outcome: Delivery,
}

impl RunnerTransport for StaticTransport {
    fn send(&self, _request: &RunnerRequest) -> Delivery {
        self.outcome.clone()
    }

    fn notify(&self, _request: &RunnerRequest) -> Delivery {
        self.outcome.clone()
    }
}

fn runners(codec: Box<dyn RunnerCodec>, outcome: Delivery) -> Runners {
    Runners::new(codec, Box::new(StaticTransport { outcome }))
}

// Runners.test.ts:315 — `simulateRemoteSerialization` runs the payload
// through the protocol codec (one encode, one decode), records that codec on
// the message, and the receiver sees the decoded payload.
#[test]
fn runner_send_local_uses_the_protocol_codec() {
    let codec = CountingCodec::new("codecFor");
    let calls = Arc::clone(&codec.calls);
    let runners = runners(Box::new(codec), Delivery::Delivered);
    let mut request = RunnerRequest::new("0", "TestRpc", r#"{"id":1}"#, false);

    let mut received = None;
    runners
        .send_local(&mut request, true, &mut |incoming| received = Some(incoming))
        .expect("a local send does not fail");
    let incoming = received.expect("the local send delivered");
    assert_eq!(
        incoming.payload, r#"{"id":1}"#,
        "the payload survives the codec round trip"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2, "one encode and one decode");
    assert_eq!(request.encoded_by.as_deref(), Some("codecFor"));

    // A later encode with another codec re-encodes and re-records, which is
    // the pin's `Message.serialize(message, codecForJson)` assertion.
    let plain = PlainJsonCodec;
    request.encode_with(&plain);
    assert_eq!(request.encoded_by.as_deref(), Some("json"));
}

// Runners.test.ts:362 — a server-delivered defect is a delivery: the request
// resolves with a failure exit instead of the send failing.
#[test]
fn runner_send_delivered_defect_resolves_the_request() {
    let runners = runners(Box::new(PlainJsonCodec), Delivery::Defect("boom".to_owned()));
    let mut request = RunnerRequest::new("1", "TestRpc", r#"{"id":1}"#, false);

    let mut replies = Vec::new();
    let result = runners.send(&mut request, &mut |reply| replies.push(reply));
    assert!(result.is_ok(), "a delivered defect is not a transport failure");
    assert_eq!(
        replies,
        vec![RunnerReply::WithExit {
            request_id: "1".to_owned(),
            failure: Some("boom".to_owned()),
        }]
    );
}

// Runners.test.ts:386 — an undelivered request is RunnerUnavailable.
#[test]
fn runner_send_transport_failure_maps_to_runner_unavailable() {
    let runners = runners(
        Box::new(PlainJsonCodec),
        Delivery::TransportFailure("socket closed".to_owned()),
    );
    let mut request = RunnerRequest::new("1", "TestRpc", r#"{"id":1}"#, false);

    match runners.send(&mut request, &mut |_| panic!("no reply for an undelivered request")) {
        Err(ClusterError::RunnerUnavailable(message)) => {
            assert!(message.contains("socket closed"), "got {message}")
        }
        Err(other) => panic!("expected RunnerUnavailable, got {}", other.message()),
        Ok(()) => panic!("expected RunnerUnavailable"),
    }
}

// Runners.test.ts:399 — a volatile notification whose transport failed is
// RunnerUnavailable, because nothing would retry it.
#[test]
fn runner_notify_volatile_transport_failure_maps_to_runner_unavailable() {
    let runners = runners(
        Box::new(PlainJsonCodec),
        Delivery::TransportFailure("socket closed".to_owned()),
    );
    let request = RunnerRequest::new("1", "TestRpc", r#"{"id":1}"#, false);

    match runners.notify(&request) {
        Err(ClusterError::RunnerUnavailable(message)) => {
            assert!(message.contains("socket closed"), "got {message}")
        }
        Err(other) => panic!("expected RunnerUnavailable, got {}", other.message()),
        Ok(()) => panic!("expected RunnerUnavailable"),
    }
}

// Runners.test.ts:416 — a persisted notification is recovered from storage,
// so its transport failure is ignored.
#[test]
fn runner_notify_persisted_transport_failure_is_ignored() {
    let runners = runners(
        Box::new(PlainJsonCodec),
        Delivery::TransportFailure("socket closed".to_owned()),
    );
    let request = RunnerRequest::new("1", "TestRpcPersisted", r#"{"id":1}"#, true);

    assert!(
        runners.notify(&request).is_ok(),
        "storage recovery owns a persisted notification"
    );
}

// Runners.test.ts:429 — the same delivered defect on a persisted request
// takes the storage-recovery path instead of resolving in place.
#[test]
fn runner_send_delivered_defect_on_persisted_request_maps_to_runner_unavailable() {
    let runners = runners(Box::new(PlainJsonCodec), Delivery::Defect("boom".to_owned()));
    let mut request = RunnerRequest::new("1", "TestRpcPersisted", r#"{"id":1}"#, true);

    match runners.send(&mut request, &mut |_| panic!("a persisted defect resolves through storage")) {
        Err(ClusterError::RunnerUnavailable(message)) => {
            assert!(message.contains("boom"), "got {message}")
        }
        Err(other) => panic!("expected RunnerUnavailable, got {}", other.message()),
        Ok(()) => panic!("expected RunnerUnavailable"),
    }
}

// Runners.test.ts:442 — the outer runner payload is written by the protocol
// codec and the entity payload rides the hole already encoded.
#[test]
fn runner_send_encodes_the_entity_payload_and_leaves_the_hole() {
    let runners = runners(
        Box::new(CountingCodec::new("codecForJsonString")),
        Delivery::Defect("boom".to_owned()),
    );
    let request = RunnerRequest::new("2", "TestRpc", r#"{"id":1}"#, false);

    let wire = runners.encode_request(&request);
    assert!(
        wire.starts_with('"') && wire.ends_with('"'),
        "the outer payload is a JSON string: {wire}"
    );
    let envelope = PlainJsonCodec.decode(&wire);
    assert!(
        envelope.contains(r#""payload":"{\"id\":1}""#),
        "the entity payload is carried, not re-encoded: {envelope}"
    );
}

#[test]
fn entity_server_does_not_freeze_the_client_context() {
    // Entity.test.ts:98 — a service provided on the acquiring fiber stays
    // with the client; the entity server observes none of it.
    use cluster::{EntityClient, EntityServer};
    let client = EntityClient::acquire("1").with_service("CallerId", "A");
    let server = EntityServer::new("1", None);

    assert_eq!(client.service("CallerId"), Some("A"), "the client holds it");
    assert_eq!(server.service("CallerId"), None, "the server does not");
}

#[test]
fn entity_fatal_defects_isolate_per_flag() {
    // Entity.test.ts:110, an it.each over [true, false, omitted]: the bad
    // request always dies with the original defect, and the good request
    // survives only while fatal defects are disabled.
    use cluster::{EntityServer, HandlerOutcome, ReplyOutcome};
    for (flag, good_survives) in [(Some(true), true), (Some(false), false), (None, false)] {
        let mut server = EntityServer::new("one", flag);
        assert_eq!(
            server.run(HandlerOutcome::Defect("fixture defect".to_string())),
            ReplyOutcome::Defect("fixture defect".to_string()),
            "bad request, disableFatalDefects = {flag:?}"
        );
        let good = server.run(HandlerOutcome::Success(42));
        if good_survives {
            assert_eq!(good, ReplyOutcome::Success(42), "flag {flag:?}");
        } else {
            assert_eq!(
                good,
                ReplyOutcome::Defect("fixture defect".to_string()),
                "the original defect reaches the next request, flag {flag:?}"
            );
        }
    }
}

#[test]
fn entity_typed_failures_stay_request_local() {
    // Entity.test.ts:121, the same it.each: a typed failure never poisons
    // the entity, so the next request succeeds under every label.
    use cluster::{EntityServer, HandlerOutcome, ReplyOutcome};
    for flag in [Some(true), Some(false), None] {
        let mut server = EntityServer::new("one", flag);
        assert_eq!(
            server.run(HandlerOutcome::TypedFailure("typed failure".to_string())),
            ReplyOutcome::TypedFailure("typed failure".to_string()),
            "flag {flag:?}"
        );
        assert_eq!(
            server.run(HandlerOutcome::Success(42)),
            ReplyOutcome::Success(42),
            "flag {flag:?}"
        );
    }
}

#[test]
fn entity_fatal_defects_do_not_cross_entity_ids() {
    // Entity.test.ts:129 — the fatal state belongs to one entity id; the
    // neighbouring entity keeps answering.
    use cluster::{EntityServer, HandlerOutcome, ReplyOutcome};
    let mut bad = EntityServer::new("bad", Some(false));
    let mut good = EntityServer::new("good", Some(false));

    assert_eq!(
        bad.run(HandlerOutcome::Defect("fixture defect".to_string())),
        ReplyOutcome::Defect("fixture defect".to_string())
    );
    assert_eq!(
        good.run(HandlerOutcome::Success(42)),
        ReplyOutcome::Success(42)
    );
}

#[test]
fn entity_stream_reply_answers_with_a_stream() {
    // Entity.test.ts:138 — `replier.succeed(req, Stream.make(1, 2, 3))`
    // reaches the client as three chunks and an end.
    let mut registry = ReplyRegistry::new();
    registry.succeed_stream("req", &[1, 2, 3]);

    assert_eq!(entity_values(&registry, "req"), vec![1, 2, 3]);
    assert!(entity_stream_ended(&registry, "req"));
}

#[test]
fn entity_queue_reply_answers_with_a_dequeue() {
    // Entity.test.ts:159 — a dequeue offered 1, 2, 3 and then ended; an
    // un-ended dequeue leaves the stream open.
    use cluster::ReplyQueue;
    let mut queue = ReplyQueue::new();
    for value in [1, 2, 3] {
        queue.offer(value);
    }
    queue.end();
    let mut registry = ReplyRegistry::new();
    registry.succeed_queue("req", &queue);

    assert_eq!(entity_values(&registry, "req"), vec![1, 2, 3]);
    assert!(entity_stream_ended(&registry, "req"));

    let mut pending = ReplyQueue::new();
    pending.offer(1);
    let mut open = ReplyRegistry::new();
    open.succeed_queue("req", &pending);
    assert_eq!(entity_values(&open, "req"), vec![1]);
    assert!(!entity_stream_ended(&open, "req"));
}
