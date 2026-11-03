//! Replay-digest guarded start (`start_guarded`): a changed re-render of a run is a typed mismatch, never a second run.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.

use elm_effect_journal::plan::ReplayMismatch;
use elm_effect_journal::store::JournalStore;

use crate::engine::{DurableEngine, EngineError, RunStatus, StartOptions, StartOutcome};
use crate::runtime::OurEngine;
use crate::storage::EngineStorage;

/// Storage conformance for digests, re-exported until the integration
/// bead (§8) adds the crate-root re-export.
pub use crate::storage::digest_storage_laws;

/// Outcome of [`OurEngine::start_guarded`](crate::OurEngine::start_guarded).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GuardedStart {
    /// Fresh run; first attempt may dispatch.
    Started {
        /// Fence generation after intent.
        generation: u64,
    },
    /// The digest matches and the key already had a row — no second dispatch.
    Already {
        /// Existing status.
        status: RunStatus,
    },
    /// The digest changed on a live or terminal run: nothing was written
    /// and no key was minted.
    Mismatch(ReplayMismatch),
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Begin or re-attach a run only when its replay digest still matches.
    ///
    /// The stored digest is read first. When it is present and different
    /// **and** a journal row or run row exists, the re-render changed and
    /// the call returns [`Mismatch`](GuardedStart::Mismatch) without
    /// writing anything. When it is present and different but no row
    /// exists, the previous attempt crashed after the digest write and
    /// before the intent, so the digest is overwritten and the start
    /// proceeds. When absent, the digest is recorded **before** starting,
    /// so a crash cannot start an unrecorded run. Equal digests behave
    /// exactly like [`start_with`](DurableEngine::start_with).
    ///
    /// A store without digest support fails loudly with
    /// [`EngineError::Storage`] before anything starts.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the store or journal cannot read or write.
    pub fn start_guarded(
        &mut self,
        key: &str,
        now_ms: u64,
        opts: &StartOptions,
        digest: &str,
    ) -> Result<GuardedStart, EngineError> {
        if let Some(stored) = self.storage.get_digest(key)? {
            if stored != digest {
                let row_exists =
                    self.journal.get(key)?.is_some() || self.storage.get_run(key)?.is_some();
                if row_exists {
                    return Ok(GuardedStart::Mismatch(ReplayMismatch {
                        prepared: stored,
                        redigest: digest.to_string(),
                    }));
                }
                self.storage.set_digest(key, digest)?;
            }
        } else {
            self.storage.set_digest(key, digest)?;
        }
        match self.start_with(key, now_ms, opts)? {
            StartOutcome::Started { generation } => Ok(GuardedStart::Started { generation }),
            StartOutcome::Already { status } => Ok(GuardedStart::Already { status }),
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
    digest: &str,
) -> Result<u64, String> {
    match engine
        .start_guarded(key, 0, &StartOptions::default(), digest)
        .map_err(law_fail)?
    {
        GuardedStart::Started { generation } => Ok(generation),
        other => Err(format!("fresh key {key} must start: {other:?}")),
    }
}

/// Conformance laws for the guarded start. A sibling runs these against
/// its own engine; each law uses a fresh key so laws never interfere.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
pub fn guard_laws<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("first start records the digest", {
            (|| -> Result<(), String> {
                law_started(engine, "law-gd-first", "v1")?;
                // The record proves itself: a changed digest now mismatches
                // and names what was recorded.
                match engine
                    .start_guarded("law-gd-first", 0, &StartOptions::default(), "v2")
                    .map_err(law_fail)?
                {
                    GuardedStart::Mismatch(mismatch) => {
                        law_check(mismatch.prepared == "v1", "the recorded digest is named")?;
                        law_check(mismatch.redigest == "v2", "the new digest is named")
                    }
                    other => Err(format!("changed digest must mismatch: {other:?}")),
                }
            })()
        }),
        ("replay with the same digest re-attaches", {
            (|| -> Result<(), String> {
                law_started(engine, "law-gd-replay", "v1")?;
                match engine
                    .start_guarded("law-gd-replay", 0, &StartOptions::default(), "v1")
                    .map_err(law_fail)?
                {
                    GuardedStart::Already { status } => {
                        law_check(status.key == "law-gd-replay", "re-attach keeps the key")
                    }
                    other => Err(format!("same digest must re-attach: {other:?}")),
                }
            })()
        }),
        ("changed digest on a live run mints no key", {
            (|| -> Result<(), String> {
                let generation = law_started(engine, "law-gd-live", "v1")?;
                let before = engine.query("law-gd-live").map_err(law_fail)?;
                match engine
                    .start_guarded("law-gd-live", 0, &StartOptions::default(), "v2")
                    .map_err(law_fail)?
                {
                    GuardedStart::Mismatch(_) => {}
                    other => return Err(format!("changed digest must mismatch: {other:?}")),
                }
                let after = engine.query("law-gd-live").map_err(law_fail)?;
                law_check(before == after, "a mismatch writes nothing")?;
                law_check(
                    after.generation == generation,
                    "the fence does not move on mismatch",
                )
            })()
        }),
        ("changed digest on a terminal run mismatches", {
            (|| -> Result<(), String> {
                let generation = law_started(engine, "law-gd-terminal", "v1")?;
                engine
                    .settle(
                        "law-gd-terminal",
                        generation,
                        elm_effect_journal::EffectState::Settled,
                    )
                    .map_err(law_fail)?;
                match engine
                    .start_guarded("law-gd-terminal", 0, &StartOptions::default(), "v2")
                    .map_err(law_fail)?
                {
                    GuardedStart::Mismatch(mismatch) => {
                        law_check(mismatch.prepared == "v1", "the recorded digest is named")
                    }
                    other => Err(format!(
                        "terminal rerun with a new digest must mismatch: {other:?}"
                    )),
                }
            })()
        }),
        (
            "equal digest on a terminal run re-attaches like start_with",
            {
                (|| -> Result<(), String> {
                    let generation = law_started(engine, "law-gd-term-same", "v1")?;
                    engine
                        .settle(
                            "law-gd-term-same",
                            generation,
                            elm_effect_journal::EffectState::Settled,
                        )
                        .map_err(law_fail)?;
                    match engine
                        .start_guarded("law-gd-term-same", 0, &StartOptions::default(), "v1")
                        .map_err(law_fail)?
                    {
                        GuardedStart::Already { status } => law_check(
                            status.phase == crate::engine::RunPhase::Settled,
                            "terminal re-attach reports Settled",
                        ),
                        other => Err(format!("same digest must re-attach: {other:?}")),
                    }
                })()
            },
        ),
    ]
}
