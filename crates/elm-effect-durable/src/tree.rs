//! Structured child cancellation (`plan_cancel_children`, `cancel_children`): cancel every non-terminal child of a run.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.

use elm_effect_journal::store::JournalStore;

use crate::OurEngine;
use crate::engine::{DurableEngine, EngineError, StartOutcome};
use crate::storage::EngineStorage;

/// Outcome of [`OurEngine::cancel_children`], in input order.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CancelReport {
    /// Keys newly cancelled by this call.
    pub cancelled: Vec<String>,
    /// Keys already terminal (a `Terminal` race lands here, never an error).
    pub already_terminal: Vec<String>,
    /// Keys with no run row.
    pub missing: Vec<String>,
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Cancel every non-terminal child in `keys`.
    ///
    /// `NotFound` lands in `missing`; an already-terminal run (or a
    /// `Terminal` race inside [`cancel`](DurableEngine::cancel)) lands in
    /// `already_terminal`. Any `Storage` / `Journal` error returns at once;
    /// a re-run is idempotent (cancelled stays cancelled, terminal stays
    /// terminal, missing stays missing).
    pub fn cancel_children(
        &mut self,
        keys: &[String],
        now_ms: u64,
    ) -> Result<CancelReport, EngineError> {
        let mut report = CancelReport::default();
        for key in keys {
            let phase = match self.query(key) {
                Ok(status) => status.phase,
                Err(EngineError::NotFound(_)) => {
                    report.missing.push(key.clone());
                    continue;
                }
                Err(other) => return Err(other),
            };
            if phase.is_terminal() {
                report.already_terminal.push(key.clone());
                continue;
            }
            match self.cancel(key, now_ms) {
                Ok(()) => report.cancelled.push(key.clone()),
                Err(EngineError::Terminal { .. }) => {
                    report.already_terminal.push(key.clone());
                }
                Err(other) => return Err(other),
            }
        }
        Ok(report)
    }
}

fn law_fail(got: impl std::fmt::Debug) -> String {
    format!("{got:?}")
}

fn law_check(cond: bool, what: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.to_string()) }
}

/// Conformance laws for [`OurEngine::cancel_children`].
///
/// Each law uses fresh keys so laws never interfere. Returns
/// `(name, outcome)` per law; a host fails the suite when any outcome is
/// `Err`.
pub fn cancel_tree_laws<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("cancels mixed phases", law_mixed_phases(engine)),
        ("parked child clears its park", law_parked_clears(engine)),
        ("unknown child cancels", law_unknown_cancels(engine)),
        ("re-run is a no-op", law_rerun_noop(engine)),
        (
            "pairs with plan_join fail-fast",
            law_pairs_with_join(engine),
        ),
        ("ordering follows input", law_ordering(engine)),
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

fn law_mixed_phases<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    use crate::engine::RunPhase;
    use elm_effect_journal::EffectState;
    started_generation(engine, "ct-run")?;
    started_generation(engine, "ct-park")?;
    engine
        .park_until("ct-park", 1_000, 60, 0, None)
        .map_err(law_fail)?;
    started_generation(engine, "ct-unk")?;
    engine.mark_unknown("ct-unk").map_err(law_fail)?;
    let fence_ok = started_generation(engine, "ct-ok")?;
    let claimed_ok = engine
        .claim("ct-ok", "w", 10_000, 0, fence_ok)
        .map_err(law_fail)?
        .ok_or("live claim must return a row")?;
    engine
        .settle("ct-ok", claimed_ok.generation, EffectState::Settled)
        .map_err(law_fail)?;
    let fence_bad = started_generation(engine, "ct-bad")?;
    let claimed_bad = engine
        .claim("ct-bad", "w", 10_000, 0, fence_bad)
        .map_err(law_fail)?
        .ok_or("live claim must return a row")?;
    engine
        .settle("ct-bad", claimed_bad.generation, EffectState::Failed)
        .map_err(law_fail)?;
    started_generation(engine, "ct-gone")?;
    engine.cancel("ct-gone", 0).map_err(law_fail)?;

    let report = engine
        .cancel_children(
            &[
                "ct-run".into(),
                "ct-park".into(),
                "ct-unk".into(),
                "ct-ok".into(),
                "ct-bad".into(),
                "ct-gone".into(),
                "ct-absent".into(),
            ],
            0,
        )
        .map_err(law_fail)?;
    law_check(
        report.cancelled
            == vec![
                "ct-run".to_string(),
                "ct-park".to_string(),
                "ct-unk".to_string(),
            ],
        &format!("live children cancel: {:?}", report.cancelled),
    )?;
    law_check(
        report.already_terminal
            == vec![
                "ct-ok".to_string(),
                "ct-bad".to_string(),
                "ct-gone".to_string(),
            ],
        &format!("terminal children listed: {:?}", report.already_terminal),
    )?;
    law_check(
        report.missing == vec!["ct-absent".to_string()],
        "absent child is missing",
    )?;
    law_check(
        engine.query("ct-run").map_err(law_fail)?.phase == RunPhase::Cancelled,
        "running child now cancelled",
    )
}

