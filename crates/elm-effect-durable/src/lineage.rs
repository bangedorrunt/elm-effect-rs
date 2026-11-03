//! Lineage rollover (`continue_as_new`): bounded history for recurring runs.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::{SettleRejection, check_settle};
use elm_effect_journal::store::{JournalStore, Row};

use crate::OurEngine;
use crate::engine::{DurableEngine, EngineError, StartOptions, StartOutcome};
use crate::storage::EngineStorage;

/// Next-epoch key for a recurring run: `"{base}:epoch:{epoch}"`.
#[must_use]
pub fn rollover_key(base: &str, epoch: u64) -> String {
    format!("{base}:epoch:{epoch}")
}

/// Outcome of [`OurEngine::continue_as_new`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum ContinueOutcome {
    /// The previous epoch closed and the next epoch is live (`Started` on a
    /// fresh key, `Already` when the next epoch pre-existed or this is a
    /// re-run completing a crashed rollover).
    Continued {
        /// Start outcome for the next epoch.
        started: StartOutcome,
        /// Settled journal row that closed the previous epoch.
        closed: Row,
    },
    /// The previous epoch refused the close; the next epoch was not started.
    Rejected(SettleRejection),
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Roll a recurring run into its next epoch.
    ///
    /// Order is the safety rule: (1) fence-check `from_key` against the
    /// journal row and refuse `Rejected` before starting anything; (2)
    /// `start_with(to_key)` (idempotent: `Already` is fine); (3) settle
    /// `from_key` to `Settled`. A crash after (2) leaves both epochs live
    /// and a re-run completes (3). A re-run where `from_key` is already
    /// `Settled` at the same generation and `to_key` exists is `Continued`
    /// (idempotent). Never settle first.
    pub fn continue_as_new(
        &mut self,
        from_key: &str,
        generation: u64,
        to_key: &str,
        now_ms: u64,
        opts: &StartOptions,
    ) -> Result<ContinueOutcome, EngineError> {
        // (1) Fence-check before starting anything.
        let current = match self.journal.get(from_key)? {
            Some(row) => row,
            None => return Ok(ContinueOutcome::Rejected(SettleRejection::NotFound)),
        };
        // Idempotent re-run: our own close already landed at this fence and
        // the next epoch exists. Re-attach (idempotent) and report it.
        if current.state == EffectState::Settled && current.generation == generation {
            if self.journal.get(to_key)?.is_some() {
                let started = self.start_with(to_key, now_ms, opts)?;
                return Ok(ContinueOutcome::Continued {
                    started,
                    closed: current,
                });
            }
            // Settled with no next epoch: a foreign close, not our
            // rollover. Name the refusal.
            return Ok(ContinueOutcome::Rejected(SettleRejection::Terminal {
                state: current.state,
            }));
        }
        if let Err(rejection) = check_settle(
            Some((current.state, current.generation)),
            generation,
            EffectState::Settled,
        ) {
            return Ok(ContinueOutcome::Rejected(rejection));
        }
        // (2) Start the next epoch. `Already` (even terminal) is fine: the
        // previous epoch still closes below.
        let started = self.start_with(to_key, now_ms, opts)?;
        // (3) Close the previous epoch.
        match self.settle(from_key, generation, EffectState::Settled)? {
            Some(closed) => Ok(ContinueOutcome::Continued { started, closed }),
            None => {
                // A host journal that refuses outside the pure fence after
                // the check passed; the fence did not move for the caller.
                let actual = self
                    .journal
                    .get(from_key)?
                    .map(|row| row.generation)
                    .unwrap_or(generation);
                Ok(ContinueOutcome::Rejected(
                    SettleRejection::StaleGeneration {
                        expected: generation,
                        actual,
                    },
                ))
            }
        }
    }
}

fn law_fail(got: impl std::fmt::Debug) -> String {
    format!("{got:?}")
}

fn law_check(cond: bool, what: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.to_string()) }
}

/// Conformance laws for [`OurEngine::continue_as_new`].
///
/// Each law uses fresh keys so laws never interfere. Returns
/// `(name, outcome)` per law; a host fails the suite when any outcome is
/// `Err`.
pub fn lineage_laws<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("happy path rolls forward", law_happy_path(engine)),
        ("re-run after close is Continued", law_rerun(engine)),
        (
            "stale fence refuses without starting next",
            law_stale(engine),
        ),
        ("terminal prev refuses", law_terminal(engine)),
        ("terminal next still closes prev", law_next_terminal(engine)),
        ("key helper formats epochs", law_key_format()),
    ]
}

fn started_generation<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
    key: &str,
) -> Result<u64, String> {
    match engine.start(key, 0).map_err(law_fail)? {
        StartOutcome::Started { generation } => Ok(generation),
        StartOutcome::Already { status } => Err(format!("{key} already started: {status:?}")),
    }
}

fn is_continued(outcome: ContinueOutcome) -> Result<(StartOutcome, Row), String> {
    match outcome {
        ContinueOutcome::Continued { started, closed } => Ok((started, closed)),
        ContinueOutcome::Rejected(r) => Err(format!("expected Continued, got Rejected({r:?})")),
    }
}

fn law_happy_path<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    use crate::engine::RunPhase;
    let from = rollover_key("lg-happy", 0);
    let to = rollover_key("lg-happy", 1);
    let fence = started_generation(engine, &from)?;
    let (started, closed) = is_continued(
        engine
            .continue_as_new(&from, fence, &to, 0, &StartOptions::default())
            .map_err(law_fail)?,
    )?;
    law_check(
        matches!(started, StartOutcome::Started { .. }),
        "next epoch starts",
    )?;
    law_check(closed.key == from, "close names the previous epoch")?;
    law_check(
        engine.query(&to).map_err(law_fail)?.phase == RunPhase::Running,
        "next epoch runs",
    )?;
    law_check(
        engine
            .journal()
            .get(&from)
            .map_err(law_fail)?
            .is_some_and(|row| row.state == EffectState::Settled),
        "previous epoch settles in the journal",
    )
}

