//! Journal store (ADR-0002 Phase 9, claim/settle/offsets).
//!
//! A [`JournalStore`] records intent *before* the provider runs, keyed by
//! a stable key so retries reuse the same row. Every method is fallible:
//! a host store returns [`JournalError`], while [`InMemoryStore`] never
//! fails. Claiming sets owner, lease
//! deadline, and `generation + 1` atomically and returns the row;
//! settling carries the generation and a stale generation settles
//! nothing. An attempt carries the generation too, so a zombie that lost
//! its lease cannot spend the live holder's budget or dead-letter its row.
//! Terminal states never move. `Accepted` advances only through
//! explicit transitions. `Unknown` has no default exit: settling names an
//! explicit target and the golden table admits only
//! `Settled`/`Failed`/`Unknown`/`Cancelled` from there. Exhausted attempts
//! become a dead-letter row: [`EffectState::Failed`] plus
//! [`Disposition::TerminalFailed`], matching toron's column (no parallel
//! invention). The next offset and the state derived from it commit in one
//! lock step; offsets move only forward. Reading below retention returns
//! [`Read::Gone`] with the first replayable offset; callers pass `-1` for
//! latest or an explicit offset, never a silent skip.
use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::EffectState;

/// A host journal could not complete an operation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// The journal is temporarily unavailable; a retry may succeed.
    #[error("journal unavailable: {0}")]
    Unavailable(String),
    /// The journal returned data that violates its contract.
    #[error("journal corrupt: {0}")]
    Corrupt(String),
    /// The journal rejected the operation.
    #[error("journal rejected: {0}")]
    Rejected(String),
}

/// Graph-admission disposition of a dead row, mirroring toron's column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Disposition {
    /// Keep going.
    Continue,
    /// Retry the step.
    Retry,
    /// Route elsewhere.
    Reroute,
    /// Blocked on an external condition.
    Blocked,
    /// Terminally failed (dead letters land here).
    TerminalFailed,
    /// Cancelled.
    Cancelled,
    /// Needs reconciliation.
    Reconcile,
}

impl Disposition {
    /// Canonical wire spelling, as toron has it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Retry => "retry",
            Self::Reroute => "reroute",
            Self::Blocked => "blocked",
            Self::TerminalFailed => "terminal_failed",
            Self::Cancelled => "cancelled",
            Self::Reconcile => "reconcile",
        }
    }
}

impl std::fmt::Display for Disposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Disposition {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "continue" => Ok(Self::Continue),
            "retry" => Ok(Self::Retry),
            "reroute" => Ok(Self::Reroute),
            "blocked" => Ok(Self::Blocked),
            "terminal_failed" => Ok(Self::TerminalFailed),
            "cancelled" => Ok(Self::Cancelled),
            "reconcile" => Ok(Self::Reconcile),
            _ => Err(format!("unknown disposition: {s}")),
        }
    }
}

/// One journal row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Row {
    /// Stable intent key (retries reuse it).
    pub key: String,
    /// Current state.
    pub state: EffectState,
    /// Claiming owner, if claimed.
    pub owner: Option<String>,
    /// Lease deadline (millis), if claimed.
    pub lease_deadline: Option<i64>,
    /// Fence counter, bumped per claim.
    pub generation: u64,
    /// Log offset of the last committed change.
    pub offset: i64,
    /// Dead-letter disposition, if dead.
    pub disposition: Option<Disposition>,
    /// Provider attempts so far.
    pub attempts: u32,
    /// Attempts allowed before dead-lettering.
    pub max_attempts: u32,
}

/// Offset read outcome.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Read {
    /// Entry at this offset.
    Entry {
        /// Offset read.
        offset: i64,
        /// Row snapshot.
        row: Row,
    },
    /// Offset fell below retention; start here instead.
    Gone {
        /// First replayable offset.
        first_replayable: i64,
    },
}

