//! Lease heartbeat (`heartbeat`): a live worker extends its claim without moving the fence; a zombie or cancelled holder learns it must stop.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`];
//! the `DurableEngine` port trait is unchanged.

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::{HeartbeatRejection, check_heartbeat};
use elm_effect_journal::store::{JournalError, JournalStore, Row};

use crate::engine::{DurableEngine, EngineError};
use crate::runtime::OurEngine;
use crate::storage::EngineStorage;

/// Outcome of [`OurEngine::heartbeat`](crate::OurEngine::heartbeat).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum HeartbeatOutcome {
    /// The lease moved to `lease_deadline`; the fence did not move.
    Renewed {
        /// The new lease deadline in ms.
        lease_deadline: i64,
        /// The journal row after the renew.
        row: Row,
    },
    /// The run was cancelled: the worker stops, it never retries the lease.
    Cancelled,
    /// The renew was refused; the latest journal row is projected onto
    /// storage so it never stays stale.
    Rejected(HeartbeatRejection),
}

/// Diagnostic view of an execution lease.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LeaseStatus {
    /// Active lease held by `owner` until `deadline_ms`.
    Active {
        /// Owner holding the lease.
        owner: String,
        /// Generation counter of the lease.
        generation: u64,
        /// Milliseconds remaining before the lease lapses.
        ttl_remaining_ms: u64,
    },
    /// Expired lease: deadline has elapsed without renewal.
    Expired {
        /// Last owner who claimed the lease.
        owner: String,
        /// Generation counter when the lease was claimed.
        generation: u64,
        /// Milliseconds since the deadline elapsed.
        overdue_ms: u64,
    },
    /// No lease currently held (absent, un-claimed, terminal, or zero-lease).
    Unheld,
}

impl<S: EngineStorage, J: JournalStore> OurEngine<S, J> {
    /// Extend the lease on a live holder without moving the fence.
    ///
    /// `lease_ms` must be positive: a zero lease is a rejected journal write,
    /// not a no-op. On a refused renew the latest journal row is projected
    /// onto storage and mapped: a `Cancelled` row becomes
    /// [`Cancelled`](HeartbeatOutcome::Cancelled) (the cancel piggyback);
    /// anything else is the [`check_heartbeat`] refusal for the latest row,
    /// falling back to `StaleGeneration` when a host journal refuses outside
    /// the pure fence.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the journal or the store cannot
    /// read or write.
    pub fn heartbeat(
        &mut self,
        key: &str,
        generation: u64,
        owner: &str,
        lease_ms: u64,
        now_ms: i64,
    ) -> Result<HeartbeatOutcome, EngineError> {
        let lease_ms = i64::try_from(lease_ms).unwrap_or(i64::MAX);
        if lease_ms <= 0 {
            return Err(EngineError::Journal(JournalError::Rejected(
                "heartbeat lease_ms must be positive".into(),
            )));
        }
        let deadline = now_ms.saturating_add(lease_ms);
        if let Some(row) = self
            .journal
            .renew_lease(key, owner, deadline, now_ms, generation)?
        {
            self.project_row(&row)?;
            return Ok(HeartbeatOutcome::Renewed {
                lease_deadline: deadline,
                row,
            });
        }
        let Some(latest) = self.journal.get(key)? else {
            return Ok(HeartbeatOutcome::Rejected(HeartbeatRejection::NotFound));
        };
        self.project_row(&latest)?;
        if latest.state == EffectState::Cancelled {
            return Ok(HeartbeatOutcome::Cancelled);
        }
        let rejection = check_heartbeat(
            latest.state,
            latest.owner.as_deref(),
            latest.lease_deadline,
            owner,
            now_ms,
            generation,
            latest.generation,
        )
        .err()
        // A host journal that refuses outside the pure fence; the fence
        // did not move for the caller.
        .unwrap_or(HeartbeatRejection::StaleGeneration {
            expected: generation,
            actual: latest.generation,
        });
        Ok(HeartbeatOutcome::Rejected(rejection))
    }

