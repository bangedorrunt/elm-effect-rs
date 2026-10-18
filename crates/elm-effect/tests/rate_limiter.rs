//! `RateLimiter` rows from `packages/effect/test/persistence/RateLimiter.test.ts`
//! at the effect@4.0.0 pin, run against `../src/stdlib/rate_limiter.rs`.
//!
//! The pin drives `TestClock`; this port injects a fake clock, so the same rows
//! run without sleeping.

#[path = "../src/stdlib/rate_limiter.rs"]
mod rate_limiter;

use rate_limiter::{
    AdaptiveConsumeOptions, AdaptiveFeedbackOptions, AdaptivePhase, Algorithm, ConsumeOptions,
    LimiterClock, OnExceeded, RateLimiter,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Virtual milliseconds. `sleep` records what was asked for instead of parking:
/// the rows that sleep assert the remaining count, and a parked fake clock could
/// only be woken by a test that is itself blocked.
struct FakeClock {
    now: Arc<AtomicU64>,
    slept: Arc<AtomicU64>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(0)),
            slept: Arc::new(AtomicU64::new(0)),
        }
    }

    fn advance(&self, duration: Duration) {
        self.now
            .fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
    }

    fn handle(&self) -> Arc<dyn LimiterClock> {
        Arc::new(Self {
            now: Arc::clone(&self.now),
            slept: Arc::clone(&self.slept),
        })
    }

    fn slept_ms(&self) -> u64 {
        self.slept.load(Ordering::SeqCst)
    }
}

impl LimiterClock for FakeClock {
    fn now_millis(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }

    fn sleep(&self, duration: Duration) {
        self.slept
            .fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
    }
}

fn minute() -> Duration {
    Duration::from_secs(60)
}

fn options<'a>(
    key: &'a str,
    algorithm: Algorithm,
    on_exceeded: OnExceeded,
    tokens: u32,
) -> ConsumeOptions<'a> {
    let mut options = ConsumeOptions::new(key, 5, minute());
    options.algorithm = algorithm;
    options.on_exceeded = on_exceeded;
    options.tokens = tokens;
    options
}

/// RateLimiter.test.ts:10 `supports partially applied sleep`. Binding the
/// limiter first and choosing the options later is the same call.
#[test]
fn rate_limiter_sleeps_partially_applied() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    // The pin passes no `onExceeded` and no `tokens`, so the defaults are what
    // this row exercises: fixed window, fail, one token.
    let opts = ConsumeOptions::new("partial", 5, minute());
    let mut sleeper = limiter.sleeper();
    let result = sleeper.call(&opts).expect("first call is admitted");
    assert_eq!(result.remaining, 4);
    // Admitted at once, so the sleep shape parks for nothing.
    assert_eq!(clock.slept_ms(), 0);
}

/// RateLimiter.test.ts:26 `supports uncurried sleep`.
#[test]
fn rate_limiter_sleeps_uncurried() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let opts = options("direct", Algorithm::FixedWindow, OnExceeded::Fail, 1);
    let result = rate_limiter::sleep(&mut limiter, &opts).expect("first call is admitted");
    assert_eq!(result.remaining, 4);
}

/// RateLimiter.test.ts:105 `reports a full-window reset for an exact
/// reservation`.
#[test]
fn fixed_window_reports_a_full_window_for_an_exact_reservation() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let opts = options("exact", Algorithm::FixedWindow, OnExceeded::Delay, 5);
    let result = limiter.consume(&opts).expect("within the limit");
    assert_eq!(result.limit, 5);
    assert_eq!(result.remaining, 0);
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.reset_after, minute());
}

/// RateLimiter.test.ts:60 `reports a fresh partial reset lifetime without
/// rounding`. One token reserves its share of the window, not the whole window.
#[test]
fn fixed_window_reports_a_fresh_partial_reset() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let opts = options("partial", Algorithm::FixedWindow, OnExceeded::Delay, 1);
    let result = limiter.consume(&opts).expect("within the limit");
    assert_eq!(result.limit, 5);
    assert_eq!(result.remaining, 4);
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.reset_after, Duration::from_secs(12));
}

/// RateLimiter.test.ts:78 `subtracts elapsed time from reset metadata`.
#[test]
fn fixed_window_subtracts_elapsed_time_from_the_reset() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let four = options("elapsed", Algorithm::FixedWindow, OnExceeded::Delay, 4);
    limiter.consume(&four).expect("within the limit");

    clock.advance(Duration::from_secs(10));
    let one = options("elapsed", Algorithm::FixedWindow, OnExceeded::Delay, 1);
    let result = limiter.consume(&one).expect("within the limit");
    assert_eq!(result.limit, 5);
    assert_eq!(result.remaining, 0);
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.reset_after, Duration::from_secs(50));
}

