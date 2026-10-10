//! Journal store (ADR-0002 Phase 9, claim/settle/offsets).
//!
//! A [`JournalStore`] records intent *before* the provider runs, keyed by
//! a stable key so retries reuse the same row. Claiming sets owner, lease
//! deadline, and `generation + 1` atomically and returns the row;
//! settling carries the generation and a stale generation settles
//! nothing. Terminal states never move. `Accepted` advances only through
//! explicit transitions. `Unknown` has no default exit: settling names an
//! explicit target and the golden table admits only
//! `Settled`/`Failed`/`Unknown`/`Cancelled` from there. Exhausted attempts
//! become a dead-letter row: [`EffectState::Failed`] plus
//! [`Disposition::TerminalFailed`], matching toron's column (no parallel
//! invention). The next offset and the state derived from it commit in one
//! lock step; offsets move only forward. Reading below retention returns
//! [`Read::Gone`] with the first replayable offset; callers pass `-1` for
//! latest or an explicit offset, never a silent skip.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{EffectState, can_transition};

/// Graph-admission disposition of a dead row, mirroring toron's column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// One journal row.
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
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
    fn record_intent(&self, key: &str, max_attempts: u32) -> Row;
    /// Claim: owner + deadline + generation+1, atomically; returns the row.
    fn claim(&self, key: &str, owner: &str, lease_deadline: i64) -> Option<Row>;
    /// Settle toward `to` carrying `generation`; stale generations,
    /// illegal edges, and terminal rows settle nothing.
    fn settle(&self, key: &str, generation: u64, to: EffectState) -> Option<Row>;
    /// Note one provider attempt; exhaustion dead-letters the row.
    fn attempt(&self, key: &str) -> Option<Row>;
    /// Read one offset (`-1` means latest).
    fn read(&self, offset: i64) -> Option<Read>;
    /// Current row for a key, if any.
    fn get(&self, key: &str) -> Option<Row>;
}

/// In-memory journal: one lock commits offset and state together.
pub struct InMemoryStore {
    inner: Mutex<Inner>,
    retention: usize,
}

struct Inner {
    rows: BTreeMap<String, Row>,
    log: Vec<(i64, String, EffectState)>,
    next_offset: i64,
}

impl InMemoryStore {
    /// Build keeping the last `retention` offset entries.
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
        inner.rows.insert(key.to_string(), row);
        inner
            .log
            .push((offset, key.to_string(), inner.rows[key].state));
        while inner.log.len() > self.retention {
            inner.log.remove(0);
        }
    }

    fn first_replayable(inner: &Inner) -> i64 {
        inner.log.first().map(|(o, _, _)| *o).unwrap_or(0)
    }
}

impl JournalStore for InMemoryStore {
    fn record_intent(&self, key: &str, max_attempts: u32) -> Row {
        let mut inner = self.inner.lock().unwrap();
        if let Some(row) = inner.rows.get(key) {
            return row.clone();
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
        inner.rows[key].clone()
    }

    fn claim(&self, key: &str, owner: &str, lease_deadline: i64) -> Option<Row> {
        let mut inner = self.inner.lock().unwrap();
        let mut row = inner.rows.get(key)?.clone();
        row.owner = Some(owner.to_string());
        row.lease_deadline = Some(lease_deadline);
        row.generation += 1;
        self.commit(&mut inner, key, &row);
        Some(inner.rows[key].clone())
    }

    fn settle(&self, key: &str, generation: u64, to: EffectState) -> Option<Row> {
        let mut inner = self.inner.lock().unwrap();
        let row = inner.rows.get(key)?.clone();
        if row.generation != generation {
            return None;
        }
        if row.state.is_terminal() {
            return None;
        }
        if !can_transition(row.state, to) {
            return None;
        }
        let mut next = row.clone();
        next.state = to;
        self.commit(&mut inner, key, &next);
        Some(inner.rows[key].clone())
    }

    fn attempt(&self, key: &str) -> Option<Row> {
        let mut inner = self.inner.lock().unwrap();
        let mut row = inner.rows.get(key)?.clone();
        row.attempts += 1;
        if row.attempts > row.max_attempts && !row.state.is_terminal() {
            row.state = EffectState::Failed;
            row.disposition = Some(Disposition::TerminalFailed);
        }
        self.commit(&mut inner, key, &row);
        Some(inner.rows[key].clone())
    }

    fn read(&self, offset: i64) -> Option<Read> {
        let inner = self.inner.lock().unwrap();
        if offset == -1 {
            let (o, k, _) = inner.log.last()?;
            return Some(Read::Entry {
                offset: *o,
                row: inner.rows[k].clone(),
            });
        }
        let first = Self::first_replayable(&inner);
        if offset < first {
            return Some(Read::Gone {
                first_replayable: first,
            });
        }
        inner
            .log
            .iter()
            .find(|(o, _, _)| *o == offset)
            .map(|(o, k, _)| Read::Entry {
                offset: *o,
                row: inner.rows[k].clone(),
            })
    }

    fn get(&self, key: &str) -> Option<Row> {
        self.inner.lock().unwrap().rows.get(key).cloned()
    }
}
