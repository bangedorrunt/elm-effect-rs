//! Latched durable signals (§3): success, refusals, replay, faults, restart, live Loop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use elm_effect::{ClockImpl, Duration, Effect, ManagedRuntime, TestClock};
use elm_effect_durable::signal::{
    AwaitOutcome, SignalOutcome, SignalRow, signal_laws, signal_storage_laws,
};
use elm_effect_durable::{
    DurableEngine, EngineError, EngineMsg, EngineStorage, MemStorage, OurEngine, ParkOutcome,
    ParkedRow, RunPhase, RunRow, StartOutcome, StorageError,
};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::durable_log::MemLog;
use elm_effect_journal::store::{InMemoryStore, JournalError, JournalStore, Read, Row};

fn started<S: EngineStorage, J: JournalStore>(engine: &mut OurEngine<S, J>, key: &str) {
    match engine.start(key, 0).expect("control start") {
        StartOutcome::Started { .. } => {}
        StartOutcome::Already { .. } => panic!("fresh key must start: {key}"),
    }
}

fn clock_ms(clock: &TestClock) -> u64 {
    clock.now_millis()
}

#[test]
fn law_suites_stay_green() {
    let mut store = MemStorage::new();
    for (name, outcome) in signal_storage_laws(&mut store) {
        assert!(outcome.is_ok(), "signal storage law failed: {name}");
    }
    let mut engine = OurEngine::new();
    for (name, outcome) in signal_laws(&mut engine) {
        assert!(outcome.is_ok(), "signal law failed: {name}");
    }
}

#[test]
fn await_without_signals_parks_and_drains_on_time() {
    let clock = TestClock::new();
    clock.set_time(1_000);
    let mut engine = OurEngine::new();
    started(&mut engine, "waiter");
    match engine
        .await_signal("waiter", 5_000, 60, clock_ms(&clock), None)
        .expect("await")
    {
        AwaitOutcome::Parked(ParkOutcome::Wait { until_ms, .. }) => {
            assert_eq!(until_ms, 5_000);
        }
        other => panic!("empty await must park: {other:?}"),
    }
    assert_eq!(
        engine.query("waiter").expect("query").phase,
        RunPhase::Parked
    );
    clock.adjust(Duration::from_millis(3_999));
    assert!(
        engine
            .drain_due(clock_ms(&clock))
            .expect("drain")
            .is_empty(),
        "one ms early drains nothing"
    );
    clock.adjust(Duration::from_millis(1));
    let wakes = engine.drain_due(clock_ms(&clock)).expect("drain");
    assert_eq!(wakes.len(), 1, "deadline wakes exactly once");
    let EngineMsg::Woke { overdue_ms, .. } = &wakes[0];
    assert_eq!(*overdue_ms, 0);
}

#[test]
fn await_repark_keeps_one_park_and_ceiling_refuses() {
    let mut engine = OurEngine::new();
    started(&mut engine, "repark");
    assert!(matches!(
        engine
            .await_signal("repark", 5_000, 60, 0, None)
            .expect("await"),
        AwaitOutcome::Parked(ParkOutcome::Wait { .. })
    ));
    assert!(matches!(
        engine
            .await_signal("repark", 9_000, 60, 0, None)
            .expect("await"),
        AwaitOutcome::Parked(ParkOutcome::Wait {
            until_ms: 9_000,
            ..
        })
    ));
    assert_eq!(
        engine.parked().expect("parked").len(),
        1,
        "re-parking keeps one park"
    );
    match engine
        .await_signal("repark", 10_000_000, 200_000, 0, Some(86_400))
        .expect("await")
    {
        AwaitOutcome::Parked(ParkOutcome::RefusedOverCeiling { .. }) => {}
        other => panic!("over-ceiling await must refuse: {other:?}"),
    }
    assert_eq!(
        engine.query("repark").expect("query").phase,
        RunPhase::Parked,
        "a refused await keeps the earlier park"
    );
}

#[test]
fn await_past_deadline_resumes_now() {
    let mut engine = OurEngine::new();
    started(&mut engine, "late");
    match engine
        .await_signal("late", 500, 60, 1_000, None)
        .expect("await")
    {
        AwaitOutcome::Parked(ParkOutcome::ResumeNow { overdue_ms }) => {
            assert_eq!(overdue_ms, 500);
        }
        other => panic!("past deadline must resume now: {other:?}"),
    }
    assert_eq!(
        engine.query("late").expect("query").phase,
        RunPhase::Running
    );
}

