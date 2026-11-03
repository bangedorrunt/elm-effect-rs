//! Persistence for runs and parks. In-memory ships here; toron binds its store.

use crate::engine::{RunPhase, StorageError};

/// One parked wait. Idempotent on `(key, until_ms)` at the engine layer:
/// parking the same key to the same instant keeps one row (ADR-0005 /
/// journal pending rule).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ParkedRow {
    /// Run / journal key.
    pub key: String,
    /// Absolute wake time (ms).
    pub until_ms: u64,
    /// Park generation (bumps when re-parking to a *different* instant).
    pub park_generation: u64,
}

/// Durable run metadata the engine stores.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

/// One latched signal. `payload` is an opaque host-serialized string; the
/// engine never parses it. `seq` orders signals per key (`max + 1` at latch).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SignalRow {
    /// Run / journal key.
    pub key: String,
    /// Host-chosen id; a late duplicate never re-latches.
    pub signal_id: String,
    /// Opaque host-serialized payload.
    pub payload: String,
    /// Per-key latch order, from 1.
    pub seq: u64,
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
    /// Latch a signal for `key`, assigning `seq = max + 1` per key.
    /// Returns `false` when `(key, signal_id)` already exists, consumed or
    /// not, so a late duplicate never re-latches and does nothing else.
    /// A store that does not adopt signals fails loudly only when called.
    fn put_signal(
        &mut self,
        _key: &str,
        _signal_id: &str,
        _payload: &str,
    ) -> Result<bool, StorageError> {
        Err(StorageError::Rejected(
            "signals unsupported by this store".into(),
        ))
    }
    /// Unconsumed signals for `key` in `seq` order.
    fn pending_signals(&self, _key: &str) -> Result<Vec<SignalRow>, StorageError> {
        Err(StorageError::Rejected(
            "signals unsupported by this store".into(),
        ))
    }
    /// Unconsumed signals for `key` in `seq` order, marking them consumed.
    fn take_signals(&mut self, _key: &str) -> Result<Vec<SignalRow>, StorageError> {
        Err(StorageError::Rejected(
            "signals unsupported by this store".into(),
        ))
    }
    /// Drop all signals for `key`. Pure cleanup: an unsupporting store
    /// never held signals, so the default is `Ok(())`.
    fn clear_signals(&mut self, _key: &str) -> Result<(), StorageError> {
        Ok(())
    }
}

/// In-memory storage for tests and single-process hosts.
#[derive(Clone, Debug, Default)]
pub struct MemStorage {
    runs: std::collections::BTreeMap<String, RunRow>,
    /// Keyed by `(key, until_ms)` so the same instant is one row.
    parks: std::collections::BTreeMap<(String, u64), ParkedRow>,
    /// Latched signals keyed by `(key, signal_id)`; a consumed id stays
    /// reserved so a late duplicate never re-latches.
    signals: std::collections::BTreeMap<(String, String), SignalRow>,
    /// Consumed `(key, signal_id)` ids.
    consumed: std::collections::BTreeSet<(String, String)>,
    /// Next `seq` per key (monotonic; never reset by takes or clears).
    next_seq: std::collections::BTreeMap<String, u64>,
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

/// Conformance laws for latched signals on any [`EngineStorage`]. Opt-in
/// per rule 4: [`storage_laws`] keeps its case list; a sibling host runs
/// these against its own store to adopt signals explicitly.
///
/// Returns `(name, outcome)` per law. A host fails the suite when any
/// outcome is `Err`.
///
/// Re-exported through the feature module until the integration bead
/// adds the crate-root re-export (§8).
pub fn signal_storage_laws(
    store: &mut dyn EngineStorage,
) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("first latch assigns seq 1", law_signal_seq(store)),
        ("duplicate id never re-latches", law_signal_duplicate(store)),
        ("pending reads seq order", law_signal_order(store)),
        ("take consumes and stays consumed", law_signal_take(store)),
        ("clear drops the key", law_signal_clear(store)),
        ("keys are isolated", law_signal_isolation(store)),
    ]
}

fn law_signal_seq(store: &mut dyn EngineStorage) -> Result<(), String> {
    law_ok(
        store.put_signal("sig-seq", "a", "{}").map_err(law_err)?,
        "first latch writes",
    )?;
    let pending = store.pending_signals("sig-seq").map_err(law_err)?;
    law_ok(
        pending.len() == 1 && pending[0].seq == 1 && pending[0].payload == "{}",
        "first latch carries seq 1 and the payload",
    )
}

