//! Wave-12 persistence fence: store, limiter, queue, fake Redis.
//! Loads `persistence.rs` by path until the `persistence` feature lands
//! in the unstable manifest (BoldSummit-held).

#[path = "../src/persistence.rs"]
mod persistence;

use persistence::{FakeRedis, KeyValueStore, PersistedQueue, RateLimiter, Verdict};

// ------------------------------------------------------------------- store ---

#[test]
fn kv_set_get_missing() {
    let mut db = KeyValueStore::new();
    db.set("a", "1");
    assert_eq!(db.get("a"), Some("1".to_string()));
    assert_eq!(db.get("nope"), None);
    assert!(db.has("a"));
    assert!(!db.has("nope"));
}

#[test]
fn kv_remove_clear() {
    let mut db = KeyValueStore::new();
    db.set("a", "1");
    assert!(db.remove("a"));
    assert!(!db.remove("a"));
    db.set("b", "2");
    db.clear();
    assert!(!db.has("b"));
}

#[test]
fn kv_modify_paths() {
    let mut db = KeyValueStore::new();
    db.set("a", "1");
    assert_eq!(
        db.modify("a", |v| Some(v.unwrap() + "2")),
        Some("12".to_string())
    );
    assert_eq!(db.modify("missing", |_| None), None);
    assert!(!db.has("missing"));
}

#[test]
fn kv_set_many_no_ttl() {
    let mut db = KeyValueStore::new();
    db.set_many(&[("a", "1"), ("b", "2")]);
    assert_eq!(db.get("a"), Some("1".to_string()));
    assert_eq!(db.get("b"), Some("2".to_string()));
}

#[test]
fn kv_prefix_scopes_keys() {
    let mut db = KeyValueStore::new();
    let mut scoped = db.prefixed("prefix/");
    scoped.set("foo", "bar");
    scoped.modify("foo", |v| Some(v.unwrap() + "bar"));
    assert_eq!(scoped.get("foo"), Some("barbar".to_string()));
    assert!(scoped.has("foo"));
    drop(scoped);
    assert_eq!(db.get("prefix//foo"), Some("barbar".to_string()));
}

#[test]
fn kv_ttl_rounds_up_and_expires() {
    let mut db = KeyValueStore::new();
    db.set_ttl("a", "1", 1.5);
    assert_eq!(db.get("a"), Some("1".to_string()));
    db.advance(1);
    assert_eq!(db.get("a"), Some("1".to_string()));
    db.advance(1);
    assert_eq!(db.get("a"), None);
}

// ----------------------------------------------------------------- limiter ---

#[test]
fn limiter_fixed_window_reset() {
    let rl = RateLimiter::fixed_window(100, 2);
    assert!(matches!(rl.consume("k"), Verdict::Admit { .. }));
    assert!(matches!(rl.consume("k"), Verdict::Admit { .. }));
    match rl.consume("k") {
        Verdict::Reject { retry_after_ms } => assert!(retry_after_ms <= 100),
        Verdict::Admit { .. } => panic!("window exhausted"),
    }
    rl.advance(100);
    assert!(matches!(rl.consume("k"), Verdict::Admit { .. }));
}

#[test]
fn limiter_token_refill() {
    let rl = RateLimiter::token_bucket(2.0, 1.0);
    assert!(matches!(rl.consume_tokens("k", 2.0), Verdict::Admit { .. }));
    assert!(matches!(
        rl.consume_tokens("k", 1.0),
        Verdict::Reject { .. }
    ));
    rl.advance(1);
    assert!(matches!(rl.consume_tokens("k", 1.0), Verdict::Admit { .. }));
}

#[test]
fn limiter_adaptive_cooldown_clamps() {
    let rl = RateLimiter::token_bucket(10.0, 1.0);
    // Zero feedback clamps to the minimum cooldown.
    assert_eq!(rl.feedback_429("k", 0), 1000);
    assert_eq!(rl.cooldown_remaining("k"), 1000);
    // Excessive feedback clamps to the maximum.
    assert_eq!(rl.feedback_429("k", 10_000_000), 60_000);
    rl.advance(60_000);
    assert_eq!(rl.cooldown_remaining("k"), 0);
}

