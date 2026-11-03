//! Replay-digest guarded start (§6): records, re-attaches, mismatches, recovers.

use std::cell::Cell;
use std::rc::Rc;

use elm_effect::{ClockImpl, TestClock};
use elm_effect_durable::guard::{GuardedStart, digest_storage_laws, guard_laws};
use elm_effect_durable::{
    DurableEngine, EngineError, EngineStorage, MemStorage, OurEngine, ParkedRow, RunPhase, RunRow,
    StartOptions, StorageError,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::store::{InMemoryStore, JournalError, JournalStore, Read, Row};

fn opts() -> StartOptions {
    StartOptions::default()
}

fn clock_ms(clock: &TestClock) -> u64 {
    clock.now_millis()
}

#[test]
fn law_suites_stay_green() {
    let mut store = MemStorage::new();
    for (name, outcome) in digest_storage_laws(&mut store) {
        assert!(outcome.is_ok(), "digest storage law failed: {name}");
    }
    let mut engine = OurEngine::new();
    for (name, outcome) in guard_laws(&mut engine) {
        assert!(outcome.is_ok(), "guard law failed: {name}");
    }
}

#[test]
fn guarded_start_records_reattaches_and_mismatches() {
    let clock = TestClock::new();
    clock.set_time(7_000);
    let mut engine = OurEngine::new();

    let generation = match engine
        .start_guarded("gd", clock_ms(&clock), &opts(), "code-v1")
        .expect("first start")
    {
        GuardedStart::Started { generation } => generation,
        other => panic!("first start must dispatch: {other:?}"),
    };

    match engine
        .start_guarded("gd", clock_ms(&clock), &opts(), "code-v1")
        .expect("replay")
    {
        GuardedStart::Already { status } => {
            assert_eq!(status.generation, generation);
            assert_eq!(status.phase, RunPhase::Running);
        }
        other => panic!("same digest must re-attach: {other:?}"),
    }

    let before = engine.query("gd").expect("query");
    match engine
        .start_guarded("gd", clock_ms(&clock), &opts(), "code-v2")
        .expect("changed digest")
    {
        GuardedStart::Mismatch(mismatch) => {
            assert_eq!(mismatch.prepared, "code-v1");
            assert_eq!(mismatch.redigest, "code-v2");
        }
        other => panic!("changed digest must mismatch: {other:?}"),
    }
    assert_eq!(
        engine.query("gd").expect("query"),
        before,
        "a mismatch writes nothing and mints no key"
    );
    assert!(
        engine.parked().expect("parked").is_empty(),
        "a mismatch parks nothing"
    );
}

#[test]
fn guarded_start_roundtrips_serde() {
    let started = GuardedStart::Started { generation: 2 };
    assert_eq!(
        serde_json::from_str::<GuardedStart>(&serde_json::to_string(&started).expect("serialize"))
            .expect("deserialize"),
        started
    );
}

/// Shared failure flag: the test flips it, the wrappers read it.
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

/// Wraps [`MemStorage`], failing one named store method.
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

    fn put_signal(
        &mut self,
        key: &str,
        signal_id: &str,
        payload: &str,
    ) -> Result<bool, StorageError> {
        self.inner.put_signal(key, signal_id, payload)
    }

    fn pending_signals(
        &self,
        key: &str,
    ) -> Result<Vec<elm_effect_durable::signal::SignalRow>, StorageError> {
        self.inner.pending_signals(key)
    }

    fn take_signals(
        &mut self,
        key: &str,
    ) -> Result<Vec<elm_effect_durable::signal::SignalRow>, StorageError> {
        self.inner.take_signals(key)
    }

    fn clear_signals(&mut self, key: &str) -> Result<(), StorageError> {
        self.inner.clear_signals(key)
    }

    fn get_digest(&self, key: &str) -> Result<Option<String>, StorageError> {
        if self.flag.trips("get_digest") {
            return Err(Self::injected());
        }
        self.inner.get_digest(key)
    }

    fn set_digest(&mut self, key: &str, digest: &str) -> Result<(), StorageError> {
        if self.flag.trips("set_digest") {
            return Err(Self::injected());
        }
        self.inner.set_digest(key, digest)
    }
}

/// Wraps [`InMemoryStore`], failing one named journal method.
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

fn is_storage(err: &EngineError) -> bool {
    matches!(err, EngineError::Storage(_))
}

