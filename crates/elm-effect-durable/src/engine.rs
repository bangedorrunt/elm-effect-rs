//! Port types: scheduling verbs only (ADR-0005).

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::{ClaimRejection, SettleRejection};
use elm_effect_journal::store::{JournalError, Row};

use crate::storage::ParkedRow;

/// A host store could not complete an operation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    /// The store is temporarily unavailable; a retry may succeed.
    #[error("storage unavailable: {0}")]
    Unavailable(String),
    /// The store returned data that violates its contract.
    #[error("storage corrupt: {0}")]
    Corrupt(String),
    /// The store rejected the operation.
    #[error("storage rejected: {0}")]
    Rejected(String),
}

/// Errors from the durable engine port.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The run is absent and the verb required one.
    #[error("run not found: {0}")]
    NotFound(String),
    /// The run is cancelled or otherwise terminal for this verb.
    #[error("run {key} is terminal ({phase})")]
    Terminal {
        /// Run key.
        key: String,
        /// Terminal phase observed.
        phase: RunPhase,
    },
    /// The outcome is unknown; reconcile before resuming or parking.
    #[error("run {key} outcome is unknown; reconcile first")]
    Uncertain {
        /// Run key.
        key: String,
    },
    /// The journal failed or refused an internal transition.
    #[error("{0}")]
    Journal(#[from] JournalError),
    /// The host store failed.
    #[error("{0}")]
    Storage(#[from] StorageError),
}

/// Coarse phase a [`query`](DurableEngine::query) reports.
///
/// Maps onto journal vocabulary without replacing it: `Parked` is resumable
/// (not terminal); `Cancelled` / `Settled` / `Failed` are terminal.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum RunPhase {
    /// Intent recorded / dispatch in flight.
    Running,
    /// Durable suspension until a wake time.
    Parked,
    /// Terminal cancel.
    Cancelled,
    /// Terminal success (journal Settled).
    Settled,
    /// Terminal typed failure (journal Failed).
    Failed,
    /// Outcome unknown — no default exit (journal Unknown).
    Unknown,
}

impl RunPhase {
    /// Terminal phases never resume through the engine. `Unknown` is not
    /// terminal, but it must reconcile before park or wake.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Settled | Self::Failed)
    }

    /// Lowercase wire name (`"running"`, `"parked"`, …). Matches
    /// [`Display`](std::fmt::Display) and [`FromStr`](std::str::FromStr).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Parked => "parked",
            Self::Cancelled => "cancelled",
            Self::Settled => "settled",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

/// Rejected [`RunPhase`] wire name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown run phase: {0}")]
pub struct ParseRunPhaseError(pub String);

impl std::str::FromStr for RunPhase {
    type Err = ParseRunPhaseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(Self::Running),
            "parked" => Ok(Self::Parked),
            "cancelled" => Ok(Self::Cancelled),
            "settled" => Ok(Self::Settled),
            "failed" => Ok(Self::Failed),
            "unknown" => Ok(Self::Unknown),
            _ => Err(ParseRunPhaseError(s.to_string())),
        }
    }
}

impl std::fmt::Display for RunPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Options for [`DurableEngine::start_with`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct StartOptions {
    /// Attempts allowed on the journal row before dead-lettering.
    pub max_attempts: u32,
    /// Lease window (ms) the starting claim holds. `0` expires immediately:
    /// this engine schedules, the host holds the execution lease.
    pub lease_ms: u64,
    /// Owner recorded on the starting claim.
    pub owner: String,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            lease_ms: 0,
            owner: "engine".to_string(),
        }
    }
}

/// Snapshot returned by [`DurableEngine::query`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[must_use]
pub struct RunStatus {
    /// Run / journal / producer key.
    pub key: String,
    /// Coarse phase.
    pub phase: RunPhase,
    /// Claim / fence generation.
    pub generation: u64,
    /// Absolute wake time when [`RunPhase::Parked`].
    pub until_ms: Option<u64>,
}

