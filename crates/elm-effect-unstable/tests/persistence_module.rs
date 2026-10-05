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
    assert_eq!(db.modify("a", |v| Some(v.unwrap() + "2")), Some("12".to_string()));
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
    assert!(matches!(rl.consume_tokens("k", 1.0), Verdict::Reject { .. }));
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