/// Durable journal contract.
pub trait JournalStore {
    /// Record intent (idempotent on key); returns the row.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the host store cannot read or write.
    fn record_intent(&self, key: &str, max_attempts: u32) -> Result<Row, JournalError>;
    /// Claim: owner + deadline + generation+1, atomically; returns the row.
    /// Fenced like settle: terminal rows never move, a stale
    /// `expected_generation` claims nothing, and a live lease under
    /// another owner is never stolen — reclaim only after expiry
    /// (`lease_deadline <= now_ms`) or by the same owner renewing.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the host store cannot read or write.
    fn claim(
        &self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, JournalError>;
    /// Settle toward `to` carrying `generation`; stale generations,
    /// illegal edges, and terminal rows settle nothing.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the host store cannot read or write.
    fn settle(
        &self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, JournalError>;
    /// Note one provider attempt for the holder of `generation`; a stale
    /// generation, or a terminal row, notes nothing. Fenced like
    /// [`settle`](Self::settle), so a zombie that lost its lease cannot
    /// dead-letter the row a live worker now holds. Exhaustion
    /// dead-letters the row.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the host store cannot read or write.
    fn attempt(&self, key: &str, generation: u64) -> Result<Option<Row>, JournalError>;
    /// Read one offset (`-1` means latest).
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the host store cannot read.
    fn read(&self, offset: i64) -> Result<Option<Read>, JournalError>;
    /// Current row for a key, if any.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the host store cannot read.
    fn get(&self, key: &str) -> Result<Option<Row>, JournalError>;
}

/// In-memory journal: one lock commits offset and state together.
pub struct InMemoryStore {
    inner: Mutex<Inner>,
    retention: usize,
}

struct Inner {
    rows: BTreeMap<String, Row>,
    /// Offset and the row as it was committed there. The row is kept whole,
    /// not its state: a read of an old offset must answer what the row was
    /// then, not what it has become since.
    log: Vec<(i64, Row)>,
    next_offset: i64,
}

impl InMemoryStore {
    /// Build keeping the last `retention` offset entries.
    #[must_use]
    pub fn new(retention: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                rows: BTreeMap::new(),
                log: Vec::new(),
                next_offset: 0,
            }),
            retention: retention.max(1),
        }
    }

    fn commit(&self, inner: &mut Inner, key: &str, row: &Row) {
        let offset = inner.next_offset;
        inner.next_offset += 1;
        let mut row = row.clone();
        row.offset = offset;
        inner.rows.insert(key.to_string(), row.clone());
        inner.log.push((offset, row));
        while inner.log.len() > self.retention {
            inner.log.remove(0);
        }
    }

    fn first_replayable(inner: &Inner) -> i64 {
        inner.log.first().map(|(o, _)| *o).unwrap_or(0)
    }
}

impl JournalStore for InMemoryStore {
    fn record_intent(&self, key: &str, max_attempts: u32) -> Result<Row, JournalError> {
        let mut inner = self.inner.lock().expect("journal lock poisoned");
        if let Some(row) = inner.rows.get(key) {
            return Ok(row.clone());
        }
        let row = Row {
            key: key.to_string(),
            state: EffectState::Prepared,
            owner: None,
            lease_deadline: None,
            generation: 0,
            offset: -1,
            disposition: None,
            attempts: 0,
            max_attempts,
        };
        self.commit(&mut inner, key, &row);
        Ok(inner.rows[key].clone())
    }

    fn claim(
        &self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, JournalError> {
        let mut inner = self.inner.lock().expect("journal lock poisoned");
        let Some(row) = inner.rows.get(key).cloned() else {
            return Ok(None);
        };
        if !super::plan::may_claim(
            row.state,
            row.owner.as_deref(),
            row.lease_deadline,
            owner,
            now_ms,
            expected_generation,
            row.generation,
        ) {
            return Ok(None);
        }
        let mut row = row;
        row.owner = Some(owner.to_string());
        row.lease_deadline = Some(lease_deadline);
        row.generation += 1;
        self.commit(&mut inner, key, &row);
        Ok(Some(inner.rows[key].clone()))
    }

    fn settle(
        &self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, JournalError> {
        let mut inner = self.inner.lock().expect("journal lock poisoned");
        let Some(row) = inner.rows.get(key).cloned() else {
            return Ok(None);
        };
        if super::plan::check_settle(Some((row.state, row.generation)), generation, to).is_err() {
            return Ok(None);
        }
        let mut next = row.clone();
        next.state = to;
        self.commit(&mut inner, key, &next);
        Ok(Some(inner.rows[key].clone()))
    }

    fn attempt(&self, key: &str, generation: u64) -> Result<Option<Row>, JournalError> {
        let mut inner = self.inner.lock().expect("journal lock poisoned");
        let Some(mut row) = inner.rows.get(key).cloned() else {
            return Ok(None);
        };
        // Fencing beats counting: a worker whose lease was taken over must
        // not be able to spend the new holder's attempt budget.
        if row.generation != generation || row.state.is_terminal() {
            return Ok(None);
        }
        row.attempts += 1;
        if row.attempts > row.max_attempts {
            row.state = EffectState::Failed;
            row.disposition = Some(Disposition::TerminalFailed);
        }
        self.commit(&mut inner, key, &row);
        Ok(Some(inner.rows[key].clone()))
    }

    fn read(&self, offset: i64) -> Result<Option<Read>, JournalError> {
        let inner = self.inner.lock().expect("journal lock poisoned");
        if offset == -1 {
            let Some((o, row)) = inner.log.last() else {
                return Ok(None);
            };
            return Ok(Some(Read::Entry {
                offset: *o,
                row: row.clone(),
            }));
        }
        let first = Self::first_replayable(&inner);
        if offset < first {
            return Ok(Some(Read::Gone {
                first_replayable: first,
            }));
        }
        Ok(inner
            .log
            .iter()
            .find(|(o, _)| *o == offset)
            .map(|(o, row)| Read::Entry {
                offset: *o,
                row: row.clone(),
            }))
    }

    fn get(&self, key: &str) -> Result<Option<Row>, JournalError> {
        Ok(self
            .inner
            .lock()
            .expect("journal lock poisoned")
            .rows
            .get(key)
            .cloned())
    }
}