/// Result of [`DurableEngine::start`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[must_use]
pub enum StartOutcome {
    /// Fresh run; first attempt may dispatch.
    Started {
        /// Fence generation after intent.
        generation: u64,
    },
    /// Key already had a live or terminal row — no second dispatch.
    Already {
        /// Existing status.
        status: RunStatus,
    },
}

/// Result of [`DurableEngine::park_until`], mirroring journal `ParkDecision`
/// without re-deriving it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[must_use]
pub enum ParkOutcome {
    /// Wait until `until_ms` (intent persisted).
    Wait {
        /// Absolute deadline.
        until_ms: u64,
        /// Planned wait in ms.
        wait_ms: u64,
    },
    /// Deadline already passed; resume now.
    ResumeNow {
        /// `now_ms - until_ms`.
        overdue_ms: u64,
    },
    /// Asked wait exceeds the ceiling — typed refuse, not a shorter sleep.
    RefusedOverCeiling {
        /// Asked max wait seconds.
        asked_secs: u64,
        /// Ceiling that refused.
        ceiling_secs: u64,
    },
}

/// Result of [`DurableEngine::wake_checked`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[must_use]
pub enum WakeOutcome {
    /// Parked run resumed to `Running`.
    Woken {
        /// `now_ms - until_ms` (0 when on time or early).
        overdue_ms: u64,
        /// Park generation that was woken.
        park_generation: u64,
    },
    /// Run was already live and not parked; any stale park was cleared.
    NotParked {
        /// Current live phase.
        phase: RunPhase,
    },
}

/// Result of [`DurableEngine::claim_checked`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum ClaimOutcome {
    /// Claim succeeded, bumped generation, and projected onto storage.
    Claimed(Row),
    /// Claim was refused by the fence; latest journal state projected.
    Rejected(ClaimRejection),
}

/// Result of [`DurableEngine::settle_checked`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum SettleOutcome {
    /// Settle succeeded and projected onto storage.
    Settled(Row),
    /// Settle was refused by the fence; latest journal state projected.
    Rejected(SettleRejection),
}

/// Scheduling port. Journal and `DurableLog` stay elsewhere.
pub trait DurableEngine {
    /// Begin or re-attach a run. Idempotent on `key`: a second start does not
    /// dispatch again.
    fn start(&mut self, key: &str, now_ms: u64) -> Result<StartOutcome, EngineError> {
        self.start_with(key, now_ms, &StartOptions::default())
    }

    /// Begin or re-attach a run with explicit journal options (attempt
    /// budget, claim lease, claim owner). `start` delegates with
    /// [`StartOptions::default`].
    fn start_with(
        &mut self,
        key: &str,
        now_ms: u64,
        opts: &StartOptions,
    ) -> Result<StartOutcome, EngineError>;

    /// Park until `until_ms`. Decision is [`elm_effect_journal::plan::plan_park`].
    fn park_until(
        &mut self,
        key: &str,
        until_ms: u64,
        max_wait_secs: u64,
        now_ms: u64,
        ceiling_secs: Option<u64>,
    ) -> Result<ParkOutcome, EngineError>;

    /// Wake one parked run now. Returns overdue ms when the deadline had passed.
    fn wake(&mut self, key: &str, now_ms: u64) -> Result<Option<u64>, EngineError>;

    /// Wake one parked run now, reporting the park generation. [`wake`](Self::wake)
    /// delegates: `Woken` maps to its `overdue_ms`, `NotParked` to `None`.
    fn wake_checked(&mut self, key: &str, now_ms: u64) -> Result<WakeOutcome, EngineError>;

    /// Terminal cancel.
    fn cancel(&mut self, key: &str, now_ms: u64) -> Result<(), EngineError>;

    /// Read status; `NotFound` when absent.
    fn query(&self, key: &str) -> Result<RunStatus, EngineError>;

    /// Drain parks whose `until_ms <= now_ms` into wakes. Pure selection is
    /// [`crate::due_wakes`]; this applies them.
    fn drain_due(&mut self, now_ms: u64) -> Result<Vec<crate::EngineMsg>, EngineError>;

