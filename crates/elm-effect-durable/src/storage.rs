//! Persistence for runs and parks. In-memory ships here; toron binds its store.

use crate::engine::RunPhase;

/// One parked wait. Idempotent on `(key, until_ms)` at the engine layer:
/// parking the same key to the same instant keeps one row (ADR-0005 /
/// journal pending rule).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkedRow {
    /// Run / journal key.
    pub key: String,
    /// Absolute wake time (ms).
    pub until_ms: u64,
    /// Park generation (bumps when re-parking to a *different* instant).
    pub park_generation: u64,
}

/// Durable run metadata the engine stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRow {
    /// Run / journal / producer key.
    pub key: String,
    /// Coarse phase.
    pub phase: RunPhase,
    /// Fence generation (claim/settle).
    pub generation: u64,
    /// When [`RunPhase::Parked`], the absolute wake time.
    pub until_ms: Option<u64>,
    /// Attempt counter for dispatch planning.
    pub attempt: i64,
}

/// Host-persisted engine state. Prefer implementing this on toron's
/// `WorkflowEffectStore` rather than opening SQLite inside this crate
/// (ADR-0002 journal section; ADR-0005).
pub trait EngineStorage {
    /// Read one run.
    fn get_run(&self, key: &str) -> Option<RunRow>;
    /// Insert or replace a run row.
    fn put_run(&mut self, row: RunRow);
    /// All parked rows.
    fn list_parked(&self) -> Vec<ParkedRow>;
    /// Upsert a park. Returns `true` when written, `false` when an identical
    /// `(key, until_ms)` row already existed (one row rule).
    fn upsert_park(&mut self, row: ParkedRow) -> bool;
    /// Remove parks for `key` (any generation).
    fn clear_parks(&mut self, key: &str);
}

/// In-memory storage for tests and single-process hosts.
#[derive(Clone, Debug, Default)]
pub struct MemStorage {
    runs: std::collections::BTreeMap<String, RunRow>,
    /// Keyed by `(key, until_ms)` so the same instant is one row.
    parks: std::collections::BTreeMap<(String, u64), ParkedRow>,
}

impl MemStorage {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl EngineStorage for MemStorage {
    fn get_run(&self, key: &str) -> Option<RunRow> {
        self.runs.get(key).cloned()
    }

    fn put_run(&mut self, row: RunRow) {
        self.runs.insert(row.key.clone(), row);
    }

    fn list_parked(&self) -> Vec<ParkedRow> {
        self.parks.values().cloned().collect()
    }

    fn upsert_park(&mut self, row: ParkedRow) -> bool {
        let k = (row.key.clone(), row.until_ms);
        if self.parks.contains_key(&k) {
            return false;
        }
        // Drop any other park for this key (one active park per run).
        self.parks.retain(|(key, _), _| key != &row.key);
        self.parks.insert(k, row);
        true
    }

    fn clear_parks(&mut self, key: &str) {
        self.parks.retain(|(k, _), _| k != key);
    }
}
