//! Retry backoff planning: pure, no clock, no I/O (ADR-0005 extension).
//!
//! [`BackoffPolicy`] names the wait between attempts; [`plan_retry`]
//! turns a recorded attempt count into either the next [`RetryPlan::Backoff`]
//! or [`RetryPlan::Exhausted`]. The exhaustion boundary is the journal
//! dead-letter rule (`attempts > max_attempts`, see
//! [`InMemoryStore::attempt`](crate::store::InMemoryStore::attempt)): the
//! planner exhausts exactly when the journal dead-letters, pinned by test.

use serde::{Deserialize, Serialize};

/// Wait between attempts. No jitter: jitter needs randomness and the host
/// adds it to the delay it passes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackoffPolicy {
    /// The same wait after every attempt.
    Fixed {
        /// Wait in ms.
        delay_ms: u64,
    },
    /// `base_ms * factor^(attempt-1)`, saturating and capped at `cap_ms`.
    /// A factor below 2 behaves as a fixed `base_ms`.
    Exponential {
        /// First wait in ms.
        base_ms: u64,
        /// Growth per attempt.
        factor: u32,
        /// No wait exceeds this.
        cap_ms: u64,
    },
}

impl BackoffPolicy {
    /// Wait in ms for `attempt` (1-based). Saturating; an exponential never
    /// exceeds its cap.
    #[must_use]
    pub fn delay_ms(&self, attempt: u32) -> u64 {
        match *self {
            Self::Fixed { delay_ms } => delay_ms,
            Self::Exponential {
                base_ms,
                factor,
                cap_ms,
            } => {
                if factor < 2 {
                    return base_ms.min(cap_ms);
                }
                let mut delay = base_ms.min(cap_ms);
                for _ in 1..attempt.max(1) {
                    delay = delay.saturating_mul(u64::from(factor));
                    if delay >= cap_ms {
                        return cap_ms;
                    }
                }
                delay.min(cap_ms)
            }
        }
    }
}

/// What one recorded attempt means for the next wait.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RetryPlan {
    /// Wait `delay_ms` until `until_ms`, then re-drive the same fence.
    Backoff {
        /// Attempts recorded (the count that earned this wait).
        attempt: u32,
        /// Wait in ms from `now_ms`.
        delay_ms: u64,
        /// Absolute re-drive instant in ms.
        until_ms: u64,
    },
    /// The journal already dead-lettered this count; there is no next wait.
    Exhausted {
        /// Attempts recorded.
        attempts: u32,
        /// Attempts the row allowed.
        max_attempts: u32,
    },
}

/// Plan the wait after `attempts_recorded` attempts against `max_attempts`.
/// Exhausts exactly when [`attempts_recorded`](RetryPlan::Exhausted) exceeds
/// the budget — the same edge that dead-letters the journal row.
#[must_use]
pub fn plan_retry(
    attempts_recorded: u32,
    max_attempts: u32,
    policy: &BackoffPolicy,
    now_ms: u64,
) -> RetryPlan {
    if attempts_recorded > max_attempts {
        return RetryPlan::Exhausted {
            attempts: attempts_recorded,
            max_attempts,
        };
    }
    let delay_ms = policy.delay_ms(attempts_recorded.max(1));
    RetryPlan::Backoff {
        attempt: attempts_recorded,
        delay_ms,
        until_ms: now_ms.saturating_add(delay_ms),
    }
}
