//! `RateLimiter` port (`persistence/RateLimiter.ts` at the pin): the memory
//! store's two algorithms and the `consume` arithmetic on top of them.
//!
//! The pin splits the work in two: a store that keeps the counter or bucket,
//! and `consume` that turns the store's answer into delay/remaining/reset
//! metadata. This port keeps that split in one type but the arithmetic is the
//! pin's, millisecond for millisecond.
//!
//! Two things the pin gets from Effect are explicit here:
//!
//! - time is a [`LimiterClock`] so tests advance a fake clock instead of
//!   sleeping, and
//! - the adaptive state machine (`adaptiveConsume`/`adaptiveFeedback`) is not
//!   ported: it needs the feedback loop's 429 handling, which is a separate
//!   unit of work.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Time source for the limiter. `sleep` may park; a virtual clock advances
/// only when the test says so.
pub trait LimiterClock: Send + Sync + 'static {
    fn now_millis(&self) -> u64;
    /// Park for `duration`. Virtual clocks wake when the test advances them.
    fn sleep(&self, duration: Duration);
}

/// Wall clock: real time, real sleep.
#[derive(Clone, Copy, Debug)]
pub struct SystemLimiterClock;

impl LimiterClock for SystemLimiterClock {
    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Which counter the store keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// One counter per window; a reservation extends the window by its share.
    FixedWindow,
    /// A bucket of `limit` tokens refilled at `window / limit` per token.
    TokenBucket,
}

/// What happens past the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnExceeded {
    /// Return the delay needed instead of failing.
    Delay,
    /// Fail with [`RateLimitExceeded`].
    Fail,
}

/// One `consume` call's parameters (`RateLimiter.sleep` options).
#[derive(Clone, Debug)]
pub struct ConsumeOptions<'a> {
    pub algorithm: Algorithm,
    pub on_exceeded: OnExceeded,
    pub key: &'a str,
    pub limit: u32,
    pub window: Duration,
    /// Tokens this call consumes. Defaults to one.
    pub tokens: u32,
}

impl<'a> ConsumeOptions<'a> {
    /// The pin's defaults: fixed window, fail, one token.
    pub fn new(key: &'a str, limit: u32, window: Duration) -> Self {
        Self {
            algorithm: Algorithm::FixedWindow,
            on_exceeded: OnExceeded::Fail,
            key,
            limit,
            window,
            tokens: 1,
        }
    }
}

/// What an admitted call learns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsumeResult {
    /// How long to wait before running (zero when admitted now).
    pub delay: Duration,
    pub limit: u32,
    /// Tokens left in the current window; negative past the limit.
    pub remaining: i64,
    /// Lifetime left on the reservation that admitted this call.
    pub reset_after: Duration,
}

/// The limit was hit and `OnExceeded::Fail` asked for an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimitExceeded {
    pub key: String,
    pub retry_after: Duration,
    pub limit: u32,
    pub remaining: i64,
}

#[derive(Debug)]
struct Counter {
    count: i64,
    expires_at: u64,
}

#[derive(Debug)]
struct Bucket {
    /// Signed: `OnExceeded::Delay` lets the bucket go negative.
    tokens: i64,
    last_refill: u64,
}

