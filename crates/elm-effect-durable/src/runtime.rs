//! Our single-host engine (ADR-0005).

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::{
    DispatchPlan, EffectView, ProviderCapability, plan_attempt, plan_park,
};
use elm_effect_journal::plan::{ParkDecision, PendingKey, PendingRow, PendingStore, park_intent};
use elm_effect_journal::store::{InMemoryStore, JournalStore};

use crate::engine::{DurableEngine, EngineError, ParkOutcome, RunPhase, RunStatus, StartOutcome};
use crate::scheduler::due_wakes;
use crate::storage::{EngineStorage, MemStorage, ParkedRow, RunRow};

/// Msg the wake loop feeds back into an Elm fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineMsg {
    /// A parked run is due (or overdue).
    Woke {
        /// Run key.
        key: String,
        /// Overdue gap in ms (0 if exactly on time).
        overdue_ms: u64,
        /// Park generation woken.
        park_generation: u64,
    },
    /// Run was cancelled.
    Cancelled {
        /// Run key.
        key: String,
    },
}

/// Default durable engine: storage + journal fencing + injected clock consumer.
///
/// `now_ms` is always passed in by the caller (or by [`Self::drain_due`]'s
/// argument). Pair with `elm_effect::TestClock` in tests: advance the clock,
/// then pass `clock.current_ms()` (or your own `u64`) into these methods.
pub struct OurEngine<S: EngineStorage = MemStorage> {
    storage: S,
    /// Journal rows for intent / claim / settle fencing (L14).
    journal: InMemoryStore,
    /// Pending park intents (journal pending vocabulary); kept in sync with
    /// [`EngineStorage`] parks for restart recovery via storage.
    pending: PendingStore,
    /// Next park generation per key.
    next_park_gen: std::collections::BTreeMap<String, u64>,
}

impl OurEngine<MemStorage> {
    /// In-memory engine.
    pub fn new() -> Self {
        Self::with_storage(MemStorage::new())
    }
}

impl Default for OurEngine<MemStorage> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: EngineStorage> OurEngine<S> {
    /// Engine over a host store (e.g. a future toron adapter).
    pub fn with_storage(storage: S) -> Self {
        Self {
            storage,
            journal: InMemoryStore::new(1024),
            pending: PendingStore::new(),
            next_park_gen: std::collections::BTreeMap::new(),
        }
    }

    /// Borrow the journal store (conformance / host reconcile).
    pub fn journal(&self) -> &InMemoryStore {
        &self.journal
    }

