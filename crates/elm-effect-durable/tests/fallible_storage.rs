//! Fault-injection for fallible storage seams (ADR-0005).
//!
//! Every `EngineStorage` method returns `Result<_, StorageError>`;
//! `get_park`, `due_parks`, `commit_run`, `commit_park` keep default impls.

use std::cell::Cell;
use std::rc::Rc;

use elm_effect_durable::{
    DurableEngine, EngineError, EngineStorage, MemStorage, OurEngine, ParkOutcome, ParkedRow,
    RunPhase, RunRow, StartOutcome, StorageError,
};

/// Shared failure flag: the test flips it, the store reads it.
#[derive(Clone)]
struct FailFlag(Rc<Cell<&'static str>>);

impl FailFlag {
    fn new() -> Self {
        Self(Rc::new(Cell::new("")))
    }

    fn set(&self, name: &'static str) {
        self.0.set(name);
    }

    /// `commit_run` is `put_run` then `clear_parks`; `commit_park` is
    /// `upsert_park` then `put_run`; `get_park` / `due_parks` read through
    /// `list_parked`. Naming the higher-level method fails its primitive.
    fn trips(&self, primitive: &str) -> bool {
        let flag = self.0.get();
        flag == primitive
            || (flag == "commit_run" && (primitive == "put_run" || primitive == "clear_parks"))
            || (flag == "commit_park" && (primitive == "put_run" || primitive == "upsert_park"))
            || ((flag == "get_park" || flag == "due_parks") && primitive == "list_parked")
    }
}

/// Wraps [`MemStorage`], failing one named method at a time.
/// `get_park`, `due_parks`, `commit_run`, `commit_park` keep default impls.
struct FaultyStore {
    inner: MemStorage,
    flag: FailFlag,
}

impl FaultyStore {
    fn faulty(flag: &FailFlag) -> Self {
        Self {
            inner: MemStorage::new(),
            flag: flag.clone(),
        }
    }

    fn injected() -> StorageError {
        StorageError::Unavailable("injected".into())
    }
}

impl EngineStorage for FaultyStore {
    fn get_run(&self, key: &str) -> Result<Option<RunRow>, StorageError> {
        if self.flag.trips("get_run") {
            return Err(Self::injected());
        }
        self.inner.get_run(key)
    }

    fn put_run(&mut self, row: RunRow) -> Result<(), StorageError> {
        if self.flag.trips("put_run") {
            return Err(Self::injected());
        }
        self.inner.put_run(row)
    }

    fn list_parked(&self) -> Result<Vec<ParkedRow>, StorageError> {
        if self.flag.trips("list_parked") {
            return Err(Self::injected());
        }
        self.inner.list_parked()
    }

    fn upsert_park(&mut self, row: ParkedRow) -> Result<bool, StorageError> {
        if self.flag.trips("upsert_park") {
            return Err(Self::injected());
        }
        self.inner.upsert_park(row)
    }

    fn clear_parks(&mut self, key: &str) -> Result<(), StorageError> {
        if self.flag.trips("clear_parks") {
            return Err(Self::injected());
        }
        self.inner.clear_parks(key)
    }
}

fn is_storage(err: &EngineError) -> bool {
    matches!(err, EngineError::Storage(_))
}

#[test]
fn start_propagates_storage_error() {
    for fail_on in ["get_run", "put_run", "commit_run", "clear_parks"] {
        let flag = FailFlag::new();
        flag.set(fail_on);
        let mut eng = OurEngine::with_storage(FaultyStore::faulty(&flag));
        let err = eng
            .start("k", 0)
            .expect_err("start must propagate storage error");
        assert!(
            is_storage(&err),
            "start({fail_on}): expected Storage, got {err:?}"
        );
        flag.set("");
        // The journal intent is recorded before the storage write, so a
        // failed start may leave a queryable Running row; it must never
        // leave a terminal phase behind.
        if fail_on == "get_run" {
            assert!(
                eng.query("k").is_err(),
                "start({fail_on}): query must fail when get_run fails"
            );
        } else {
            match eng.query("k") {
                Ok(status) => assert_eq!(
                    status.phase,
                    RunPhase::Running,
                    "start({fail_on}): readable status must stay Running"
                ),
                Err(EngineError::NotFound(_)) => {}
                Err(EngineError::Storage(_)) => {}
                Err(other) => panic!("start({fail_on}): unexpected query error {other:?}"),
            }
        }
    }
}

#[test]
fn park_until_propagates_storage_error_without_fake_wait() {
    for fail_on in [
        "get_park",
        "commit_park",
        "put_run",
        "upsert_park",
        "list_parked",
    ] {
        let flag = FailFlag::new();
        let mut eng = OurEngine::with_storage(FaultyStore::faulty(&flag));
        let _ = eng.start("p", 0).expect("control start");
        flag.set(fail_on);
        let err = eng
            .park_until("p", 10_000, 60, 0, None)
            .expect_err("park_until must propagate storage error");
        assert!(
            is_storage(&err),
            "park_until({fail_on}): expected Storage, got {err:?}"
        );
        flag.set("");
        let status = eng.query("p").expect("run stays queryable");
        assert_eq!(
            status.phase,
            RunPhase::Running,
            "park_until({fail_on}): run must stay Running"
        );
    }
}

#[test]
fn drain_due_propagates_due_parks_error_with_no_wakes() {
    let flag = FailFlag::new();
    let mut eng = OurEngine::with_storage(FaultyStore::faulty(&flag));
    let _ = eng.start("d", 0).expect("control start");
    let _ = eng
        .park_until("d", 1_000, 60, 0, None)
        .expect("control park");
    flag.set("due_parks");
    let err = eng
        .drain_due(2_000)
        .expect_err("drain_due must propagate due_parks error");
    assert!(is_storage(&err), "drain_due: expected Storage, got {err:?}");
}

#[test]
fn query_propagates_get_run_error() {
    let flag = FailFlag::new();
    let mut eng = OurEngine::with_storage(FaultyStore::faulty(&flag));
    let _ = eng.start("q", 0).expect("control start");
    flag.set("get_run");
    let err = eng
        .query("q")
        .expect_err("query must propagate get_run error");
    assert!(is_storage(&err), "query: expected Storage, got {err:?}");
}

#[test]
fn control_without_failure_behaves_normally() {
    let flag = FailFlag::new();
    let mut eng = OurEngine::with_storage(FaultyStore::faulty(&flag));
    let started = eng.start("c", 0).expect("control start");
    assert!(matches!(started, StartOutcome::Started { .. }));
    let out = eng
        .park_until("c", 5_000, 60, 0, None)
        .expect("control park");
    assert!(matches!(out, ParkOutcome::Wait { .. }));
    let wakes = eng.drain_due(6_000).expect("control drain");
    assert_eq!(wakes.len(), 1, "control drain must wake the park");
    let parked = eng.parked().expect("control parked");
    assert!(parked.is_empty(), "drained park must be gone");
}