fn law_parked_clears<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    use crate::engine::RunPhase;
    started_generation(engine, "ct-parked")?;
    engine
        .park_until("ct-parked", 1_000, 60, 0, None)
        .map_err(law_fail)?;
    law_check(
        engine.query("ct-parked").map_err(law_fail)?.phase == RunPhase::Parked,
        "child parks first",
    )?;
    let report = engine
        .cancel_children(&["ct-parked".to_string()], 0)
        .map_err(law_fail)?;
    law_check(
        report.cancelled == vec!["ct-parked".to_string()],
        "parked child cancels",
    )?;
    law_check(
        engine.query("ct-parked").map_err(law_fail)?.phase == RunPhase::Cancelled,
        "parked child now cancelled",
    )?;
    law_check(
        engine.parked().map_err(law_fail)?.is_empty(),
        "cancel clears the park",
    )
}

fn law_unknown_cancels<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    use crate::engine::RunPhase;
    started_generation(engine, "ct-unknown")?;
    engine.mark_unknown("ct-unknown").map_err(law_fail)?;
    let report = engine
        .cancel_children(&["ct-unknown".to_string()], 0)
        .map_err(law_fail)?;
    law_check(
        report.cancelled == vec!["ct-unknown".to_string()],
        "unknown child cancels",
    )?;
    law_check(
        engine.query("ct-unknown").map_err(law_fail)?.phase == RunPhase::Cancelled,
        "unknown child now cancelled",
    )
}

fn law_rerun_noop<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    started_generation(engine, "ct-rerun")?;
    let first = engine
        .cancel_children(&["ct-rerun".to_string(), "ct-re-gone".to_string()], 0)
        .map_err(law_fail)?;
    law_check(
        first.cancelled == vec!["ct-rerun".to_string()],
        "first run cancels",
    )?;
    law_check(
        first.missing == vec!["ct-re-gone".to_string()],
        "absent is missing",
    )?;
    let second = engine
        .cancel_children(&["ct-rerun".to_string(), "ct-re-gone".to_string()], 0)
        .map_err(law_fail)?;
    law_check(second.cancelled.is_empty(), "re-run cancels nothing")?;
    law_check(
        second.already_terminal == vec!["ct-rerun".to_string()],
        "cancelled child is already terminal",
    )?;
    law_check(
        second.missing == vec!["ct-re-gone".to_string()],
        "missing stays missing",
    )
}