#[test]
fn limiter_learns_after_cooldown() {
    let rl = RateLimiter::token_bucket(10.0, 1.0);
    rl.feedback_429("k", 1000);
    assert!(matches!(rl.admit_or_learn("k"), Verdict::Reject { .. }));
    rl.advance(1000);
    assert!(matches!(rl.admit_or_learn("k"), Verdict::Admit { .. }));
    rl.note_accepted("k");
    rl.expire_learned("k");
    assert!(matches!(rl.admit_or_learn("k"), Verdict::Admit { .. }));
}

// ------------------------------------------------------------------- queue ---

#[test]
fn queue_retry_grows_with_attempts() {
    let mut q = PersistedQueue::new();
    assert_eq!(q.retry_delay("k", 100), 100);
    assert_eq!(q.retry_delay("k", 100), 200);
    assert_eq!(q.retry_delay("k", 100), 400);
}

#[test]
fn queue_cleanup_prunes() {
    let mut q = PersistedQueue::new();
    q.complete("a", 0);
    q.complete("b", 100);
    assert_eq!(q.completed_len(), 2);
    assert_eq!(q.prune(150, 200), 1);
    assert_eq!(q.completed_len(), 1);
}

// ------------------------------------------------------------------- redis ---

#[test]
fn redis_clear_empty_succeeds() {
    let mut r = FakeRedis::new();
    r.clear();
    r.subscribe("a").unwrap();
    r.clear();
    assert_eq!(r.dequeue("a", false).unwrap(), None);
}

#[test]
fn redis_proto_key_survives() {
    let mut r = FakeRedis::new();
    let sha = r.script_load("__proto__ = 1");
    assert!(r.eval_sha(&sha).is_ok());
    assert_eq!(r.eval_sha("nope"), Err("NOSCRIPT".to_string()));
    r.script_reload();
    assert_eq!(r.eval_sha(&sha), Err("NOSCRIPT".to_string()));
}

#[test]
fn redis_subscribe_dequeue_shutdown() {
    let mut r = FakeRedis::new();
    r.subscribe("news").unwrap();
    assert_eq!(r.publish("news", "hi"), 1);
    assert_eq!(r.dequeue("news", false).unwrap(), Some("hi".to_string()));
    assert_eq!(r.dequeue("news", false).unwrap(), None);
    assert!(r.dequeue("news", true).is_err());
    r.shutdown();
    assert_eq!(r.dequeue("news", false).unwrap(), None);
    assert!(r.subscribe("news").is_err());
}

// ------------------------------------------------- persistable + cache

#[derive(Debug, PartialEq)]
struct Counter(u64);

impl persistence::Persistable for Counter {
    fn persist_key(&self) -> String {
        "counter".to_string()
    }

    fn encode_value(&self) -> Vec<u8> {
        self.0.to_le_bytes().to_vec()
    }

    fn decode_value(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(|b| Self(u64::from_le_bytes(b)))
    }

    fn ttl_millis(&self) -> Option<u64> {
        Some(1_000)
    }
}

#[test]
fn persistable_save_load_roundtrip_with_ttl() {
    use persistence::Persistable as _;
    let mut store = KeyValueStore::new();
    Counter(7).save(&mut store);
    assert_eq!(Counter::load(&store, "counter"), Some(Counter(7)));
    store.advance(1_001);
    assert_eq!(Counter::load(&store, "counter"), None);
}

#[test]
fn persistable_missing_key_is_none() {
    use persistence::Persistable as _;
    let store = KeyValueStore::new();
    assert_eq!(Counter::load(&store, "counter"), None);
}

#[test]
fn cache_computes_once_then_serves_hits() {
    use persistence::{Persistable as _, PersistedCache};
    let mut store = KeyValueStore::new();
    let cache = PersistedCache::new("ns");
    let mut calls = 0;
    let first = cache.get_or_compute(&mut store, "k", None, || {
        calls += 1;
        Counter(calls)
    });
    let second = cache.get_or_compute(&mut store, "k", None, || {
        calls += 1;
        Counter(calls)
    });
    assert_eq!((first, second, calls), (Counter(1), Counter(1), 1));
}

