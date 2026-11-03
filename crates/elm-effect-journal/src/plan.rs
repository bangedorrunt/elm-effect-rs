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
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Default ceiling: 86400 s (one day).
pub const DEFAULT_CEILING_SECS: u64 = 86_400;

/// What a park computation decides.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParkIntent {
    /// Absolute deadline in ms.
    pub until_ms: u64,
    /// Planned wait in ms.
    pub wait_ms: u64,
}

/// Build the pre-wait value.
#[must_use]
pub fn park_intent(until_ms: u64, wait_ms: u64) -> ParkIntent {
    ParkIntent { until_ms, wait_ms }
}

/// Pure park computation.
///
/// - `ceiling_secs` defaults to [`DEFAULT_CEILING_SECS`];
/// - `max_wait_secs` above the ceiling refuses (typed, no clamping);
/// - `max_wait_secs == 0` with a future `until_ms` waits the real remaining
///   duration, fenced by the ceiling on the asked seconds
///   (`raw_wait_ms.div_ceil(1_000)`), so a zero cap cannot bypass it;
/// - `until_ms` at or before `now_ms` resumes now with the overdue gap.
#[must_use]
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
            overdue_ms: now_ms - until_ms,
        };
    }
    let raw_wait_ms = until_ms - now_ms;
    let wait_ms = if max_wait_secs == 0 {
        let asked_secs = raw_wait_ms.div_ceil(1_000);
        if asked_secs > ceiling {
            return ParkDecision::RefusedOverCeiling {
                asked_secs,
                ceiling_secs: ceiling,
            };
        }
        raw_wait_ms
    } else {
        raw_wait_ms.min(max_wait_secs.saturating_mul(1_000))
    };
    ParkDecision::Wait {
        wait_ms,
        intent: park_intent(until_ms, wait_ms),
    }
}

/// Identity of one parked wait.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PendingKey {
    /// Run the wait belongs to.
    pub run_id: String,
    /// Promise being parked on.
    pub promise_id: String,
    /// Park generation (bumps per re-park of the same promise).
    pub park_generation: u64,
}

/// One persisted pending row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    rows: BTreeMap<PendingKey, PendingRow>,
}

impl PendingStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
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
    #[must_use]
    pub fn get(&self, key: &PendingKey) -> Option<&PendingRow> {
        self.rows.get(key)
    }

    /// Stored row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// `true` with no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// Dispatch planning: pure implementation of the toron reconcile table
// (`toron/crates/toron-workflow/src/engine/reconcile.rs::plan_attempt`).
// A park is interruption, never a terminal return or a blind retry: it
// resumes with the SAME re-drive key (`ResumeParked`), never `DispatchNew`.

use crate::{EffectState, can_transition};

/// The journaled facts one dispatch decision needs (no I/O to build).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderCapability {
    /// Redispatching the same key is safe.
    pub supports_idempotency_key: bool,
    /// The provider can be asked what happened first.
    pub supports_reconcile_query: bool,
}

/// What the dispatcher may do next.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DispatchPlan {
    /// No intent or a fresh intent: commit intent, then dispatch attempt one.
    DispatchNew,
    /// Uncertainty with an idempotent provider: re-dispatch the SAME
    /// logical key as this next attempt number.
    RetrySameKey {
        /// The attempt number this re-dispatch becomes.
        attempt: i64,
    },
    /// Uncertainty with a queryable provider: ask what happened before
    /// dispatch. Never a second dispatch first.
    ReconcileFirst {
        /// Why the row is uncertain, for the log and the caller.
        reason: String,
    },
    /// No safe move exists (live owner elsewhere, reconciler owns the row,
    /// or no provider contract): do not dispatch, say why.
    RefuseBlindRetry {
        /// What blocked the dispatch (a live owner, a reconciler, or no
        /// provider contract).
        reason: String,
    },
    /// The row is terminal: return the stored outcome, dispatch nothing.
    DuplicateTerminal,
    /// A park is interruption: resume at its wake time with the same key,
    /// attempt at least one.
    ResumeParked {
        /// The attempt number the resume becomes; at least one.
        attempt: i64,
    },
}

