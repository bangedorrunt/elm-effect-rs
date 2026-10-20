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
//! - the sleep shape is a method plus a bound [`Sleeper`] rather than a dual
//!   function.
//!
//! The adaptive half ported here is the store's own state machine
//! (`adaptiveConsume`/`adaptiveFeedback`), not the `consume` engine.

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

/// Adaptive limiter phase (`AdaptivePhase` in the pin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdaptivePhase {
    /// No 429 seen yet: the caller uses its own fallback limits.
    Inactive,
    /// The server asked for a wait; nothing is admitted.
    Cooldown,
    /// The wait is over; token counts are being watched.
    Learning,
    /// The server's window and token count are inferred; the limiter admits
    /// against the learned shape instead of the fallback.
    Learned,
}

/// One `adaptiveConsume` call's parameters.
#[derive(Clone, Debug)]
pub struct AdaptiveConsumeOptions<'a> {
    pub key: &'a str,
    pub tokens: u32,
    /// Player limits used while the phase is inactive or learning.
    pub fallback_limit: u32,
    pub fallback_window: Duration,
}

/// What the adaptive limiter says to do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveConsumeResult {
    pub delay: Duration,
    /// Bumped on every phase change; feedback for an older epoch is stale.
    pub epoch: u64,
    pub phase: AdaptivePhase,
}

/// Feedback from the call the limiter admitted (usually a 429).
#[derive(Clone, Debug)]
pub struct AdaptiveFeedbackOptions<'a> {
    pub key: &'a str,
    pub epoch: u64,
    pub tokens: u32,
    /// HTTP status; anything but 429 is ignored.
    pub status: u16,
    /// The server's `Retry-After`, if it sent one.
    pub retry_after: Option<Duration>,
}

/// State kept per key once a 429 arrives.
#[derive(Debug)]
struct AdaptiveState {
    phase: AdaptivePhase,
    epoch: u64,
    cooldown_until: u64,
    learning_started_at: u64,
    observed_tokens: i64,
    learned_limit: i64,
    learned_window_ms: i64,
    expires_at: u64,
}

/// How long a state outlives its own window; the pin's 60s grace.
const ADAPTIVE_STATE_TTL_GRACE_MS: u64 = 60_000;
/// The largest window the limiter will believe, an hour.
const ADAPTIVE_STATE_MAX_WINDOW_MS: u64 = 60 * 60 * 1_000;

