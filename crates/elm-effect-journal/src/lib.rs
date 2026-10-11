//! Effect journal (ADR-0002 Phase 9, Law L14 registry part).
//!
//! One [`EffectState`] enum tracks an effect row: `Prepared`, `InFlight`,
//! `Accepted`, `Settled`, `Failed`, `Unknown`, `ReconcilePending`,
//! `Cancelled`, `Parked`. `Absent` is "no row", NOT a variant: there is no
//! second enum and no boundary mirror. [`can_transition`] is the golden
//! table exactly as toron-core has it (20 pairs); in particular `Parked`
//! has NO effect-row edge, so nothing enters or leaves `Parked` through
//! this table. `Parked` is resumable: not terminal, not a failure, not
//! settled, which keeps park, success, and typed failure three distinct
//! outcomes. [`settle_from_exit`] folds an [`Exit`](elm_effect::Exit)
//! into the registry vocabulary (`Success` settles; any failure fails).
//!
//! Public modules: [`durable_log`] (the L16 append trait, now fencing a
//! stale producer epoch through [`AppendError::StaleEpoch`](durable_log::AppendError::StaleEpoch)),
//! [`laws`] (trait-level law checks any `DurableLog` implementation must
//! pass, so a sibling's own log runs the same checks as [`MemLog`](durable_log::MemLog)),
//! [`store`] (claim/settle), [`plan`] (`plan_attempt`, `plan_park`,
//! `may_settle`), [`epoch`] (`EpochLog`, `Fence::StaleEpoch`) and [`stream`]
//! (`Gone`, `Closed`, snapshot). `plan`, `epoch` and `stream` used to be
//! reachable only through test-side `#[path]` includes; they are wired here
//! so a sibling can depend on them (ADR-0002 Phase 9b, L16).

use serde::{Deserialize, Serialize};

pub mod durable_log;
pub mod epoch;
pub mod laws;
pub mod plan;
pub mod store;
pub mod stream;

/// Lifecycle state of one effect row. No `Absent`: absence is no row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectState {
    /// Recorded, not yet dispatched.
    Prepared,
    /// Dispatched, outcome pending.
    InFlight,
    /// Outcome accepted, settlement pending.
    Accepted,
    /// Finished successfully.
    Settled,
    /// Finished with a typed failure.
    Failed,
    /// Outcome unknown (dispatch lost track).
    Unknown,
    /// Unknown and queued for reconciliation.
    ReconcilePending,
    /// Cancelled before finishing.
    Cancelled,
    /// Parked: resumable, non-terminal, non-failure. No table edge.
    Parked,
}

impl EffectState {
    /// True for finished states (settled, failed, cancelled).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Failed | Self::Cancelled)
    }

    /// True for typed failure only.
    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    /// True for successful settlement only.
    pub const fn is_settled(self) -> bool {
        matches!(self, Self::Settled)
    }

    /// True only for the resumable parked state.
    pub const fn is_resumable(self) -> bool {
        matches!(self, Self::Parked)
    }
}

impl std::fmt::Display for EffectState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Prepared => "prepared",
            Self::InFlight => "in_flight",
            Self::Accepted => "accepted",
            Self::Settled => "settled",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::ReconcilePending => "reconcile_pending",
            Self::Cancelled => "cancelled",
            Self::Parked => "parked",
        };
        f.write_str(name)
    }
}

impl std::str::FromStr for EffectState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "prepared" => Ok(Self::Prepared),
            "in_flight" => Ok(Self::InFlight),
            "accepted" => Ok(Self::Accepted),
            "settled" => Ok(Self::Settled),
            "failed" => Ok(Self::Failed),
            "unknown" => Ok(Self::Unknown),
            "reconcile_pending" => Ok(Self::ReconcilePending),
            "cancelled" => Ok(Self::Cancelled),
            "parked" => Ok(Self::Parked),
            _ => Err(format!("unknown effect state: {s}")),
        }
    }
}

/// Golden transition table, exactly as toron-core. `const fn` so the
/// registry and tests share one definition.
pub const fn can_transition(from: EffectState, to: EffectState) -> bool {
    use EffectState as S;
    matches!(
        (from, to),
        (S::Prepared, S::InFlight)
            | (S::Prepared, S::Failed)
            | (S::Prepared, S::Cancelled)
            | (S::InFlight, S::Accepted)
            | (S::InFlight, S::Settled)
            | (S::InFlight, S::Failed)
            | (S::InFlight, S::Unknown)
            | (S::InFlight, S::Cancelled)
            | (S::Accepted, S::Settled)
            | (S::Accepted, S::Failed)
            | (S::Accepted, S::Unknown)
            | (S::Accepted, S::Cancelled)
            | (S::Unknown, S::ReconcilePending)
            | (S::Unknown, S::Cancelled)
            | (S::ReconcilePending, S::Settled)
            | (S::ReconcilePending, S::Failed)
            | (S::ReconcilePending, S::Unknown)
            | (S::ReconcilePending, S::Cancelled)
    )
}

/// Fold an [`Exit`](elm_effect::Exit) into registry vocabulary.
pub fn settle_from_exit<A, E>(exit: &elm_effect::Exit<A, E>) -> EffectState {
    match exit {
        elm_effect::Exit::Success(_) => EffectState::Settled,
        elm_effect::Exit::Failure(_) => EffectState::Failed,
    }
}

/// Checked step: the target state on a legal edge, else rejection.
pub const fn transition(
    from: EffectState,
    to: EffectState,
) -> Result<EffectState, TransitionError> {
    if can_transition(from, to) {
        Ok(to)
    } else {
        Err(TransitionError)
    }
}

/// Registry rejection.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("illegal transition")]
pub struct TransitionError;
