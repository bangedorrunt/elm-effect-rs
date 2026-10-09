//! Elm loop joint fence: every exit becomes a message, durable commands
//! append through the log before they run (and skip when append fails),
//! subscriber handles read later sends, and command lists run as bounded
//! map-reduce windows.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use elm_effect::{Cause, Defect, Effect, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::{DurableLog, MemLog, Offset};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Counter: additive model, durable persists, explosive and defective
// commands, failure reporting into the model.

#[derive(Clone, Debug, Default, PartialEq)]
struct CModel {
    count: i64,
    failures: Vec<Failure>,
    reported: Vec<Failure>,
}

#[derive(Clone, Debug, PartialEq)]
enum CMsg {
    Add(i64),
    Explode(String),
    Defect,
    Interrupt,
    Empty,
    Failed(Failure),
    Noop,
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
            Self::Persist { .. } => Durability::Durable { key: "count".to_string() },
            Self::Explode(_) | Self::Defect | Self::Interrupt | Self::Empty => Durability::Ephemeral,
        }
    }
}

struct Counter;

impl Program for Counter {
    type Model = CModel;
    type Msg = CMsg;
    type Cmd = CCmd;

    fn init(_boot: &Boot) -> Update<CModel, CCmd> {
        Update::quiet(CModel::default())
    }

    fn update(model: &CModel, msg: &CMsg) -> Update<CModel, CCmd> {
        match msg {
            CMsg::Add(n) => Update::with_commands(
                CModel { count: model.count + n, ..model.clone() },
                vec![CCmd::Persist { count: model.count + n }],
            ),
            CMsg::Explode(why) => Update::with_commands(model.clone(), vec![CCmd::Explode(why.clone())]),
            CMsg::Defect => Update::with_commands(model.clone(), vec![CCmd::Defect]),
            CMsg::Interrupt => Update::with_commands(model.clone(), vec![CCmd::Interrupt]),
            CMsg::Empty => Update::with_commands(model.clone(), vec![CCmd::Empty]),
            CMsg::Failed(failure) => {
                let mut m = model.clone();
                m.failures.push(failure.clone());
                Update::quiet(m)
            }
            CMsg::Noop => Update::quiet(model.clone()),
            CMsg::Report => {
                let mut m = model.clone();
                m.reported = m.failures.clone();
                Update::quiet(m)
            }
        }
    }

    fn failure(failure: &Failure) -> CMsg {
        CMsg::Failed(failure.clone())
    }
}

/// Every test that can run `CCmd::Persist` takes this lock: the counter is
/// a process-wide static, so an unlocked test silently breaks the counts
/// the provider tests assert. Measured: `subscriber_reads_model_after_later_sends`
/// folded `Add`, which persists, from outside the lock and both provider tests
/// failed under parallel threads.
static PROVIDER_LOCK: Mutex<()> = Mutex::new(());
static PROVIDER_RUNS: AtomicUsize = AtomicUsize::new(0);

struct Run;

impl Interpret<Counter> for Run {
    type Error = InterpretError;

    fn interpret(cmd: &CCmd) -> Effect<CMsg, InterpretError> {
        match cmd {
            CCmd::Persist { .. } => {
                PROVIDER_RUNS.fetch_add(1, Ordering::SeqCst);
                Effect::sync(|| CMsg::Noop)
            }
            CCmd::Explode(why) => Effect::fail(InterpretError(why.clone())),
            CCmd::Defect => Effect::die(Defect::new("kaboom")),
            CCmd::Interrupt => Effect::interrupt(),
            CCmd::Empty => Effect::fail_cause(Cause::empty()),
        }
    }
}

fn boot_app(log: Arc<dyn DurableLog>, producer: Producer) -> Loop<Counter, Run> {
    Loop::<Counter, Run>::boot(&Boot::default(), ManagedRuntime::new(), log, producer)
}

#[test]
fn failing_command_becomes_a_message_and_later_update_sees_it() {
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    app.send(CMsg::Explode("boom".to_string()));
    assert_eq!(app.model().failures.len(), 1, "failure reached the fold");
    // Match the kind structurally, not by grepping a rendered string.
    match &app.model().failures[0] {
        Failure::Fail(message) => assert!(message.contains("boom"), "{message}"),
        other => panic!("expected a typed failure, got {other:?}"),
    }
    // A later fold sees the failure recorded by the earlier one.
    app.send(CMsg::Report);
    assert_eq!(app.model().reported.len(), 1);
    assert!(matches!(app.model().reported[0], Failure::Fail(_)));
}