fn law_pairs_with_join<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    use crate::engine::RunPhase;
    use crate::scheduler::{JoinOutcome, plan_cancel_children, plan_join};
    use elm_effect_journal::EffectState;
    started_generation(engine, "ct-j-run")?;
    let fence_bad = started_generation(engine, "ct-j-bad")?;
    let claimed = engine
        .claim("ct-j-bad", "w", 10_000, 0, fence_bad)
        .map_err(law_fail)?
        .ok_or("live claim must return a row")?;
    engine
        .settle("ct-j-bad", claimed.generation, EffectState::Failed)
        .map_err(law_fail)?;
    let phases = ["ct-j-run", "ct-j-bad"]
        .iter()
        .map(|k| {
            engine
                .query(k)
                .map(|s| (k.to_string(), s.phase))
                .map_err(law_fail)
        })
        .collect::<Result<Vec<(String, RunPhase)>, String>>()?;
    match plan_join(&phases) {
        JoinOutcome::Failed { .. } => {}
        other => return Err(format!("fail-fast must surface Failed, got {other:?}")),
    }
    // The still-pending sibling is exactly ct-j-run.
    let pending = plan_cancel_children(&phases);
    law_check(
        pending == vec!["ct-j-run".to_string()],
        "only the live sibling pends",
    )?;
    let report = engine.cancel_children(&pending, 0).map_err(law_fail)?;
    law_check(
        report.cancelled == vec!["ct-j-run".to_string()],
        "pending sibling cancels",
    )
}

