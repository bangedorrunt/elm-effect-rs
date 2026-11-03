//! Latched durable signals (`signal`, `await_signal`, `take_signals`): a signal that arrives before the park is never lost.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.

use elm_effect_journal::store::JournalStore;

use crate::engine::{DurableEngine, EngineError, ParkOutcome, RunPhase, WakeOutcome};
use crate::runtime::OurEngine;
use crate::storage::EngineStorage;

/// Latched row type, re-exported beside the verbs that carry it.
pub use crate::storage::SignalRow;
/// Storage conformance for signals, re-exported until the integration
/// bead (§8) adds the crate-root re-export.
pub use crate::storage::signal_storage_laws;

/// Outcome of [`OurEngine::signal`](crate::OurEngine::signal).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SignalOutcome {
    /// The signal latched; when the run was parked it also woken.
    Latched {
        /// The wake when the run was parked, `None` when it was running.
        wake: Option<WakeOutcome>,
    },
    /// `(key, signal_id)` already latched, consumed or not; nothing changed.
    Duplicate,
}

/// Outcome of [`OurEngine::await_signal`](crate::OurEngine::await_signal).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AwaitOutcome {
    /// Latched signals, consumed in `seq` order; never parked.
    Signalled(Vec<SignalRow>),
    /// Nothing latched: the park decision, unchanged.
    Parked(ParkOutcome),
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Latch a signal for a live run, waking it when it is parked.
    ///
    /// [`require_live`](crate::runtime::OurEngine) runs first, so absent,
    /// terminal, and uncertain runs are typed errors and latch nothing. A
    /// duplicate `signal_id` is [`Duplicate`](SignalOutcome::Duplicate)
    /// before and after consumption. A signal is always latched, even when
    /// it also wakes: the resumed run reads its payload through
    /// [`take_signals`](OurEngine::take_signals), never through the wake.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the run is absent, terminal, or
    /// uncertain, or when the store cannot read or write.
    pub fn signal(
        &mut self,
        key: &str,
        signal_id: &str,
        payload: &str,
        now_ms: u64,
    ) -> Result<SignalOutcome, EngineError> {
        let row = self.require_live(key)?;
        if !self.storage.put_signal(key, signal_id, payload)? {
            return Ok(SignalOutcome::Duplicate);
        }
        if row.phase == RunPhase::Parked {
            let wake = self.wake_checked_impl(key, now_ms)?;
            Ok(SignalOutcome::Latched { wake: Some(wake) })
        } else {
            Ok(SignalOutcome::Latched { wake: None })
        }
    }

    /// Return latched signals, or park until `until_ms` when none wait.
    ///
    /// When pending signals exist they are consumed and returned as
    /// [`Signalled`](AwaitOutcome::Signalled) without parking (the
    /// lost-wakeup case: a signal that beat the park is never lost).
    /// Otherwise the run parks exactly as
    /// [`park_until`](DurableEngine::park_until) decides; after a `Wait`
    /// commit the pending set is re-read, and a signal that landed in the
    /// gap wakes the run and returns `Signalled` instead. `ResumeNow` and
    /// `RefusedOverCeiling` pass through as
    /// [`Parked`](AwaitOutcome::Parked).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the run is absent, terminal, or
    /// uncertain, or when the store cannot read or write.
    pub fn await_signal(
        &mut self,
        key: &str,
        until_ms: u64,
        max_wait_secs: u64,
        now_ms: u64,
        ceiling_secs: Option<u64>,
    ) -> Result<AwaitOutcome, EngineError> {
        self.require_live(key)?;
        if !self.storage.pending_signals(key)?.is_empty() {
            let rows = self.storage.take_signals(key)?;
            return Ok(AwaitOutcome::Signalled(rows));
        }
        match self.park_until(key, until_ms, max_wait_secs, now_ms, ceiling_secs)? {
            wait @ ParkOutcome::Wait { .. } => {
                if self.storage.pending_signals(key)?.is_empty() {
                    Ok(AwaitOutcome::Parked(wait))
                } else {
                    let _ = self.wake_checked_impl(key, now_ms)?;
                    let rows = self.storage.take_signals(key)?;
                    Ok(AwaitOutcome::Signalled(rows))
                }
            }
            other => Ok(AwaitOutcome::Parked(other)),
        }
    }

    /// Consume latched signals without parking. The resumed run reads its
    /// payload after `Woke` through this verb and maps it into its own `Msg`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the run is absent, terminal, or
    /// uncertain, or when the store cannot read or write.
    pub fn take_signals(&mut self, key: &str) -> Result<Vec<SignalRow>, EngineError> {
        self.require_live(key)?;
        Ok(self.storage.take_signals(key)?)
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
) -> Result<(), String> {
    use crate::engine::{StartOptions, StartOutcome};
    match engine
        .start_with(key, 0, &StartOptions::default())
        .map_err(law_fail)?
    {
        StartOutcome::Started { .. } => Ok(()),
        StartOutcome::Already { status } => Err(format!("fresh key {key} must start: {status:?}")),
    }
}

