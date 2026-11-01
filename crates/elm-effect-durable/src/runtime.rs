//! Our single-host engine (ADR-0005).

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::ParkDecision;
use elm_effect_journal::plan::{
    DispatchPlan, EffectView, ProviderCapability, may_claim, plan_attempt, plan_park,
};
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
}

/// Default durable engine: storage + journal fencing + injected clock consumer.
///
/// `now_ms` is always passed in by the caller (or by [`Self::drain_due`]'s
/// argument). Pair with `elm_effect::TestClock` in tests: advance the clock,
/// then pass `clock.current_ms()` (or your own `u64`) into these methods.
pub struct OurEngine<S: EngineStorage = MemStorage, J: JournalStore = InMemoryStore> {
    storage: S,
    /// Journal rows for intent / claim / settle fencing (L14).
    journal: J,
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

impl<S: EngineStorage> OurEngine<S, InMemoryStore> {
    /// Engine over a host store with the crate's in-memory journal.
    ///
    /// Park generations are read lazily from the store on the next
    /// `park_until`, so construction does not perform I/O.
    pub fn with_storage(storage: S) -> Self {
        Self::with_journal(storage, InMemoryStore::new(1024))
    }

    /// Fresh engine process on the same storage.
    ///
    /// Storage parks survive. Journal rows survive only when the host injects
    /// the persisted journal with [`Self::with_journal`].
    pub fn recover(storage: S) -> Self {
        Self::with_storage(storage)
    }
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Engine over a host store and a host journal.
    ///
    /// A host that persists journal rows across restart injects them here, so
    /// `start` re-attaches to the recorded intent instead of re-dispatching.
    pub fn with_journal(storage: S, journal: J) -> Self {
        Self {
            storage,
            journal,
            next_park_gen: std::collections::BTreeMap::new(),
        }
    }

    /// Borrow the journal store (conformance / host reconcile).
    pub fn journal(&self) -> &J {
        &self.journal
    }