/// Fixed-window counters and token buckets over one clock.
pub struct RateLimiter {
    clock: Arc<dyn LimiterClock>,
    counters: HashMap<String, Counter>,
    buckets: HashMap<String, Bucket>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    /// A limiter on the wall clock.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemLimiterClock))
    }

    /// A limiter on `clock` (`TestClock` wiring in tests).
    pub fn with_clock(clock: Arc<dyn LimiterClock>) -> Self {
        Self {
            clock,
            counters: HashMap::new(),
            buckets: HashMap::new(),
        }
    }

    fn now(&self) -> u64 {
        self.clock.now_millis()
    }

    /// Admit `tokens` against `options`, or say how long to wait.
    pub fn consume(&mut self, options: &ConsumeOptions<'_>) -> Result<ConsumeResult, RateLimitExceeded> {
        let window_ms = options.window.as_millis() as i64;
        let limit = options.limit as i64;
        let tokens = options.tokens as i64;
        let exceeded = |retry_after_ms: i64, remaining: i64| RateLimitExceeded {
            key: options.key.to_owned(),
            retry_after: Duration::from_millis(retry_after_ms.max(0) as u64),
            limit: options.limit,
            remaining,
        };

        // A call for more than the limit can never be admitted; the pin says so
        // without touching the store, so the counter is left alone.
        if tokens > limit {
            return match options.on_exceeded {
                OnExceeded::Fail => Err(exceeded(window_ms, 0)),
                OnExceeded::Delay => Ok(ConsumeResult {
                    delay: options.window,
                    limit: options.limit,
                    remaining: 0,
                    reset_after: options.window,
                }),
            };
        }

        // Truncated to whole milliseconds: the pin's Duration arithmetic is
        // fractional, so a window that does not divide evenly rounds down here.
        let refill_ms = window_ms / limit.max(1);

        match options.algorithm {
            Algorithm::FixedWindow => {
                let now = self.now();
                let counter = self.counters.entry(options.key.to_owned()).or_insert(Counter {
                    count: 0,
                    expires_at: now,
                });
                if counter.expires_at <= now {
                    counter.count = 0;
                    counter.expires_at = now;
                }
                // Fail mode asks the store to refuse the reservation, but still
                // reports the hypothetical count so the caller sees the excess.
                if options.on_exceeded == OnExceeded::Fail
                    && counter.count + tokens > limit
                {
                    return Err(exceeded(counter.expires_at as i64 - now as i64, 0));
                }
                counter.count += tokens;
                counter.expires_at += (refill_ms * tokens) as u64;
                let ttl = counter.expires_at as i64 - now as i64;
                let count = counter.count;

                match options.on_exceeded {
                    OnExceeded::Fail => {
                        let remaining = limit - count;
                        if remaining < 0 {
                            return Err(exceeded(ttl, 0));
                        }
                        Ok(ConsumeResult {
                            delay: Duration::ZERO,
                            limit: options.limit,
                            remaining,
                            reset_after: Duration::from_millis(ttl.max(0) as u64),
                        })
                    }
                    OnExceeded::Delay => {
                        let elapsed = count * refill_ms - ttl;
                        let window_number = (count - 1).div_euclid(limit);
                        let left = window_number * window_ms - elapsed;
                        Ok(ConsumeResult {
                            delay: Duration::from_millis(left.max(0) as u64),
                            limit: options.limit,
                            remaining: limit - count,
                            reset_after: Duration::from_millis(ttl.max(0) as u64),
                        })
                    }
                }
            }
            Algorithm::TokenBucket => {
                let now = self.now();
                let bucket = self.buckets.entry(options.key.to_owned()).or_insert(Bucket {
                    tokens: limit,
                    last_refill: now,
                });
                let refill = refill_ms.max(1) as u64;
                let added = ((now.saturating_sub(bucket.last_refill)) / refill) as i64;
                if added > 0 {
                    bucket.tokens = (bucket.tokens + added).min(limit);
                    bucket.last_refill += added as u64 * refill;
                }
                if bucket.tokens >= limit {
                    bucket.last_refill = now;
                }
                let remaining = bucket.tokens - tokens;
                if options.on_exceeded == OnExceeded::Delay || remaining >= 0 {
                    bucket.tokens = remaining;
                }
                let elapsed = now.saturating_sub(bucket.last_refill) as i64;
                let delay = ((-remaining).max(0) * refill_ms - elapsed).max(0);
                let reset_after = ((limit - remaining).max(0) * refill_ms - elapsed).max(0);

                if options.on_exceeded == OnExceeded::Fail && remaining < 0 {
                    return Err(exceeded(delay, 0));
                }
                Ok(ConsumeResult {
                    delay: Duration::from_millis(delay as u64),
                    limit: options.limit,
                    remaining,
                    reset_after: Duration::from_millis(reset_after as u64),
                })
            }
        }
    }

    /// Consume, then park for the delay the limit asks for.
    ///
    /// The pin's `RateLimiter.sleep`; `sleeper` is its partially applied shape.
    pub fn sleep(&mut self, options: &ConsumeOptions<'_>) -> Result<ConsumeResult, RateLimitExceeded> {
        let result = self.consume(options)?;
        if !result.delay.is_zero() {
            self.clock.sleep(result.delay);
        }
        Ok(result)
    }

    /// The partially applied `sleep`: bind the limiter, choose the options later.
    pub fn sleeper(&mut self) -> Sleeper<'_> {
        Sleeper { limiter: self }
    }
}

/// `RateLimiter.sleep(limiter)` before the options arrive.
pub struct Sleeper<'a> {
    limiter: &'a mut RateLimiter,
}

impl Sleeper<'_> {
    /// `limiter.consume` followed by the sleep it asks for.
    pub fn call(
        &mut self,
        options: &ConsumeOptions<'_>,
    ) -> Result<ConsumeResult, RateLimitExceeded> {
        self.limiter.sleep(options)
    }
}

/// The uncurried `RateLimiter.sleep(limiter, options)`.
pub fn sleep(
    limiter: &mut RateLimiter,
    options: &ConsumeOptions<'_>,
) -> Result<ConsumeResult, RateLimitExceeded> {
    limiter.sleep(options)
}