    /// Parked rows currently stored (for tests and host inspection).
    fn parked(&self) -> Result<Vec<ParkedRow>, EngineError>;

    /// Settle a journal row toward `to` with a generation fence, then project
    /// the result onto the run row. Stale generations return `None`; a
    /// concurrent journal update that moved the fence is projected onto
    /// storage so it never stays stale.
    fn settle(
        &mut self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, EngineError>;

    /// Settle a journal row toward `to`, reporting the fence refusal typed.
    /// [`settle`](Self::settle) delegates: `Settled` maps to its row,
    /// `Rejected` to `None`.
    fn settle_checked(
        &mut self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<SettleOutcome, EngineError>;

    /// Claim a journal row (bumps generation), then project the fence onto
    /// the run row. A claim is a dispatch, so it clears any park. A stale
    /// fence returns `None` after projecting the latest journal row.
    fn claim(
        &mut self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, EngineError>;

    /// Claim a journal row, reporting the fence refusal typed.
    /// [`claim`](Self::claim) delegates: `Claimed` maps to its row,
    /// `Rejected` to `None`.
    fn claim_checked(
        &mut self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<ClaimOutcome, EngineError>;

    /// Count one provider attempt against the journal fence. Past
    /// `max_attempts` the row dead-letters to `Failed` and the run projects
    /// terminal (clearing any park). A stale fence returns `None`.
    fn attempt(&mut self, key: &str, generation: u64) -> Result<Option<Row>, EngineError>;

    /// Mark the run `Unknown` (uncertain outcome). Only [`reconcile`](Self::reconcile)
    /// may leave it.
    fn mark_unknown(&mut self, key: &str) -> Result<(), EngineError>;

    /// Step `Unknown -> ReconcilePending -> outcome` (or
    /// `ReconcilePending -> outcome`) and project onto storage, clearing
    /// parks when terminal. Runs only on `Unknown` / `ReconcilePending`
    /// rows with a live fence; anything else returns `None`.
    fn reconcile(
        &mut self,
        key: &str,
        generation: u64,
        outcome: EffectState,
    ) -> Result<Option<Row>, EngineError>;

    /// Expire a dead lease: when the journal row is `InFlight` / `Accepted`
    /// with `lease_deadline <= now_ms`, settle it to `Unknown` and project
    /// `RunPhase::Unknown` onto storage (clearing any parks). A live lease
    /// or a non-running row returns `None` without mutating anything.
    fn expire_lease(&mut self, key: &str, now_ms: u64) -> Result<Option<Row>, EngineError>;
}

fn law_fail(got: impl std::fmt::Debug) -> String {
    format!("{got:?}")
}

fn law_check(cond: bool, what: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.to_string()) }
}

/// Conformance laws for any [`DurableEngine`]. A sibling store runs these
/// against its own engine to verify the port in one call; each law uses a
/// fresh key so laws never interfere.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
pub fn engine_laws(engine: &mut dyn DurableEngine) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("start re-attaches", law_start_reattaches(engine)),
        ("query absent is NotFound", law_query_absent(engine)),
        ("park then wake resumes", law_park_wake(engine)),
        ("cancel is terminal", law_cancel_terminal(engine)),
        (
            "start_with honors custom options",
            law_start_with_opts(engine),
        ),
        ("attempt counts a retry", law_attempt_retry(engine)),
        ("settle completes", law_settle_completes(engine)),
        ("reconcile leaves Unknown", law_reconcile_unknown(engine)),
        ("claim fences", law_claim_fences(engine)),
        (
            "stale settle projects the live fence",
            law_stale_settle_projects(engine),
        ),
        (
            "expire_lease times out a dead lease",
            law_expire_lease(engine),
        ),
        (
            "wake_checked reports Woken vs NotParked",
            law_wake_checked(engine),
        ),
        (
            "claim_checked and settle_checked report typed rejections",
            law_checked_rejections(engine),
        ),
    ]
}

