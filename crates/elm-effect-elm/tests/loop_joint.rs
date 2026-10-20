//! Elm loop joint fence: every exit becomes a message, durable commands
//! append through the log before they run (and skip when append fails),
//! subscriber handles read later sends, and command lists run as bounded
//! map-reduce windows.

use std::sync::{Arc, Mutex};

use elm_effect::{
    BuiltServices, Cause, Defect, Effect, Layer, ManagedRuntime, ServiceTag, use_service,
};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::{AppendError, DurableLog, MemLog, Offset};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Counter: additive model, durable persists, explosive and defective
// commands, failure reporting into the model.

#[derive(Clone, Debug, Default, PartialEq)]
struct CModel {
    count: i64,
    failures: Vec<Failure<InterpretError>>,
    reported: Vec<Failure<InterpretError>>,
    /// Provider runs observed by the fold. The test double holds this
    /// counter: nothing process-global, so parallel test threads cannot
    /// interleave it.
    provider_runs: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum CMsg {
    Add(i64),
    Explode(String),
    Defect,
    Interrupt,
    Empty,
    Failed(Failure<InterpretError>),
    /// Reported by the test double when its provider ran. The fold counts
    /// it; there is no static counter to guard.
    ProviderRan,
    Report,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum CCmd {
    Persist { count: i64 },
    Explode(String),
    Defect,
    Interrupt,
    Empty,
}

impl Cmd for CCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Persist { .. } => Durability::Durable {
                key: "count".to_string(),
            },
            Self::Explode(_) | Self::Defect | Self::Interrupt | Self::Empty => {
                Durability::Ephemeral
            }
        }
    }
}

struct Counter;

impl Program for Counter {
    type Model = CModel;
    type Msg = CMsg;
    type Cmd = CCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<CModel, CCmd> {
        Update::quiet(CModel::default())
    }

    fn update(model: &CModel, msg: &CMsg) -> Update<CModel, CCmd> {
        match msg {
            CMsg::Add(n) => Update::with_commands(
                CModel {
                    count: model.count + n,
                    ..model.clone()
                },
                vec![CCmd::Persist {
                    count: model.count + n,
                }],
            ),
            CMsg::Explode(why) => {
                Update::with_commands(model.clone(), vec![CCmd::Explode(why.clone())])
            }
            CMsg::Defect => Update::with_commands(model.clone(), vec![CCmd::Defect]),
            CMsg::Interrupt => Update::with_commands(model.clone(), vec![CCmd::Interrupt]),
            CMsg::Empty => Update::with_commands(model.clone(), vec![CCmd::Empty]),
            CMsg::Failed(failure) => {
                let mut m = model.clone();
                m.failures.push(failure.clone());
                Update::quiet(m)
            }
            CMsg::ProviderRan => {
                let mut m = model.clone();
                m.provider_runs += 1;
                Update::quiet(m)
            }
            CMsg::Report => {
                let mut m = model.clone();
                m.reported = m.failures.clone();
                Update::quiet(m)
            }
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> CMsg {
        CMsg::Failed(failure.clone())
    }
}

struct Run;

impl Interpret<Counter> for Run {
    fn interpret(cmd: &CCmd) -> Effect<CMsg, InterpretError> {
        match cmd {
            // The test double reports its own run. See CMsg::ProviderRan.
            CCmd::Persist { .. } => Effect::sync(|| CMsg::ProviderRan),
            CCmd::Explode(why) => Effect::fail(InterpretError(why.clone())),
            CCmd::Defect => Effect::die(Defect::new("kaboom")),
            CCmd::Interrupt => Effect::interrupt(),
            CCmd::Empty => Effect::fail_cause(Cause::empty()),
        }
    }
}

/// A log that refuses only durable intents. Messages still append, so a test
/// can fail the command path alone. A producer seq gap cannot do this: the
/// message and the intent share one seq counter, so the message fails first.
struct RefuseIntents(MemLog);

impl DurableLog for RefuseIntents {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        if envelope.contains("\"kind\":\"intent\"") {
            return Err(AppendError::Storage("injected: intent refused".to_string()));
        }
        self.0.append_json(envelope)
    }
    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.0.read_from(from)
    }
    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        self.0.compact_before(before)
    }
}

