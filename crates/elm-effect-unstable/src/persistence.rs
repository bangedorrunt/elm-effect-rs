//! Persistence backends (ADR-0002 Phase 12).
//!
//! [`KeyValueStore`] is an in-memory map with optional per-entry TTL,
//! key prefixes, and bulk writes. [`RateLimiter`] runs fixed-window and
//! token-bucket admission plus adaptive 429-feedback learning over an
//! injected millisecond clock, so tests never sleep. [`PersistedQueue`]
//! derives growing retry delays from the persisted attempt count and
//! prunes completed elements on a cadence. [`FakeRedis`] is an in-memory
//! stand-in (scripts, pub/sub, dequeue) because tests open no sockets.
//! Everything here is synchronous and dependency-free.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

// ------------------------------------------------------------------- store ---

/// Stored entry with optional deadline. Values are bytes: strings are
/// UTF-8 views, so binary values round-trip untouched.
#[derive(Clone, Debug)]
struct Entry {
    value: Vec<u8>,
    expires_at: Option<u64>,
}

/// In-memory key/value store.
#[derive(Clone, Debug, Default)]
pub struct KeyValueStore {
    map: HashMap<String, Entry>,
    now_ms: u64,
}

impl KeyValueStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the virtual clock (expiry + TTL tests).
    pub fn advance(&mut self, millis: u64) {
        self.now_ms += millis;
    }

    fn live(&self, key: &str) -> Option<&Entry> {
        self.map.get(key).filter(|e| match e.expires_at {
            Some(deadline) => self.now_ms < deadline,
            None => true,
        })
    }

    /// Insert without TTL.
    pub fn set(&mut self, key: &str, value: &str) {
        self.map.insert(key.to_string(), Entry { value: value.as_bytes().to_vec(), expires_at: None });
    }

    /// Insert many without TTL.
    pub fn set_many(&mut self, pairs: &[(&str, &str)]) {
        for (k, v) in pairs {
            self.set(k, v);
        }
    }

    /// Insert with a millisecond TTL (fractional TTLs round up).
    pub fn set_ttl(&mut self, key: &str, value: &str, ttl_millis: f64) {
        let ttl = ttl_millis.ceil().max(0.0) as u64;
        self.map.insert(
            key.to_string(),
            Entry { value: value.as_bytes().to_vec(), expires_at: Some(self.now_ms + ttl) },
        );
    }

    /// Read as UTF-8 text; missing, expired, or non-UTF-8 is [`None`].
    pub fn get(&self, key: &str) -> Option<String> {
        self.live(key).and_then(|e| String::from_utf8(e.value.clone()).ok())
    }

    /// Live entry count.
    pub fn size(&self) -> usize {
        let now = self.now_ms;
        self.map
            .values()
            .filter(|e| match e.expires_at {
                Some(deadline) => now < deadline,
                None => true,
            })
            .count()
    }

    /// Raw bytes (binary values round-trip untouched).
    pub fn get_bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.live(key).map(|e| e.value.clone())
    }

    /// Store raw bytes.
    pub fn set_bytes(&mut self, key: &str, bytes: &[u8]) {
        self.map.insert(key.to_string(), Entry { value: bytes.to_vec(), expires_at: None });
    }

    /// Presence (live entries only).
    pub fn has(&self, key: &str) -> bool {
        self.live(key).is_some()
    }

    /// Delete; true when something was there.
    pub fn remove(&mut self, key: &str) -> bool {
        self.map.remove(key).is_some()
    }

    /// Clear everything.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Update in place; [`None`] input deletes nothing and yields [`None`].
    pub fn modify(&mut self, key: &str, f: impl FnOnce(Option<String>) -> Option<String>) -> Option<String> {
        let next = f(self.get(key));
        match next {
            Some(v) => {
                self.set(key, &v);
                Some(v)
            }
            None => None,
        }
    }

    /// Modify raw bytes (binary-safe modify path).
    pub fn modify_bytes(
        &mut self,
        key: &str,
        f: impl FnOnce(Option<Vec<u8>>) -> Option<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let next = f(self.get_bytes(key));
        match next {
            Some(v) => {
                self.set_bytes(key, &v);
                Some(v)
            }
            None => None,
        }
    }

    /// Prefixed view: every key reads/writes under `prefix/`.
    pub fn prefixed(&mut self, prefix: &str) -> Prefixed<'_> {
        Prefixed { inner: self, prefix: prefix.to_string() }
    }
}

