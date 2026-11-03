//! Lineage rollover (`continue_as_new`) at the engine edge (ADR-0005 §5).
//!
//! Happy path, idempotent re-runs, stale/terminal refusals, fault-injected
//! crash between start-next and close-prev, and a live Elm Loop witness over
//! three epochs.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use elm_effect::{ClockImpl, Duration, Effect, ManagedRuntime, TestClock};
use elm_effect_durable::lineage::{ContinueOutcome, rollover_key};
use elm_effect_durable::{
    DurableEngine, EngineError, OurEngine, RunPhase, StartOptions, StartOutcome,
};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::store::{InMemoryStore, JournalError, JournalStore, Read, Row};

fn started(engine: &mut OurEngine, key: &str, now_ms: u64) -> u64 {
    match engine.start(key, now_ms).expect("start") {
        StartOutcome::Started { generation } => generation,
        StartOutcome::Already { .. } => panic!("fresh key must start: {key}"),
    }
}

#[test]
fn rollover_key_names_epochs() {
    assert_eq!(rollover_key("report", 0), "report:epoch:0");
    assert_eq!(rollover_key("report", 41), "report:epoch:41");
}

#[test]
fn happy_path_closes_prev_and_starts_next() {
    let clock = TestClock::new();
    let mut engine = OurEngine::new();
    let from = rollover_key("report", 0);
    let to = rollover_key("report", 1);
    let fence = started(&mut engine, &from, clock.now_millis());
    let outcome = engine
        .continue_as_new(
            &from,
            fence,
            &to,
            clock.now_millis(),
            &StartOptions::default(),
        )
        .expect("rollover");
    let (started_outcome, closed) = match outcome {
        ContinueOutcome::Continued { started, closed } => (started, closed),
        ContinueOutcome::Rejected(r) => panic!("live rollover must continue: {r:?}"),
    };
    assert!(matches!(started_outcome, StartOutcome::Started { .. }));
    assert_eq!(closed.key, from);
    assert_eq!(closed.state, EffectState::Settled);
    assert_eq!(engine.query(&from).expect("query").phase, RunPhase::Settled);
    assert_eq!(engine.query(&to).expect("query").phase, RunPhase::Running);
}

#[test]
fn rerun_after_close_is_continued_not_rejected() {
    let mut engine = OurEngine::new();
    let from = rollover_key("re", 0);
    let to = rollover_key("re", 1);
    let fence = started(&mut engine, &from, 0);
    let opts = StartOptions::default();
    match engine
        .continue_as_new(&from, fence, &to, 0, &opts)
        .expect("first")
    {
        ContinueOutcome::Continued { .. } => {}
        ContinueOutcome::Rejected(r) => panic!("first rollover must continue: {r:?}"),
    }
    // Same fence again: the previous epoch is already Settled at this
    // generation and the next epoch exists — idempotent Continued.
    match engine
        .continue_as_new(&from, fence, &to, 0, &opts)
        .expect("rerun")
    {
        ContinueOutcome::Continued { started, closed } => {
            assert!(matches!(started, StartOutcome::Already { .. }));
            assert_eq!(closed.state, EffectState::Settled);
        }
        ContinueOutcome::Rejected(r) => panic!("rerun must stay Continued: {r:?}"),
    }
}

#[test]
fn stale_generation_rejected_without_starting_next() {
    let mut engine = OurEngine::new();
    let from = rollover_key("stale", 0);
    let to = rollover_key("stale", 1);
    let fence = started(&mut engine, &from, 0);
    let outcome = engine
        .continue_as_new(
            &from,
            fence.wrapping_add(1),
            &to,
            0,
            &StartOptions::default(),
        )
        .expect("rollover");
    assert!(matches!(
        outcome,
        ContinueOutcome::Rejected(
            elm_effect_journal::plan::SettleRejection::StaleGeneration { .. }
        )
    ));
    assert!(
        engine.journal().get(&to).expect("get").is_none(),
        "a stale fence must not start the next epoch"
    );
}

