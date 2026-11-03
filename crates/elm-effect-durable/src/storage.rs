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

/// Conformance laws for any [`EngineStorage`]. A sibling host runs these
/// against its own store to verify the contract in one call.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
pub fn storage_laws(store: &mut dyn EngineStorage) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("put then get roundtrips", law_put_get(store)),
        ("get absent is None", law_get_absent(store)),
        ("same instant parks once", law_same_instant(store)),
        ("repark replaces the instant", law_repark(store)),
        ("commit_run clears the park", law_commit_run(store)),
        ("commit_park persists the pair", law_commit_park(store)),
        ("clear_parks removes", law_clear_parks(store)),
        ("due_parks filters the future", law_due_parks(store)),
    ]
}

fn law_ok(cond: bool, what: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.to_string()) }
}

fn law_err(got: impl std::fmt::Debug) -> String {
    format!("{got:?}")
}

fn law_put_get(store: &mut dyn EngineStorage) -> Result<(), String> {
    store
        .put_run(RunRow {
            key: "law-rw".into(),
            phase: RunPhase::Running,
            generation: 1,
            until_ms: None,
            attempt: 0,
        })
        .map_err(law_err)?;
    let row = store
        .get_run("law-rw")
        .map_err(law_err)?
        .ok_or("put row must read back")?;
    law_ok(
        row.generation == 1 && row.phase == RunPhase::Running,
        "row survives the roundtrip",
    )
}

fn law_get_absent(store: &mut dyn EngineStorage) -> Result<(), String> {
    law_ok(
        store.get_run("law-nope").map_err(law_err)?.is_none(),
        "absent run must be None",
    )
}

fn law_same_instant(store: &mut dyn EngineStorage) -> Result<(), String> {
    let park = ParkedRow {
        key: "law-park".into(),
        until_ms: 500,
        park_generation: 1,
    };
    let first = store.upsert_park(park.clone()).map_err(law_err)?;
    law_ok(first, "first park writes")?;
    let second = store.upsert_park(park).map_err(law_err)?;
    law_ok(!second, "identical park writes nothing the second time")?;
    let parks = store.list_parked().map_err(law_err)?;
    law_ok(
        parks.iter().filter(|p| p.key == "law-park").count() == 1,
        "one row for the same instant",
    )
}

fn law_repark(store: &mut dyn EngineStorage) -> Result<(), String> {
    store
        .upsert_park(ParkedRow {
            key: "law-repark".into(),
            until_ms: 100,
            park_generation: 1,
        })
        .map_err(law_err)?;
    store
        .upsert_park(ParkedRow {
            key: "law-repark".into(),
            until_ms: 200,
            park_generation: 2,
        })
        .map_err(law_err)?;
    let parks = store.list_parked().map_err(law_err)?;
    let mine: Vec<_> = parks.iter().filter(|p| p.key == "law-repark").collect();
    law_ok(
        mine.len() == 1 && mine[0].until_ms == 200,
        "one active park per run",
    )
}

fn law_commit_run(store: &mut dyn EngineStorage) -> Result<(), String> {
    store
        .put_run(RunRow {
            key: "law-cr".into(),
            phase: RunPhase::Parked,
            generation: 1,
            until_ms: Some(100),
            attempt: 0,
        })
        .map_err(law_err)?;
    store
        .upsert_park(ParkedRow {
            key: "law-cr".into(),
            until_ms: 100,
            park_generation: 1,
        })
        .map_err(law_err)?;
    store
        .commit_run(RunRow {
            key: "law-cr".into(),
            phase: RunPhase::Running,
            generation: 2,
            until_ms: None,
            attempt: 0,
        })
        .map_err(law_err)?;
    law_ok(
        store.get_park("law-cr").map_err(law_err)?.is_none(),
        "active transition drops the park",
    )
}

fn law_commit_park(store: &mut dyn EngineStorage) -> Result<(), String> {
    store
        .commit_park(
            RunRow {
                key: "law-cp".into(),
                phase: RunPhase::Parked,
                generation: 1,
                until_ms: Some(300),
                attempt: 0,
            },
            ParkedRow {
                key: "law-cp".into(),
                until_ms: 300,
                park_generation: 1,
            },
        )
        .map_err(law_err)?;
    law_ok(
        store.get_park("law-cp").map_err(law_err)?.is_some(),
        "park beside the run",
    )?;
    law_ok(
        store.get_run("law-cp").map_err(law_err)?.is_some(),
        "run beside the park",
    )
}

fn law_clear_parks(store: &mut dyn EngineStorage) -> Result<(), String> {
    store
        .upsert_park(ParkedRow {
            key: "law-cl".into(),
            until_ms: 400,
            park_generation: 1,
        })
        .map_err(law_err)?;
    store.clear_parks("law-cl").map_err(law_err)?;
    law_ok(
        store.get_park("law-cl").map_err(law_err)?.is_none(),
        "park gone after clear",
    )
}

fn law_due_parks(store: &mut dyn EngineStorage) -> Result<(), String> {
    store
        .upsert_park(ParkedRow {
            key: "law-due".into(),
            until_ms: 100,
            park_generation: 1,
        })
        .map_err(law_err)?;
    store
        .upsert_park(ParkedRow {
            key: "law-future".into(),
            until_ms: 9_999,
            park_generation: 1,
        })
        .map_err(law_err)?;
    let due = store.due_parks(200).map_err(law_err)?;
    law_ok(due.iter().any(|p| p.key == "law-due"), "due park fires")?;
    law_ok(
        due.iter().all(|p| p.key != "law-future"),
        "future park waits",
    )
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