fn law_signal_duplicate(store: &mut dyn EngineStorage) -> Result<(), String> {
    store.put_signal("sig-dup", "a", "{}").map_err(law_err)?;
    law_ok(
        !store
            .put_signal("sig-dup", "a", r#"{"v":2}"#)
            .map_err(law_err)?,
        "duplicate id before consumption latches nothing",
    )?;
    let taken = store.take_signals("sig-dup").map_err(law_err)?;
    law_ok(
        taken.len() == 1 && taken[0].payload == "{}",
        "the first payload wins",
    )?;
    law_ok(
        !store
            .put_signal("sig-dup", "a", r#"{"v":3}"#)
            .map_err(law_err)?,
        "duplicate id after consumption never re-latches",
    )
}

fn law_signal_order(store: &mut dyn EngineStorage) -> Result<(), String> {
    for id in ["c", "a", "b"] {
        store.put_signal("sig-order", id, id).map_err(law_err)?;
    }
    let pending = store.pending_signals("sig-order").map_err(law_err)?;
    law_ok(
        pending
            .iter()
            .map(|row| row.signal_id.as_str())
            .collect::<Vec<_>>()
            == ["c", "a", "b"],
        "pending reads latch (seq) order, not id order",
    )?;
    law_ok(
        pending.iter().map(|row| row.seq).collect::<Vec<_>>() == [1, 2, 3],
        "seq is max + 1 per key",
    )
}

fn law_signal_take(store: &mut dyn EngineStorage) -> Result<(), String> {
    store.put_signal("sig-take", "a", "{}").map_err(law_err)?;
    law_ok(
        store.take_signals("sig-take").map_err(law_err)?.len() == 1,
        "take returns the latched row",
    )?;
    law_ok(
        store.take_signals("sig-take").map_err(law_err)?.is_empty(),
        "a second take is empty: consumption sticks",
    )?;
    law_ok(
        store
            .pending_signals("sig-take")
            .map_err(law_err)?
            .is_empty(),
        "pending is empty after take",
    )
}

fn law_signal_clear(store: &mut dyn EngineStorage) -> Result<(), String> {
    store.put_signal("sig-clear", "a", "{}").map_err(law_err)?;
    store.clear_signals("sig-clear").map_err(law_err)?;
    law_ok(
        store
            .pending_signals("sig-clear")
            .map_err(law_err)?
            .is_empty(),
        "clear drops pending rows",
    )
}

fn law_signal_isolation(store: &mut dyn EngineStorage) -> Result<(), String> {
    store.put_signal("sig-k1", "a", "1").map_err(law_err)?;
    store.put_signal("sig-k2", "a", "2").map_err(law_err)?;
    let first = store.pending_signals("sig-k1").map_err(law_err)?;
    law_ok(
        first.len() == 1 && first[0].payload == "1" && first[0].seq == 1,
        "same id on another key latches independently with its own seq",
    )?;
    store.clear_signals("sig-k1").map_err(law_err)?;
    law_ok(
        store.pending_signals("sig-k2").map_err(law_err)?.len() == 1,
        "clear touches only its key",
    )
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

    fn put_signal(
        &mut self,
        key: &str,
        signal_id: &str,
        payload: &str,
    ) -> Result<bool, StorageError> {
        let id = (key.to_string(), signal_id.to_string());
        if self.signals.contains_key(&id) {
            return Ok(false);
        }
        let seq = self.next_seq.get(key).copied().unwrap_or(1);
        self.next_seq.insert(key.to_string(), seq.saturating_add(1));
        self.signals.insert(
            id,
            SignalRow {
                key: key.to_string(),
                signal_id: signal_id.to_string(),
                payload: payload.to_string(),
                seq,
            },
        );
        Ok(true)
    }

    fn pending_signals(&self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        let mut rows: Vec<SignalRow> = self
            .signals
            .values()
            .filter(|row| {
                row.key == key
                    && !self
                        .consumed
                        .contains(&(row.key.clone(), row.signal_id.clone()))
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.seq);
        Ok(rows)
    }

    fn take_signals(&mut self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        let rows = self.pending_signals(key)?;
        for row in &rows {
            self.consumed
                .insert((row.key.clone(), row.signal_id.clone()));
        }
        Ok(rows)
    }

    fn clear_signals(&mut self, key: &str) -> Result<(), StorageError> {
        self.signals.retain(|(k, _), _| k != key);
        self.consumed.retain(|(k, _)| k != key);
        Ok(())
    }
}