fn boot_app(log: Arc<dyn DurableLog>, producer: Producer) -> Loop<Counter, Run> {
    Loop::<Counter, Run>::boot(&Boot::default(), ManagedRuntime::new(), log, producer)
        .expect("boot")
}

#[test]
fn failing_command_becomes_a_message_and_later_update_sees_it() {
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    app.send(CMsg::Explode("boom".to_string())).expect("append");
    assert_eq!(app.model().failures.len(), 1, "failure reached the fold");
    // Match the kind and the typed value structurally, not by grepping a
    // rendered string: the fold sees the interpreter's error by value.
    match &app.model().failures[0] {
        Failure::Fail(error) => assert_eq!(error, &InterpretError("boom".to_string())),
        other => panic!("expected a typed failure, got {other:?}"),
    }
    // A later fold sees the failure recorded by the earlier one.
    app.send(CMsg::Report).expect("append");
    assert_eq!(app.model().reported.len(), 1);
    assert!(matches!(app.model().reported[0], Failure::Fail(_)));
}

#[test]
fn defect_becomes_a_message_too() {
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    app.send(CMsg::Defect).expect("append");
    assert_eq!(app.model().failures.len(), 1, "defect reached the fold");
    match &app.model().failures[0] {
        Failure::Defect(defect) => assert_eq!(defect.message, "kaboom"),
        other => panic!("expected a defect, got {other:?}"),
    }
}

#[test]
fn interrupt_becomes_a_message_and_is_not_swallowed_by_a_handler() {
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    app.send(CMsg::Interrupt).expect("append");
    assert_eq!(app.model().failures.len(), 1, "interrupt reached the fold");
    // L2: an interrupt is its own kind. A program matching Fail here must
    // not accidentally take it, and a failure handler must not swallow it.
    assert_eq!(app.model().failures[0], Failure::Interrupted);
    // An empty cause is neither a failure nor an interrupt: reading it as
    // one would hide whichever reason the cause actually lost.
    app.send(CMsg::Empty).expect("append");
    assert_eq!(
        app.model().failures[1],
        Failure::Defect(Defect::new("empty cause"))
    );
}

#[test]
fn durable_command_appends_through_the_log_before_running() {
    let log: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let mut app = boot_app(Arc::clone(&log), Producer::new("test"));
    app.send(CMsg::Add(3)).expect("append");
    // The intent is in the log, not only the in-memory vec.
    let entries = log.read_from(&Offset::Start).unwrap();
    // The log also holds the message and the outcome, so find the intent row.
    let intents: Vec<_> = entries
        .iter()
        .filter(|(_, body)| body.contains("\"kind\":\"intent\""))
        .collect();
    assert_eq!(intents.len(), 1, "exactly one durable intent");
    assert!(
        intents[0].1.contains("\"count\""),
        "payload carries the intent key"
    );
    assert_eq!(app.intents().len(), 1);
    // The provider ran, and the fold counted it. No process global.
    assert_eq!(app.model().provider_runs, 1);
}

#[test]
fn append_failure_skips_the_provider() {
    let log: Arc<dyn DurableLog> = Arc::new(RefuseIntents(MemLog::new()));
    // The intent append is refused, so the durable command must not run.
    let mut app = boot_app(Arc::clone(&log), Producer::new("test"));
    app.send(CMsg::Add(3)).expect("append");
    // The fold counted zero provider runs, so the provider did not run.
    assert_eq!(app.model().provider_runs, 0, "provider must not run");
    assert!(app.intents().is_empty());
    assert!(
        log.read_from(&Offset::Start)
            .unwrap()
            .iter()
            .all(|(_, body)| !body.contains("\"kind\":\"intent\"")),
        "no intent row reached the log"
    );
    // The fold still learns the command was dropped: a rejection is a
    // reported outcome carrying the log's typed error, not a silent skip.
    let reported = &app.model().failures;
    match reported.last() {
        Some(Failure::NotAppended(error)) => {
            assert!(matches!(error, AppendError::Storage(_)), "{error:?}")
        }
        other => panic!("expected NotAppended, got {other:?}"),
    }
}

