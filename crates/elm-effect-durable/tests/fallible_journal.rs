//! Fault-injection for fallible journal seams (ADR-0005).
//!
//! Every `JournalStore` method returns `Result<_, JournalError>`;
//! `OurEngine` maps journal failures to `EngineError::Journal`.

use std::cell::Cell;
use std::rc::Rc;

use elm_effect_durable::{
    DurableEngine, EngineError, MemStorage, OurEngine, ParkOutcome, RunPhase, StartOutcome,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::store::{InMemoryStore, JournalError, JournalStore, Read, Row};

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

    fn trips(&self, method: &str) -> bool {
        self.0.get() == method
    }
}

/// Wraps [`InMemoryStore`], failing one named method with
/// [`JournalError::Unavailable`].
struct FaultyJournal {
    inner: InMemoryStore,
    flag: FailFlag,
}

impl FaultyJournal {
    fn faulty(flag: &FailFlag) -> Self {
        Self {
            inner: InMemoryStore::new(1024),
            flag: flag.clone(),
        }
    }

    fn injected() -> JournalError {
        JournalError::Unavailable("injected".into())
    }
}

impl JournalStore for FaultyJournal {
    fn record_intent(&self, key: &str, max_attempts: u32) -> Result<Row, JournalError> {
        if self.flag.trips("record_intent") {
            return Err(Self::injected());
        }
        self.inner.record_intent(key, max_attempts)
    }

    fn claim(
        &self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, JournalError> {
        if self.flag.trips("claim") {
            return Err(Self::injected());
        }
        self.inner
            .claim(key, owner, lease_deadline, now_ms, expected_generation)
    }

    fn settle(
        &self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, JournalError> {
        if self.flag.trips("settle") {
            return Err(Self::injected());
        }
        self.inner.settle(key, generation, to)
    }

    fn attempt(&self, key: &str, generation: u64) -> Result<Option<Row>, JournalError> {
        if self.flag.trips("attempt") {
            return Err(Self::injected());
        }
        self.inner.attempt(key, generation)
    }

    fn read(&self, offset: i64) -> Result<Option<Read>, JournalError> {
        if self.flag.trips("read") {
            return Err(Self::injected());
        }
        self.inner.read(offset)
    }

    fn get(&self, key: &str) -> Result<Option<Row>, JournalError> {
        if self.flag.trips("get") {
            return Err(Self::injected());
        }
        self.inner.get(key)
    }
}

fn engine(flag: &FailFlag) -> OurEngine<MemStorage, FaultyJournal> {
    OurEngine::with_journal(MemStorage::new(), FaultyJournal::faulty(flag))
}

fn is_journal(err: &EngineError) -> bool {
    matches!(err, EngineError::Journal(_))
}

#[test]
fn start_propagates_journal_error_and_starts_nothing() {
    for fail_on in ["get", "record_intent", "claim", "settle"] {
        let flag = FailFlag::new();
        flag.set(fail_on);
        let mut eng = engine(&flag);
        let err = eng
            .start("k", 0)
            .expect_err("start must propagate journal error");
        assert!(
            is_journal(&err),
            "start({fail_on}): expected Journal, got {err:?}"
        );
        flag.set("");
        // The journal intent may have been recorded before the failing
        // fence, so a readable row must still be Running — never Started
        // through the error path, never terminal.
        match eng.query("k") {
            Ok(status) => assert_eq!(
                status.phase,
                RunPhase::Running,
                "start({fail_on}): readable status must stay Running"
            ),
            Err(EngineError::NotFound(_)) => {}
            Err(EngineError::Journal(_)) => {}
            Err(other) => panic!("start({fail_on}): unexpected query error {other:?}"),
        }
    }
}

#[test]
fn query_propagates_get_error() {
    let flag = FailFlag::new();
    let mut eng = engine(&flag);
    let _ = eng.start("q", 0).expect("control start");
    flag.set("get");
    let err = eng.query("q").expect_err("query must propagate get error");
    assert!(is_journal(&err), "query: expected Journal, got {err:?}");
}

#[test]
fn park_until_and_wake_propagate_get_error() {
    for verb in ["park_until", "wake"] {
        let flag = FailFlag::new();
        let mut eng = engine(&flag);
        let _ = eng.start("w", 0).expect("control start");
        flag.set("get");
        let err = match verb {
            "park_until" => eng
                .park_until("w", 10_000, 60, 0, None)
                .expect_err("park_until must propagate get error"),
            _ => eng.wake("w", 0).expect_err("wake must propagate get error"),
        };
        assert!(is_journal(&err), "{verb}: expected Journal, got {err:?}");
    }
}

#[test]
fn cancel_and_mark_unknown_propagate_journal_error() {
    for fail_on in ["get", "settle"] {
        let flag = FailFlag::new();
        let mut eng = engine(&flag);
        let _ = eng.start("c", 0).expect("control start");
        flag.set(fail_on);
        let err = eng
            .cancel("c", 0)
            .expect_err("cancel must propagate journal error");
        assert!(
            is_journal(&err),
            "cancel({fail_on}): expected Journal, got {err:?}"
        );
        flag.set(fail_on);
        let err = eng
            .mark_unknown("c")
            .expect_err("mark_unknown must propagate journal error");
        assert!(
            is_journal(&err),
            "mark_unknown({fail_on}): expected Journal, got {err:?}"
        );
    }
}

#[test]
fn control_without_failure_behaves_normally() {
    let flag = FailFlag::new();
    let mut eng = engine(&flag);
    let started = eng.start("ok", 0).expect("control start");
    assert!(matches!(started, StartOutcome::Started { .. }));
    let out = eng
        .park_until("ok", 5_000, 60, 0, None)
        .expect("control park");
    assert!(matches!(out, ParkOutcome::Wait { .. }));
    let overdue = eng.wake("ok", 6_000).expect("control wake");
    assert_eq!(overdue, Some(1_000));
    let status = eng.query("ok").expect("control query");
    assert_eq!(status.phase, RunPhase::Running);
}
