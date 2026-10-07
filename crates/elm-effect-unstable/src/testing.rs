//! Deterministic testing substitutes: `TestClock`, `TestConsole`, seeded `TestRng`.
//!
//! Ports `packages/effect/src/testing/` (`TestClock.ts`, `TestConsole.ts`,
//! `TestSchema.ts`). Time enters only through method calls; no threads, no
//! wall clock. Loaded in tests via `#[path]` include (the .15 precedent).

/// A deterministic millisecond clock with a queue of pending sleeps.
///
/// Mirrors `TestClock`: `adjust` advances time and wakes every sleep whose
/// deadline has passed, in deadline order.
#[derive(Debug, Default)]
pub struct TestClock {
    now_millis: u64,
    sleeps: Vec<Sleep>,
}

/// One pending sleep: wakes when the clock reaches `wake_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sleep {
    /// Caller-chosen token identifying the sleeper.
    pub id: u64,
    wake_at: u64,
}

impl TestClock {
    /// Fresh clock at time zero with no pending sleeps.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current time in milliseconds.
    pub fn now_millis(&self) -> u64 {
        self.now_millis
    }

    /// Number of sleeps still pending.
    pub fn pending(&self) -> usize {
        self.sleeps.len()
    }

    /// Jump to an absolute time; wakes due sleeps like [`Self::adjust`].
    /// Times before now are ignored (clocks never run backwards).
    pub fn set_time(&mut self, millis: u64) -> Vec<u64> {
        let delta = millis.saturating_sub(self.now_millis);
        self.adjust(delta)
    }

    /// Advance by `delta` millis, waking due sleeps in deadline order.
    /// Returns the woken ids, earliest deadline first.
    pub fn adjust(&mut self, delta: u64) -> Vec<u64> {
        self.now_millis = self.now_millis.saturating_add(delta);
        self.sleeps.sort_by_key(|s| s.wake_at);
        let split = self
            .sleeps
            .iter()
            .position(|s| s.wake_at > self.now_millis)
            .unwrap_or(self.sleeps.len());
        self.sleeps.drain(..split).map(|s| s.id).collect()
    }

    /// Register a sleep of `delay` millis from now, identified by `id`.
    /// Zero-delay sleeps wake on the next [`Self::adjust`] (even of zero).
    pub fn sleep(&mut self, id: u64, delay: u64) {
        self.sleeps.push(Sleep {
            id,
            wake_at: self.now_millis.saturating_add(delay),
        });
    }
}

/// Captured console output: stdout, stderr, and debug lines in order.
///
/// Mirrors `TestConsole`: writes append, `*_lines` read back, `clear` resets.
#[derive(Debug, Default)]
pub struct TestConsole {
    out: Vec<String>,
    err: Vec<String>,
    dbg: Vec<String>,
}

impl TestConsole {
    /// Empty console.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a stdout line.
    pub fn log(&mut self, line: impl Into<String>) {
        self.out.push(line.into());
    }

    /// Append a stderr line.
    pub fn error(&mut self, line: impl Into<String>) {
        self.err.push(line.into());
    }

    /// Append a debug line.
    pub fn debug(&mut self, line: impl Into<String>) {
        self.dbg.push(line.into());
    }

    /// Captured stdout lines, oldest first.
    pub fn out_lines(&self) -> &[String] {
        &self.out
    }

    /// Captured stderr lines, oldest first.
    pub fn err_lines(&self) -> &[String] {
        &self.err
    }

    /// Captured debug lines, oldest first.
    pub fn debug_lines(&self) -> &[String] {
        &self.dbg
    }

    /// Total lines captured across all three streams.
    pub fn len(&self) -> usize {
        self.out.len() + self.err.len() + self.dbg.len()
    }

    /// True when nothing has been captured.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop everything captured so far.
    pub fn clear(&mut self) {
        self.out.clear();
        self.err.clear();
        self.dbg.clear();
    }
}

/// Seeded deterministic RNG for schema arbitraries.
///
/// Mirrors the `TestSchema` random side: a xorshift64 generator so generated
/// values are reproducible from the seed. No external crates.
#[derive(Debug, Clone)]
pub struct TestRng {
    state: u64,
}

impl TestRng {
    /// New generator. A zero seed is mapped to a fixed nonzero default so
    /// the stream never degenerates to all zeros.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// Next raw `u64` (xorshift64*).
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform value in `0..bound`. `bound == 0` yields `0`.
    pub fn next_bounded(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    /// Random bool.
    pub fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// Random `i64` in `lo..=hi` (swapped when `lo > hi`).
    pub fn next_int_range(&mut self, lo: i64, hi: i64) -> i64 {
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        let span = hi.wrapping_sub(lo) as u64;
        lo.wrapping_add(self.next_bounded(span.wrapping_add(1)) as i64)
    }

    /// Random lowercase alphanumeric string of exactly `len` chars.
    pub fn next_string(&mut self, len: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        (0..len)
            .map(|_| ALPHABET[self.next_bounded(ALPHABET.len() as u64) as usize] as char)
            .collect()
    }
}

/// Assert `decode(encode(v)) == v`, returning the decoded value.
/// Panics naming the stage that broke the round trip.
pub fn assert_roundtrip<T, E, D>(
    value: T,
    encode: impl FnOnce(&T) -> E,
    decode: impl FnOnce(E) -> Result<T, D>,
) -> T
where
    T: PartialEq + core::fmt::Debug,
    D: core::fmt::Debug,
{
    let decoded = decode(encode(&value)).expect("decode(encode(v)) failed");
    assert_eq!(decoded, value, "round trip changed the value");
    decoded
}

/// Unwrap an expected-success test outcome, panicking with the error.
pub fn assert_succeeds<T, E: core::fmt::Debug>(result: Result<T, E>) -> T {
    result.expect("expected success")
}

/// Assert an outcome failed; returns the error for further checks.
pub fn assert_fails<T: core::fmt::Debug, E>(result: Result<T, E>) -> E {
    match result {
        Ok(v) => panic!("expected failure, got success: {v:?}"),
        Err(e) => e,
    }
}