/// Prefixed view of a [`KeyValueStore`].
pub struct Prefixed<'a> {
    inner: &'a mut KeyValueStore,
    prefix: String,
}

impl Prefixed<'_> {
    fn full(&self, key: &str) -> String {
        format!("{}/{}", self.prefix, key)
    }

    /// Insert under the prefix.
    pub fn set(&mut self, key: &str, value: &str) {
        let full = self.full(key);
        self.inner.set(&full, value);
    }

    /// Read under the prefix.
    pub fn get(&self, key: &str) -> Option<String> {
        self.inner.get(&self.full(key))
    }

    /// Presence under the prefix.
    pub fn has(&self, key: &str) -> bool {
        self.inner.has(&self.full(key))
    }

    /// Update under the prefix.
    pub fn modify(&mut self, key: &str, f: impl FnOnce(Option<String>) -> Option<String>) -> Option<String> {
        let full = self.full(key);
        self.inner.modify(&full, f)
    }
}

// ----------------------------------------------------------------- limiter ---

/// Fixed-window state per key.
#[derive(Clone, Debug, Default)]
struct Window {
    start_ms: u64,
    count: u64,
}

/// Token-bucket state per key.
#[derive(Clone, Debug)]
struct Bucket {
    tokens: f64,
    updated_ms: u64,
}

/// Adaptive 429-feedback state per key.
#[derive(Clone, Debug, Default)]
struct Adaptive {
    cooldown_until_ms: u64,
    learned_window_ms: Option<u64>,
    accepted: u64,
    epoch: u64,
    learning: bool,
}

/// Admission outcome with reset metadata.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Admitted; carries full-window reset info.
    Admit {
        /// Milliseconds until the window resets.
        reset_in_ms: u64,
    },
    /// Rejected; retry after this delay.
    Reject {
        /// Milliseconds to wait.
        retry_after_ms: u64,
    },
}

/// Rate limiter: fixed windows, token buckets, and adaptive 429 learning.
/// All time comes from the injected clock (`now_ms`), never the wall.
pub struct RateLimiter {
    window_ms: u64,
    limit: u64,
    refill_per_ms: f64,
    capacity: f64,
    windows: Mutex<HashMap<String, Window>>,
    buckets: Mutex<HashMap<String, Bucket>>,
    adaptive: Mutex<HashMap<String, Adaptive>>,
    now_ms: Mutex<u64>,
    min_cooldown_ms: u64,
    max_cooldown_ms: u64,
}

impl RateLimiter {
    /// Fixed window of `limit` admissions per `window_ms`.
    pub fn fixed_window(window_ms: u64, limit: u64) -> Self {
        Self {
            window_ms: window_ms.max(1),
            limit,
            refill_per_ms: 0.0,
            capacity: 0.0,
            windows: Mutex::new(HashMap::new()),
            buckets: Mutex::new(HashMap::new()),
            adaptive: Mutex::new(HashMap::new()),
            now_ms: Mutex::new(0),
            min_cooldown_ms: 1_000,
            max_cooldown_ms: 60_000,
        }
    }

    /// Token bucket: `capacity` tokens, refilling `per_ms` each millisecond.
    pub fn token_bucket(capacity: f64, per_ms: f64) -> Self {
        Self {
            window_ms: 0,
            limit: 0,
            refill_per_ms: per_ms,
            capacity,
            windows: Mutex::new(HashMap::new()),
            buckets: Mutex::new(HashMap::new()),
            adaptive: Mutex::new(HashMap::new()),
            now_ms: Mutex::new(0),
            min_cooldown_ms: 1_000,
            max_cooldown_ms: 60_000,
        }
    }

    /// Move the injected clock.
    pub fn advance(&self, millis: u64) {
        *self.now_ms.lock().unwrap() += millis;
    }

    fn now(&self) -> u64 {
        *self.now_ms.lock().unwrap()
    }

    /// Fixed-window admission for `key`.
    pub fn consume(&self, key: &str) -> Verdict {
        let now = self.now();
        let mut windows = self.windows.lock().unwrap();
        let window = windows.entry(key.to_string()).or_insert(Window { start_ms: now, count: 0 });
        if now - window.start_ms >= self.window_ms {
            window.start_ms = now;
            window.count = 0;
        }
        if window.count < self.limit {
            window.count += 1;
            Verdict::Admit { reset_in_ms: self.window_ms - (now - window.start_ms) }
        } else {
            Verdict::Reject { retry_after_ms: self.window_ms - (now - window.start_ms) }
        }
    }