fn law_rerun<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    let from = rollover_key("lg-rerun", 0);
    let to = rollover_key("lg-rerun", 1);
    let fence = started_generation(engine, &from)?;
    let opts = StartOptions::default();
    is_continued(
        engine
            .continue_as_new(&from, fence, &to, 0, &opts)
            .map_err(law_fail)?,
    )?;
    let (started, closed) = is_continued(
        engine
            .continue_as_new(&from, fence, &to, 0, &opts)
            .map_err(law_fail)?,
    )?;
    law_check(
        matches!(started, StartOutcome::Already { .. }),
        "re-run re-attaches the next epoch",
    )?;
    law_check(
        closed.state == EffectState::Settled,
        "re-run reports the close",
    )
}

fn law_stale<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    let from = rollover_key("lg-stale", 0);
    let to = rollover_key("lg-stale", 1);
    let fence = started_generation(engine, &from)?;
    match engine
        .continue_as_new(
            &from,
            fence.wrapping_add(1),
            &to,
            0,
            &StartOptions::default(),
        )
        .map_err(law_fail)?
    {
        ContinueOutcome::Rejected(SettleRejection::StaleGeneration { .. }) => {}
        other => return Err(format!("zombie fence must refuse stale: {other:?}")),
    }
    law_check(
        engine.journal().get(&to).map_err(law_fail)?.is_none(),
        "a stale fence must not start the next epoch",
    )
}

fn law_terminal<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    let from = rollover_key("lg-term", 0);
    let to = rollover_key("lg-term", 1);
    let fence = started_generation(engine, &from)?;
    let claimed = engine
        .claim(&from, "w", 10_000, 0, fence)
        .map_err(law_fail)?
        .ok_or("live claim must return a row")?;
    engine
        .settle(&from, claimed.generation, EffectState::Failed)
        .map_err(law_fail)?;
    match engine
        .continue_as_new(&from, claimed.generation, &to, 0, &StartOptions::default())
        .map_err(law_fail)?
    {
        ContinueOutcome::Rejected(SettleRejection::Terminal { .. }) => {}
        other => return Err(format!("terminal prev must refuse: {other:?}")),
    }
    law_check(
        engine.journal().get(&to).map_err(law_fail)?.is_none(),
        "a terminal prev must not start the next epoch",
    )
}

fn law_next_terminal<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    use crate::engine::RunPhase;
    let from = rollover_key("lg-next", 0);
    let to = rollover_key("lg-next", 1);
    let fence = started_generation(engine, &from)?;
    let to_fence = started_generation(engine, &to)?;
    let claimed = engine
        .claim(&to, "w", 10_000, 0, to_fence)
        .map_err(law_fail)?
        .ok_or("live claim must return a row")?;
    engine
        .settle(&to, claimed.generation, EffectState::Settled)
        .map_err(law_fail)?;
    let (started, closed) = is_continued(
        engine
            .continue_as_new(&from, fence, &to, 0, &StartOptions::default())
            .map_err(law_fail)?,
    )?;
    law_check(
        matches!(started, StartOutcome::Already { .. }),
        "pre-settled next re-attaches",
    )?;
    law_check(closed.state == EffectState::Settled, "prev still closes")?;
    law_check(
        engine.query(&from).map_err(law_fail)?.phase == RunPhase::Settled,
        "prev projects settled",
    )
}

fn law_key_format() -> Result<(), String> {
    law_check(rollover_key("r", 0) == "r:epoch:0", "epoch 0 formats")?;
    law_check(rollover_key("r", 7) == "r:epoch:7", "epoch 7 formats")
}

#[cfg(test)]
mod tests {
    use super::*;
    use elm_effect::{ClockImpl, TestClock};

    #[test]
    fn key_names_epochs_in_order() {
        assert_eq!(rollover_key("report", 0), "report:epoch:0");
        assert_eq!(rollover_key("report", 1), "report:epoch:1");
        assert_ne!(rollover_key("report", 1), rollover_key("report", 2));
    }

    #[test]
    fn happy_path_through_test_clock() {
        let clock = TestClock::new();
        let mut engine = OurEngine::new();
        let from = rollover_key("u-happy", 0);
        let to = rollover_key("u-happy", 1);
        let fence = match engine.start(&from, clock.now_millis()).expect("start") {
            StartOutcome::Started { generation } => generation,
            StartOutcome::Already { .. } => panic!("fresh key must start"),
        };
        clock.adjust(elm_effect::Duration::from_millis(60_000));
        let (started, closed) = is_continued(
            engine
                .continue_as_new(
                    &from,
                    fence,
                    &to,
                    clock.now_millis(),
                    &StartOptions::default(),
                )
                .expect("rollover"),
        )
        .expect("continued");
        assert!(matches!(started, StartOutcome::Started { .. }));
        use crate::engine::RunPhase;
        assert_eq!(closed.state, EffectState::Settled);
        assert_eq!(engine.query(&from).expect("query").phase, RunPhase::Settled);
        assert_eq!(engine.query(&to).expect("query").phase, RunPhase::Running);
    }

    #[test]
    fn lineage_laws_pass_on_mem_engine() {
        let mut engine = OurEngine::new();
        for (name, outcome) in lineage_laws(&mut engine) {
            assert!(outcome.is_ok(), "law {name} failed: {outcome:?}");
        }
    }
}