#[test]
fn cache_expiry_recomputes_and_invalidate_drops() {
    use persistence::{Persistable as _, PersistedCache};
    let mut store = KeyValueStore::new();
    let cache = PersistedCache::new("ns");
    let mut calls = 0u64;
    cache.get_or_compute(&mut store, "k", Some(100), || {
        calls += 1;
        Counter(calls)
    });
    store.advance(101);
    cache.get_or_compute(&mut store, "k", Some(100), || {
        calls += 1;
        Counter(calls)
    });
    assert_eq!(calls, 2);
    assert!(cache.invalidate(&mut store, "k"));
    assert!(!cache.invalidate(&mut store, "k"));
}

// ------------------------------------------------------------------ values --

#[test]
fn kv_set_get_size_counts_live_entries() {
    let mut db = KeyValueStore::new();
    assert_eq!(db.size(), 0);
    db.set("a", "1");
    db.set("b", "2");
    assert_eq!(db.size(), 2);
    db.set("a", "3");
    assert_eq!(db.size(), 2, "overwrite keeps one entry");
    assert_eq!(db.get("a"), Some("3".to_string()));
    assert!(db.remove("a"));
    assert_eq!(db.size(), 1);
}

#[test]
fn kv_binary_values_roundtrip_raw() {
    let mut db = KeyValueStore::new();
    let raw = vec![0x00, 0xff, 0x89, 0x50, 0x4e, 0x47];
    db.set_bytes("blob", &raw);
    assert_eq!(db.get_bytes("blob"), Some(raw));
    assert_eq!(
        db.get("blob"), None,
        "non-utf8 bytes have no string decoding"
    );
    db.set_bytes("blob", b"plain");
    assert_eq!(db.get("blob"), Some("plain".to_string()));
}

#[test]
fn kv_utf8_string_values_roundtrip() {
    let mut db = KeyValueStore::new();
    db.set("uni", "h\u{e9}llo \u{4e16}\u{754c} \u{1f980}");
    assert_eq!(db.get("uni"), Some("h\u{e9}llo \u{4e16}\u{754c} \u{1f980}".to_string()));
    db.set("empty", "");
    assert_eq!(db.get("empty"), Some(String::new()));
    assert!(db.has("empty"), "empty string is a present value");
    db.set("b64shaped", "aGVsbG8=");
    assert_eq!(
        db.get("b64shaped"),
        Some("aGVsbG8=".to_string()),
        "base64-shaped strings stay literal, never decoded"
    );
}

#[test]
fn kv_json_shaped_strings_survive_byte_exact() {
    let mut db = KeyValueStore::new();
    let doc = r#"{"a":[1,true,null],"b":{"c":"x"}}"#;
    db.set("doc", doc);
    assert_eq!(db.get("doc").as_deref(), Some(doc));
    assert_eq!(db.get_bytes("doc"), Some(doc.as_bytes().to_vec()));
}

#[test]
fn queue_offer_and_take_round_trip() {
    let mut q = PersistedQueue::new();
    assert!(q.offer("a", b"1".to_vec()));
    assert_eq!(q.pending_len(), 1);
    let item = q.take().expect("pending element");
    assert_eq!(item.id, "a");
    assert_eq!(item.payload, b"1".to_vec());
    assert_eq!(item.attempts, 1, "a take counts as the first attempt");
    assert_eq!(q.pending_len(), 0);
    assert_eq!(q.in_flight_len(), 1);
    assert_eq!(q.take(), None, "nothing left to take");
}

#[test]
fn queue_offer_is_idempotent_per_id() {
    let mut q = PersistedQueue::new();
    assert!(q.offer("dup", b"1".to_vec()));
    assert!(!q.offer("dup", b"2".to_vec()), "second offer rejected");
    assert_eq!(q.pending_len(), 1);
    let item = q.take().expect("one element");
    assert_eq!(item.payload, b"1".to_vec(), "first payload wins");
}

