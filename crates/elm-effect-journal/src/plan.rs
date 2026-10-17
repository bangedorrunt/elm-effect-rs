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

// Dispatch planning (0c6.43): pure port of the toron reconcile table
// (`toron/crates/toron-workflow/src/engine/reconcile.rs::plan_attempt`).
// A park is interruption, never a terminal return or a blind retry: it
// resumes with the SAME re-drive key (`ResumeParked`), never `DispatchNew`.

use crate::EffectState;

/// The journaled facts one dispatch decision needs (no I/O to build).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectView {
    /// The row's current state.
    pub state: EffectState,
    /// The next attempt number.
    pub attempt: i64,
    /// True when another owner may still hold the dispatch lease.
    pub lease_live: bool,
}

/// What the provider offers: an idempotency key, a reconcile query, both,
/// or neither. A query beats a key: never dispatch blind when asked first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapability {
    /// Redispatching the same key is safe.
    pub supports_idempotency_key: bool,
    /// The provider can be asked what happened first.
    pub supports_reconcile_query: bool,
}

/// What the dispatcher may do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchPlan {
    /// No intent or a fresh intent: commit intent, then dispatch attempt one.
    DispatchNew,
    /// Uncertainty with an idempotent provider: re-dispatch the SAME
    /// logical key as this next attempt number.
    RetrySameKey { attempt: i64 },
    /// Uncertainty with a queryable provider: ask what happened before
    /// dispatch. Never a second dispatch first.
    ReconcileFirst { reason: String },
    /// No safe move exists (live owner elsewhere, reconciler owns the row,
    /// or no provider contract): do not dispatch, say why.
    RefuseBlindRetry { reason: String },
    /// The row is terminal: return the stored outcome, dispatch nothing.
    DuplicateTerminal,
    /// A park is interruption: resume at its wake time with the same key,
    /// attempt at least one.
    ResumeParked { attempt: i64 },
}

/// Pure dispatch planning: exactly the toron reconcile table.
pub fn plan_attempt(current: Option<&EffectView>, cap: &ProviderCapability) -> DispatchPlan {
    let Some(view) = current else {
        return DispatchPlan::DispatchNew;
    };
    use crate::EffectState as S;
    match view.state {
        S::Parked => DispatchPlan::ResumeParked {
            attempt: view.attempt.max(1),
        },
        S::Prepared => DispatchPlan::DispatchNew,
        S::ReconcilePending => DispatchPlan::RefuseBlindRetry {
            reason: "reconciliation owns this effect; the dispatcher waits".into(),
        },
        S::InFlight | S::Accepted | S::Unknown => {
            if view.lease_live {
                DispatchPlan::RefuseBlindRetry {
                    reason: "dispatch lease is live under another owner".into(),
                }
            } else if cap.supports_reconcile_query {
                DispatchPlan::ReconcileFirst {
                    reason: "outcome uncertain after an expired lease; the provider is asked first"
                        .into(),
                }
            } else if cap.supports_idempotency_key {
                DispatchPlan::RetrySameKey {
                    attempt: view.attempt + 1,
                }
            } else {
                DispatchPlan::RefuseBlindRetry {
                    reason: "outcome uncertain and the provider offers no contract".into(),
                }
            }
        }
        S::Settled | S::Failed | S::Cancelled => DispatchPlan::DuplicateTerminal,
    }
}

/// Whether `presented` settles the row holding `current_token`: true only
/// when both are non-empty and equal. An empty presented token settles
/// nothing; an unclaimed row (`None`) settles for nobody.
pub fn may_settle(current_token: Option<&str>, presented: &str) -> bool {
    match current_token {
        Some(current) if !current.is_empty() && !presented.is_empty() => current == presented,
        _ => false,
    }
}

/// Claim fence (pure): true only when the row is non-terminal, the
/// presented generation is current, and no live lease under another
/// owner blocks the steal. Same-owner renew and expired-lease reclaim
/// pass; a live lease under another owner and any terminal row refuse.
pub fn may_claim(
    state: EffectState,
    current_owner: Option<&str>,
    current_deadline: Option<i64>,
    owner: &str,
    now_ms: i64,
    expected_generation: u64,
    current_generation: u64,
) -> bool {
    if state.is_terminal() {
        return false;
    }
    if expected_generation != current_generation {
        return false;
    }
    match (current_owner, current_deadline) {
        (Some(current), Some(deadline)) if current != owner && deadline > now_ms => false,
        _ => true,
    }
}

/// Replay guard: a digest mismatch is a typed error and mints no second
/// effect key (the caller simply does not proceed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayMismatch {
    /// The digest recorded at prepare time.
    pub prepared: String,
    /// The digest recomputed now.
    pub redigest: String,
}

/// Check a replay digest: `Ok(())` continues with the existing key.
pub fn check_replay(prepared_digest: &str, redigest: &str) -> Result<(), ReplayMismatch> {
    if prepared_digest == redigest {
        Ok(())
    } else {
        Err(ReplayMismatch {
            prepared: prepared_digest.to_string(),
            redigest: redigest.to_string(),
        })
    }
}

/// Terminal success, if the state has a default exit at all. `Unknown` (and
/// every non-terminal state) has none: uncertainty is not an outcome.
pub fn terminal_success(state: crate::EffectState) -> Option<bool> {
    use crate::EffectState as S;
    match state {
        S::Settled => Some(true),
        S::Failed => Some(false),
        _ => None,
    }
}