fn require_started(out: StartOutcome, what: &str) -> Result<u64, String> {
    match out {
        StartOutcome::Started { generation } => Ok(generation),
        StartOutcome::Already { status } => {
            Err(format!("{what}: expected Started, got Already({status:?})"))
        }
    }
}

fn law_start_reattaches(engine: &mut dyn DurableEngine) -> Result<(), String> {
    require_started(
        engine.start("law-start", 0).map_err(law_fail)?,
        "first start",
    )?;
    match engine.start("law-start", 0).map_err(law_fail)? {
        StartOutcome::Already { status } => {
            law_check(status.key == "law-start", "re-attach must keep the key")?;
            law_check(
                status.phase == RunPhase::Running,
                "re-attached run must be Running",
            )
        }
        StartOutcome::Started { .. } => Err("second start must not dispatch again".into()),
    }
}

fn law_query_absent(engine: &mut dyn DurableEngine) -> Result<(), String> {
    match engine.query("law-absent") {
        Err(EngineError::NotFound(key)) => {
            law_check(key == "law-absent", "NotFound must name the key")
        }
        other => Err(format!("absent query must be NotFound, got {other:?}")),
    }
}

fn law_park_wake(engine: &mut dyn DurableEngine) -> Result<(), String> {
    require_started(engine.start("law-park", 0).map_err(law_fail)?, "start")?;
    let out = engine
        .park_until("law-park", 1_000, 60, 0, None)
        .map_err(law_fail)?;
    law_check(matches!(out, ParkOutcome::Wait { .. }), "park must Wait")?;
    law_check(
        engine.query("law-park").map_err(law_fail)?.phase == RunPhase::Parked,
        "parked phase",
    )?;
    let woke = engine.wake("law-park", 1_000).map_err(law_fail)?;
    law_check(woke == Some(0), "on-time wake has no overdue")?;
    law_check(
        engine.query("law-park").map_err(law_fail)?.phase == RunPhase::Running,
        "wake resumes",
    )
}

fn law_cancel_terminal(engine: &mut dyn DurableEngine) -> Result<(), String> {
    require_started(engine.start("law-cancel", 0).map_err(law_fail)?, "start")?;
    engine.cancel("law-cancel", 0).map_err(law_fail)?;
    law_check(
        engine.query("law-cancel").map_err(law_fail)?.phase == RunPhase::Cancelled,
        "cancelled phase",
    )?;
    law_check(
        engine
            .query("law-cancel")
            .map_err(law_fail)?
            .phase
            .is_terminal(),
        "cancelled is terminal",
    )
}

fn law_start_with_opts(engine: &mut dyn DurableEngine) -> Result<(), String> {
    let opts = StartOptions {
        max_attempts: 3,
        lease_ms: 0,
        owner: "law-owner".into(),
    };
    require_started(
        engine.start_with("law-opts", 0, &opts).map_err(law_fail)?,
        "start_with",
    )?;
    law_check(
        engine.query("law-opts").map_err(law_fail)?.phase == RunPhase::Running,
        "custom start runs",
    )
}

fn law_attempt_retry(engine: &mut dyn DurableEngine) -> Result<(), String> {
    let generation = require_started(engine.start("law-attempt", 0).map_err(law_fail)?, "start")?;
    let row = engine
        .attempt("law-attempt", generation)
        .map_err(law_fail)?
        .ok_or("live attempt must return a row")?;
    law_check(row.attempts == 1, "one attempt counted")?;
    law_check(
        engine.query("law-attempt").map_err(law_fail)?.phase == RunPhase::Running,
        "retry stays Running",
    )
}

fn law_settle_completes(engine: &mut dyn DurableEngine) -> Result<(), String> {
    let generation = require_started(engine.start("law-settle", 0).map_err(law_fail)?, "start")?;
    let row = engine
        .settle("law-settle", generation, EffectState::Settled)
        .map_err(law_fail)?
        .ok_or("live settle must return a row")?;
    law_check(row.state == EffectState::Settled, "journal settled")?;
    law_check(
        engine.query("law-settle").map_err(law_fail)?.phase == RunPhase::Settled,
        "run settled",
    )?;
    law_check(
        engine
            .settle("law-settle", generation, EffectState::Failed)
            .map_err(law_fail)?
            .is_none(),
        "terminal settle is refused",
    )
}