#[test]
fn defect_becomes_a_message_too() {
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    app.send(CMsg::Defect);
    assert_eq!(app.model().failures.len(), 1, "defect reached the fold");
    match &app.model().failures[0] {
        Failure::Defect(message) => assert_eq!(message, "kaboom"),
        other => panic!("expected a defect, got {other:?}"),
    }
}

#[test]
fn interrupt_becomes_a_message_and_is_not_swallowed_by_a_handler() {
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    app.send(CMsg::Interrupt);
    assert_eq!(app.model().failures.len(), 1, "interrupt reached the fold");
    // L2: an interrupt is its own kind. A program matching Fail here must
    // not accidentally take it, and a failure handler must not swallow it.
    assert_eq!(app.model().failures[0], Failure::Interrupted);
    // An empty cause is neither a failure nor an interrupt: reading it as
    // one would hide whichever reason the cause actually lost.
    app.send(CMsg::Empty);
    assert_eq!(
        app.model().failures[1],
        Failure::Defect("empty cause".to_string())
    );
}

#[test]
fn durable_command_appends_through_the_log_before_running() {
    let _guard = PROVIDER_LOCK.lock().unwrap();
    PROVIDER_RUNS.store(0, Ordering::SeqCst);
    let log: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let mut app = boot_app(Arc::clone(&log), Producer::new("test"));
    app.send(CMsg::Add(3));
    // The intent is in the log, not only the in-memory vec.
    let entries = log.read_from(&Offset::Start).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].1.contains("\"count\""), "payload carries the intent key");
    assert_eq!(app.intents().len(), 1);
    assert_eq!(PROVIDER_RUNS.load(Ordering::SeqCst), 1, "provider ran");
}

#[test]
fn append_failure_skips_the_provider() {
    let _guard = PROVIDER_LOCK.lock().unwrap();
    PROVIDER_RUNS.store(0, Ordering::SeqCst);
    let log: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    // seq 3 on an empty log is a gap: the append fails, the durable
    // command must not run.
    let producer = Producer { producer_id: "test".to_string(), epoch: 0, seq: 3 };
    let mut app = boot_app(Arc::clone(&log), producer);
    app.send(CMsg::Add(3));
    assert_eq!(PROVIDER_RUNS.load(Ordering::SeqCst), 0, "provider must not run");
    assert!(app.intents().is_empty());
    assert!(log.read_from(&Offset::Start).unwrap().is_empty());
    // The fold still learns the command was dropped: a rejection is a
    // reported outcome, not a silent skip.
    let reported = &app.model().failures;
    match reported.last() {
        Some(Failure::NotAppended(message)) => assert!(message.contains("seq gap"), "{message}"),
        other => panic!("expected NotAppended, got {other:?}"),
    }
}

#[test]
fn subscriber_reads_model_after_later_sends() {
    // Add folds into a durable Persist, so this test writes the shared
    // provider counter and takes the same lock the provider tests do.
    let _guard = PROVIDER_LOCK.lock().unwrap();
    let mut app = boot_app(Arc::new(MemLog::new()), Producer::new("test"));
    let sub = app.subscriber();
    app.send(CMsg::Add(1));
    assert_eq!(sub.read().count, 1, "handle taken before the send sees it");
    app.send(CMsg::Add(4));
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

#[derive(Clone, Debug, PartialEq)]
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

    fn failure(_failure: &Failure) -> WMsg {
        WMsg::NextDone(0)
    }
}

/// Serializes the two window tests around the shared order log.
static WINDOW_LOCK: Mutex<()> = Mutex::new(());
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct WinRun;

impl Interpret<Windowed> for WinRun {
    type Error = InterpretError;

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
    .with_bound(bound)
}

fn interleaved(prefix: &str, n: u32) -> Vec<String> {
    (1..=n).flat_map(|i| [format!("{prefix}{i}"), format!("next{i}")]).collect()
}

#[test]
fn windows_finish_before_the_next_starts() {
    let _guard = WINDOW_LOCK.lock().unwrap();
    ORDER.lock().unwrap().clear();
    let mut app = boot_windowed(1);
    app.send(WMsg::Start);
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
    );
    app.send(WMsg::Start);
    let order: Vec<String> = std::mem::take(&mut *ORDER.lock().unwrap());
    let mut expected: Vec<String> = (1..=8).map(|i| format!("run{i}")).collect();
    expected.extend((1..=8).map(|i| format!("next{i}")));
    expected.push("run9".to_string());
    expected.push("next9".to_string());
    assert_eq!(order, expected);
}