/// A missing, zero or negative `Retry-After` becomes 1ms; an oversized one is
/// cut to the hour ceiling.
fn clamp_adaptive_millis(millis: u64) -> u64 {
    millis.clamp(1, ADAPTIVE_STATE_MAX_WINDOW_MS)
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
    adaptive: HashMap<String, AdaptiveState>,
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
            adaptive: HashMap::new(),
        }
    }

    fn now(&self) -> u64 {
        self.clock.now_millis()
    }

    /// Admit `tokens` against `options`, or say how long to wait.
    pub fn consume(
        &mut self,
        options: &ConsumeOptions<'_>,
    ) -> Result<ConsumeResult, RateLimitExceeded> {
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
                let counter = self
                    .counters
                    .entry(options.key.to_owned())
                    .or_insert(Counter {
                        count: 0,
                        expires_at: now,
                    });
                if counter.expires_at <= now {
                    counter.count = 0;
                    counter.expires_at = now;
                }
                // Fail mode asks the store to refuse the reservation, but still
                // reports the hypothetical count so the caller sees the excess.
                if options.on_exceeded == OnExceeded::Fail && counter.count + tokens > limit {
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
                let bucket = self
                    .buckets
                    .entry(options.key.to_owned())
                    .or_insert(Bucket {
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

    /// Look the state up, dropping it when its window has passed.
    fn adaptive_state(&mut self, key: &str, now: u64) -> Option<&mut AdaptiveState> {
        let live = self
            .adaptive
            .get(key)
            .is_some_and(|state| state.expires_at > now);
        if !live {
            self.adaptive.remove(key);
        }
        self.adaptive.get_mut(key)
    }

    /// What to do now under the adaptive state, or the fallback while inactive.
    pub fn adaptive_consume(
        &mut self,
        options: &AdaptiveConsumeOptions<'_>,
    ) -> AdaptiveConsumeResult {
        let now = self.clock.now_millis();
        let Some(state) = self.adaptive_state(options.key, now) else {
            return AdaptiveConsumeResult {
                delay: Duration::ZERO,
                epoch: 0,
                phase: AdaptivePhase::Inactive,
            };
        };
        let tokens = options.tokens as i64;
        match state.phase {
            AdaptivePhase::Inactive => AdaptiveConsumeResult {
                delay: Duration::ZERO,
                epoch: state.epoch,
                phase: AdaptivePhase::Inactive,
            },
            AdaptivePhase::Cooldown => {
                if state.cooldown_until > now {
                    return AdaptiveConsumeResult {
                        delay: Duration::from_millis(state.cooldown_until - now),
                        epoch: state.epoch,
                        phase: AdaptivePhase::Cooldown,
                    };
                }
                // The wait is over: this first admitted call starts a learning
                // epoch and seeds the observed count with its own tokens.
                state.phase = AdaptivePhase::Learning;
                state.epoch += 1;
                state.learning_started_at = now;
                state.observed_tokens = tokens;
                state.expires_at =
                    now + options.fallback_window.as_millis() as u64 + ADAPTIVE_STATE_TTL_GRACE_MS;
                AdaptiveConsumeResult {
                    delay: Duration::ZERO,
                    epoch: state.epoch,
                    phase: AdaptivePhase::Learning,
                }
            }
            AdaptivePhase::Learning => {
                state.observed_tokens += tokens;
                AdaptiveConsumeResult {
                    delay: Duration::ZERO,
                    epoch: state.epoch,
                    phase: AdaptivePhase::Learning,
                }
            }
            AdaptivePhase::Learned => {
                let refill_ms = if state.learned_limit > 0 {
                    state.learned_window_ms / state.learned_limit
                } else {
                    0
                };
                if state.cooldown_until <= now {
                    state.observed_tokens = 0;
                    state.cooldown_until = now;
                }
                state.observed_tokens += tokens;
                state.cooldown_until += (refill_ms * tokens) as u64;

                let ttl = (state.cooldown_until as i64 - now as i64).max(0);
                let window_number =
                    (state.observed_tokens - 1).div_euclid(state.learned_limit.max(1));
                let elapsed = state.observed_tokens * refill_ms - ttl;
                let remaining = window_number * state.learned_window_ms - elapsed;
                AdaptiveConsumeResult {
                    delay: Duration::from_millis(remaining.max(0) as u64),
                    epoch: state.epoch,
                    phase: AdaptivePhase::Learned,
                }
            }
        }
    }

    /// Fold the server's answer back into the state.
    ///
    /// Ignored unless it is a 429 with a `Retry-After` for the current epoch:
    /// stale feedback would otherwise overwrite a window already learned.
    pub fn adaptive_feedback(&mut self, options: &AdaptiveFeedbackOptions<'_>) {
        let Some(retry_after) = options.retry_after else {
            return;
        };
        if options.status != 429 {
            return;
        }
        let retry_ms = clamp_adaptive_millis(retry_after.as_millis() as u64);
        let now = self.clock.now_millis();
        let cooldown_until = now + retry_ms;

        if self.adaptive_state(options.key, now).is_none() {
            if options.epoch != 0 {
                return;
            }
            self.adaptive.insert(
                options.key.to_owned(),
                AdaptiveState {
                    phase: AdaptivePhase::Cooldown,
                    epoch: 0,
                    cooldown_until,
                    learning_started_at: 0,
                    observed_tokens: 0,
                    learned_limit: 0,
                    learned_window_ms: 0,
                    expires_at: cooldown_until + ADAPTIVE_STATE_TTL_GRACE_MS,
                },
            );
            return;
        }

        let state = self.adaptive.get_mut(options.key).expect("just checked");
        if state.epoch != options.epoch {
            return;
        }
        let reset_to_cooldown = |state: &mut AdaptiveState| {
            state.phase = AdaptivePhase::Cooldown;
            state.cooldown_until = cooldown_until;
            state.learning_started_at = 0;
            state.observed_tokens = 0;
            state.learned_limit = 0;
            state.learned_window_ms = 0;
            state.expires_at = cooldown_until + ADAPTIVE_STATE_TTL_GRACE_MS;
        };
        match state.phase {
            AdaptivePhase::Inactive => {}
            AdaptivePhase::Cooldown => {
                state.cooldown_until = state.cooldown_until.max(cooldown_until);
                state.expires_at = state.cooldown_until + ADAPTIVE_STATE_TTL_GRACE_MS;
            }
            AdaptivePhase::Learning => {
                // Tokens accepted before the rejection: the pin subtracts the
                // rejected call's own tokens from the observed count.
                let accepted = state.observed_tokens - options.tokens as i64;
                if accepted <= 0 {
                    reset_to_cooldown(state);
                    return;
                }
                let learned_window =
                    clamp_adaptive_millis(now.saturating_sub(state.learning_started_at) + retry_ms);
                state.phase = AdaptivePhase::Learned;
                state.epoch += 1;
                state.cooldown_until = state.learning_started_at + learned_window;
                state.observed_tokens = accepted;
                state.learned_limit = accepted;
                state.learned_window_ms = learned_window as i64;
                state.expires_at = now + learned_window + ADAPTIVE_STATE_TTL_GRACE_MS;
            }
            AdaptivePhase::Learned => reset_to_cooldown(state),
        }
    }

    /// Consume, then park for the delay the limit asks for.
    ///
    /// The pin's `RateLimiter.sleep`; `sleeper` is its partially applied shape.
    pub fn sleep(
        &mut self,
        options: &ConsumeOptions<'_>,
    ) -> Result<ConsumeResult, RateLimitExceeded> {
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