#[test]
fn digest_write_faults_leave_nothing_started() {
    for fail_on in ["get_digest", "set_digest"] {
        let flag = FailFlag::new();
        let mut engine = OurEngine::with_storage(FaultyStore::faulty(&flag));
        flag.set(fail_on);
        let err = engine
            .start_guarded("gfault", 0, &opts(), "v1")
            .expect_err("guarded start must propagate storage error");
        assert!(
            is_storage(&err),
            "start_guarded({fail_on}): expected Storage, got {err:?}"
        );
        flag.set("");
        assert!(
            matches!(
                engine
                    .start_guarded("gfault", 0, &opts(), "v1")
                    .expect("retry"),
                GuardedStart::Started { .. }
            ),
            "start_guarded({fail_on}): retry must dispatch"
        );
    }
}

#[test]
fn crash_between_digest_write_and_intent_recovers() {
    let flag = FailFlag::new();
    let mut engine = OurEngine::with_journal(MemStorage::new(), FaultyJournal::faulty(&flag));
    flag.set("record_intent");
    let err = engine
        .start_guarded("gcrash", 0, &opts(), "v1")
        .expect_err("intent failure must propagate");
    assert!(
        matches!(err, EngineError::Journal(_)),
        "expected Journal, got {err:?}"
    );
    // The digest committed; the intent never did: nothing started.
    assert!(matches!(
        engine.query("gcrash"),
        Err(EngineError::NotFound(_))
    ));
    assert!(
        engine
            .journal()
            .get("gcrash")
            .expect("journal read")
            .is_none()
    );
    flag.set("");
    // A re-render under a new digest overwrites the orphan and dispatches.
    assert!(
        matches!(
            engine
                .start_guarded("gcrash", 0, &opts(), "v2")
                .expect("retry"),
            GuardedStart::Started { .. }
        ),
        "orphaned digest must be overwritten, not mismatched"
    );
    assert_eq!(
        engine.query("gcrash").expect("query").phase,
        RunPhase::Running
    );
}

#[test]
fn journal_read_fault_propagates_and_retry_succeeds() {
    let flag = FailFlag::new();
    let mut engine = OurEngine::with_journal(MemStorage::new(), FaultyJournal::faulty(&flag));
    assert!(matches!(
        engine
            .start_guarded("gread", 0, &opts(), "v1")
            .expect("control"),
        GuardedStart::Started { .. }
    ));
    flag.set("get");
    let err = engine
        .start_guarded("gread", 0, &opts(), "v2")
        .expect_err("guarded start must propagate journal error");
    assert!(
        matches!(err, EngineError::Journal(_)),
        "expected Journal, got {err:?}"
    );
    flag.set("");
    assert!(
        matches!(
            engine
                .start_guarded("gread", 0, &opts(), "v2")
                .expect("retry"),
            GuardedStart::Mismatch(_)
        ),
        "retry must see the recorded digest and mismatch"
    );
}

/// A store that never adopted digests: every guarded start fails loudly.
struct NoDigestStore {
    inner: MemStorage,
}

impl EngineStorage for NoDigestStore {
    fn get_run(&self, key: &str) -> Result<Option<RunRow>, StorageError> {
        self.inner.get_run(key)
    }

    fn put_run(&mut self, row: RunRow) -> Result<(), StorageError> {
        self.inner.put_run(row)
    }

    fn list_parked(&self) -> Result<Vec<ParkedRow>, StorageError> {
        self.inner.list_parked()
    }

    fn upsert_park(&mut self, row: ParkedRow) -> Result<bool, StorageError> {
        self.inner.upsert_park(row)
    }

    fn clear_parks(&mut self, key: &str) -> Result<(), StorageError> {
        self.inner.clear_parks(key)
    }
}

#[test]
fn store_without_digest_support_fails_loudly_and_starts_nothing() {
    let mut engine = OurEngine::with_storage(NoDigestStore {
        inner: MemStorage::new(),
    });
    let err = engine
        .start_guarded("gunsupported", 0, &opts(), "v1")
        .expect_err("unsupported digests must fail");
    assert!(
        matches!(err, EngineError::Storage(StorageError::Rejected(_))),
        "expected loud Rejected, got {err:?}"
    );
    assert!(matches!(
        engine.query("gunsupported"),
        Err(EngineError::NotFound(_))
    ));
    assert!(
        engine
            .journal()
            .get("gunsupported")
            .expect("journal read")
            .is_none(),
        "nothing was started"
    );
}