fn law_reconcile_unknown(engine: &mut dyn DurableEngine) -> Result<(), String> {
    let generation = require_started(engine.start("law-reconcile", 0).map_err(law_fail)?, "start")?;
    engine.mark_unknown("law-reconcile").map_err(law_fail)?;
    let row = engine
        .reconcile("law-reconcile", generation, EffectState::Settled)
        .map_err(law_fail)?
        .ok_or("reconcile must return a row")?;
    law_check(row.state == EffectState::Settled, "reconciled to Settled")?;
    law_check(
        engine.query("law-reconcile").map_err(law_fail)?.phase == RunPhase::Settled,
        "run settled after reconcile",
    )
}

fn law_claim_fences(engine: &mut dyn DurableEngine) -> Result<(), String> {
    let generation = require_started(engine.start("law-claim", 0).map_err(law_fail)?, "start")?;
    let row = engine
        .claim("law-claim", "worker", 10_000, 0, generation)
        .map_err(law_fail)?
        .ok_or("live claim must return a row")?;
    law_check(row.generation > generation, "claim bumps the fence")?;
    law_check(
        engine
            .claim("law-claim", "zombie", 10_000, 0, generation)
            .map_err(law_fail)?
            .is_none(),
        "stale claim is refused",
    )
}

fn law_expire_lease(engine: &mut dyn DurableEngine) -> Result<(), String> {
    // A live lease refuses: no mutation, still Running.
    let live_opts = StartOptions {
        max_attempts: 8,
        lease_ms: 10_000,
        owner: "law-expire".into(),
    };
    require_started(
        engine
            .start_with("law-expire-live", 0, &live_opts)
            .map_err(law_fail)?,
        "live start",
    )?;
    law_check(
        engine
            .expire_lease("law-expire-live", 0)
            .map_err(law_fail)?
            .is_none(),
        "live lease must refuse expire_lease",
    )?;
    law_check(
        engine.query("law-expire-live").map_err(law_fail)?.phase == RunPhase::Running,
        "refused expire keeps the run Running",
    )?;
    // An expired lease moves InFlight to Unknown, clearing the park.
    require_started(
        engine.start("law-expire-dead", 0).map_err(law_fail)?,
        "start",
    )?;
    let _ = engine
        .park_until("law-expire-dead", 1_000, 60, 0, None)
        .map_err(law_fail)?;
    let updated = engine
        .expire_lease("law-expire-dead", 0)
        .map_err(law_fail)?
        .ok_or("expired lease must transition to Unknown")?;
    law_check(
        updated.state == EffectState::Unknown,
        "expired lease settles the journal to Unknown",
    )?;
    law_check(
        engine.query("law-expire-dead").map_err(law_fail)?.phase == RunPhase::Unknown,
        "storage projects Unknown after expire",
    )?;
    law_check(
        engine.parked().map_err(law_fail)?.is_empty(),
        "expire clears the park",
    )?;
    // Reconcile recovers cleanly to Settled.
    let row = engine
        .reconcile("law-expire-dead", updated.generation, EffectState::Settled)
        .map_err(law_fail)?
        .ok_or("reconcile must return a row after expire")?;
    law_check(row.state == EffectState::Settled, "reconciled to Settled")?;
    law_check(
        engine.query("law-expire-dead").map_err(law_fail)?.phase == RunPhase::Settled,
        "run settled after reconcile",
    )
}

