//! Port types: scheduling verbs only (ADR-0005).

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
    /// The journal refused an internal transition (an engine invariant).
    #[error("journal: {0}")]
    Journal(String),
    /// The host store failed.
    #[error("{0}")]
    Storage(#[from] StorageError),
}

/// Coarse phase a [`query`](DurableEngine::query) reports.
///
/// Maps onto journal vocabulary without replacing it: `Parked` is resumable
/// (not terminal); `Cancelled` / `Settled` / `Failed` are terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Settled | Self::Failed)
    }
}

impl std::fmt::Display for RunPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Running => "running",
            Self::Parked => "parked",
            Self::Cancelled => "cancelled",
            Self::Settled => "settled",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        };
        f.write_str(s)
    }
}

/// Snapshot returned by [`DurableEngine::query`].
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Scheduling port. Journal and `DurableLog` stay elsewhere.
pub trait DurableEngine {
    /// Begin or re-attach a run. Idempotent on `key`: a second start does not
    /// dispatch again.
    fn start(&mut self, key: &str, now_ms: u64) -> Result<StartOutcome, EngineError>;

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

    /// Terminal cancel.
    fn cancel(&mut self, key: &str, now_ms: u64) -> Result<(), EngineError>;

    /// Read status; `NotFound` when absent.
    fn query(&self, key: &str) -> Result<RunStatus, EngineError>;

    /// Drain parks whose `until_ms <= now_ms` into wakes. Pure selection is
    /// [`crate::due_wakes`]; this applies them.
    fn drain_due(&mut self, now_ms: u64) -> Result<Vec<crate::EngineMsg>, EngineError>;

    /// Parked rows currently stored (for tests and host inspection).
    fn parked(&self) -> Result<Vec<ParkedRow>, EngineError>;
}