    /// Token-bucket admission: `cost` tokens, accumulating delay reporting.
    pub fn consume_tokens(&self, key: &str, cost: f64) -> Verdict {
        let now = self.now();
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets
            .entry(key.to_string())
            .or_insert(Bucket { tokens: self.capacity, updated_ms: now });
        bucket.tokens = (bucket.tokens + (now - bucket.updated_ms) as f64 * self.refill_per_ms)
            .min(self.capacity);
        bucket.updated_ms = now;
        if bucket.tokens >= cost {
            bucket.tokens -= cost;
            Verdict::Admit { reset_in_ms: 0 }
        } else {
            let deficit = cost - bucket.tokens;
            Verdict::Reject { retry_after_ms: (deficit / self.refill_per_ms).ceil() as u64 }
        }
    }

    /// 429 feedback: enter or extend cooldown. Zero feedback clamps to the
    /// minimum cooldown; excessive values clamp to the maximum.
    pub fn feedback_429(&self, key: &str, retry_after_ms: u64) -> u64 {
        let now = self.now();
        let clamped = retry_after_ms.clamp(self.min_cooldown_ms, self.max_cooldown_ms);
        let mut adaptive = self.adaptive.lock().unwrap();
        let state = adaptive.entry(key.to_string()).or_default();
        state.cooldown_until_ms = now + clamped;
        state.learning = false;
        clamped
    }

    /// Remaining cooldown, if cooling down.
    pub fn cooldown_remaining(&self, key: &str) -> u64 {
        let now = self.now();
        self.adaptive
            .lock()
            .unwrap()
            .get(key)
            .map(|s| s.cooldown_until_ms.saturating_sub(now))
            .unwrap_or(0)
    }

    /// Admit-or-learn step: cooling requests report the delay; the first
    /// admitted post-cooldown request starts learning at zero delay.
    pub fn admit_or_learn(&self, key: &str) -> Verdict {
        let now = self.now();
        let mut adaptive = self.adaptive.lock().unwrap();
        let state = adaptive.entry(key.to_string()).or_default();
        if now < state.cooldown_until_ms {
            return Verdict::Reject { retry_after_ms: state.cooldown_until_ms - now };
        }
        if state.epoch == 0 || now >= state.cooldown_until_ms {
            state.learning = true;
            state.accepted = 0;
        }
        Verdict::Admit { reset_in_ms: 0 }
    }

    /// Record one accepted request during learning.
    pub fn note_accepted(&self, key: &str) {
        if let Some(state) = self.adaptive.lock().unwrap().get_mut(key) {
            if state.learning {
                state.accepted += 1;
            }
        }
    }

    /// Expire learned state (bounded windows expire with it).
    pub fn expire_learned(&self, key: &str) {
        if let Some(state) = self.adaptive.lock().unwrap().get_mut(key) {
            state.learned_window_ms = None;
            state.learning = false;
        }
    }
}

// ------------------------------------------------------------------- queue ---

/// Persisted attempt queue: growing delays plus interval pruning.
#[derive(Clone, Debug, Default)]
pub struct PersistedQueue {
    attempts: HashMap<String, u64>,
    completed: VecDeque<(String, u64)>,
    pending: VecDeque<QueuedItem>,
    in_flight: HashMap<String, QueuedItem>,
    max_attempts: u64,
    dedupe: HashMap<String, String>,
}

/// One queued element with its attempt count.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedItem {
    /// Caller id (dedupe key scope).
    pub id: String,
    /// Payload bytes.
    pub payload: Vec<u8>,
    /// Attempts so far (starts at 1 on first delivery).
    pub attempts: u64,
}

impl PersistedQueue {
    /// Empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retry delay for `key`: base doubling per persisted attempt.
    pub fn retry_delay(&mut self, key: &str, base_ms: u64) -> u64 {
        let attempt = self.attempts.entry(key.to_string()).or_insert(0);
        *attempt += 1;
        base_ms.saturating_mul(1u64 << (*attempt).min(20).saturating_sub(1))
    }

