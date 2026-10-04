//! `ExecutionPlan` port (`ExecutionPlan.ts` at the pin): the plan as a value.
//!
//! A plan states how many attempts an operation may take before it counts
//! as failed. [`ExecutionPlan::run`] drives a fallible operation, counting
//! attempts and stopping early on interruption: attempt telemetry the tag
//! emits as events is returned here as the attempt count.
//!
//! Residuals (stream/layer runtime surface, out of timebox): per-step
//! fallback layers, partial-stream metadata, `CurrentMetadata` context,
//! observer hooks, and the merge-step combinator.

/// Attempt budget. Zero attempts is rejected: an operation that may not run
/// cannot be planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionPlan {
    max_attempts: usize,
}

/// Too few attempts to plan anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoAttempts;

/// Outcome plus the attempts spent getting there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned<T, E> {
    /// Operation outcome.
    pub outcome: Result<T, E>,
    /// Attempts spent (1-based count of tries).
    pub attempts: usize,
    /// Whether an interruption stopped the plan early.
    pub interrupted: bool,
}

impl ExecutionPlan {
    /// Plan with at most `max_attempts` tries. `Err` on zero.
    pub fn new(max_attempts: usize) -> Result<Self, NoAttempts> {
        if max_attempts == 0 {
            return Err(NoAttempts);
        }
        Ok(Self { max_attempts })
    }

    /// Attempt budget.
    pub fn max_attempts(&self) -> usize {
        self.max_attempts
    }

    /// Drive `operation`, passing the 1-based attempt number and an
    /// interruption probe. The first attempt always runs; afterwards the
    /// plan stops retrying when `interrupted()` turns true (keeping the
    /// last outcome) or the budget is spent. Stops on the first `Ok`.
    pub fn run<T, E>(
        &self,
        interrupted: &dyn Fn() -> bool,
        mut operation: impl FnMut(usize, &dyn Fn() -> bool) -> Result<T, E>,
    ) -> Planned<T, E> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            match operation(attempts, interrupted) {
                ok @ Ok(_) => {
                    return Planned {
                        outcome: ok,
                        attempts,
                        interrupted: false,
                    };
                }
                Err(e) => {
                    if attempts >= self.max_attempts || interrupted() {
                        return Planned {
                            outcome: Err(e),
                            attempts,
                            interrupted: interrupted(),
                        };
                    }
                }
            }
        }
    }
}
