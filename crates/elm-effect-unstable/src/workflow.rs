//! Workflow module (`workflow/Activity`, `DurableClock`, `DurableDeferred`,
//! `DurableQueue` at the pin, plus compensation).
//!
//! - [`Activity`]: idempotency-keyed units. [`execute`](Activity::execute)
//!   runs the implementation once per key and replays the stored result on
//!   retry ("activity results are stored and replayed").
//! - [`DurableClock`]: adjustable now plus interruptible waits.
//! - [`DurableDeferred`]: one-shot cell with blocking waiters.
//! - [`DurableQueue`]: FIFO with blocking takes and shutdown.
//! - [`compensate`](compensate): compensation runs on failure and on cancel
//!   ONLY, in reverse registration order; success runs none.
//!
//! Residuals: persistence across restarts (stores are in-memory here),
//! distributed worker dispatch, and clock-drift reconciliation.

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Activity implementation: unit of work returning a result.
type RunFn<T, E> = Box<dyn Fn() -> Result<T, E> + Send + Sync>;

/// Idempotency-keyed unit of work.
pub struct Activity<T, E> {
    /// Stable key: retries with the same key replay the stored result.
    pub key: String,
    run: RunFn<T, E>,
}

impl<T, E> Activity<T, E> {
    /// Build from a key and its implementation (`make`).
    pub fn make(key: &str, run: impl Fn() -> Result<T, E> + Send + Sync + 'static) -> Self {
        Self {
            key: key.to_owned(),
            run: Box::new(run),
        }
    }
}

/// Activity result store: first execution wins, replays are free.
#[derive(Default)]
pub struct ActivityStore<T, E> {
    results: Mutex<HashMap<String, Result<T, E>>>,
    runs: Mutex<HashMap<String, usize>>,
}

impl<T: Clone, E: Clone> ActivityStore<T, E> {
    /// Empty store.
    pub fn new() -> Self {
        Self {
            results: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
        }
    }

    /// Execute once per key; replay the stored result afterwards.
    pub fn execute(&self, activity: &Activity<T, E>) -> Result<T, E> {
        {
            let results = self.results.lock().expect("store");
            if let Some(cached) = results.get(&activity.key) {
                return cached.clone();
            }
        }
        let outcome = (activity.run)();
        *self
            .runs
            .lock()
            .expect("store")
            .entry(activity.key.clone())
            .or_insert(0) += 1;
        self.results
            .lock()
            .expect("store")
            .insert(activity.key.clone(), outcome.clone());
        outcome
    }

    /// Implementation runs so far for `key` (test aid: proves replay).
    pub fn runs(&self, key: &str) -> usize {
        self.runs
            .lock()
            .expect("store")
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// Retry up to `attempts` times, replaying through the store.
    pub fn retry(&self, activity: &Activity<T, E>, attempts: usize) -> Result<T, E> {
        let mut attempts = attempts.max(1);
        loop {
            match self.execute(activity) {
                ok @ Ok(_) => return ok,
                Err(e) => {
                    attempts -= 1;
                    if attempts == 0 {
                        return Err(e);
                    }
                    self.results.lock().expect("store").remove(&activity.key);
                }
            }
        }
    }
}

/// Adjustable clock with interruptible waits.
#[derive(Debug)]
pub struct DurableClock {
    now_ms: Mutex<u64>,
}

impl DurableClock {
    /// Clock starting at `now_ms`.
    pub fn new(now_ms: u64) -> Self {
        Self {
            now_ms: Mutex::new(now_ms),
        }
    }

    /// Current time in ms.
    pub fn now_ms(&self) -> u64 {
        *self.now_ms.lock().expect("clock")
    }

    /// Advance virtual time.
    pub fn advance(&self, delta_ms: u64) {
        *self.now_ms.lock().expect("clock") += delta_ms;
    }