/// Pure dispatch planning: exactly the toron reconcile table.
#[must_use]
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
#[must_use]
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
/// Defined through [`check_claim`]: one fence, two views.
#[must_use]
pub fn may_claim(
    state: EffectState,
    current_owner: Option<&str>,
    current_deadline: Option<i64>,
    owner: &str,
    now_ms: i64,
    expected_generation: u64,
    current_generation: u64,
) -> bool {
    check_claim(
        state,
        current_owner,
        current_deadline,
        owner,
        now_ms,
        expected_generation,
        current_generation,
    )
    .is_ok()
}

/// Replay guard: a digest mismatch is a typed error and mints no second
/// effect key (the caller simply does not proceed).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
#[error("replay digest mismatch: prepared {prepared:?} != redigest {redigest:?}")]
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
#[must_use]
pub fn terminal_success(state: crate::EffectState) -> Option<bool> {
    use crate::EffectState as S;
    match state {
        S::Settled => Some(true),
        S::Failed => Some(false),
        _ => None,
    }
}

/// Why a claim was refused. The typed form of [`may_claim`]: every `false`
/// from `may_claim` maps to exactly one variant here, in the same check
/// order (terminal, then generation, then lease).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
pub enum ClaimRejection {
    /// No row for the key; there is nothing to claim.
    #[error("no row for the key; there is nothing to claim")]
    NotFound,
    /// The row is finished; terminal rows never move.
    #[error("terminal row {state} never moves")]
    Terminal {
        /// The finished state that refused the claim.
        state: EffectState,
    },
    /// The presented generation is not current; a zombie claims nothing.
    #[error("stale generation: presented {expected} != current {actual}")]
    StaleGeneration {
        /// The generation the caller presented.
        expected: u64,
        /// The generation the row actually holds.
        actual: u64,
    },
    /// A live lease under another owner blocks the steal.
    #[error("live lease under {owner} until {lease_deadline} blocks the steal")]
    LeaseLive {
        /// The owner holding the live lease.
        owner: String,
        /// The lease deadline in ms.
        lease_deadline: i64,
    },
}

/// Typed claim fence: `Ok(())` claims, `Err` names why not. Agrees with
/// [`may_claim`] on every input (`Ok` exactly when `may_claim` is true).
pub fn check_claim(
    state: EffectState,
    current_owner: Option<&str>,
    current_deadline: Option<i64>,
    owner: &str,
    now_ms: i64,
    expected_generation: u64,
    current_generation: u64,
) -> Result<(), ClaimRejection> {
    if state.is_terminal() {
        return Err(ClaimRejection::Terminal { state });
    }
    if expected_generation != current_generation {
        return Err(ClaimRejection::StaleGeneration {
            expected: expected_generation,
            actual: current_generation,
        });
    }
    if let (Some(current), Some(deadline)) = (current_owner, current_deadline)
        && current != owner
        && deadline > now_ms
    {
        return Err(ClaimRejection::LeaseLive {
            owner: current.to_string(),
            lease_deadline: deadline,
        });
    }
    Ok(())
}

