//! Cluster rest: sharding config, runner storage, resource map.

#[path = "../src/cluster.rs"]
#[allow(dead_code)]
mod cluster;

use cluster::{
    MemoryRunnerStorage, ReplyRegistry, ResourceMap, RunnerHealth, ShardingConfig, StoredReply,
    WaiterFault,
};

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