/// RateLimiter.test.ts:105 `reports the exact reserved lifetime without
/// changing overflow delay`. Six single-token calls reserve six shares, so the
/// overflow delay is one window and the reset metadata is 72s, not 60s.
#[test]
fn fixed_window_reports_the_exact_reserved_lifetime() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let opts = options("overflow", Algorithm::FixedWindow, OnExceeded::Delay, 1);
    for _ in 0..5 {
        limiter.consume(&opts).expect("delay mode never fails");
    }
    let result = limiter.consume(&opts).expect("delay mode never fails");
    assert_eq!(result.limit, 5);
    assert_eq!(result.remaining, -1);
    assert_eq!(result.delay, minute());
    assert_eq!(result.reset_after, Duration::from_secs(72));
}

/// RateLimiter.test.ts:125 `returns accumulated delays after the fixed window is
/// exceeded`. The delay steps a whole window per full window of excess, and
/// elapsed time eats into it.
#[test]
fn fixed_window_accumulates_delays_after_the_window_is_exceeded() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let opts = options("a", Algorithm::FixedWindow, OnExceeded::Delay, 1);

    // 1 + 3
    for _ in 0..4 {
        limiter.consume(&opts).expect("delay mode never fails");
    }
    let mut result = limiter.consume(&opts).expect("delay mode never fails"); // 5
    assert_eq!(result.delay, Duration::ZERO);
    result = limiter.consume(&opts).expect("delay mode never fails"); // 6
    assert_eq!(result.delay, minute());

    for _ in 0..3 {
        limiter.consume(&opts).expect("delay mode never fails"); // 7, 8, 9
    }
    result = limiter.consume(&opts).expect("delay mode never fails"); // 10
    assert_eq!(result.delay, minute());
    result = limiter.consume(&opts).expect("delay mode never fails"); // 11
    assert_eq!(result.delay, Duration::from_secs(120));

    clock.advance(Duration::from_secs(30));
    result = limiter.consume(&opts).expect("delay mode never fails"); // 12
    assert_eq!(result.delay, Duration::from_secs(90));

    clock.advance(Duration::from_secs(45));
    result = limiter.consume(&opts).expect("delay mode never fails"); // 13
    assert_eq!(result.delay, Duration::from_secs(45));
}

/// RateLimiter.test.ts:202 `returns delay based on the token refill rate`.
#[test]
fn token_bucket_returns_the_refill_rate_as_delay() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let opts = options("a", Algorithm::TokenBucket, OnExceeded::Delay, 1);
    let refill = Duration::from_secs(12);

    for _ in 0..4 {
        limiter.consume(&opts).expect("delay mode never fails"); // 1 + 3
    }
    let mut result = limiter.consume(&opts).expect("delay mode never fails"); // 5
    assert_eq!(result.delay, Duration::ZERO);
    result = limiter.consume(&opts).expect("delay mode never fails"); // 6
    assert_eq!(result.delay, refill);
    result = limiter.consume(&opts).expect("delay mode never fails"); // 7
    assert_eq!(result.delay, refill * 2);

    clock.advance(minute());
    result = limiter.consume(&opts).expect("delay mode never fails");
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.remaining, 2);
}

/// RateLimiter.test.ts:231 `fails until enough tokens are refilled`.
#[test]
fn token_bucket_fails_until_enough_tokens_refill() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let opts = options("a", Algorithm::TokenBucket, OnExceeded::Fail, 1);

    for _ in 0..4 {
        limiter.consume(&opts).expect("within the limit"); // 1 + 3
    }
    let result = limiter.consume(&opts).expect("within the limit"); // 5
    assert_eq!(result.delay, Duration::ZERO);

    let error = limiter.consume(&opts).expect_err("bucket is empty"); // 6
    assert_eq!(error.retry_after, Duration::from_secs(12));
    assert_eq!(error.remaining, 0);

    clock.advance(Duration::from_secs(36));
    let result = limiter.consume(&opts).expect("three tokens refilled");
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.remaining, 2);
}

// --- adaptive ---

fn adaptive<'a>(key: &'a str, tokens: u32) -> AdaptiveConsumeOptions<'a> {
    AdaptiveConsumeOptions {
        key,
        tokens,
        fallback_limit: 5,
        fallback_window: minute(),
    }
}