    /// Query the status of the execution lease for `key`.
    ///
    /// Non-mutating diagnostic query: evaluates `lease_deadline` against `now_ms`.
    /// Terminal, absent, zero-lease, or un-claimed runs return [`LeaseStatus::Unheld`].
    pub fn query_lease(&self, key: &str, now_ms: u64) -> Result<LeaseStatus, EngineError> {
        let Some(row) = self.journal.get(key)? else {
            return Ok(LeaseStatus::Unheld);
        };
        if row.state.is_terminal() {
            return Ok(LeaseStatus::Unheld);
        }
        let (Some(owner), Some(deadline)) = (row.owner, row.lease_deadline) else {
            return Ok(LeaseStatus::Unheld);
        };
        if deadline <= 0 {
            return Ok(LeaseStatus::Unheld);
        }
        let deadline = deadline as u64;
        if deadline > now_ms {
            Ok(LeaseStatus::Active {
                owner,
                generation: row.generation,
                ttl_remaining_ms: deadline - now_ms,
            })
        } else {
            Ok(LeaseStatus::Expired {
                owner,
                generation: row.generation,
                overdue_ms: now_ms.saturating_sub(deadline),
            })
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
    owner: &str,
    lease_ms: u64,
) -> Result<u64, String> {
    use crate::engine::{StartOptions, StartOutcome};
    let opts = StartOptions {
        max_attempts: 8,
        lease_ms,
        owner: owner.into(),
    };
    match engine.start_with(key, 0, &opts).map_err(law_fail)? {
        StartOutcome::Started { generation } => Ok(generation),
        StartOutcome::Already { status } => Err(format!("fresh key {key} must start: {status:?}")),
    }
}

/// Conformance laws for the heartbeat verb. A sibling runs these against its
/// own engine; each law uses a fresh key so laws never interfere.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
pub fn heartbeat_laws<S: EngineStorage, J: JournalStore>(
    engine: &mut OurEngine<S, J>,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("renew keeps the fence and moves the deadline", {
            let generation = law_started(engine, "law-hb-renew", "worker", 10_000);
            generation.and_then(|generation| {
                match engine
                    .heartbeat("law-hb-renew", generation, "worker", 10_000, 5_000)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Renewed {
                        lease_deadline,
                        row,
                    } => {
                        law_check(lease_deadline == 15_000, "deadline moves to now + lease")?;
                        law_check(row.generation == generation, "the fence does not move")?;
                        law_check(
                            row.owner.as_deref() == Some("worker"),
                            "the owner does not move",
                        )
                    }
                    other => Err(format!("live holder must renew: {other:?}")),
                }
            })
        }),
        ("renewed lease delays expiry", {
            let generation = law_started(engine, "law-hb-expire", "worker", 1_000);
            generation.and_then(|generation| {
                match engine
                    .heartbeat("law-hb-expire", generation, "worker", 10_000, 500)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Renewed { .. } => {}
                    other => return Err(format!("live holder must renew: {other:?}")),
                }
                // Past the original deadline but before the renewed one: no-op.
                law_check(
                    engine
                        .expire_lease("law-hb-expire", 1_000)
                        .map_err(law_fail)?
                        .is_none(),
                    "renewed lease survives its old deadline",
                )?;
                law_check(
                    engine.query("law-hb-expire").map_err(law_fail)?.phase
                        == crate::engine::RunPhase::Running,
                    "the run stays Running",
                )?;
                // At the renewed deadline the lease lapses to Unknown.
                let updated = engine
                    .expire_lease("law-hb-expire", 10_500)
                    .map_err(law_fail)?
                    .ok_or("lapsed lease must expire to Unknown")?;
                law_check(
                    updated.state == EffectState::Unknown,
                    "expiry settles the journal to Unknown",
                )
            })
        }),
        ("zombie generation is rejected", {
            let generation = law_started(engine, "law-hb-zombie", "worker", 10_000);
            generation.and_then(|generation| {
                match engine
                    .heartbeat("law-hb-zombie", 0, "worker", 10_000, 0)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Rejected(HeartbeatRejection::StaleGeneration {
                        expected,
                        actual,
                    }) => {
                        law_check(expected == 0, "the presented generation is named")?;
                        law_check(actual == generation, "the live fence is named")
                    }
                    other => Err(format!("zombie must be StaleGeneration: {other:?}")),
                }
            })
        }),
        ("foreign owner is rejected", {
            let generation = law_started(engine, "law-hb-owner", "worker", 10_000);
            generation.and_then(|generation| {
                match engine
                    .heartbeat("law-hb-owner", generation, "thief", 10_000, 0)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Rejected(HeartbeatRejection::NotOwner { owner }) => {
                        law_check(owner.as_deref() == Some("worker"), "the holder is named")
                    }
                    other => Err(format!("foreign owner must be NotOwner: {other:?}")),
                }
            })
        }),
        ("lapsed lease is rejected", {
            let generation = law_started(engine, "law-hb-lapsed", "worker", 1_000);
            generation.and_then(|generation| {
                match engine
                    .heartbeat("law-hb-lapsed", generation, "worker", 10_000, 1_000)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Rejected(HeartbeatRejection::LeaseExpired {
                        lease_deadline,
                    }) => law_check(
                        lease_deadline == Some(1_000),
                        "the lapsed deadline is named",
                    ),
                    other => Err(format!("lapsed lease must be LeaseExpired: {other:?}")),
                }
            })
        }),
        ("cancelled row reports Cancelled", {
            let generation = law_started(engine, "law-hb-cancel", "worker", 10_000);
            generation.and_then(|generation| {
                engine.cancel("law-hb-cancel", 0).map_err(law_fail)?;
                match engine
                    .heartbeat("law-hb-cancel", generation, "worker", 10_000, 0)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Cancelled => Ok(()),
                    other => Err(format!("cancelled row must piggyback Cancelled: {other:?}")),
                }
            })
        }),
        ("terminal row is rejected", {
            (|| -> Result<(), String> {
                let generation = law_started(engine, "law-hb-terminal", "worker", 10_000)?;
                engine
                    .settle("law-hb-terminal", generation, EffectState::Settled)
                    .map_err(law_fail)?;
                match engine
                    .heartbeat("law-hb-terminal", generation, "worker", 10_000, 0)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Rejected(HeartbeatRejection::Terminal { state }) => {
                        law_check(state == EffectState::Settled, "the terminal state is named")
                    }
                    other => Err(format!("terminal row must be Terminal: {other:?}")),
                }
            })()
        }),
        ("non-running row is rejected", {
            (|| -> Result<(), String> {
                // A bare intent never claimed past Prepared: no live lease.
                engine
                    .journal()
                    .record_intent("law-hb-prepared", 8)
                    .map_err(law_fail)?;
                match engine
                    .heartbeat("law-hb-prepared", 0, "worker", 10_000, 0)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Rejected(HeartbeatRejection::NotRunning { state }) => {
                        law_check(state == EffectState::Prepared, "prepared holds no lease")
                    }
                    other => Err(format!("prepared row must be NotRunning: {other:?}")),
                }
            })()
        }),
        ("absent key is rejected", {
            (|| -> Result<(), String> {
                match engine
                    .heartbeat("law-hb-nope", 0, "nobody", 10_000, 0)
                    .map_err(law_fail)?
                {
                    HeartbeatOutcome::Rejected(HeartbeatRejection::NotFound) => Ok(()),
                    other => Err(format!("absent key must be NotFound: {other:?}")),
                }
            })()
        }),
        ("zero lease is a rejected write", {
            let generation = law_started(engine, "law-hb-zero", "worker", 10_000);
            generation.and_then(|generation| {
                match engine.heartbeat("law-hb-zero", generation, "worker", 0, 0) {
                    Err(EngineError::Journal(_)) => Ok(()),
                    other => Err(format!("zero lease must be a journal Rejected: {other:?}")),
                }
            })
        }),
    ]
}
