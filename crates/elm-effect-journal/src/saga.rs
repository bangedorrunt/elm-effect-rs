//! Pure Saga compensation planner (Resonate/Temporal shape, no I/O).
//!
//! [`plan_saga_rollback`] walks forward steps in strict LIFO (reverse)
//! order, collecting the stored [`SagaStep::compensate_key`] of every
//! [`SagaStepStatus::Completed`] step. Steps already
//! [`Compensated`](SagaStepStatus::Compensated), [`Failed`](SagaStepStatus::Failed),
//! or [`Skipped`](SagaStepStatus::Skipped) are left out, so a rollback that
//! crashed halfway resumes idempotently: re-planning over the updated steps
//! never re-emits a compensation that already ran.

use serde::{Deserialize, Serialize};

/// What happened to one saga step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SagaStepStatus {
    /// Ran and needs compensating on rollback.
    Completed,
    /// Already compensated; never emitted again.
    Compensated,
    /// Never completed; has nothing to undo.
    Failed,
    /// Never ran; has nothing to undo.
    Skipped,
}

/// One forward step plus its precomputed compensation key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaStep {
    /// The forward step's key.
    pub step_key: String,
    /// The key of the compensation for this step (see [`compensate_key`]).
    pub compensate_key: String,
    /// What happened to this step.
    pub status: SagaStepStatus,
}

/// The rollback to run: compensations in the order to execute them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaPlan {
    /// Compensation keys, strict LIFO over the completed steps.
    pub compensations: Vec<String>,
    /// True when there is nothing left to compensate.
    pub all_compensated: bool,
}

/// Build the compensation key for one step of a run.
#[must_use]
pub fn compensate_key(run_key: &str, step: &str) -> String {
    format!("{run_key}:compensate:{step}")
}

/// Plan a rollback: reverse the forward steps, keep only the completed
/// ones. `all_compensated` is true exactly when no compensation remains
/// (nothing completed, or every completed step already compensated).
#[must_use]
pub fn plan_saga_rollback(steps: &[SagaStep]) -> SagaPlan {
    let compensations: Vec<String> = steps
        .iter()
        .rev()
        .filter(|s| s.status == SagaStepStatus::Completed)
        .map(|s| s.compensate_key.clone())
        .collect();
    let all_compensated = compensations.is_empty();
    SagaPlan {
        compensations,
        all_compensated,
    }
}
