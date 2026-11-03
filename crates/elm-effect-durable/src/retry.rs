//! Retry backoff through park (`retry_after`): count the attempt, then wait.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.

use elm_effect_journal::retry::{BackoffPolicy, RetryPlan, plan_retry};
use elm_effect_journal::store::{JournalError, JournalStore, Row};

use crate::engine::{DurableEngine, EngineError, ParkOutcome};
use crate::runtime::OurEngine;
use crate::storage::EngineStorage;

/// Outcome of [`OurEngine::retry_after`](crate::OurEngine::retry_after).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RetryOutcome {
    /// The attempt counted and the run parks to `until_ms`.
    Parked {
        /// Attempts recorded (the count that earned this wait).
        attempt: u32,
        /// Absolute re-drive instant in ms.
        until_ms: u64,
        /// The park that now holds the run.
        park: ParkOutcome,
    },
    /// The journal already dead-lettered this count; the run projects
    /// terminal and any park is cleared.
    DeadLettered {
        /// The journal row after the exhausting attempt.
        row: Row,
    },
    /// The fence moved (or never existed); nothing was counted.
    Stale,
    /// The planned wait exceeds the ceiling; the attempt still counted.
    Refused {
        /// Asked `max_wait_secs`.
        asked_secs: u64,
        /// Ceiling that refused it.
        ceiling_secs: u64,
    },
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Count one provider attempt, then park to the backoff instant.
    ///
    /// The attempt is counted before the park commits. A crash between the
    /// two loses one backoff delay (the run retries immediately) and never
    /// double-dispatches or loses the attempt count. This ordering is a
    /// guarantee, not atomicity: no transaction spans the two writes.
    ///
    /// A fenced attempt (`None`) is [`Stale`](RetryOutcome::Stale). A
    /// terminal row is [`DeadLettered`](RetryOutcome::DeadLettered) — the
    /// journal already dead-lettered it and the projection clears parks. An
    /// over-ceiling wait is [`Refused`](RetryOutcome::Refused) with the
    /// attempt still counted.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the journal or the store cannot
    /// read or write.
    pub fn retry_after(
        &mut self,
        key: &str,
        generation: u64,
        policy: &BackoffPolicy,
        max_wait_secs: u64,
        now_ms: u64,
        ceiling_secs: Option<u64>,
    ) -> Result<RetryOutcome, EngineError> {
        let Some(row) = self.attempt_impl(key, generation)? else {
            return Ok(RetryOutcome::Stale);
        };
        if row.state.is_terminal() {
            return Ok(RetryOutcome::DeadLettered { row });
        }
        let RetryPlan::Backoff {
            attempt, until_ms, ..
        } = plan_retry(row.attempts, row.max_attempts, policy, now_ms)
        else {
            // Unreachable while the planner and the journal share the
            // exhaustion edge (pinned by test); a disagreement is corruption.
            return Err(EngineError::Journal(JournalError::Corrupt(
                "retry planner exhausted a live attempt".into(),
            )));
        };
        let park = self.park_until(key, until_ms, max_wait_secs, now_ms, ceiling_secs)?;
        match park {
            ParkOutcome::RefusedOverCeiling {
                asked_secs,
                ceiling_secs,
            } => Ok(RetryOutcome::Refused {
                asked_secs,
                ceiling_secs,
            }),
            wait => Ok(RetryOutcome::Parked {
                attempt,
                until_ms,
                park: wait,
            }),
        }
    }
}

fn law_fail(got: impl std::fmt::Debug) -> String {
    format!("{got:?}")
}

fn law_check(cond: bool, what: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.to_string()) }
}

fn law_started<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
    key: &str,
    max_attempts: u32,
) -> Result<u64, String> {
    use crate::engine::{StartOptions, StartOutcome};
    let opts = StartOptions {
        max_attempts,
        ..StartOptions::default()
    };
    match engine.start_with(key, 0, &opts).map_err(law_fail)? {
        StartOutcome::Started { generation } => Ok(generation),
        StartOutcome::Already { status } => Err(format!("fresh key {key} must start: {status:?}")),
    }
}