fn law_ordering<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Result<(), String> {
    for key in ["ct-o1", "ct-o2", "ct-o3"] {
        started_generation(engine, key)?;
    }
    let report = engine
        .cancel_children(
            &[
                "ct-o3".to_string(),
                "ct-o1".to_string(),
                "ct-o2".to_string(),
            ],
            0,
        )
        .map_err(law_fail)?;
    law_check(
        report.cancelled
            == vec![
                "ct-o3".to_string(),
                "ct-o1".to_string(),
                "ct-o2".to_string(),
            ],
        &format!("input order kept: {:?}", report.cancelled),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::RunPhase;
    use crate::scheduler::plan_cancel_children;
    use elm_effect::{ClockImpl, TestClock};
    use elm_effect_journal::EffectState;

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pure_planner_keeps_slice_order_and_skips_terminal() {
        let children = vec![
            ("a".to_string(), RunPhase::Running),
            ("b".to_string(), RunPhase::Settled),
            ("c".to_string(), RunPhase::Parked),
            ("d".to_string(), RunPhase::Failed),
            ("e".to_string(), RunPhase::Unknown),
            ("f".to_string(), RunPhase::Cancelled),
        ];
        assert_eq!(
            plan_cancel_children(&children),
            vec!["a".to_string(), "c".to_string(), "e".to_string()]
        );
    }

    #[test]
    fn mixed_phases_report_in_input_order() {
        let clock = TestClock::new();
        let mut eng = OurEngine::new();
        eng.start("kid-run", clock.now_millis()).unwrap();
        eng.start("kid-park", clock.now_millis()).unwrap();
        eng.park_until("kid-park", 1_000, 60, clock.now_millis(), None)
            .unwrap();
        eng.start("kid-unk", clock.now_millis()).unwrap();
        eng.mark_unknown("kid-unk").unwrap();
        eng.start("kid-ok", clock.now_millis()).unwrap();
        let fence = eng
            .claim("kid-ok", "w", 10_000, 0, 1)
            .unwrap()
            .unwrap()
            .generation;
        eng.settle("kid-ok", fence, EffectState::Settled).unwrap();
        eng.start("kid-gone", clock.now_millis()).unwrap();
        eng.cancel("kid-gone", clock.now_millis()).unwrap();

        let report = eng
            .cancel_children(
                &keys(&[
                    "kid-run",
                    "kid-park",
                    "kid-unk",
                    "kid-ok",
                    "kid-gone",
                    "kid-absent",
                ]),
                clock.now_millis(),
            )
            .unwrap();
        assert_eq!(report.cancelled, keys(&["kid-run", "kid-park", "kid-unk"]));
        assert_eq!(report.already_terminal, keys(&["kid-ok", "kid-gone"]));
        assert_eq!(report.missing, keys(&["kid-absent"]));
        assert_eq!(eng.query("kid-run").unwrap().phase, RunPhase::Cancelled);
        // Park cleared on the parked cancel.
        assert!(eng.parked().unwrap().is_empty());
    }

    #[test]
    fn rerun_is_a_noop() {
        let mut eng = OurEngine::new();
        eng.start("re", 0).unwrap();
        let first = eng.cancel_children(&keys(&["re", "gone"]), 0).unwrap();
        assert_eq!(first.cancelled, keys(&["re"]));
        assert_eq!(first.missing, keys(&["gone"]));
        let second = eng.cancel_children(&keys(&["re", "gone"]), 0).unwrap();
        assert!(second.cancelled.is_empty());
        assert_eq!(second.already_terminal, keys(&["re"]));
        assert_eq!(second.missing, keys(&["gone"]));
    }

    #[test]
    fn pairs_with_plan_join_fail_fast() {
        use crate::scheduler::{JoinOutcome, plan_join};
        let mut eng = OurEngine::new();
        eng.start("j-run", 0).unwrap();
        eng.start("j-bad", 0).unwrap();
        let fence = eng
            .claim("j-bad", "w", 10_000, 0, 1)
            .unwrap()
            .unwrap()
            .generation;
        eng.settle("j-bad", fence, EffectState::Failed).unwrap();
        let phases = ["j-run", "j-bad"]
            .iter()
            .map(|k| (k.to_string(), eng.query(k).unwrap().phase))
            .collect::<Vec<_>>();
        assert!(matches!(plan_join(&phases), JoinOutcome::Failed { .. }));
        let pending = plan_cancel_children(&phases);
        assert_eq!(pending, vec!["j-run".to_string()]);
        let report = eng.cancel_children(&pending, 0).unwrap();
        assert_eq!(report.cancelled, vec!["j-run".to_string()]);
    }

    #[test]
    fn cancel_tree_laws_pass_on_mem_engine() {
        let mut eng = OurEngine::new();
        for (name, outcome) in cancel_tree_laws(&mut eng) {
            assert!(outcome.is_ok(), "law {name} failed: {outcome:?}");
        }
    }

    #[test]
    fn faulty_store_fails_clean_then_retry_succeeds() {
        use crate::engine::StorageError;
        use crate::storage::{EngineStorage, MemStorage, RunRow};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone)]
        struct FaultyStore {
            inner: MemStorage,
            fail: Arc<AtomicBool>,
        }
        impl EngineStorage for FaultyStore {
            fn get_run(&self, key: &str) -> Result<Option<RunRow>, StorageError> {
                if self.fail.load(Ordering::SeqCst) {
                    return Err(StorageError::Unavailable("boom".into()));
                }
                self.inner.get_run(key)
            }
            fn put_run(&mut self, row: RunRow) -> Result<(), StorageError> {
                self.inner.put_run(row)
            }
            fn list_parked(&self) -> Result<Vec<crate::storage::ParkedRow>, StorageError> {
                self.inner.list_parked()
            }
            fn upsert_park(
                &mut self,
                row: crate::storage::ParkedRow,
            ) -> Result<bool, StorageError> {
                self.inner.upsert_park(row)
            }
            fn clear_parks(&mut self, key: &str) -> Result<(), StorageError> {
                self.inner.clear_parks(key)
            }
        }

        let fail = Arc::new(AtomicBool::new(false));
        let mut eng = OurEngine::with_storage(FaultyStore {
            inner: MemStorage::new(),
            fail: fail.clone(),
        });
        eng.start("fk", 0).unwrap();
        fail.store(true, Ordering::SeqCst);
        let err = eng.cancel_children(&keys(&["fk"]), 0).unwrap_err();
        assert!(matches!(err, EngineError::Storage(_)));
        // Nothing changed: still Running.
        fail.store(false, Ordering::SeqCst);
        assert_eq!(eng.query("fk").unwrap().phase, RunPhase::Running);
        let report = eng.cancel_children(&keys(&["fk"]), 0).unwrap();
        assert_eq!(report.cancelled, keys(&["fk"]));
    }
}