/// Why a heartbeat was refused. The typed form of the renew fence: terminal
/// rows never renew, a zombie generation renews nothing, a non-running row
/// holds no live lease, a foreign owner steals nothing, and a lapsed
/// deadline renews nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
pub enum HeartbeatRejection {
    /// No row for the key; there is nothing to renew.
    #[error("no row for the key; there is nothing to renew")]
    NotFound,
    /// The row is finished; terminal rows never renew.
    #[error("terminal row {state} never renews")]
    Terminal {
        /// The finished state that refused the heartbeat.
        state: EffectState,
    },
    /// The presented generation is not current; a zombie renews nothing.
    #[error("stale generation: presented {expected} != current {actual}")]
    StaleGeneration {
        /// The generation the caller presented.
        expected: u64,
        /// The generation the row actually holds.
        actual: u64,
    },
    /// The row holds no live lease; only `InFlight` and `Accepted` rows renew.
    #[error("row is {state}, not a live lease holder")]
    NotRunning {
        /// The state that refused the heartbeat.
        state: EffectState,
    },
    /// A live lease under another owner blocks the renew.
    #[error("lease held by {owner:?}, not the heartbeating worker")]
    NotOwner {
        /// The owner that actually holds the lease, if any.
        owner: Option<String>,
    },
    /// The lease already lapsed; a dead lease renews nothing (expire it).
    #[error("lease already expired (deadline {lease_deadline:?})")]
    LeaseExpired {
        /// The deadline that lapsed, if the row ever held one.
        lease_deadline: Option<i64>,
    },
}

/// Typed renew fence: `Ok(())` renews, `Err` names why not. Check order is
/// terminal, then generation, then running state, then owner, then lease. A
/// `None` deadline (or one at or before `now_ms`) is already expired.
pub fn check_heartbeat(
    state: EffectState,
    current_owner: Option<&str>,
    current_deadline: Option<i64>,
    owner: &str,
    now_ms: i64,
    expected_generation: u64,
    current_generation: u64,
) -> Result<(), HeartbeatRejection> {
    if state.is_terminal() {
        return Err(HeartbeatRejection::Terminal { state });
    }
    if expected_generation != current_generation {
        return Err(HeartbeatRejection::StaleGeneration {
            expected: expected_generation,
            actual: current_generation,
        });
    }
    if !matches!(state, EffectState::InFlight | EffectState::Accepted) {
        return Err(HeartbeatRejection::NotRunning { state });
    }
    if current_owner != Some(owner) {
        return Err(HeartbeatRejection::NotOwner {
            owner: current_owner.map(str::to_string),
        });
    }
    if current_deadline.is_none_or(|deadline| deadline <= now_ms) {
        return Err(HeartbeatRejection::LeaseExpired {
            lease_deadline: current_deadline,
        });
    }
    Ok(())
}

/// Why a settle was refused. The typed form of the store's settle fence:
/// stale generations, illegal edges, and terminal rows settle nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
pub enum SettleRejection {
    /// No row for the key; there is nothing to settle.
    #[error("no row for the key; there is nothing to settle")]
    NotFound,
    /// The row is finished; terminal rows never move.
    #[error("terminal row {state} never moves")]
    Terminal {
        /// The finished state that refused the settle.
        state: EffectState,
    },
    /// The presented generation is not current; a zombie settles nothing.
    #[error("stale generation: presented {expected} != current {actual}")]
    StaleGeneration {
        /// The generation the caller presented.
        expected: u64,
        /// The generation the row actually holds.
        actual: u64,
    },
    /// The golden table admits no such edge.
    #[error("illegal transition from {from} to {to}")]
    IllegalTransition {
        /// The row's current state.
        from: EffectState,
        /// The requested target state.
        to: EffectState,
    },
}

/// Typed settle fence: `Ok(())` settles, `Err` names why not. Check order
/// is absence, then terminal, then generation, then the golden table.
pub fn check_settle(
    current: Option<(EffectState, u64)>,
    expected_generation: u64,
    to: EffectState,
) -> Result<(), SettleRejection> {
    let Some((from, generation)) = current else {
        return Err(SettleRejection::NotFound);
    };
    if from.is_terminal() {
        return Err(SettleRejection::Terminal { state: from });
    }
    if generation != expected_generation {
        return Err(SettleRejection::StaleGeneration {
            expected: expected_generation,
            actual: generation,
        });
    }
    if !can_transition(from, to) {
        return Err(SettleRejection::IllegalTransition { from, to });
    }
    Ok(())
}