fn law_wake_checked(engine: &mut dyn DurableEngine) -> Result<(), String> {
    require_started(engine.start("law-wakecheck", 0).map_err(law_fail)?, "start")?;
    // Live but never parked: NotParked names the Running phase.
    match engine.wake_checked("law-wakecheck", 0).map_err(law_fail)? {
        WakeOutcome::NotParked { phase } => {
            law_check(phase == RunPhase::Running, "live run reports Running")?
        }
        WakeOutcome::Woken { .. } => return Err("unparked wake must be NotParked".into()),
    }
    let out = engine
        .park_until("law-wakecheck", 1_000, 60, 0, None)
        .map_err(law_fail)?;
    law_check(matches!(out, ParkOutcome::Wait { .. }), "park must Wait")?;
    match engine
        .wake_checked("law-wakecheck", 1_200)
        .map_err(law_fail)?
    {
        WakeOutcome::Woken {
            overdue_ms,
            park_generation,
        } => {
            law_check(overdue_ms == 200, "overdue names the gap")?;
            law_check(park_generation >= 1, "park generation is live")?;
        }
        WakeOutcome::NotParked { .. } => return Err("parked wake must be Woken".into()),
    }
    law_check(
        engine.query("law-wakecheck").map_err(law_fail)?.phase == RunPhase::Running,
        "wake resumes",
    )
}

fn law_checked_rejections(engine: &mut dyn DurableEngine) -> Result<(), String> {
    // Absent rows reject as NotFound on both verbs.
    law_check(
        matches!(
            engine
                .claim_checked("law-absent-claim", "w", 10_000, 0, 0)
                .map_err(law_fail)?,
            ClaimOutcome::Rejected(ClaimRejection::NotFound)
        ),
        "absent claim rejects NotFound",
    )?;
    law_check(
        matches!(
            engine
                .settle_checked("law-absent-settle", 0, EffectState::Settled)
                .map_err(law_fail)?,
            SettleOutcome::Rejected(SettleRejection::NotFound)
        ),
        "absent settle rejects NotFound",
    )?;
    // Live claim succeeds; a zombie fence rejects stale.
    let generation = require_started(engine.start("law-checked", 0).map_err(law_fail)?, "start")?;
    let claimed = match engine
        .claim_checked("law-checked", "worker", 10_000, 0, generation)
        .map_err(law_fail)?
    {
        ClaimOutcome::Claimed(row) => row,
        ClaimOutcome::Rejected(r) => return Err(format!("live claim must succeed: {r:?}")),
    };
    law_check(
        matches!(
            engine
                .claim_checked("law-checked", "zombie", 10_000, 0, generation)
                .map_err(law_fail)?,
            ClaimOutcome::Rejected(ClaimRejection::StaleGeneration { .. })
        ),
        "zombie claim rejects StaleGeneration",
    )?;
    // Live settle succeeds; terminal rows reject terminal.
    match engine
        .settle_checked("law-checked", claimed.generation, EffectState::Settled)
        .map_err(law_fail)?
    {
        SettleOutcome::Settled(row) => {
            law_check(row.state == EffectState::Settled, "journal settled")?
        }
        SettleOutcome::Rejected(r) => return Err(format!("live settle must succeed: {r:?}")),
    }
    law_check(
        matches!(
            engine
                .settle_checked("law-checked", claimed.generation, EffectState::Failed)
                .map_err(law_fail)?,
            SettleOutcome::Rejected(SettleRejection::Terminal { .. })
        ),
        "terminal settle rejects Terminal",
    )
}

fn law_stale_settle_projects(engine: &mut dyn DurableEngine) -> Result<(), String> {
    let generation = require_started(engine.start("law-stale", 0).map_err(law_fail)?, "start")?;
    let claimed = engine
        .claim("law-stale", "worker", 10_000, 0, generation)
        .map_err(law_fail)?
        .ok_or("claim must return a row")?;
    law_check(
        engine
            .settle("law-stale", generation, EffectState::Settled)
            .map_err(law_fail)?
            .is_none(),
        "stale settle is refused",
    )?;
    let status = engine.query("law-stale").map_err(law_fail)?;
    law_check(
        status.generation == claimed.generation,
        "storage carries the live fence after a stale settle",
    )
}