#[test]
fn outcomes_and_rows_roundtrip_serde() {
    let latched = SignalOutcome::Latched { wake: None };
    assert_eq!(
        serde_json::from_str::<SignalOutcome>(&serde_json::to_string(&latched).expect("serialize"))
            .expect("deserialize"),
        latched
    );
    let parked = AwaitOutcome::Parked(ParkOutcome::ResumeNow { overdue_ms: 3 });
    assert_eq!(
        serde_json::from_str::<AwaitOutcome>(&serde_json::to_string(&parked).expect("serialize"))
            .expect("deserialize"),
        parked
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

    fn trips(&self, primitive: &str) -> bool {
        let flag = self.0.get();
        flag == primitive
            || (flag == "commit_run" && (primitive == "put_run" || primitive == "clear_parks"))
            || (flag == "commit_park" && (primitive == "put_run" || primitive == "upsert_park"))
            || ((flag == "get_park" || flag == "due_parks") && primitive == "list_parked")
    }
}

/// Wraps [`MemStorage`], failing one named method at a time. The inner
/// store is shared, so a test can restart an engine on the same state.
struct FaultyStore {
    inner: Rc<RefCell<MemStorage>>,
    flag: FailFlag,
}

impl FaultyStore {
    fn faulty(flag: &FailFlag) -> Self {
        Self::shared(&Rc::new(RefCell::new(MemStorage::new())), flag)
    }

    fn shared(inner: &Rc<RefCell<MemStorage>>, flag: &FailFlag) -> Self {
        Self {
            inner: inner.clone(),
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
        self.inner.borrow().get_run(key)
    }

    fn put_run(&mut self, row: RunRow) -> Result<(), StorageError> {
        if self.flag.trips("put_run") {
            return Err(Self::injected());
        }
        self.inner.borrow_mut().put_run(row)
    }

    fn list_parked(&self) -> Result<Vec<ParkedRow>, StorageError> {
        if self.flag.trips("list_parked") {
            return Err(Self::injected());
        }
        self.inner.borrow().list_parked()
    }

    fn upsert_park(&mut self, row: ParkedRow) -> Result<bool, StorageError> {
        if self.flag.trips("upsert_park") {
            return Err(Self::injected());
        }
        self.inner.borrow_mut().upsert_park(row)
    }

    fn clear_parks(&mut self, key: &str) -> Result<(), StorageError> {
        if self.flag.trips("clear_parks") {
            return Err(Self::injected());
        }
        self.inner.borrow_mut().clear_parks(key)
    }

    fn put_signal(
        &mut self,
        key: &str,
        signal_id: &str,
        payload: &str,
    ) -> Result<bool, StorageError> {
        if self.flag.trips("put_signal") {
            return Err(Self::injected());
        }
        self.inner.borrow_mut().put_signal(key, signal_id, payload)
    }

    fn pending_signals(&self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        if self.flag.trips("pending_signals") {
            return Err(Self::injected());
        }
        self.inner.borrow().pending_signals(key)
    }

    fn take_signals(&mut self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        if self.flag.trips("take_signals") {
            return Err(Self::injected());
        }
        self.inner.borrow_mut().take_signals(key)
    }

    fn clear_signals(&mut self, key: &str) -> Result<(), StorageError> {
        if self.flag.trips("clear_signals") {
            return Err(Self::injected());
        }
        self.inner.borrow_mut().clear_signals(key)
    }
}

fn is_storage(err: &EngineError) -> bool {
    matches!(err, EngineError::Storage(_))
}

#[test]
fn latched_signal_survives_engine_restart() {
    let flag = FailFlag::new();
    let shared = Rc::new(RefCell::new(MemStorage::new()));
    let mut engine = OurEngine::with_storage(FaultyStore::shared(&shared, &flag));
    started(&mut engine, "restart");
    assert!(matches!(
        engine
            .signal("restart", "s1", "payload", 0)
            .expect("signal"),
        SignalOutcome::Latched { wake: None }
    ));
    drop(engine);
    let mut recovered = OurEngine::recover(FaultyStore::shared(&shared, &flag));
    let rows = recovered
        .take_signals("restart")
        .expect("take after restart");
    assert_eq!(
        rows,
        vec![SignalRow {
            key: "restart".into(),
            signal_id: "s1".into(),
            payload: "payload".into(),
            seq: 1,
        }]
    );
}

#[test]
fn signal_fault_leaves_nothing_latched_and_retry_succeeds() {
    for fail_on in ["get_run", "put_signal"] {
        let flag = FailFlag::new();
        let mut engine = OurEngine::with_storage(FaultyStore::faulty(&flag));
        started(&mut engine, "fault");
        flag.set(fail_on);
        let err = engine
            .signal("fault", "s1", "{}", 0)
            .expect_err("signal must propagate storage error");
        assert!(
            is_storage(&err),
            "signal({fail_on}): expected Storage, got {err:?}"
        );
        flag.set("");
        assert!(
            matches!(
                engine.signal("fault", "s1", "{}", 0).expect("retry"),
                SignalOutcome::Latched { wake: None }
            ),
            "signal({fail_on}): retry must latch"
        );
        assert_eq!(
            engine.query("fault").expect("query").phase,
            RunPhase::Running
        );
    }
}

#[test]
fn parked_signal_wake_fault_keeps_payload_recoverable() {
    for fail_on in ["get_park", "commit_run", "put_run", "clear_parks"] {
        let flag = FailFlag::new();
        let mut engine = OurEngine::with_storage(FaultyStore::faulty(&flag));
        started(&mut engine, "wfault");
        engine
            .park_until("wfault", 10_000, 60, 0, None)
            .expect("control park");
        flag.set(fail_on);
        let err = engine
            .signal("wfault", "s1", "ping", 1_000)
            .expect_err("wake must propagate storage error");
        assert!(
            is_storage(&err),
            "signal({fail_on}): expected Storage, got {err:?}"
        );
        flag.set("");
        // The latch committed before the wake failed: the retry names the
        // duplicate instead of double-latching, and the payload is intact.
        assert!(
            matches!(
                engine.signal("wfault", "s1", "ping", 1_000).expect("retry"),
                SignalOutcome::Duplicate
            ),
            "signal({fail_on}): retry must report Duplicate, not re-latch"
        );
        let rows = engine.take_signals("wfault").expect("take");
        assert_eq!(
            rows.len(),
            1,
            "signal({fail_on}): payload survives the failed wake"
        );
        assert_eq!(rows[0].payload, "ping");
    }
}

#[test]
fn await_and_take_faults_leave_run_unparked() {
    for fail_on in ["pending_signals", "commit_park", "upsert_park"] {
        let flag = FailFlag::new();
        let mut engine = OurEngine::with_storage(FaultyStore::faulty(&flag));
        started(&mut engine, "afault");
        flag.set(fail_on);
        let err = engine
            .await_signal("afault", 10_000, 60, 0, None)
            .expect_err("await must propagate storage error");
        assert!(
            is_storage(&err),
            "await({fail_on}): expected Storage, got {err:?}"
        );
        flag.set("");
        assert!(
            matches!(
                engine
                    .await_signal("afault", 10_000, 60, 0, None)
                    .expect("retry"),
                AwaitOutcome::Parked(ParkOutcome::Wait { .. })
            ),
            "await({fail_on}): retry must park"
        );
    }
    {
        let flag = FailFlag::new();
        let mut engine = OurEngine::with_storage(FaultyStore::faulty(&flag));
        started(&mut engine, "tfault");
        engine
            .signal("tfault", "s1", "{}", 0)
            .expect("control latch");
        flag.set("take_signals");
        let err = engine
            .take_signals("tfault")
            .expect_err("take must propagate storage error");
        assert!(is_storage(&err), "take: expected Storage, got {err:?}");
        flag.set("");
        assert_eq!(
            engine.take_signals("tfault").expect("retry").len(),
            1,
            "take retry must consume the latched row"
        );
    }
}

/// Store that latches a racing signal inside the first park commit,
/// simulating a host that shares the store across handles: the signal
/// lands between the pending check and the park commit.
struct GapStore {
    inner: MemStorage,
    injected: bool,
}

impl EngineStorage for GapStore {
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

    fn commit_park(&mut self, run: RunRow, park: ParkedRow) -> Result<(), StorageError> {
        if !self.injected {
            self.injected = true;
            self.inner.put_signal(&run.key, "race", "{}")?;
        }
        self.inner.commit_park(run, park)
    }

    fn put_signal(
        &mut self,
        key: &str,
        signal_id: &str,
        payload: &str,
    ) -> Result<bool, StorageError> {
        self.inner.put_signal(key, signal_id, payload)
    }

    fn pending_signals(&self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        self.inner.pending_signals(key)
    }

    fn take_signals(&mut self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        self.inner.take_signals(key)
    }

    fn clear_signals(&mut self, key: &str) -> Result<(), StorageError> {
        self.inner.clear_signals(key)
    }
}

#[test]
fn signal_in_the_park_gap_wakes_instead_of_sleeping() {
    let mut engine = OurEngine::with_storage(GapStore {
        inner: MemStorage::new(),
        injected: false,
    });
    started(&mut engine, "gap");
    match engine
        .await_signal("gap", 10_000, 60, 0, None)
        .expect("await")
    {
        AwaitOutcome::Signalled(rows) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].signal_id, "race");
        }
        AwaitOutcome::Parked(_) => panic!("a signal in the gap must win over the park"),
    }
    assert_eq!(engine.query("gap").expect("query").phase, RunPhase::Running);
    assert!(engine.parked().expect("parked").is_empty());
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