fn feedback(
    key: &str,
    epoch: u64,
    tokens: u32,
    retry_after: Option<Duration>,
) -> AdaptiveFeedbackOptions<'_> {
    AdaptiveFeedbackOptions {
        key,
        epoch,
        tokens,
        status: 429,
        retry_after,
    }
}

/// RateLimiter.test.ts:264 `uses the inactive store path until 429 Retry-After
/// feedback`. A 200 or a 429 without Retry-After changes nothing.
#[test]
fn adaptive_stays_inactive_until_a_429_with_retry_after() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let mut result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Inactive);

    let mut ok = feedback("a", result.epoch, 1, Some(Duration::from_secs(1)));
    ok.status = 200;
    limiter.adaptive_feedback(&ok);
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.phase, AdaptivePhase::Inactive);

    limiter.adaptive_feedback(&feedback("a", result.epoch, 1, None));
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Inactive);
}

/// RateLimiter.test.ts:308 `starts cooldown on the first 429 Retry-After
/// feedback`. Only the key that saw the 429 is affected.
#[test]
fn adaptive_starts_cooldown_on_the_first_429() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(5))));

    let result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(5));
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Cooldown);

    let other = limiter.adaptive_consume(&adaptive("b", 1));
    assert_eq!(other.delay, Duration::ZERO);
    assert_eq!(other.epoch, 0);
    assert_eq!(other.phase, AdaptivePhase::Inactive);
}

/// RateLimiter.test.ts:349 `clamps zero Retry-After feedback to the minimum
/// cooldown`.
#[test]
fn adaptive_clamps_a_zero_retry_after_to_one_millisecond() {
    let mut limiter = RateLimiter::with_clock(FakeClock::new().handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::ZERO)));

    let result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_millis(1));
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Cooldown);
}

/// RateLimiter.test.ts:380 `bounds excessive adaptive Retry-After cooldowns`.
/// Two hours of wait is cut to the hour ceiling, and the cooldown then hands
/// over to learning.
#[test]
fn adaptive_bounds_an_excessive_retry_after() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback(
        "a",
        first.epoch,
        1,
        Some(Duration::from_secs(2 * 60 * 60)),
    ));

    let mut result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(60 * 60));
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Cooldown);

    clock.advance(Duration::from_secs(60 * 60));
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, 1);
    assert_eq!(result.phase, AdaptivePhase::Learning);
}

/// RateLimiter.test.ts:423 `returns the remaining cooldown delay`.
#[test]
fn adaptive_returns_the_remaining_cooldown() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(10))));
    clock.advance(Duration::from_secs(4));

    let result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(6));
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Cooldown);
}

/// RateLimiter.test.ts:455 `starts learning on the first admitted post-cooldown
/// request`.
#[test]
fn adaptive_starts_learning_after_the_cooldown() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(10))));
    clock.advance(Duration::from_secs(10));

    let result = limiter.adaptive_consume(&adaptive("a", 2));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, 1);
    assert_eq!(result.phase, AdaptivePhase::Learning);
}

/// RateLimiter.test.ts:487 `keeps learning requests at zero delay while
/// observing token counts`.
#[test]
fn adaptive_keeps_learning_at_zero_delay() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    let learning = limiter.adaptive_consume(&adaptive("a", 2));
    assert_eq!(learning.phase, AdaptivePhase::Learning);

    let result = limiter.adaptive_consume(&adaptive("a", 3));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, learning.epoch);
    assert_eq!(result.phase, AdaptivePhase::Learning);
}

/// RateLimiter.test.ts:527 `learns the accepted token count and inferred window
/// from learning feedback`. Two accepted tokens over a 10s window: the third
/// call in the window waits 10s.
#[test]
fn adaptive_learns_the_accepted_tokens_and_window() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    let learning = limiter.adaptive_consume(&adaptive("a", 2));
    clock.advance(Duration::from_secs(4));
    let rejected = limiter.adaptive_consume(&adaptive("a", 3));
    assert_eq!(learning.epoch, rejected.epoch);

    limiter.adaptive_feedback(&feedback(
        "a",
        rejected.epoch,
        3,
        Some(Duration::from_secs(6)),
    ));
    clock.advance(Duration::from_secs(6));

    let mut result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.phase, AdaptivePhase::Learned);
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.phase, AdaptivePhase::Learned);
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(10));
    assert_eq!(result.phase, AdaptivePhase::Learned);
}