#[test]
fn queue_dedupes_independently_per_queue() {
    let mut first = PersistedQueue::new();
    let mut second = PersistedQueue::new();
    assert!(first.offer("same", b"a".to_vec()));
    assert!(
        second.offer("same", b"b".to_vec()),
        "a second queue is unaffected by the first"
    );
    assert_eq!(first.take().expect("first").payload, b"a".to_vec());
    assert_eq!(second.take().expect("second").payload, b"b".to_vec());
}

#[test]
fn queue_does_not_redeliver_in_flight_elements() {
    let mut q = PersistedQueue::new();
    q.offer("a", b"1".to_vec());
    let taken = q.take().expect("taken");
    assert_eq!(q.pending_len(), 0, "in-flight is not pending again");
    assert_eq!(q.take(), None);
    assert_eq!(taken.attempts, 1);
}

#[test]
fn queue_fail_redelivers_until_attempts_exhausted() {
    let mut q = PersistedQueue::new();
    q.offer("a", b"1".to_vec());
    let first = q.take().expect("first attempt");
    let again = q.fail(&first.id, 3).expect("attempts remain: redelivered");
    assert_eq!(again.attempts, first.attempts, "attempt count is carried");
    assert_eq!(q.pending_len(), 1);
    let second = q.take().expect("redelivered is taken again");
    assert_eq!(second.attempts, 2);
    q.fail(&second.id, 3).expect("still attempts left");
    let third = q.take().expect("third attempt");
    assert_eq!(third.attempts, 3);
    assert_eq!(
        q.fail(&third.id, 3),
        None,
        "exhausted: dead-lettered, never redelivered"
    );
    assert_eq!(q.pending_len(), 0);
    assert_eq!(q.in_flight_len(), 0);
}

#[test]
fn queue_retry_delay_follows_schedule() {
    let mut q = PersistedQueue::new();
    q.offer("a", b"1".to_vec());
    let item = q.take().expect("taken");
    q.fail(&item.id, 5).expect("redelivered");
    // Later attempts back off further from the same base.
    let base = 100;
    let first = q.retry_delay(&item.id, base);
    let second = q.retry_delay(&item.id, base);
    assert!(second >= first, "delay never shrinks: {first} then {second}");
    assert!(first >= base, "first delay starts at the base: {first}");
}

#[test]
fn queue_complete_records_once_and_prunes() {
    let mut q = PersistedQueue::new();
    q.offer("a", b"1".to_vec());
    let item = q.take().expect("taken");
    q.complete(&item.id, 1_000);
    assert_eq!(q.completed_len(), 1);
    assert_eq!(q.prune(500, 2_000), 1, "older than the window: pruned");
    assert_eq!(q.completed_len(), 0);
    assert_eq!(
        q.prune(10_000, 3_000),
        0,
        "inside the window: kept"
    );
}

#[test]
fn queue_interrupt_releases_the_element_to_the_next_taker() {
    // PersistedQueueTest.ts:57 "interrupt": a taker holds the element and is
    // interrupted mid-processing, so it never acks. The element must return
    // for a waiting taker rather than being lost, and the waiting taker must
    // not be handed it while the first taker still holds it.
    let mut q = PersistedQueue::new();
    q.offer("a", b"42".to_vec());

    let held = q.take().expect("the first taker holds the element");
    assert_eq!(held.attempts, 1);
    assert_eq!(
        q.take(),
        None,
        "nothing else is pending while the element is in flight"
    );

    let released = q.abandon(&held.id).expect("the interrupted hold is released");
    assert_eq!(released.id, "a");
    assert_eq!(released.payload, b"42".to_vec());

    let next = q.take().expect("the waiting taker receives it");
    assert_eq!(next.id, "a");
    assert_eq!(next.payload, b"42".to_vec());
    assert_eq!(next.attempts, 2, "the interruption counts as an attempt");
    assert_eq!(q.pending_len(), 0);
    assert_eq!(q.in_flight_len(), 1, "it is in flight again, not duplicated");
}
