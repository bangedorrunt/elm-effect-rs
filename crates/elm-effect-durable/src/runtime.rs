//! Our single-host engine (ADR-0005).

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::ParkDecision;
use elm_effect_journal::plan::{
    DispatchPlan, EffectView, ProviderCapability, may_claim, plan_attempt, plan_park,
};
use elm_effect_journal::store::{InMemoryStore, JournalError, JournalStore, Row};

use crate::engine::{
    DurableEngine, EngineError, ParkOutcome, RunPhase, RunStatus, StartOptions, StartOutcome,
};
use crate::scheduler::due_wakes;
use crate::storage::{EngineStorage, MemStorage, ParkedRow, RunRow};

/// Msg the wake loop feeds back into an Elm fold.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
    /// the result onto the run row. See [`DurableEngine::settle`].
    pub fn settle(
        &mut self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, EngineError> {
        self.settle_impl(key, generation, to)
    }

    /// Claim a journal row (bumps generation), then project the fence onto the
    /// run row. See [`DurableEngine::claim`].
    pub fn claim(
        &mut self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, EngineError> {
        self.claim_impl(key, owner, lease_deadline, now_ms, expected_generation)
    }

    /// Count one provider attempt against the journal fence.
    /// See [`DurableEngine::attempt`].
    pub fn attempt(&mut self, key: &str, generation: u64) -> Result<Option<Row>, EngineError> {
        self.attempt_impl(key, generation)
    }

    /// Expire a dead lease, settling `InFlight` / `Accepted` to `Unknown`.
    /// See [`DurableEngine::expire_lease`].
    pub fn expire_lease(&mut self, key: &str, now_ms: u64) -> Result<Option<Row>, EngineError> {
        self.expire_lease_impl(key, now_ms)
    }

    /// Step `Unknown -> ReconcilePending -> outcome`.
    /// See [`DurableEngine::reconcile`].
    pub fn reconcile(
        &mut self,
        key: &str,
        generation: u64,
        outcome: EffectState,
    ) -> Result<Option<Row>, EngineError> {
        self.reconcile_impl(key, generation, outcome)
    }

    /// Begin or re-attach a run with explicit journal options.
    /// See [`DurableEngine::start_with`].
    pub fn start_with(
        &mut self,
        key: &str,
        now_ms: u64,
        opts: &StartOptions,
    ) -> Result<StartOutcome, EngineError> {
        self.start_with_impl(key, now_ms, opts)
    }

    /// Project a journal row onto storage, creating the run row when a
    /// host-injected journal arrived first. Active transitions (settle,
    /// claim, reconcile, dead-letter) clear parks: a park beside a moved
    /// run is stale and would block the drain.
    fn project_row(&mut self, row: &Row) -> Result<(), EngineError> {
        let mut run = self.storage.get_run(&row.key)?.unwrap_or(RunRow {
            key: row.key.clone(),
            phase: phase_of(row.state),
            generation: row.generation,
            until_ms: None,
            attempt: i64::from(row.attempts),
        });
        run.generation = row.generation;
        run.attempt = i64::from(row.attempts);
        run.phase = phase_of(row.state);
        run.until_ms = None;
        self.storage.commit_run(run)?;
        Ok(())
    }

    /// Project a retry counter: a live park survives (the run still waits),
    /// only the fence and the attempt count move. A dead-letter (`Failed`)
    /// is terminal, so it clears the park through [`project_row`](Self::project_row).
    fn project_attempt(&mut self, row: &Row) -> Result<(), EngineError> {
        if row.state.is_terminal() {
            return self.project_row(row);
        }
        match self.storage.get_run(&row.key)? {
            Some(mut run) => {
                run.generation = row.generation;
                run.attempt = i64::from(row.attempts);
                self.storage.put_run(run)?;
            }
            None => self.storage.put_run(RunRow {
                key: row.key.clone(),
                phase: phase_of(row.state),
                generation: row.generation,
                until_ms: None,
                attempt: i64::from(row.attempts),
            })?,
        }
        Ok(())
    }

    fn settle_impl(
        &mut self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, EngineError> {
        let Some(row) = self.journal.settle(key, generation, to)? else {
            // A concurrent journal update moved the fence. Project the
            // latest row so storage never stays stale, then report the
            // stale fence.
            if let Some(latest) = self.journal.get(key)? {
                self.project_row(&latest)?;
            }
            return Ok(None);
        };
        self.project_row(&row)?;
        Ok(Some(row))
    }

    fn claim_impl(
        &mut self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, EngineError> {
        let Some(row) =
            self.journal
                .claim(key, owner, lease_deadline, now_ms, expected_generation)?
        else {
            if let Some(latest) = self.journal.get(key)? {
                self.project_row(&latest)?;
            }
            return Ok(None);
        };
        self.project_row(&row)?;
        Ok(Some(row))
    }

    fn attempt_impl(&mut self, key: &str, generation: u64) -> Result<Option<Row>, EngineError> {
        let Some(row) = self.journal.attempt(key, generation)? else {
            return Ok(None);
        };
        self.project_attempt(&row)?;
        Ok(Some(row))
    }

    fn expire_lease_impl(&mut self, key: &str, now_ms: u64) -> Result<Option<Row>, EngineError> {
        let Some(row) = self.journal.get(key)? else {
            return Ok(None);
        };
        if !matches!(row.state, EffectState::InFlight | EffectState::Accepted) {
            return Ok(None);
        }
        let Some(deadline) = row.lease_deadline else {
            return Ok(None);
        };
        if deadline > journal_ms(now_ms) {
            return Ok(None);
        }
        let Some(updated) = self
            .journal
            .settle(key, row.generation, EffectState::Unknown)?
        else {
            if let Some(latest) = self.journal.get(key)? {
                self.project_row(&latest)?;
            }
            return Ok(None);
        };
        self.project_row(&updated)?;
        Ok(Some(updated))
    }

    fn reconcile_impl(
        &mut self,
        key: &str,
        generation: u64,
        outcome: EffectState,
    ) -> Result<Option<Row>, EngineError> {
        let current = match self.journal.get(key)? {
            Some(row) if row.generation == generation => row,
            // Stale fence or absent row: project the latest journal row so
            // storage never stays stale, then report the stale fence.
            _ => {
                if let Some(latest) = self.journal.get(key)? {
                    self.project_row(&latest)?;
                }
                return Ok(None);
            }
        };
        // Step through ReconcilePending; settle keeps the generation, so one
        // fence covers both edges.
        if current.state == EffectState::Unknown {
            if self
                .journal
                .settle(key, generation, EffectState::ReconcilePending)?
                .is_none()
            {
                if let Some(latest) = self.journal.get(key)? {
                    self.project_row(&latest)?;
                }
                return Ok(None);
            }
        } else if current.state != EffectState::ReconcilePending {
            // Only uncertain rows reconcile; settled, failed, running, and
            // parked rows are not reconcile inputs.
            return Ok(None);
        }
        let Some(row) = self.journal.settle(key, generation, outcome)? else {
            if let Some(latest) = self.journal.get(key)? {
                self.project_row(&latest)?;
            }
            return Ok(None);
        };
        self.project_row(&row)?;
        Ok(Some(row))
    }

    /// Mark the run Unknown (uncertain outcome). Only reconcile may leave it.
    /// See [`DurableEngine::mark_unknown`].
    pub fn mark_unknown(&mut self, key: &str) -> Result<(), EngineError> {
        self.mark_unknown_impl(key)
    }

    fn mark_unknown_impl(&mut self, key: &str) -> Result<(), EngineError> {
        let mut row = self
            .storage
            .get_run(key)?
            .ok_or_else(|| EngineError::NotFound(key.into()))?;
        if let Some(j) = self.journal.get(key)?
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
        if let Some(j) = self.journal.get(key)?
            && j.state != EffectState::Unknown
        {
            let settled = self
                .journal
                .settle(&j.key, j.generation, EffectState::Unknown)?
                .ok_or_else(|| {
                    EngineError::Journal(JournalError::Rejected("refused Unknown".into()))
                })?;
            row.generation = settled.generation;
            row.attempt = i64::from(settled.attempts);
        }
        row.phase = RunPhase::Unknown;
        row.until_ms = None;
        self.storage.commit_run(row)?;
        Ok(())
    }

    fn start_with_impl(
        &mut self,
        key: &str,
        now_ms: u64,
        opts: &StartOptions,
    ) -> Result<StartOutcome, EngineError> {
        if let Some(j) = self.journal.get(key)? {
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

        let j = self.journal.record_intent(key, opts.max_attempts)?;
        let now = journal_ms(now_ms);
        let lease_deadline = now.saturating_add(i64::try_from(opts.lease_ms).unwrap_or(i64::MAX));
        // Another owner with a live lease is already handling this prepared
        // intent. Re-attach; never steal it or fail the start. Project the
        // journal row when storage has none, so the re-attach is queryable.
        if !may_claim(
            j.state,
            j.owner.as_deref(),
            j.lease_deadline,
            &opts.owner,
            now,
            j.generation,
            j.generation,
        ) {
            if j.state == EffectState::Prepared && self.storage.get_run(key)?.is_none() {
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
        // The generation fences stale settle attempts. With the default
        // `lease_ms: 0` the lease is already expired: this engine schedules,
        // the host holds the execution lease.
        let claimed = self
            .journal
            .claim(key, &opts.owner, lease_deadline, now, j.generation)?
            .ok_or_else(|| {
                EngineError::Journal(JournalError::Rejected("claim after intent failed".into()))
            })?;
        let inflight = self
            .journal
            .settle(key, claimed.generation, EffectState::InFlight)?
            .ok_or_else(|| {
                EngineError::Journal(JournalError::Rejected("Prepared→InFlight failed".into()))
            })?;
        // One storage write after the journal reaches InFlight: a crash
        // before it leaves a journal the next start completes, never a
        // half-fenced run row.
        self.storage.commit_run(RunRow {
            key: key.to_string(),
            phase: RunPhase::Running,
            generation: inflight.generation,
            until_ms: None,
            attempt: i64::from(inflight.attempts),
        })?;
        Ok(StartOutcome::Started {
            generation: inflight.generation,
        })
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
        if let Some(journal) = self.journal.get(key)? {
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
        self.start_with_impl(key, now_ms, &StartOptions::default())
    }

    fn start_with(
        &mut self,
        key: &str,
        now_ms: u64,
        opts: &StartOptions,
    ) -> Result<StartOutcome, EngineError> {
        self.start_with_impl(key, now_ms, opts)
    }

    fn settle(
        &mut self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, EngineError> {
        self.settle_impl(key, generation, to)
    }

    fn claim(
        &mut self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, EngineError> {
        self.claim_impl(key, owner, lease_deadline, now_ms, expected_generation)
    }

    fn attempt(&mut self, key: &str, generation: u64) -> Result<Option<Row>, EngineError> {
        self.attempt_impl(key, generation)
    }

    fn mark_unknown(&mut self, key: &str) -> Result<(), EngineError> {
        self.mark_unknown_impl(key)
    }

    fn reconcile(
        &mut self,
        key: &str,
        generation: u64,
        outcome: EffectState,
    ) -> Result<Option<Row>, EngineError> {
        self.reconcile_impl(key, generation, outcome)
    }

    fn expire_lease(&mut self, key: &str, now_ms: u64) -> Result<Option<Row>, EngineError> {
        self.expire_lease_impl(key, now_ms)
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
        if let Some(j) = self.journal.get(key)? {
            if j.state.is_terminal() {
                self.storage.clear_parks(key)?;
                return Err(EngineError::Terminal {
                    key: key.into(),
                    phase: phase_of(j.state),
                });
            }
            let settled = self
                .journal
                .settle(&j.key, j.generation, EffectState::Cancelled)?
                .ok_or_else(|| {
                    EngineError::Journal(JournalError::Rejected("refused cancel".into()))
                })?;
            row.generation = settled.generation;
            row.attempt = i64::from(settled.attempts);
        }
        row.phase = RunPhase::Cancelled;
        row.until_ms = None;
        self.storage.commit_run(row)?;
        Ok(())
    }

    fn query(&self, key: &str) -> Result<RunStatus, EngineError> {
        if let Some(j) = self.journal.get(key)? {
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