/// RateLimiter.test.ts:601 `ignores stale epoch feedback`.
#[test]
fn adaptive_ignores_stale_epoch_feedback() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    let learning = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback(
        "a",
        first.epoch,
        1,
        Some(Duration::from_secs(10)),
    ));

    let result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, learning.epoch);
    assert_eq!(result.phase, AdaptivePhase::Learning);
}

/// RateLimiter.test.ts:648 `prevents repeated learning 429 feedback from
/// corrupting learned state`. The second feedback for the learning epoch
/// arrives after that epoch became learned, so it is dropped.
#[test]
fn adaptive_repeated_learning_feedback_cannot_corrupt_learned_state() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    limiter.adaptive_consume(&adaptive("a", 1));
    let rejected = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", rejected.epoch, 1, Some(Duration::from_secs(5))));
    limiter.adaptive_feedback(&feedback(
        "a",
        rejected.epoch,
        1,
        Some(Duration::from_secs(30)),
    ));

    let learned = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(learned.delay, Duration::from_secs(5));
    assert_eq!(learned.epoch, rejected.epoch + 1);
    assert_eq!(learned.phase, AdaptivePhase::Learned);
}

/// RateLimiter.test.ts:708 `same-epoch learned 429 feedback enters cooldown and
/// can only extend it`.
#[test]
fn adaptive_learned_feedback_enters_cooldown_and_extends() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    limiter.adaptive_consume(&adaptive("a", 1));
    let rejected = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", rejected.epoch, 1, Some(Duration::from_secs(5))));

    let learned = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", learned.epoch, 1, Some(Duration::from_secs(5))));
    limiter.adaptive_feedback(&feedback(
        "a",
        learned.epoch,
        1,
        Some(Duration::from_secs(10)),
    ));

    let result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(10));
    assert_eq!(result.epoch, learned.epoch);
    assert_eq!(result.phase, AdaptivePhase::Cooldown);
}

/// RateLimiter.test.ts:782 `stores only cooldown when no tokens were accepted
/// during learning`. Every observed token was the rejected call's own, so
/// nothing is learned and the cooldown stands.
#[test]
fn adaptive_stores_only_cooldown_when_nothing_was_accepted() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    let learning = limiter.adaptive_consume(&adaptive("a", 2));
    limiter.adaptive_feedback(&feedback("a", learning.epoch, 2, Some(Duration::from_secs(5))));

    let mut result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(5));
    assert_eq!(result.epoch, learning.epoch);
    assert_eq!(result.phase, AdaptivePhase::Cooldown);

    clock.advance(Duration::from_secs(5));
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, learning.epoch + 1);
    assert_eq!(result.phase, AdaptivePhase::Learning);
}

/// RateLimiter.test.ts:840 `expires learned adaptive state`. The learned state
/// lives its window plus the 60s grace, then the key is inactive again.
#[test]
fn adaptive_learned_state_expires() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_secs(1))));
    clock.advance(Duration::from_secs(1));

    limiter.adaptive_consume(&adaptive("a", 1));
    let rejected = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", rejected.epoch, 1, Some(Duration::from_secs(1))));

    clock.advance(Duration::from_secs(61));
    let result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Inactive);
}

/// RateLimiter.test.ts:895 `bounds learned adaptive windows and expires the
/// bounded state`. A two-hour Retry-After learns an hour-long window, and the
/// state expires an hour plus the grace later.
#[test]
fn adaptive_learned_window_is_bounded_and_expires() {
    let clock = FakeClock::new();
    let mut limiter = RateLimiter::with_clock(clock.handle());
    let first = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback("a", first.epoch, 1, Some(Duration::from_millis(1))));
    clock.advance(Duration::from_millis(1));

    limiter.adaptive_consume(&adaptive("a", 1));
    let rejected = limiter.adaptive_consume(&adaptive("a", 1));
    limiter.adaptive_feedback(&feedback(
        "a",
        rejected.epoch,
        1,
        Some(Duration::from_secs(2 * 60 * 60)),
    ));

    let mut result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::from_secs(60 * 60));
    assert_eq!(result.phase, AdaptivePhase::Learned);

    clock.advance(Duration::from_secs(61 * 60));
    result = limiter.adaptive_consume(&adaptive("a", 1));
    assert_eq!(result.delay, Duration::ZERO);
    assert_eq!(result.epoch, 0);
    assert_eq!(result.phase, AdaptivePhase::Inactive);
}