/// Conformance laws for the retry verb. A sibling runs these against its
/// own engine; each law uses a fresh key so laws never interfere.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
pub fn retry_laws<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("backoff parks to the planned instant", {
            (|| -> Result<(), String> {
                use elm_effect_journal::retry::BackoffPolicy;
                let generation = law_started(engine, "law-rt-park", 8)?;
                match engine
                    .retry_after(
                        "law-rt-park",
                        generation,
                        &BackoffPolicy::Fixed { delay_ms: 500 },
                        60,
                        0,
                        None,
                    )
                    .map_err(law_fail)?
                {
                    RetryOutcome::Parked {
                        attempt,
                        until_ms,
                        park,
                    } => {
                        law_check(attempt == 1, "first failure counts one")?;
                        law_check(until_ms == 500, "park lands on the plan")?;
                        law_check(
                            matches!(park, ParkOutcome::Wait { .. }),
                            "a future instant waits",
                        )
                    }
                    other => Err(format!("first failure must park: {other:?}")),
                }
            })()
        }),
        ("exhaustion dead-letters and clears the park", {
            (|| -> Result<(), String> {
                use elm_effect_journal::EffectState;
                use elm_effect_journal::retry::BackoffPolicy;
                let generation = law_started(engine, "law-rt-dead", 0)?;
                let _ = engine
                    .park_until("law-rt-dead", 5_000, 60, 0, None)
                    .map_err(law_fail)?;
                match engine
                    .retry_after(
                        "law-rt-dead",
                        generation,
                        &BackoffPolicy::Fixed { delay_ms: 500 },
                        60,
                        0,
                        None,
                    )
                    .map_err(law_fail)?
                {
                    RetryOutcome::DeadLettered { row } => {
                        law_check(row.state == EffectState::Failed, "journal dead-letters")?;
                        law_check(
                            engine
                                .parked()
                                .map_err(law_fail)?
                                .iter()
                                .all(|park| park.key != "law-rt-dead"),
                            "dead-letter clears the park",
                        )
                    }
                    other => Err(format!("exhaustion must dead-letter: {other:?}")),
                }
            })()
        }),
        ("over-ceiling refuses but counts the attempt", {
            (|| -> Result<(), String> {
                use elm_effect_journal::retry::BackoffPolicy;
                let generation = law_started(engine, "law-rt-ceil", 8)?;
                match engine
                    .retry_after(
                        "law-rt-ceil",
                        generation,
                        &BackoffPolicy::Fixed { delay_ms: 500 },
                        200_000,
                        0,
                        Some(86_400),
                    )
                    .map_err(law_fail)?
                {
                    RetryOutcome::Refused {
                        asked_secs,
                        ceiling_secs,
                    } => {
                        law_check(asked_secs == 200_000, "the ask is named")?;
                        law_check(ceiling_secs == 86_400, "the ceiling is named")?;
                        let row = engine
                            .journal()
                            .get("law-rt-ceil")
                            .map_err(law_fail)?
                            .ok_or("refused attempt still writes its count")?;
                        law_check(row.attempts == 1, "the refused attempt counts")
                    }
                    other => Err(format!("over-ceiling must refuse: {other:?}")),
                }
            })()
        }),
        ("stale fence is Stale and projects", {
            (|| -> Result<(), String> {
                use elm_effect_journal::retry::BackoffPolicy;
                let generation = law_started(engine, "law-rt-stale", 8)?;
                let claimed = engine
                    .claim("law-rt-stale", "worker", 10_000, 0, generation)
                    .map_err(law_fail)?
                    .ok_or("live claim must return a row")?;
                match engine
                    .retry_after(
                        "law-rt-stale",
                        generation,
                        &BackoffPolicy::Fixed { delay_ms: 500 },
                        60,
                        0,
                        None,
                    )
                    .map_err(law_fail)?
                {
                    RetryOutcome::Stale => law_check(
                        engine.query("law-rt-stale").map_err(law_fail)?.generation
                            == claimed.generation,
                        "storage carries the live fence",
                    ),
                    other => Err(format!("zombie fence must be Stale: {other:?}")),
                }
            })()
        }),
    ]
}
