//! An attempt budget as a value.
//!
//! Effect 4's `ExecutionPlan` is a different type: ordered fallback steps,
//! each with a `Context`/`Layer`, attempts, a `while` predicate, a schedule,
//! and `CurrentMetadata`/events. This crate does not port that surface.
//! `AttemptPlan` is the small durable-execution seam kept for commands that
//! need a bounded number of tries before their outcome settles.

/// Attempt budget. Zero attempts is rejected: an operation that may not run
/// cannot be planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptPlan {
    max_attempts: usize,
}

/// Too few attempts to plan anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoAttempts;

impl std::fmt::Display for NoAttempts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttemptPlan: attempts must be greater than 0")
    }
}

impl std::error::Error for NoAttempts {}

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

impl AttemptPlan {
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
                    // Probe once: a side-effecting probe would otherwise be
                    // able to answer differently on the two calls.
                    let stopped = interrupted();
                    if attempts >= self.max_attempts || stopped {
                        return Planned {
                            outcome: Err(e),
                            attempts,
                            interrupted: stopped,
                        };
                    }
                }
            }
        }
    }
}