#[test]
fn stale_epoch_producer_is_fenced_and_skips_the_provider() {
    // A newer incarnation of "test" already appended under epoch 1. A loop
    // booted as the old incarnation (epoch 0) is a zombie: its durable
    // command must be fenced at the log and its provider must not run.
    let log: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    log.append_json(&elm_effect_journal::laws::envelope("test", 1, 0, "null"))
        .unwrap();
    let mut app = boot_app(Arc::clone(&log), Producer::new("test"));
    app.send(CMsg::Add(3));
    assert_eq!(
        app.model().provider_runs,
        0,
        "a fenced provider must not run"
    );
    assert!(app.intents().is_empty());
    assert_eq!(
        log.read_from(&Offset::Start).unwrap().len(),
        1,
        "nothing written"
    );
    match app.model().failures.last() {
        Some(Failure::NotAppended(AppendError::StaleEpoch {
            producer,
            sent,
            current,
        })) => {
            assert_eq!((producer.as_str(), *sent, *current), ("test", 0, 1));
        }
        other => panic!("expected NotAppended(StaleEpoch), got {other:?}"),
    }
    // Negative control: the current incarnation (epoch 1, seq 1) appends.
    let producer = Producer {
        producer_id: "test".to_string(),
        epoch: 1,
        seq: 1,
    };
    let mut current = boot_app(Arc::clone(&log), producer);
    current.send(CMsg::Add(3));
    assert_eq!(current.model().provider_runs, 1);
    // Four rows, not two: this Loop journals fold inputs as well as intents
    // (recovery replays both), so the current incarnation writes its message,
    // its intent, and the provider's outcome message. The zombie above wrote
    // nothing: no duplicate rows, no replay of its fenced envelope.
    let rows = log.read_from(&Offset::Start).unwrap();
    let kinds: Vec<&str> = rows
        .iter()
        .map(|(_, body)| {
            if body.contains("\"kind\":\"intent\"") {
                "intent"
            } else if body.contains("\"kind\":\"msg\"") {
                "msg"
            } else {
                "direct"
            }
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["direct", "msg", "intent", "msg"],
        "direct + message + intent + outcome, nothing from the zombie"
    );
}

#[test]
fn subscriber_reads_model_after_later_sends() {
    // The fold owns the provider run count, so nothing else shares state
    // with the provider tests: no lock, no static, parallel threads safe.
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    let sub = app.subscriber();
    app.send(CMsg::Add(1)).expect("append");
    assert_eq!(sub.read().count, 1, "handle taken before the send sees it");
    app.send(CMsg::Add(4)).expect("append");
    assert_eq!(sub.read().count, 5, "handle sees every later fold");
}

// ---------------------------------------------------------------------------
// Windowed jobs: Do(i) records "run{i}" and folds to Done(i), which emits
// Next(i); Next(i) records "next{i}" and folds to quiet. The shared order
// log distinguishes "window completed before the next window" from "all
// commands ran first".

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum WCmd {
    Do(u32),
    Next(u32),
}

impl Cmd for WCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum WMsg {
    Start,
    Done(u32),
    NextDone(u32),
}

struct Windowed;

impl Program for Windowed {
    type Model = u32;
    type Msg = WMsg;
    type Cmd = WCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<u32, WCmd> {
        Update::quiet(0)
    }

    fn update(model: &u32, msg: &WMsg) -> Update<u32, WCmd> {
        match msg {
            WMsg::Start => {
                let cmds: Vec<WCmd> = (1..=9).map(WCmd::Do).collect();
                Update::with_commands(*model, cmds)
            }
            WMsg::Done(i) => Update::with_commands(*model, vec![WCmd::Next(*i)]),
            WMsg::NextDone(_) => Update::quiet(*model),
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> WMsg {
        WMsg::NextDone(0)
    }
}

/// Serializes the two window tests around the shared order log.
static WINDOW_LOCK: Mutex<()> = Mutex::new(());
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct WinRun;

impl Interpret<Windowed> for WinRun {
    fn interpret(cmd: &WCmd) -> Effect<WMsg, InterpretError> {
        match cmd {
            WCmd::Do(i) => {
                ORDER.lock().unwrap().push(format!("run{i}"));
                let n = *i;
                Effect::sync(move || WMsg::Done(n))
            }
            WCmd::Next(i) => {
                ORDER.lock().unwrap().push(format!("next{i}"));
                let n = *i;
                Effect::sync(move || WMsg::NextDone(n))
            }
        }
    }
}

fn boot_windowed(bound: usize) -> Loop<Windowed, WinRun> {
    Loop::<Windowed, WinRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("test"),
    )
    .expect("boot")
    .with_bound(bound)
}

fn interleaved(prefix: &str, n: u32) -> Vec<String> {
    (1..=n)
        .flat_map(|i| [format!("{prefix}{i}"), format!("next{i}")])
        .collect()
}

#[test]
fn windows_finish_before_the_next_starts() {
    let _guard = WINDOW_LOCK.lock().unwrap();
    ORDER.lock().unwrap().clear();
    let mut app = boot_windowed(1);
    app.send(WMsg::Start).expect("append");
    let order: Vec<String> = std::mem::take(&mut *ORDER.lock().unwrap());
    // bound 1: each command runs, folds, and cascades before the next runs.
    assert_eq!(order, interleaved("run", 9));
}

#[test]
fn default_bound_is_eight() {
    let _guard = WINDOW_LOCK.lock().unwrap();
    ORDER.lock().unwrap().clear();
    // No with_bound: default 8. The first window holds run1..run8, then
    // their folds cascade (next1..next8), then window two runs run9.
    let mut app = Loop::<Windowed, WinRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("test"),
    )
    .expect("boot");
    app.send(WMsg::Start).expect("append");
    let order: Vec<String> = std::mem::take(&mut *ORDER.lock().unwrap());
    let mut expected: Vec<String> = (1..=8).map(|i| format!("run{i}")).collect();
    expected.extend((1..=8).map(|i| format!("next{i}")));
    expected.push("run9".to_string());
    expected.push("next9".to_string());
    assert_eq!(order, expected);
}
// ---------------------------------------------------------------------------
// Overlap: a window's commands run concurrently, and still in order.

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum SCmd {
    Slow(u32),
}

impl Cmd for SCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum SMsg {
    /// Emits the window. A message, so the bound is already set when the
    /// window runs (boot's init window runs at the default bound before
    /// `with_bound` could apply).
    Launch,
    Done(u32),
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SModel {
    done: Vec<u32>,
    waits: Vec<u32>,
}

struct Overlapping;

impl Program for Overlapping {
    type Model = SModel;
    type Msg = SMsg;
    type Cmd = SCmd;
    type Err = InterpretError;

    fn init(boot: &Boot) -> Update<SModel, SCmd> {
        let waits = boot
            .get("waits")
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.is_empty())
            .filter_map(|w| w.parse().ok())
            .collect();
        Update::quiet(SModel {
            done: Vec::new(),
            waits,
        })
    }

    fn update(model: &SModel, msg: &SMsg) -> Update<SModel, SCmd> {
        match msg {
            SMsg::Launch => {
                let window = model.waits.iter().map(|w| SCmd::Slow(*w)).collect();
                Update::with_commands(model.clone(), window)
            }
            SMsg::Done(n) => {
                let mut m = model.clone();
                m.done.push(*n);
                Update::quiet(m)
            }
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> SMsg {
        SMsg::Done(u32::MAX)
    }
}

struct SlowRun;

impl Interpret<Overlapping> for SlowRun {
    fn interpret(cmd: &SCmd) -> Effect<SMsg, InterpretError> {
        let n = match cmd {
            SCmd::Slow(n) => *n,
        };
        // A provider that holds its thread: four of them overlapped cost
        // one wait, sequenced they cost four.
        Effect::sync(move || {
            std::thread::sleep(std::time::Duration::from_millis(n as u64));
            SMsg::Done(n)
        })
    }
}

fn boot_overlapping(bound: usize) -> (Loop<Overlapping, SlowRun>, std::time::Instant) {
    let boot = Boot::parse(vec!["waits=50,50,50,50".to_string()]);
    let started = std::time::Instant::now();
    let app = Loop::<Overlapping, SlowRun>::boot(
        &boot,
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("test"),
    )
    .expect("boot")
    .with_bound(bound);
    (app, started)
}

#[test]
fn a_window_overlaps_its_commands_and_keeps_their_order() {
    let (mut app, _started) = boot_overlapping(8);
    let started = std::time::Instant::now();
    app.send(SMsg::Launch).expect("append");
    let elapsed = started.elapsed();
    // 4 x 50ms: overlapped the window costs about one wait (~50ms),
    // sequenced about four (~200ms). 120ms sits in the empty middle, so a
    // loaded box cannot fake either answer by drifting.
    assert!(
        elapsed < std::time::Duration::from_millis(120),
        "a 4-command window of 50ms each took {elapsed:?}: it did not overlap"
    );
    // The overlap must not reorder the reduce.
    assert_eq!(app.model().done, vec![50, 50, 50, 50]);
}

#[test]
fn a_sequential_window_costs_the_sum_of_its_commands() {
    let (mut app, _started) = boot_overlapping(1);
    let started = std::time::Instant::now();
    app.send(SMsg::Launch).expect("append");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "bound 1 should run one at a time, costing about 200ms, took {elapsed:?}"
    );
    assert_eq!(app.model().done, vec![50, 50, 50, 50]);
}

/// A provider that panics inside a concurrent window: the runtime's run
/// edge catches the unwind, so the fold still gets a defect message and the
/// loop survives. Without that guard, `handle.join().unwrap()` would tear
/// the loop down on one bad provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum PCmd {
    Boom(i64),
}

impl Cmd for PCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum PMsg {
    /// Emits three panicking providers in one window.
    Went,
    /// The fold saw this defect.
    SawDefect,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct PModel {
    defects: usize,
}

struct Panicky;

impl Program for Panicky {
    type Model = PModel;
    type Msg = PMsg;
    type Cmd = PCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<PModel, PCmd> {
        Update::quiet(PModel::default())
    }

    fn update(model: &PModel, msg: &PMsg) -> Update<PModel, PCmd> {
        match msg {
            PMsg::Went => Update::with_commands(
                model.clone(),
                vec![PCmd::Boom(1), PCmd::Boom(2), PCmd::Boom(3)],
            ),
            PMsg::SawDefect => {
                let mut m = model.clone();
                m.defects += 1;
                Update::quiet(m)
            }
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> PMsg {
        // A panic is a defect, never a typed failure: L2 keeps them apart,
        // and this handler takes the defect path only.
        PMsg::SawDefect
    }
}

struct BoomRun;

impl Interpret<Panicky> for BoomRun {
    fn interpret(cmd: &PCmd) -> Effect<PMsg, InterpretError> {
        let n = match cmd {
            PCmd::Boom(n) => *n,
        };
        Effect::sync(move || panic!("provider {n} panicked"))
    }
}

/// A panic while *building* the effect (`interpret`) happens before the run,
/// on the calling thread. It is not a provider outcome, so it must not be
/// caught and re-labelled: the loop's contract is that a dropped or
/// misclassified outcome is impossible, not that every outcome is survivable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum BCmd {
    Landmine,
}

impl Cmd for BCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum BMsg {
    Went,
    SawDefect,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct BModel {
    explode: bool,
    defects: usize,
}

struct Landmine;

impl Program for Landmine {
    type Model = BModel;
    type Msg = BMsg;
    type Cmd = BCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<BModel, BCmd> {
        Update::quiet(BModel::default())
    }

    fn update(model: &BModel, msg: &BMsg) -> Update<BModel, BCmd> {
        match msg {
            BMsg::Went => {
                Update::with_commands(model.clone(), vec![BCmd::Landmine, BCmd::Landmine])
            }
            BMsg::SawDefect => {
                let mut m = model.clone();
                m.defects += 1;
                Update::quiet(m)
            }
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> BMsg {
        BMsg::SawDefect
    }
}

struct LandmineRun;

impl Interpret<Landmine> for LandmineRun {
    fn interpret(cmd: &BCmd) -> Effect<BMsg, InterpretError> {
        // Runs off the run edge: outside its catch_unwind, on the calling
        // thread. A panic here is a bug in the interpreter, not a provider
        // outcome, and must reach the host rather than become a message.
        match cmd {
            BCmd::Landmine => panic!("interpreter exploded while building the effect"),
        }
    }
}

#[test]
#[should_panic(expected = "interpreter exploded")]
fn a_panic_in_interpret_is_not_swallowed_as_a_provider_outcome() {
    let mut app = Loop::<Landmine, LandmineRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("test"),
    )
    .expect("boot");
    app.send(BMsg::Went).expect("append");
}

#[test]
fn a_panicking_provider_in_a_window_becomes_a_defect_not_a_crash() {
    let mut app = Loop::<Panicky, BoomRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("test"),
    )
    .expect("boot");
    app.send(PMsg::Went).expect("append");
    // Each of the three panicked, and the loop folded three defects
    // instead of dying on the first `join`.
    assert_eq!(app.model().defects, 3);
}

// ---------------------------------------------------------------------------
// A service-holding interpreter. `interpret` is static and takes no
// parameters a host could use to hand it the world, so `use_service` inside
// one could never close its tag and the loop's built services were read
// back for tests but never consulted at a run. `interpret_in_env` is the
// seam: the loop passes its own `BuiltServices` in, and the interpreter
// closes the tag with `provide_from`.

/// The world a single-threaded host would hand an interpreter.
#[derive(Clone, Debug, PartialEq)]
struct LedgerWorld {
    connection: String,
}

struct LedgerTag;

impl ServiceTag for LedgerTag {
    type Service = LedgerWorld;
    const ID: &'static str = "test/loop-ledger";
}

#[derive(Clone, Debug, Default, PartialEq)]
struct LedgerModel {
    seen: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum LedgerMsg {
    Read,
    ReadBack(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum LedgerCmd {
    Read,
}

impl Cmd for LedgerCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

struct LedgerReader;

impl Program for LedgerReader {
    type Model = LedgerModel;
    type Msg = LedgerMsg;
    type Cmd = LedgerCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<LedgerModel, LedgerCmd> {
        Update::quiet(LedgerModel::default())
    }

    fn update(model: &LedgerModel, msg: &LedgerMsg) -> Update<LedgerModel, LedgerCmd> {
        match msg {
            LedgerMsg::Read => Update::with_commands(model.clone(), vec![LedgerCmd::Read]),
            LedgerMsg::ReadBack(value) => {
                let mut next = model.clone();
                next.seen.push(value.clone());
                Update::quiet(next)
            }
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> LedgerMsg {
        LedgerMsg::ReadBack(format!("failure: {failure:?}"))
    }
}

impl Interpret<LedgerReader> for LedgerReader {
    /// The static form, which cannot reach the world. A loop that runs this
    /// one dies with a named defect rather than guessing a default.
    fn interpret(_cmd: &LedgerCmd) -> Effect<LedgerMsg, InterpretError> {
        Effect::die(Defect::new(
            "interpret called without an environment: boot the loop with a services runtime",
        ))
    }

    fn interpret_in_env(
        _cmd: &LedgerCmd,
        services: Option<&BuiltServices>,
    ) -> Effect<LedgerMsg, InterpretError> {
        let Some(services) = services else {
            return Effect::die(Defect::new(
                "no services: boot the loop with ManagedRuntime::make",
            ));
        };
        // The composition a real interpreter writes: resolve the tag, widen
        // its `Infallible` channel to the program's typed error, then close
        // the tag out of the runtime's layer instead of capturing a value.
        use_service::<LedgerTag>()
            .map(|world| world.connection)
            .map_err(|_: std::convert::Infallible| InterpretError("unreachable".into()))
            .map(LedgerMsg::ReadBack)
            .provide_from(services)
    }
}

fn boot_ledger(runtime: ManagedRuntime) -> Loop<LedgerReader, LedgerReader> {
    Loop::<LedgerReader, LedgerReader>::boot(
        &Boot::default(),
        runtime,
        Arc::new(MemLog::new()),
        Producer::new("test"),
    )
    .expect("boot")
}

#[test]
fn a_services_runtime_reaches_a_static_interpreter() {
    let layer = Layer::<LedgerTag, std::convert::Infallible, ()>::from_value(LedgerWorld {
        connection: "sqlite://app.db".to_string(),
    });
    let mut app = boot_ledger(ManagedRuntime::make(&layer).expect("layer builds"));

    app.send(LedgerMsg::Read).expect("append");

    assert_eq!(
        app.model().seen,
        vec!["sqlite://app.db".to_string()],
        "the interpreter reached the world the loop was booted with"
    );
}

#[test]
fn a_loop_without_services_refuses_loudly_instead_of_defaulting() {
    // `ManagedRuntime::new` builds no layer, so the interpreter dies with a
    // named defect. That defect is folded, not raised: the loop stays alive
    // and every other command in the window still reports.
    let mut app = boot_ledger(ManagedRuntime::new());

    app.send(LedgerMsg::Read).expect("append");

    assert_eq!(app.model().seen.len(), 1, "the defect reached the fold");
    assert!(
        app.model().seen[0].contains("no services"),
        "the fold saw the named defect, got: {}",
        app.model().seen[0]
    );
}
