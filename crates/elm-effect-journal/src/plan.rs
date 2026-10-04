//! Park planning (`plan_park`) plus the pending-row store.
//!
//! [`plan_park`] is pure: the host supplies `now_ms`, so replay may move a
//! park to a resume, and that movement is the point. A wait above `ceiling`
//! is a typed [`Refused`](ParkDecision::RefusedOverCeiling), never a shorter
//! sleep; a past `until_ms` is [`ResumeNow`](ParkDecision::ResumeNow) with
//! the overdue gap, never wrapping.
//!
//! Parks persist as pending rows keyed by `(run_id, promise_id,
//! park_generation)`: [`upsert_pending`](PendingStore::upsert_pending) is
//! idempotent, a second upsert with the same key writes nothing. `ResumeNow`
//! builds no intent; [`park_intent`] builds the value before the wait. No
//! effect-row write to `parked` happens here: `Parked` stays parseable and
//! non-terminal in the registry vocabulary.

use std::collections::HashMap;

/// Default ceiling: 86400 s (one day).
pub const DEFAULT_CEILING_SECS: u64 = 86_400;

/// What a park computation decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParkDecision {
    /// The deadline already passed; resume now, reporting how overdue in ms.
    ResumeNow {
        /// `now_ms - until_ms`, saturating (never wraps).
        overdue_ms: u64,
    },
    /// Wait, with the pre-built intent.
    Wait {
        /// Milliseconds to wait.
        wait_ms: u64,
        /// Value built before the wait.
        intent: ParkIntent,
    },
    /// The asked wait exceeds the ceiling: typed refusal, not a shorter sleep.
    RefusedOverCeiling {
        /// Asked `max_wait_secs`.
        asked_secs: u64,
        /// Ceiling that refused it.
        ceiling_secs: u64,
    },
}

/// Value built before a wait (never built for `ResumeNow`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkIntent {
    /// Absolute deadline in ms.
    pub until_ms: u64,
    /// Planned wait in ms.
    pub wait_ms: u64,
}

/// Build the pre-wait value.
pub fn park_intent(until_ms: u64, wait_ms: u64) -> ParkIntent {
    ParkIntent { until_ms, wait_ms }
}

/// Pure park computation.
///
/// - `ceiling_secs` defaults to [`DEFAULT_CEILING_SECS`];
/// - `max_wait_secs` above the ceiling refuses (typed, no clamping);
/// - `until_ms` at or before `now_ms` resumes now with the overdue gap.
pub fn plan_park(
    until_ms: u64,
    max_wait_secs: u64,
    now_ms: u64,
    ceiling_secs: Option<u64>,
) -> ParkDecision {
    let ceiling = ceiling_secs.unwrap_or(DEFAULT_CEILING_SECS);
    if max_wait_secs > ceiling {
        return ParkDecision::RefusedOverCeiling {
            asked_secs: max_wait_secs,
            ceiling_secs: ceiling,
        };
    }
    if until_ms <= now_ms {
        return ParkDecision::ResumeNow {
            overdue_ms: now_ms.saturating_sub(until_ms),
        };
    }
    let wait_ms = (until_ms - now_ms).min(max_wait_secs.saturating_mul(1_000));
    ParkDecision::Wait {
        wait_ms,
        intent: park_intent(until_ms, wait_ms),
    }
}

/// Identity of one parked wait.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PendingKey {
    /// Run the wait belongs to.
    pub run_id: String,
    /// Promise being parked on.
    pub promise_id: String,
    /// Park generation (bumps per re-park of the same promise).
    pub park_generation: u64,
}

/// One persisted pending row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRow {
    /// Idempotency key.
    pub key: PendingKey,
    /// Pre-built wait value.
    pub intent: ParkIntent,
}

/// In-memory pending store (the host persists rows; the unit owns the
/// idempotency rule).
#[derive(Clone, Debug, Default)]
pub struct PendingStore {
    rows: HashMap<PendingKey, PendingRow>,
}

impl PendingStore {
    /// Empty store.
    pub fn new() -> Self {
        Self {
            rows: HashMap::new(),
        }
    }

    /// Insert; `true` when written, `false` when the key already existed
    /// (the stored row is untouched: a second upsert writes nothing).
    pub fn upsert_pending(&mut self, row: PendingRow) -> bool {
        if self.rows.contains_key(&row.key) {
            return false;
        }
        self.rows.insert(row.key.clone(), row);
        true
    }

    /// Look up a pending row.
    pub fn get(&self, key: &PendingKey) -> Option<&PendingRow> {
        self.rows.get(key)
    }

    /// Stored row count.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// `true` with no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