#[test]
fn journal_fault_propagates_as_journal_and_retry_succeeds() {
    let flag = FailFlag::new();
    let mut engine = OurEngine::with_journal(MemStorage::new(), FaultyJournal::faulty(&flag));
    started(&mut engine, "jfault");
    flag.set("get");
    let err = engine
        .signal("jfault", "s1", "{}", 0)
        .expect_err("signal must propagate journal error");
    assert!(
        matches!(err, EngineError::Journal(_)),
        "signal(get): expected Journal, got {err:?}"
    );
    flag.set("");
    assert!(
        matches!(
            engine.signal("jfault", "s1", "{}", 0).expect("retry"),
            SignalOutcome::Latched { wake: None }
        ),
        "retry after journal recovery must latch"
    );
}

// --- live Loop witness: a signal that beats the park reaches the fold ---

#[derive(Clone, Debug, PartialEq)]
struct SigModel {
    payloads: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum SigMsg {
    Signalled { payload: String },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum SigCmd {
    Wait { key: String },
}

impl Cmd for SigCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Wait { key } => Durability::Durable { key: key.clone() },
        }
    }
}

struct Sig;

impl Program for Sig {
    type Model = SigModel;
    type Msg = SigMsg;
    type Cmd = SigCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<SigModel, SigCmd> {
        Update::quiet(SigModel { payloads: vec![] })
    }