    /// Settle a journal row toward `to` with a generation fence, then project
    /// the result onto the run row.
    ///
    /// Stale generations return `None`. A settle is an active transition, so
    /// it clears any park. `Unknown` has no default exit: the golden table
    /// refuses `Unknown → Settled` without reconcile (`Unknown →
    /// ReconcilePending` first).
    pub fn settle(
        &mut self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<elm_effect_journal::store::Row>, EngineError> {
        let Some(row) = self.journal.settle(key, generation, to) else {
            return Ok(None);
        };
        self.apply_journal_row(&row)?;
        Ok(Some(row))
    }

    /// Claim a journal row (bumps generation), then project the fence onto the
    /// run row. A claim is a dispatch, so it clears any park.
    pub fn claim(
        &mut self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<elm_effect_journal::store::Row>, EngineError> {
        let Some(row) = self
            .journal
            .claim(key, owner, lease_deadline, now_ms, expected_generation)
        else {
            return Ok(None);
        };
        if let Some(mut run) = self.storage.get_run(key)? {
            run.generation = row.generation;
            run.attempt = i64::from(row.attempts);
            run.phase = RunPhase::Running;
            run.until_ms = None;
            self.storage.commit_run(run)?;
        }
        Ok(Some(row))
    }

    fn apply_journal_row(
        &mut self,
        row: &elm_effect_journal::store::Row,
    ) -> Result<(), EngineError> {
        let Some(mut run) = self.storage.get_run(&row.key)? else {
            return Ok(());
        };
        run.generation = row.generation;
        run.attempt = i64::from(row.attempts);
        run.phase = phase_of(row.state);
        run.until_ms = None;
        // A settle is an active transition, not a wait; a park row beside it
        // is stale and would block the drain.
        self.storage.commit_run(run)?;
        Ok(())
    }

    /// Mark the run Unknown (uncertain outcome). Only reconcile may leave it.
    pub fn mark_unknown(&mut self, key: &str) -> Result<(), EngineError> {
        let mut row = self
            .storage
            .get_run(key)?
            .ok_or_else(|| EngineError::NotFound(key.into()))?;
        if let Some(j) = self.journal.get(key)
            && j.state.is_terminal()
        {
            return Err(EngineError::Terminal {
                key: key.into(),
                phase: phase_of(j.state),
            });
        }
        if row.phase.is_terminal() {
            return Err(EngineError::Terminal {
                key: key.into(),
                phase: row.phase,
            });
        }
        // Drive the journal to Unknown when it is not there already. A refused
        // edge is a real inconsistency, not a silent success.
        if let Some(j) = self.journal.get(key)
            && j.state != EffectState::Unknown
        {
            let settled = self
                .journal
                .settle(&j.key, j.generation, EffectState::Unknown)
                .ok_or_else(|| EngineError::Journal("refused Unknown".into()))?;
            row.generation = settled.generation;
            row.attempt = i64::from(settled.attempts);
        }
        row.phase = RunPhase::Unknown;
        row.until_ms = None;
        self.storage.commit_run(row)?;
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

    /// The journal is authoritative for lifecycle; storage contributes the
    /// park (a park is an engine state, not a journal edge).
    fn status_with_journal(
        &self,
        key: &str,
        journal: &elm_effect_journal::store::Row,
    ) -> Result<RunStatus, EngineError> {
        let mut status = match self.storage.get_run(key)? {
            Some(row) => Self::status_of(&row),
            None => RunStatus {
                key: key.to_string(),
                phase: phase_of(journal.state),
                generation: journal.generation,
                until_ms: None,
            },
        };
        if journal.generation > status.generation {
            // The journal has a newer fence than storage. A stale Parked row
            // must not hide a claim that already moved the run forward.
            status.phase = phase_of(journal.state);
            status.until_ms = None;
        }
        match journal.state {
            EffectState::Settled
            | EffectState::Failed
            | EffectState::Cancelled
            | EffectState::Unknown
            | EffectState::ReconcilePending => {
                status.phase = phase_of(journal.state);
                status.until_ms = None;
            }
            EffectState::Parked => status.phase = RunPhase::Parked,
            EffectState::Prepared | EffectState::InFlight | EffectState::Accepted => {
                if status.phase != RunPhase::Parked {
                    status.phase = RunPhase::Running;
                    status.until_ms = None;
                }
            }
        }
        status.generation = journal.generation;
        Ok(status)
    }

    fn require_live(&self, key: &str) -> Result<RunRow, EngineError> {
        let row = self
            .storage
            .get_run(key)?
            .ok_or_else(|| EngineError::NotFound(key.into()))?;
        if let Some(journal) = self.journal.get(key) {
            if journal.state.is_terminal() {
                return Err(EngineError::Terminal {
                    key: key.into(),
                    phase: phase_of(journal.state),
                });
            }
            if matches!(
                journal.state,
                EffectState::Unknown | EffectState::ReconcilePending
            ) {
                return Err(EngineError::Uncertain { key: key.into() });
            }
        }
        if row.phase.is_terminal() {
            return Err(EngineError::Terminal {
                key: key.into(),
                phase: row.phase,
            });
        }
        if row.phase == RunPhase::Unknown {
            return Err(EngineError::Uncertain { key: key.into() });
        }
        Ok(row)
    }
}

impl<S: EngineStorage, J: JournalStore> DurableEngine for OurEngine<S, J> {
    fn start(&mut self, key: &str, now_ms: u64) -> Result<StartOutcome, EngineError> {
        if let Some(j) = self.journal.get(key) {
            let view = EffectView {
                state: j.state,
                attempt: i64::from(j.attempts),
                lease_live: j
                    .lease_deadline
                    .is_some_and(|deadline| deadline > journal_ms(now_ms)),
            };
            let plan = plan_attempt(
                Some(&view),
                &ProviderCapability {
                    supports_idempotency_key: true,
                    supports_reconcile_query: false,
                },
            );
            // The one case start completes: an intent was recorded but the
            // claim never happened (crash between intent and dispatch). Every
            // other existing row re-attaches; retries and reconciliation are
            // explicit host verbs.
            if !(j.state == EffectState::Prepared && matches!(plan, DispatchPlan::DispatchNew)) {
                if self.storage.get_run(key)?.is_none() {
                    // A host-injected journal may arrive before its run row.
                    // Re-attach still leaves a row the other verbs can use.
                    self.storage.commit_run(RunRow {
                        key: key.to_string(),
                        phase: phase_of(j.state),
                        generation: j.generation,
                        until_ms: None,
                        attempt: i64::from(j.attempts),
                    })?;
                }
                return Ok(StartOutcome::Already {
                    status: self.status_with_journal(key, &j)?,
                });
            }
        } else if let Some(existing) = self.storage.get_run(key)?
            && existing.phase != RunPhase::Running
        {
            // A park, terminal row, or unknown outcome is already the answer.
            return Ok(StartOutcome::Already {
                status: Self::status_of(&existing),
            });
        }

        let j = self.journal.record_intent(key, 8);
        let now = journal_ms(now_ms);
        // Another owner with a live lease is already handling this prepared
        // intent. Re-attach; never steal it or fail the start.
        if !may_claim(
            j.state,
            j.owner.as_deref(),
            j.lease_deadline,
            "engine",
            now,
            j.generation,
            j.generation,
        ) {
            return Ok(StartOutcome::Already {
                status: self.status_with_journal(key, &j)?,
            });
        }
        // Write the run row before the journal fence. A crash between the two
        // still leaves a row the host can query and park; the final commit
        // below carries the claimed generation.
        self.storage.commit_run(RunRow {
            key: key.to_string(),
            phase: RunPhase::Running,
            generation: j.generation,
            until_ms: None,
            attempt: i64::from(j.attempts),
        })?;
        // The generation fences stale settle attempts. The lease is already
        // expired: this engine schedules, the host holds the execution lease.
        let claimed = self
            .journal
            .claim(key, "engine", now, now, j.generation)
            .ok_or_else(|| EngineError::Journal("claim after intent failed".into()))?;
        let inflight = self
            .journal
            .settle(key, claimed.generation, EffectState::InFlight)
            .ok_or_else(|| EngineError::Journal("Prepared→InFlight failed".into()))?;
        let row = RunRow {
            key: key.to_string(),
            phase: RunPhase::Running,
            generation: inflight.generation,
            until_ms: None,
            attempt: i64::from(inflight.attempts),
        };
        self.storage.commit_run(row)?;
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
                self.storage.commit_run(row)?;
                Ok(ParkOutcome::ResumeNow { overdue_ms })
            }
            ParkDecision::Wait { wait_ms, intent } => {
                // Re-parking the same key to the same instant keeps the
                // existing generation; only a different instant bumps it.
                let existing = self.storage.get_park(key)?;
                if let Some(park) = &existing {
                    let generation = self.next_park_gen.entry(key.to_string()).or_insert(0);
                    *generation = (*generation).max(park.park_generation);
                }
                let park_generation = match existing {
                    Some(park) if park.until_ms == intent.until_ms => park.park_generation,
                    _ => {
                        let generation = self.next_park_gen.entry(key.to_string()).or_insert(0);
                        *generation = generation.saturating_add(1);
                        *generation
                    }
                };
                row.phase = RunPhase::Parked;
                row.until_ms = Some(intent.until_ms);
                self.storage.commit_park(
                    row,
                    ParkedRow {
                        key: key.to_string(),
                        until_ms: intent.until_ms,
                        park_generation,
                    },
                )?;
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
            // Already running: idempotent wake. A park row beside it is stale.
            self.storage.clear_parks(key)?;
            return Ok(None);
        }
        let until = row.until_ms.unwrap_or(now_ms);
        let overdue = now_ms.saturating_sub(until);
        row.phase = RunPhase::Running;
        row.until_ms = None;
        self.storage.commit_run(row)?;
        Ok(Some(overdue))
    }

    fn cancel(&mut self, key: &str, _now_ms: u64) -> Result<(), EngineError> {
        let mut row = self
            .storage
            .get_run(key)?
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
            if j.state.is_terminal() {
                self.storage.clear_parks(key)?;
                return Err(EngineError::Terminal {
                    key: key.into(),
                    phase: phase_of(j.state),
                });
            }
            let settled = self
                .journal
                .settle(&j.key, j.generation, EffectState::Cancelled)
                .ok_or_else(|| EngineError::Journal("refused cancel".into()))?;
            row.generation = settled.generation;
            row.attempt = i64::from(settled.attempts);
        }
        row.phase = RunPhase::Cancelled;
        row.until_ms = None;
        self.storage.commit_run(row)?;
        Ok(())
    }

    fn query(&self, key: &str) -> Result<RunStatus, EngineError> {
        if let Some(j) = self.journal.get(key) {
            return self.status_with_journal(key, &j);
        }
        self.storage
            .get_run(key)?
            .map(|r| Self::status_of(&r))
            .ok_or_else(|| EngineError::NotFound(key.into()))
    }

    fn drain_due(&mut self, now_ms: u64) -> Result<Vec<EngineMsg>, EngineError> {
        let intents = due_wakes(&self.storage.due_parks(now_ms)?, now_ms);
        let mut msgs = Vec::with_capacity(intents.len());
        for intent in intents {
            match self.wake(&intent.key, now_ms) {
                Ok(Some(overdue)) => msgs.push(EngineMsg::Woke {
                    key: intent.key,
                    overdue_ms: overdue,
                    park_generation: intent.park_generation,
                }),
                // Already running, terminal, unknown, or orphaned: the park
                // is stale. Clear it and keep the drain alive.
                Ok(None)
                | Err(
                    EngineError::NotFound(_)
                    | EngineError::Terminal { .. }
                    | EngineError::Uncertain { .. },
                ) => self.storage.clear_parks(&intent.key)?,
                Err(other) => return Err(other),
            }
        }
        Ok(msgs)
    }

    fn parked(&self) -> Result<Vec<ParkedRow>, EngineError> {
        self.storage.list_parked().map_err(Into::into)
    }
}

fn phase_of(state: EffectState) -> RunPhase {
    match state {
        EffectState::Prepared | EffectState::InFlight | EffectState::Accepted => RunPhase::Running,
        EffectState::Parked => RunPhase::Parked,
        EffectState::Cancelled => RunPhase::Cancelled,
        EffectState::Settled => RunPhase::Settled,
        EffectState::Failed => RunPhase::Failed,
        EffectState::Unknown | EffectState::ReconcilePending => RunPhase::Unknown,
    }
}

fn journal_ms(now_ms: u64) -> i64 {
    i64::try_from(now_ms).unwrap_or(i64::MAX)
}