    /// Settle a journal row toward `to` with a generation fence.
    ///
    /// Stale generations return `None`. `Unknown` has no default exit: the
    /// golden table refuses `Unknown → Settled` without reconcile
    /// (`Unknown → ReconcilePending` first).
    pub fn settle(
        &self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Option<elm_effect_journal::store::Row> {
        self.journal.settle(key, generation, to)
    }

    /// Claim a journal row (bumps generation). Used by conformance and hosts.
    pub fn claim(
        &self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Option<elm_effect_journal::store::Row> {
        self.journal
            .claim(key, owner, lease_deadline, now_ms, expected_generation)
    }

    /// Mark the run Unknown (uncertain outcome). Only reconcile may leave it.
    pub fn mark_unknown(&mut self, key: &str) -> Result<(), EngineError> {
        let mut row = self
            .storage
            .get_run(key)
            .ok_or_else(|| EngineError::NotFound(key.into()))?;
        if row.phase.is_terminal() {
            return Err(EngineError::Terminal {
                key: key.into(),
                phase: row.phase,
            });
        }
        // Drive journal InFlight → Unknown when possible.
        if let Some(j) = self.journal.get(key) {
            let _ = self
                .journal
                .settle(&j.key, j.generation, EffectState::Unknown);
        }
        row.phase = RunPhase::Unknown;
        self.storage.put_run(row);
        Ok(())
    }

    fn status_of(row: &RunRow) -> RunStatus {
        RunStatus {
            key: row.key.clone(),
            phase: row.phase,
            generation: row.generation,
            until_ms: row.until_ms,
        }
    }

    fn require_live(&self, key: &str) -> Result<RunRow, EngineError> {
        let row = self
            .storage
            .get_run(key)
            .ok_or_else(|| EngineError::NotFound(key.into()))?;
        if row.phase.is_terminal() {
            return Err(EngineError::Terminal {
                key: key.into(),
                phase: row.phase,
            });
        }
        Ok(row)
    }
}

impl<S: EngineStorage> DurableEngine for OurEngine<S> {
    fn start(&mut self, key: &str, _now_ms: u64) -> Result<StartOutcome, EngineError> {
        if let Some(existing) = self.storage.get_run(key) {
            // Replay / re-attach: never a second dispatch for the same key.
            let view = EffectView {
                state: match existing.phase {
                    RunPhase::Running => EffectState::InFlight,
                    RunPhase::Parked => EffectState::Parked,
                    RunPhase::Cancelled => EffectState::Cancelled,
                    RunPhase::Settled => EffectState::Settled,
                    RunPhase::Failed => EffectState::Failed,
                    RunPhase::Unknown => EffectState::Unknown,
                },
                attempt: existing.attempt,
                lease_live: false,
            };
            let plan = plan_attempt(
                Some(&view),
                &ProviderCapability {
                    supports_idempotency_key: true,
                    supports_reconcile_query: false,
                },
            );
            match plan {
                DispatchPlan::DuplicateTerminal
                | DispatchPlan::RefuseBlindRetry { .. }
                | DispatchPlan::ResumeParked { .. }
                | DispatchPlan::RetrySameKey { .. }
                | DispatchPlan::ReconcileFirst { .. }
                | DispatchPlan::DispatchNew => {
                    // Even DispatchNew on an existing Prepared-like row: we
                    // already recorded intent; surface Already so the host
                    // does not run the effect twice.
                    return Ok(StartOutcome::Already {
                        status: Self::status_of(&existing),
                    });
                }
            }
        }

        let j = self.journal.record_intent(key, 8);
        let row = RunRow {
            key: key.to_string(),
            phase: RunPhase::Running,
            generation: j.generation,
            until_ms: None,
            attempt: 1,
        };
        self.storage.put_run(row);
        // Move Prepared → InFlight via claim so settle fencing has a generation.
        let claimed = self
            .journal
            .claim(key, "engine", i64::MAX, 0, j.generation)
            .ok_or_else(|| EngineError::Storage("claim after intent failed".into()))?;
        // Claim fences the row but leaves Prepared; dispatch is InFlight.
        let inflight = self
            .journal
            .settle(key, claimed.generation, EffectState::InFlight)
            .ok_or_else(|| EngineError::Storage("Prepared→InFlight failed".into()))?;
        if let Some(mut r) = self.storage.get_run(key) {
            r.generation = inflight.generation;
            self.storage.put_run(r);
        }
        Ok(StartOutcome::Started {
            generation: inflight.generation,
        })
    }

    fn park_until(
        &mut self,
        key: &str,
        until_ms: u64,
        max_wait_secs: u64,
        now_ms: u64,
        ceiling_secs: Option<u64>,
    ) -> Result<ParkOutcome, EngineError> {
        let mut row = self.require_live(key)?;
        match plan_park(until_ms, max_wait_secs, now_ms, ceiling_secs) {
            ParkDecision::RefusedOverCeiling {
                asked_secs,
                ceiling_secs,
            } => Ok(ParkOutcome::RefusedOverCeiling {
                asked_secs,
                ceiling_secs,
            }),
            ParkDecision::ResumeNow { overdue_ms } => {
                row.phase = RunPhase::Running;
                row.until_ms = None;
                self.storage.clear_parks(key);
                self.storage.put_run(row);
                Ok(ParkOutcome::ResumeNow { overdue_ms })
            }
            ParkDecision::Wait { wait_ms, intent } => {
                let park_gen = {
                    let g = self.next_park_gen.entry(key.to_string()).or_insert(0);
                    *g += 1;
                    *g
                };
                let pending_key = PendingKey {
                    run_id: key.to_string(),
                    promise_id: "park".into(),
                    park_generation: park_gen,
                };
                // Same (run, promise, gen) writes once; we also key storage by
                // (key, until_ms) so the same instant is one row even across gens.
                let _ = self.pending.upsert_pending(PendingRow {
                    key: pending_key,
                    intent: park_intent(intent.until_ms, intent.wait_ms),
                });
                let wrote = self.storage.upsert_park(ParkedRow {
                    key: key.to_string(),
                    until_ms: intent.until_ms,
                    park_generation: park_gen,
                });
                if !wrote {
                    // Identical (key, until_ms) already present: keep existing
                    // generation on the stored row; do not bump phase twice.
                    if let Some(existing) = self
                        .storage
                        .list_parked()
                        .into_iter()
                        .find(|p| p.key == key && p.until_ms == intent.until_ms)
                    {
                        row.phase = RunPhase::Parked;
                        row.until_ms = Some(existing.until_ms);
                        self.storage.put_run(row);
                        return Ok(ParkOutcome::Wait {
                            until_ms: intent.until_ms,
                            wait_ms,
                        });
                    }
                }
                row.phase = RunPhase::Parked;
                row.until_ms = Some(intent.until_ms);
                self.storage.put_run(row);
                Ok(ParkOutcome::Wait {
                    until_ms: intent.until_ms,
                    wait_ms,
                })
            }
        }
    }

    fn wake(&mut self, key: &str, now_ms: u64) -> Result<Option<u64>, EngineError> {
        let mut row = self.require_live(key)?;
        if row.phase != RunPhase::Parked {
            // Already running: idempotent wake.
            return Ok(None);
        }
        let until = row.until_ms.unwrap_or(now_ms);
        let overdue = now_ms.saturating_sub(until);
        row.phase = RunPhase::Running;
        row.until_ms = None;
        self.storage.clear_parks(key);
        self.storage.put_run(row);
        Ok(Some(overdue))
    }

    fn cancel(&mut self, key: &str, _now_ms: u64) -> Result<(), EngineError> {
        let mut row = self
            .storage
            .get_run(key)
            .ok_or_else(|| EngineError::NotFound(key.into()))?;
        if row.phase == RunPhase::Cancelled {
            return Ok(());
        }
        if matches!(row.phase, RunPhase::Settled | RunPhase::Failed) {
            return Err(EngineError::Terminal {
                key: key.into(),
                phase: row.phase,
            });
        }
        if let Some(j) = self.journal.get(key) {
            let _ = self
                .journal
                .settle(&j.key, j.generation, EffectState::Cancelled);
        }
        row.phase = RunPhase::Cancelled;
        row.until_ms = None;
        self.storage.clear_parks(key);
        self.storage.put_run(row);
        Ok(())
    }

    fn query(&self, key: &str) -> Result<RunStatus, EngineError> {
        self.storage
            .get_run(key)
            .map(|r| Self::status_of(&r))
            .ok_or_else(|| EngineError::NotFound(key.into()))
    }

    fn drain_due(&mut self, now_ms: u64) -> Result<Vec<EngineMsg>, EngineError> {
        let intents = due_wakes(&self.storage.list_parked(), now_ms);
        let mut msgs = Vec::with_capacity(intents.len());
        for intent in intents {
            let overdue = self.wake(&intent.key, now_ms)?.unwrap_or(intent.overdue_ms);
            msgs.push(EngineMsg::Woke {
                key: intent.key,
                overdue_ms: overdue,
                park_generation: intent.park_generation,
            });
        }
        Ok(msgs)
    }

    fn parked(&self) -> Vec<ParkedRow> {
        self.storage.list_parked()
    }
}

/// Rebuild an engine from storage alone (restart recovery).
///
/// Journal in-memory rows are empty after restart; the host that persists
/// journal separately will rehydrate it. Parks come from [`EngineStorage`].
impl OurEngine<MemStorage> {
    /// Fresh engine process on the same [`MemStorage`] snapshot.
    pub fn recover(storage: MemStorage) -> Self {
        let mut engine = Self::with_storage(storage);
        // Re-seed next_park_gen from existing parks so generations stay monotonic.
        for park in engine.storage.list_parked() {
            let entry = engine.next_park_gen.entry(park.key.clone()).or_insert(0);
            *entry = (*entry).max(park.park_generation);
        }
        engine
    }
}
