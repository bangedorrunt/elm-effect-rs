//! Persistence for runs and parks. In-memory ships here; toron binds its store.

use crate::engine::{RunPhase, StorageError};

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
    /// Attempts recorded on the journal row (0 before the first dispatch).
    pub attempt: i64,
}

/// Host-persisted engine state. Prefer implementing this on toron's
/// `WorkflowEffectStore` rather than opening SQLite inside this crate
/// (ADR-0002 journal section; ADR-0005).
pub trait EngineStorage {
    /// Read one run.
    fn get_run(&self, key: &str) -> Result<Option<RunRow>, StorageError>;
    /// Insert or replace a run row.
    fn put_run(&mut self, row: RunRow) -> Result<(), StorageError>;
    /// All parked rows.
    fn list_parked(&self) -> Result<Vec<ParkedRow>, StorageError>;
    /// The active park for `key`, if any. A host that indexes parks by key
    /// overrides the default scan.
    fn get_park(&self, key: &str) -> Result<Option<ParkedRow>, StorageError> {
        Ok(self.list_parked()?.into_iter().find(|park| park.key == key))
    }
    /// Parks due at `now_ms`. A host with a time index overrides the default
    /// scan; the pure selection still runs in [`due_wakes`](crate::due_wakes).
    fn due_parks(&self, now_ms: u64) -> Result<Vec<ParkedRow>, StorageError> {
        Ok(self
            .list_parked()?
            .into_iter()
            .filter(|park| park.until_ms <= now_ms)
            .collect())
    }
    /// Upsert a park. Any other park for the same key is replaced (one active
    /// park per run). Returns `true` when written, `false` when the identical
    /// `(key, until_ms)` row already existed.
    fn upsert_park(&mut self, row: ParkedRow) -> Result<bool, StorageError>;
    /// Remove parks for `key` (any generation).
    fn clear_parks(&mut self, key: &str) -> Result<(), StorageError>;
    /// Persist a run row and drop any park for it as one unit.
    ///
    /// The default writes the run then clears the park. A host with a
    /// transaction should override this. A crash can leave a stale park beside
    /// the new row (the drain clears it), never a run without its park.
    fn commit_run(&mut self, run: RunRow) -> Result<(), StorageError> {
        let key = run.key.clone();
        self.put_run(run)?;
        self.clear_parks(&key)
    }
    /// Persist the run row and its park as one unit. The pair replaces any
    /// other park for the same key.
    ///
    /// The default writes the park then the run. A host with a transaction
    /// should override this. A crash can leave a park beside the old row (the
    /// drain clears it), never a parked run without its wake row.
    fn commit_park(&mut self, run: RunRow, park: ParkedRow) -> Result<(), StorageError> {
        self.upsert_park(park)?;
        self.put_run(run)
    }
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
    fn get_run(&self, key: &str) -> Result<Option<RunRow>, StorageError> {
        Ok(self.runs.get(key).cloned())
    }

    fn put_run(&mut self, row: RunRow) -> Result<(), StorageError> {
        self.runs.insert(row.key.clone(), row);
        Ok(())
    }

    fn list_parked(&self) -> Result<Vec<ParkedRow>, StorageError> {
        Ok(self.parks.values().cloned().collect())
    }

    fn upsert_park(&mut self, row: ParkedRow) -> Result<bool, StorageError> {
        let k = (row.key.clone(), row.until_ms);
        let existed = self.parks.contains_key(&k);
        // Keep the identical row, drop any other instant for this key.
        self.parks
            .retain(|(key, until), _| key != &row.key || *until == row.until_ms);
        self.parks.insert(k, row);
        Ok(!existed)
    }

    fn clear_parks(&mut self, key: &str) -> Result<(), StorageError> {
        self.parks.retain(|(k, _), _| k != key);
        Ok(())
    }
}