    /// Mark completed (prunable on the configured interval).
    pub fn complete(&mut self, key: &str, now_ms: u64) {
        self.completed.push_back((key.to_string(), now_ms));
    }

    /// Prune completions older than `older_than_ms`.
    pub fn prune(&mut self, older_than_ms: u64, now_ms: u64) -> usize {
        let before = self.completed.len();
        self.completed.retain(|(_, at)| now_ms.saturating_sub(*at) < older_than_ms);
        before - self.completed.len()
    }

    /// Completed count (test introspection).
    pub fn completed_len(&self) -> usize {
        self.completed.len()
    }

    /// Offer an element; same id twice dedupes within this queue.
    pub fn offer(&mut self, id: &str, payload: Vec<u8>) -> bool {
        if self.dedupe.contains_key(id) {
            return false;
        }
        self.dedupe.insert(id.to_string(), id.to_string());
        self.pending.push_back(QueuedItem { id: id.to_string(), payload, attempts: 0 });
        true
    }

    /// Take the next element, marking it in-flight with attempt 1+.
    /// Elements that fail to decode are dead-lettered, never redelivered.
    pub fn take(&mut self) -> Option<QueuedItem> {
        let mut item = self.pending.pop_front()?;
        item.attempts = self.attempts.get(&item.id).copied().unwrap_or(0) + 1;
        self.attempts.insert(item.id.clone(), item.attempts);
        self.in_flight.insert(item.id.clone(), item.clone());
        Some(item)
    }

    /// Acknowledge: completed, prunable later.
    pub fn ack(&mut self, id: &str, now_ms: u64) -> bool {
        if self.in_flight.remove(id).is_some() {
            self.completed.push_back((id.to_string(), now_ms));
            true
        } else {
            false
        }
    }

    /// Fail: redeliver while attempts remain, else dead-letter.
    /// Returns the redelivered item, or [`None`] when dead.
    pub fn fail(&mut self, id: &str, max_attempts: u64) -> Option<QueuedItem> {
        let item = self.in_flight.remove(id)?;
        if item.attempts >= max_attempts {
            return None;
        }
        self.pending.push_back(item.clone());
        Some(item)
    }

    /// Pending depth (test introspection).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// In-flight count (test introspection).
    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }
}

// ------------------------------------------------------------------- redis ---

/// In-memory Redis stand-in: scripts, pub/sub, dequeue. No sockets.
#[derive(Clone, Debug, Default)]
pub struct FakeRedis {
    scripts: HashMap<String, String>,
    channels: HashMap<String, Vec<String>>,
    listeners: HashMap<String, usize>,
    down: bool,
}

impl FakeRedis {
    /// Empty fake.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a script; `__proto__` stays a plain own key.
    pub fn script_load(&mut self, source: &str) -> String {
        let sha = format!("{:x}", source.len() * 31 + 7);
        self.scripts.insert(sha.clone(), source.to_string());
        sha
    }

    /// Reload all scripts (recovers SCRIPT LOAD failures).
    pub fn script_reload(&mut self) {
        self.scripts.clear();
    }

    /// Run a script by sha; unknown sha reports NOSCRIPT.
    pub fn eval_sha(&self, sha: &str) -> Result<String, String> {
        self.scripts
            .get(sha)
            .cloned()
            .ok_or_else(|| "NOSCRIPT".to_string())
    }

    /// Publish; returns listener count.
    pub fn publish(&mut self, channel: &str, message: &str) -> usize {
        self.channels.entry(channel.to_string()).or_default().push(message.to_string());
        self.listeners.get(channel).copied().unwrap_or(0)
    }

    /// Subscribe a listener; setup failures surface here.
    pub fn subscribe(&mut self, channel: &str) -> Result<(), String> {
        if self.down {
            return Err("setup failed".to_string());
        }
        *self.listeners.entry(channel.to_string()).or_insert(0) += 1;
        Ok(())
    }

    /// Next message for a listener; listener errors surface as [`Err`].
    pub fn dequeue(&mut self, channel: &str, fail_listener: bool) -> Result<Option<String>, String> {
        if fail_listener {
            return Err("listener failed".to_string());
        }
        Ok(self.channels.get_mut(channel).and_then(|q| {
            if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            }
        }))
    }

    /// Shut down: releases the subscriber with no more messages.
    pub fn shutdown(&mut self) {
        self.down = true;
        self.channels.clear();
    }
}