/// Conformance laws for latched signals. A sibling runs these against its
/// own engine; each law uses a fresh key so laws never interfere.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
pub fn signal_laws<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("signal before park never parks", {
            (|| -> Result<(), String> {
                law_started(engine, "law-sig-early")?;
                match engine
                    .signal("law-sig-early", "s1", r#"{"v":1}"#, 0)
                    .map_err(law_fail)?
                {
                    SignalOutcome::Latched { wake } => {
                        law_check(wake.is_none(), "running run latches without waking")?
                    }
                    SignalOutcome::Duplicate => {
                        return Err("first signal must latch".into());
                    }
                }
                match engine
                    .await_signal("law-sig-early", 10_000, 60, 0, None)
                    .map_err(law_fail)?
                {
                    AwaitOutcome::Signalled(rows) => {
                        law_check(rows.len() == 1, "one latched row")?;
                        law_check(rows[0].payload == r#"{"v":1}"#, "payload survives")?;
                    }
                    AwaitOutcome::Parked(_) => return Err("latched signal must not park".into()),
                }
                law_check(
                    engine.query("law-sig-early").map_err(law_fail)?.phase == RunPhase::Running,
                    "the run never parked",
                )?;
                law_check(
                    engine.parked().map_err(law_fail)?.is_empty(),
                    "no park row beside the run",
                )
            })()
        }),
        ("signal while parked wakes and latches", {
            (|| -> Result<(), String> {
                law_started(engine, "law-sig-parked")?;
                let parked = engine
                    .park_until("law-sig-parked", 10_000, 60, 0, None)
                    .map_err(law_fail)?;
                law_check(
                    matches!(parked, ParkOutcome::Wait { .. }),
                    "control park waits",
                )?;
                match engine
                    .signal("law-sig-parked", "s1", "ping", 1_000)
                    .map_err(law_fail)?
                {
                    SignalOutcome::Latched { wake } => match wake {
                        Some(WakeOutcome::Woken { overdue_ms, .. }) => {
                            law_check(overdue_ms == 0, "early wake has no overdue")?
                        }
                        other => return Err(format!("parked signal must wake: {other:?}")),
                    },
                    SignalOutcome::Duplicate => return Err("first signal must latch".into()),
                }
                law_check(
                    engine.query("law-sig-parked").map_err(law_fail)?.phase == RunPhase::Running,
                    "the run resumed",
                )?;
                let rows = engine.take_signals("law-sig-parked").map_err(law_fail)?;
                law_check(
                    rows.len() == 1 && rows[0].payload == "ping",
                    "the wake kept its latched payload",
                )
            })()
        }),
        ("duplicate id stays duplicate after consumption", {
            (|| -> Result<(), String> {
                law_started(engine, "law-sig-dup")?;
                law_check(
                    matches!(
                        engine
                            .signal("law-sig-dup", "s1", "one", 0)
                            .map_err(law_fail)?,
                        SignalOutcome::Latched { .. }
                    ),
                    "first signal latches",
                )?;
                law_check(
                    matches!(
                        engine
                            .signal("law-sig-dup", "s1", "two", 0)
                            .map_err(law_fail)?,
                        SignalOutcome::Duplicate
                    ),
                    "duplicate before consumption latches nothing",
                )?;
                let rows = engine.take_signals("law-sig-dup").map_err(law_fail)?;
                law_check(
                    rows.len() == 1 && rows[0].payload == "one",
                    "the first payload wins",
                )?;
                law_check(
                    matches!(
                        engine
                            .signal("law-sig-dup", "s1", "three", 0)
                            .map_err(law_fail)?,
                        SignalOutcome::Duplicate
                    ),
                    "duplicate after consumption never re-latches",
                )
            })()
        }),
        ("take reads seq order", {
            (|| -> Result<(), String> {
                law_started(engine, "law-sig-order")?;
                for id in ["c", "a", "b"] {
                    engine
                        .signal("law-sig-order", id, id, 0)
                        .map_err(law_fail)?;
                }
                let rows = engine.take_signals("law-sig-order").map_err(law_fail)?;
                law_check(
                    rows.iter()
                        .map(|row| row.signal_id.as_str())
                        .collect::<Vec<_>>()
                        == ["c", "a", "b"],
                    "take reads latch (seq) order",
                )
            })()
        }),
        ("terminal run refuses and clears", {
            (|| -> Result<(), String> {
                law_started(engine, "law-sig-terminal")?;
                engine
                    .signal("law-sig-terminal", "s1", "{}", 0)
                    .map_err(law_fail)?;
                let generation = engine
                    .query("law-sig-terminal")
                    .map_err(law_fail)?
                    .generation;
                engine
                    .settle(
                        "law-sig-terminal",
                        generation,
                        elm_effect_journal::EffectState::Settled,
                    )
                    .map_err(law_fail)?;
                match engine.signal("law-sig-terminal", "s2", "{}", 0) {
                    Err(EngineError::Terminal { .. }) => {}
                    other => return Err(format!("terminal signal must refuse: {other:?}")),
                }
                law_check(
                    engine
                        .storage
                        .pending_signals("law-sig-terminal")
                        .map_err(law_fail)?
                        .is_empty(),
                    "settle clears latched signals",
                )
            })()
        }),
        ("absent and uncertain runs refuse", {
            (|| -> Result<(), String> {
                match engine.signal("law-sig-nope", "s1", "{}", 0) {
                    Err(EngineError::NotFound(key)) => {
                        law_check(key == "law-sig-nope", "NotFound names the key")?
                    }
                    other => return Err(format!("absent signal must be NotFound: {other:?}")),
                }
                law_started(engine, "law-sig-uncertain")?;
                engine.mark_unknown("law-sig-uncertain").map_err(law_fail)?;
                match engine.signal("law-sig-uncertain", "s1", "{}", 0) {
                    Err(EngineError::Uncertain { .. }) => {}
                    other => return Err(format!("uncertain signal must refuse: {other:?}")),
                }
                match engine.await_signal("law-sig-uncertain", 10_000, 60, 0, None) {
                    Err(EngineError::Uncertain { .. }) => {}
                    other => return Err(format!("uncertain await must refuse: {other:?}")),
                }
                Ok(())
            })()
        }),
    ]
}