    /// Block until `target_ms` or `interrupted()`. Returns `true` on time.
    pub fn wait_until(&self, target_ms: u64, interrupted: &dyn Fn() -> bool) -> bool {
        loop {
            if self.now_ms() >= target_ms {
                return true;
            }
            if interrupted() {
                return false;
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
}

/// One-shot cell with blocking waiters.
#[derive(Debug, Default)]
pub struct DurableDeferred<T> {
    state: Mutex<Option<T>>,
    changed: Condvar,
}

impl<T> DurableDeferred<T> {
    /// Empty cell.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    /// Complete once; later completions are ignored, returning `false`.
    pub fn complete(&self, value: T) -> bool {
        let mut guard = self.state.lock().expect("deferred");
        if guard.is_some() {
            return false;
        }
        *guard = Some(value);
        self.changed.notify_all();
        true
    }

    /// Block until completed or interrupted. Clones the value out.
    pub fn await_value(&self, interrupted: &dyn Fn() -> bool) -> Option<T>
    where
        T: Clone,
    {
        let mut guard = self.state.lock().expect("deferred");
        loop {
            if let Some(value) = guard.as_ref() {
                return Some(value.clone());
            }
            if interrupted() {
                return None;
            }
            let (g, _) = self
                .changed
                .wait_timeout(guard, Duration::from_millis(1))
                .expect("deferred");
            guard = g;
        }
    }

    /// `true` once completed.
    pub fn is_done(&self) -> bool {
        self.state.lock().expect("deferred").is_some()
    }
}

/// FIFO with blocking takes and shutdown.
#[derive(Debug)]
pub struct DurableQueue<T> {
    items: Mutex<VecDeque<T>>,
    changed: Condvar,
    shutdown: Mutex<bool>,
}

impl<T> DurableQueue<T> {
    /// Empty queue. (`Default` is not derived on purpose: `T` carries no
    /// `Default` bound anywhere in this module.)
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
            changed: Condvar::new(),
            shutdown: Mutex::new(false),
        }
    }

    /// Append one item.
    pub fn offer(&self, item: T) {
        self.items.lock().expect("queue").push_back(item);
        self.changed.notify_one();
    }

    /// Take one item, blocking until present, shutdown, or interrupted.
    /// `None` on shutdown/interrupt (a shutdown wakes every waiter).
    pub fn take(&self, interrupted: &dyn Fn() -> bool) -> Option<T> {
        let mut guard = self.items.lock().expect("queue");
        loop {
            if let Some(item) = guard.pop_front() {
                return Some(item);
            }
            if *self.shutdown.lock().expect("queue") || interrupted() {
                return None;
            }
            let (g, _) = self
                .changed
                .wait_timeout(guard, Duration::from_millis(1))
                .expect("queue");
            guard = g;
        }
    }

    /// Queued item count.
    pub fn len(&self) -> usize {
        self.items.lock().expect("queue").len()
    }

    /// `true` with no items.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Shut down: waiters wake with `None`, takes stay `None`.
    pub fn shutdown(&self) {
        *self.shutdown.lock().expect("queue") = true;
        self.changed.notify_all();
    }

    /// `true` after [`shutdown`](DurableQueue::shutdown).
    pub fn is_shutdown(&self) -> bool {
        *self.shutdown.lock().expect("queue")
    }
}

/// Compensation scope: register undos, run the body. Compensations fire on
/// failure and on cancel ONLY, in reverse order; success runs none.
pub struct CompScope {
    compensations: Vec<Box<dyn FnOnce() + Send>>,
}

impl CompScope {
    fn new() -> Self {
        Self {
            compensations: Vec::new(),
        }
    }

    /// Register one compensation (runs LIFO on failure/cancel).
    pub fn compensate(&mut self, undo: impl FnOnce() + Send + 'static) {
        self.compensations.push(Box::new(undo));
    }
}

/// Run `body` with a compensation scope. Success with a live probe runs
/// no compensation; failure runs them LIFO and keeps the error; a true
/// `interrupted()` probe runs them LIFO and maps through `on_cancel`.
/// Compensation fires on failure and on cancel ONLY.
pub fn compensate<T, E>(
    interrupted: &dyn Fn() -> bool,
    on_cancel: impl FnOnce() -> E,
    body: impl FnOnce(&mut CompScope) -> Result<T, E>,
) -> Result<T, E> {
    let mut scope = CompScope::new();
    match body(&mut scope) {
        Ok(value) => {
            if interrupted() {
                while let Some(undo) = scope.compensations.pop() {
                    undo();
                }
                Err(on_cancel())
            } else {
                Ok(value)
            }
        }
        Err(e) => {
            while let Some(undo) = scope.compensations.pop() {
                undo();
            }
            Err(e)
        }
    }
}

const SHA_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Raw SHA-256 over bytes (stdlib only: no hash crate on the ADR-0002 list).
/// Round constants verified programmatically (cube-root fractional bits).
fn sha256(input: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(input.len() + 72);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for block in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// First 16 bytes of SHA-256 as lowercase hex: the workflow
/// `makeHashDigest` in `packages/effect/src/workflow/internal/crypto.ts`.
pub fn hash_digest(input: &str) -> String {
    let digest = sha256(input.as_bytes());
    let mut hex = String::with_capacity(32);
    for byte in &digest[..16] {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Deterministic execution id: `len:tag:key` hashed, so tags and keys
/// cannot collide across the boundary (`Order`+`Pay-42` vs `Order-Pay`+`42`).
/// Ports `makeExecutionIdFromPayload` in `Workflow.ts`.
pub fn execution_id(tag: &str, idempotency_key: &str) -> String {
    hash_digest(&format!("{}:{tag}:{idempotency_key}", tag.len()))
}
