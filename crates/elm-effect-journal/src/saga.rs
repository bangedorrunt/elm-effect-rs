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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SagaStep {
    /// The forward step's key.
    pub step_key: String,
    /// The key of the compensation for this step (see [`compensate_key`]).
    pub compensate_key: String,
    /// What happened to this step.
    pub status: SagaStepStatus,
}

/// The rollback to run: compensations in the order to execute them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Coordinator for tracking and rolling back compensable saga workflow steps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaCoordinator {
    run_key: String,
    steps: Vec<SagaStep>,
}

impl SagaCoordinator {
    /// Create a new coordinator for `run_key`.
    #[must_use]
    pub fn new(run_key: impl Into<String>) -> Self {
        Self {
            run_key: run_key.into(),
            steps: Vec::new(),
        }
    }

    /// Access the run key associated with this coordinator.
    #[must_use]
    pub fn run_key(&self) -> &str {
        &self.run_key
    }

    /// Check whether a step with `step_name` has been recorded.
    #[must_use]
    pub fn has_step(&self, step_name: &str) -> bool {
        let key = format!("{}:{}", self.run_key, step_name);
        self.steps.iter().any(|s| s.step_key == key)
    }

    /// Record a forward step in [`SagaStepStatus::Completed`] status.
    ///
    /// Step names should be distinct within a run to ensure unique compensation keys.
    pub fn record_completed(&mut self, step_name: &str) {
        self.steps.push(SagaStep {
            step_key: format!("{}:{}", self.run_key, step_name),
            compensate_key: compensate_key(&self.run_key, step_name),
            status: SagaStepStatus::Completed,
        });
    }

    /// Record a forward step in [`SagaStepStatus::Failed`] status.
    ///
    /// Step names should be distinct within a run.
    pub fn record_failed(&mut self, step_name: &str) {
        self.steps.push(SagaStep {
            step_key: format!("{}:{}", self.run_key, step_name),
            compensate_key: compensate_key(&self.run_key, step_name),
            status: SagaStepStatus::Failed,
        });
    }

    /// Record a forward step in [`SagaStepStatus::Skipped`] status.
    ///
    /// Step names should be distinct within a run.
    pub fn record_skipped(&mut self, step_name: &str) {
        self.steps.push(SagaStep {
            step_key: format!("{}:{}", self.run_key, step_name),
            compensate_key: compensate_key(&self.run_key, step_name),
            status: SagaStepStatus::Skipped,
        });
    }

    /// Mark the most recent uncompensated completed step matching `comp_key` as [`SagaStepStatus::Compensated`].
    ///
    /// Evaluates in reverse (LIFO) order to mirror the rollback plan.
    /// Returns `true` if a completed step was marked, or `false` if none matched.
    pub fn mark_compensated(&mut self, comp_key: &str) -> bool {
        for step in self.steps.iter_mut().rev() {
            if step.compensate_key == comp_key && step.status == SagaStepStatus::Completed {
                step.status = SagaStepStatus::Compensated;
                return true;
            }
        }
        false
    }

    /// Plan the LIFO rollback over currently completed steps.
    #[must_use]
    pub fn rollback_plan(&self) -> SagaPlan {
        plan_saga_rollback(&self.steps)
    }

    /// Access the underlying step manifest.
    #[must_use]
    pub fn steps(&self) -> &[SagaStep] {
        &self.steps
    }
}
