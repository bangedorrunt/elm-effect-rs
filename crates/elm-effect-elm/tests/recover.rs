//! Outcome journaling: every fold input is in the log, so a loop rebuilt from
//! that log restores its model without running a provider, and its producer
//! resumes the sequence. One test function, because the provider counter is
//! process-global and test functions in one binary run in parallel.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Effect, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::{DurableLog, MemLog, Offset};
use serde::{Deserialize, Serialize};

static PROVIDER_RUNS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    Add(i64),
    Ran,
    Failed(Failure<InterpretError>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Step {
    Persist(i64),
}

impl Cmd for Step {
    fn durability(&self) -> Durability {
        Durability::Durable {
            key: "count".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Model {
    count: i64,
    provider_runs: usize,
}

struct Counter;

impl Program for Counter {
    type Model = Model;
    type Msg = Msg;
    type Cmd = Step;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<Model, Step> {
        Update::quiet(Model::default())
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, Step> {
        match msg {
            Msg::Add(n) => {
                let count = model.count + n;
                Update::with_commands(
                    Model {
                        count,
                        ..model.clone()
                    },
                    vec![Step::Persist(count)],
                )
            }
            Msg::Ran => Update::quiet(Model {
                provider_runs: model.provider_runs + 1,
                ..model.clone()
            }),
            Msg::Failed(_) => Update::quiet(model.clone()),
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> Msg {
        Msg::Failed(failure.clone())
    }
}

struct Run;

impl Interpret<Counter> for Run {
    fn interpret(cmd: &Step) -> Effect<Msg, InterpretError> {
        let Step::Persist(_) = cmd;
        Effect::sync(|| {
            PROVIDER_RUNS.fetch_add(1, Ordering::SeqCst);
            Msg::Ran
        })
    }
}

#[test]
fn recovery_rebuilds_the_model_and_resumes_the_producer() {
    let log: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let mut app = Loop::<Counter, Run>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::clone(&log),
        Producer::new("p"),
    )
    .unwrap();
    app.send(Msg::Add(3)).unwrap();
    app.send(Msg::Add(4)).unwrap();
    let live = app.model().clone();
    assert_eq!(live.count, 7);
    assert_eq!(live.provider_runs, 2);
    drop(app);

    // Recovery reads the log. It must not run a provider again.
    let runs_before = PROVIDER_RUNS.load(Ordering::SeqCst);
    let mut rebuilt = Loop::<Counter, Run>::recover(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::clone(&log),
        Producer::new("p"),
    )
    .unwrap();
    assert_eq!(
        rebuilt.model(),
        &live,
        "model rebuilt from journaled messages"
    );
    assert_eq!(rebuilt.intents().len(), 2, "intents restored from the log");
    assert_eq!(
        PROVIDER_RUNS.load(Ordering::SeqCst),
        runs_before,
        "recovery must not run providers"
    );

    // The producer resumed after the last record, so its next append is
    // accepted. A wrong resume point would be a sequence gap here.
    rebuilt.send(Msg::Add(1)).unwrap();
    assert_eq!(rebuilt.model().count, 8);

    // A snapshot bounds the next recovery: it starts from the snapshot model
    // and must match the live model exactly, with no provider run.
    rebuilt.snapshot().unwrap();
    let live_after_snapshot = rebuilt.model().clone();
    rebuilt.send(Msg::Add(10)).unwrap();
    let live_final = rebuilt.model().clone();
    assert_eq!(
        log.read_from(&Offset::Start).unwrap().len(),
        4,
        "the snapshot compacted the earlier records: snapshot, then message, intent, outcome"
    );
    drop(rebuilt);
    let runs_before = PROVIDER_RUNS.load(Ordering::SeqCst);
    let again = Loop::<Counter, Run>::recover(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::clone(&log),
        Producer::new("p"),
    )
    .unwrap();
    assert_eq!(
        again.model(),
        &live_final,
        "snapshot plus later records rebuild the model"
    );
    assert_ne!(
        live_after_snapshot, live_final,
        "the later send changed the model"
    );
    assert_eq!(PROVIDER_RUNS.load(Ordering::SeqCst), runs_before);
}