#[test]
fn terminal_prev_rejected() {
    let mut engine = OurEngine::new();
    let from = rollover_key("term", 0);
    let to = rollover_key("term", 1);
    let fence = started(&mut engine, &from, 0);
    let claimed = engine
        .claim(&from, "w", 10_000, 0, fence)
        .expect("claim")
        .expect("live");
    engine
        .settle(&from, claimed.generation, EffectState::Failed)
        .expect("fail");
    let outcome = engine
        .continue_as_new(&from, claimed.generation, &to, 0, &StartOptions::default())
        .expect("rollover");
    assert!(matches!(
        outcome,
        ContinueOutcome::Rejected(elm_effect_journal::plan::SettleRejection::Terminal { .. })
    ));
    assert!(engine.journal().get(&to).expect("get").is_none());
}

#[test]
fn next_already_terminal_still_closes_prev() {
    let mut engine = OurEngine::new();
    let from = rollover_key("nxt", 0);
    let to = rollover_key("nxt", 1);
    let fence = started(&mut engine, &from, 0);
    started(&mut engine, &to, 0);
    let to_fence = engine.query(&to).expect("query").generation;
    let claimed = engine
        .claim(&to, "w", 10_000, 0, to_fence)
        .expect("claim")
        .expect("live");
    engine
        .settle(&to, claimed.generation, EffectState::Settled)
        .expect("settle to");
    match engine
        .continue_as_new(&from, fence, &to, 0, &StartOptions::default())
        .expect("rollover")
    {
        ContinueOutcome::Continued { started, closed } => {
            assert!(matches!(started, StartOutcome::Already { .. }));
            assert_eq!(closed.state, EffectState::Settled);
        }
        ContinueOutcome::Rejected(r) => panic!("prev must still close: {r:?}"),
    }
    assert_eq!(engine.query(&from).expect("query").phase, RunPhase::Settled);
}

/// One-shot fault: fails `settle` for the armed key once, then behaves.
/// Starts disarmed so setup verbs run clean; the test arms it after `start`.
struct FailSettleOnce {
    inner: InMemoryStore,
    armed: Rc<Cell<Option<String>>>,
}

impl FailSettleOnce {
    fn disarmed() -> (Self, Rc<Cell<Option<String>>>) {
        let armed = Rc::new(Cell::new(None));
        (
            Self {
                inner: InMemoryStore::new(1024),
                armed: armed.clone(),
            },
            armed,
        )
    }
}

impl JournalStore for FailSettleOnce {
    fn record_intent(&self, key: &str, max_attempts: u32) -> Result<Row, JournalError> {
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
        self.inner
            .claim(key, owner, lease_deadline, now_ms, expected_generation)
    }

    fn renew_lease(
        &self,
        key: &str,
        owner: &str,
        lease_deadline: i64,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Row>, JournalError> {
        self.inner
            .renew_lease(key, owner, lease_deadline, now_ms, expected_generation)
    }

    fn settle(
        &self,
        key: &str,
        generation: u64,
        to: EffectState,
    ) -> Result<Option<Row>, JournalError> {
        // One-shot and key-scoped: a non-matching settle (e.g. the next
        // epoch's own start) re-arms instead of consuming the fault.
        match self.armed.take() {
            Some(armed) if armed == key => {
                return Err(JournalError::Unavailable("injected".into()));
            }
            other => self.armed.set(other),
        }
        self.inner.settle(key, generation, to)
    }

    fn attempt(&self, key: &str, generation: u64) -> Result<Option<Row>, JournalError> {
        self.inner.attempt(key, generation)
    }

    fn read(&self, offset: i64) -> Result<Option<Read>, JournalError> {
        self.inner.read(offset)
    }

    fn get(&self, key: &str) -> Result<Option<Row>, JournalError> {
        self.inner.get(key)
    }
}

#[test]
fn crash_between_start_and_close_recovers_on_rerun() {
    use elm_effect_durable::MemStorage;
    let from = rollover_key("crash", 0);
    let to = rollover_key("crash", 1);
    let (journal, armed) = FailSettleOnce::disarmed();
    let mut engine = OurEngine::with_journal(MemStorage::new(), journal);
    let fence = match engine.start(&from, 0).expect("start") {
        StartOutcome::Started { generation } => generation,
        StartOutcome::Already { .. } => panic!("fresh key must start"),
    };
    // Arm only now: the crash lands between start-next and close-prev.
    armed.set(Some(from.clone()));
    // The close-prev settle fails once: the whole verb fails, and the next
    // epoch (whose own settles were unaffected) stays live.
    let err = engine
        .continue_as_new(&from, fence, &to, 0, &StartOptions::default())
        .expect_err("injected settle failure must surface");
    assert!(matches!(err, EngineError::Journal(_)));
    assert_eq!(engine.query(&from).expect("query").phase, RunPhase::Running);
    assert_eq!(engine.query(&to).expect("query").phase, RunPhase::Running);
    // Re-run completes the close; nothing double-starts.
    match engine
        .continue_as_new(&from, fence, &to, 0, &StartOptions::default())
        .expect("retry")
    {
        ContinueOutcome::Continued { started, closed } => {
            assert!(matches!(started, StartOutcome::Already { .. }));
            assert_eq!(closed.state, EffectState::Settled);
        }
        ContinueOutcome::Rejected(r) => panic!("retry must continue: {r:?}"),
    }
    assert_eq!(engine.query(&from).expect("query").phase, RunPhase::Settled);
}

// --- live Loop witness: a recurring program rolls over three epochs ---

#[derive(Clone, Debug, PartialEq)]
struct EpochModel {
    rolled: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum EpochMsg {
    Rolled { epoch: u64 },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum EpochCmd {
    Roll { key: String },
}

impl Cmd for EpochCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Roll { key } => Durability::Durable { key: key.clone() },
        }
    }
}