    fn update(model: &SigModel, msg: &SigMsg) -> Update<SigModel, SigCmd> {
        match msg {
            SigMsg::Signalled { payload } => {
                let mut payloads = model.payloads.clone();
                payloads.push(payload.clone());
                Update::quiet(SigModel { payloads })
            }
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> SigMsg {
        SigMsg::Signalled {
            payload: "failure".into(),
        }
    }
}

struct SigRun;

impl Interpret<Sig> for SigRun {
    fn interpret(_cmd: &SigCmd) -> Effect<SigMsg, InterpretError> {
        Effect::sync(|| SigMsg::Signalled {
            payload: "interpreted".into(),
        })
    }
}

#[test]
fn signal_before_park_reaches_the_elm_fold() {
    let clock = TestClock::new();
    clock.set_time(0);
    let mut engine = OurEngine::new();
    started(&mut engine, "loop-sig");

    // The external event beats the park: it latches with no wake.
    assert!(matches!(
        engine
            .signal("loop-sig", "s1", "hello", clock_ms(&clock))
            .expect("signal"),
        SignalOutcome::Latched { wake: None }
    ));

    // The await never parks: the payload is already there.
    let rows = match engine
        .await_signal("loop-sig", 10_000, 60, clock_ms(&clock), None)
        .expect("await")
    {
        AwaitOutcome::Signalled(rows) => rows,
        AwaitOutcome::Parked(_) => panic!("latched signal must not park"),
    };
    assert_eq!(rows.len(), 1);

    // The payload crosses into the Elm fold as an ordinary message.
    let mut app = Loop::<Sig, SigRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("loop-sig"),
    )
    .expect("boot");
    for row in &rows {
        app.send(SigMsg::Signalled {
            payload: row.payload.clone(),
        })
        .expect("send");
    }
    assert_eq!(app.model().payloads, vec!["hello".to_string()]);
    assert!(
        engine.parked().expect("parked").is_empty(),
        "the witnessed run never parked and never hangs"
    );
}