struct Epoch;

impl Program for Epoch {
    type Model = EpochModel;
    type Msg = EpochMsg;
    type Cmd = EpochCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<EpochModel, EpochCmd> {
        Update::quiet(EpochModel { rolled: vec![] })
    }

    fn update(model: &EpochModel, msg: &EpochMsg) -> Update<EpochModel, EpochCmd> {
        match msg {
            EpochMsg::Rolled { epoch } => {
                let mut rolled = model.rolled.clone();
                rolled.push(*epoch);
                Update::quiet(EpochModel { rolled })
            }
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> EpochMsg {
        EpochMsg::Rolled { epoch: u64::MAX }
    }
}

struct EpochRun;

impl Interpret<Epoch> for EpochRun {
    fn interpret(_cmd: &EpochCmd) -> Effect<EpochMsg, InterpretError> {
        Effect::sync(|| EpochMsg::Rolled { epoch: u64::MAX })
    }
}

#[test]
fn recurring_program_rolls_three_epochs_without_growing_history() {
    use elm_effect_journal::durable_log::MemLog;
    let clock = TestClock::new();
    clock.set_time(0);
    let mut engine = OurEngine::new();
    let mut app = Loop::<Epoch, EpochRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("ua49"),
    )
    .expect("boot");

    let mut prev = rollover_key("report", 0);
    let mut fence = started(&mut engine, &prev, clock.now_millis());
    for epoch in 1..=2u64 {
        clock.adjust(Duration::from_millis(60_000));
        let now_ms = clock.now_millis();
        let next = rollover_key("report", epoch);
        match engine
            .continue_as_new(&prev, fence, &next, now_ms, &StartOptions::default())
            .expect("rollover")
        {
            ContinueOutcome::Continued { closed, .. } => {
                assert_eq!(closed.key, prev);
                assert_eq!(closed.state, EffectState::Settled);
            }
            ContinueOutcome::Rejected(r) => panic!("epoch {epoch} must roll: {r:?}"),
        }
        app.send(EpochMsg::Rolled { epoch }).expect("send");
        prev = next;
        fence = engine.query(&prev).expect("query").generation;
    }
    assert_eq!(app.model().rolled, vec![1, 2]);
    // History is bounded: every past epoch is closed, only the head runs.
    for epoch in 0..2u64 {
        assert_eq!(
            engine
                .query(&rollover_key("report", epoch))
                .expect("query")
                .phase,
            RunPhase::Settled,
            "epoch {epoch} must be closed"
        );
    }
    assert_eq!(
        engine
            .query(&rollover_key("report", 2))
            .expect("query")
            .phase,
        RunPhase::Running,
        "only the head epoch runs"
    );
}
