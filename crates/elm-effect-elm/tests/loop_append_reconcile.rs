//! The durable log's `Unavailable` is an unknown outcome, not a failure.
//! The loop must read the log for its producer triple before retrying, or a
//! lost acknowledgement becomes a duplicate append or a false `NotAppended`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Effect, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::{AppendError, DurableLog, MemLog, Offset};
use elm_effect_journal::stream::Status;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
enum Behavior {
    /// The append lands, but the log loses the acknowledgement.
    StoredAckLost,
    /// The append does not land, and the retry does.
    NotStoredThenStore,
    /// Every append reports an unknown outcome and stores nothing.
    AlwaysLost,
    /// The same producer triple is stored with a different payload.
    StoredDifferentPayload,
}

struct FlakyLog {
    inner: MemLog,
    behavior: Behavior,
    attempts: AtomicUsize,
}

impl FlakyLog {
    fn new(behavior: Behavior) -> Self {
        Self {
            inner: MemLog::new(),
            behavior,
            attempts: AtomicUsize::new(0),
        }
    }
}

impl DurableLog for FlakyLog {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        match (self.behavior, attempt) {
            (Behavior::StoredAckLost, 0) => {
                self.inner.append_json(envelope)?;
                Err(AppendError::Unavailable("lost ack".to_string()))
            }
            (Behavior::NotStoredThenStore, 0) => {
                Err(AppendError::Unavailable("not stored".to_string()))
            }
            (Behavior::AlwaysLost, _) => Err(AppendError::Unavailable("down".to_string())),
            (Behavior::StoredDifferentPayload, 0) => {
                let mut value: serde_json::Value =
                    serde_json::from_str(envelope).expect("valid envelope");
                value["payload"] = serde_json::json!({ "kind": "not-this-message" });
                self.inner.append_json(&value.to_string())?;
                Err(AppendError::Unavailable("lost ack".to_string()))
            }
            _ => self.inner.append_json(envelope),
        }
    }

    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.inner.read_from(from)
    }

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        self.inner.compact_before(before)
    }

    fn close(&self) -> Result<(), AppendError> {
        self.inner.close()
    }

    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        self.inner.append_json_and_close(envelope)
    }

    fn status(&self) -> Status {
        self.inner.status()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Model {
    count: i64,
    provider_runs: usize,
    failures: Vec<Failure<InterpretError>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    Add(i64),
    Ran,
    Failed(Failure<InterpretError>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Persist(i64);

impl Cmd for Persist {
    fn durability(&self) -> Durability {
        Durability::Durable {
            key: "count".to_string(),
        }
    }
}

struct App;

impl Program for App {
    type Model = Model;
    type Msg = Msg;
    type Cmd = Persist;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<Model, Persist> {
        Update::quiet(Model::default())
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, Persist> {
        match msg {
            Msg::Add(n) => Update::with_commands(
                Model {
                    count: model.count + n,
                    ..model.clone()
                },
                vec![Persist(*n)],
            ),
            Msg::Ran => Update::quiet(Model {
                provider_runs: model.provider_runs + 1,
                ..model.clone()
            }),
            Msg::Failed(failure) => {
                let mut next = model.clone();
                next.failures.push(failure.clone());
                Update::quiet(next)
            }
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> Msg {
        Msg::Failed(failure.clone())
    }
}

struct Run;

impl Interpret<App> for Run {
    fn interpret(_cmd: &Persist) -> Effect<Msg, InterpretError> {
        Effect::succeed(Msg::Ran)
    }
}

fn boot(log: Arc<FlakyLog>) -> Loop<App, Run> {
    Loop::<App, Run>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        log,
        Producer::new("reconcile"),
    )
    .expect("boot")
}

#[test]
fn a_lost_ack_is_reconciled_without_a_second_append() {
    let log = Arc::new(FlakyLog::new(Behavior::StoredAckLost));
    let mut app = boot(Arc::clone(&log));
    app.send(Msg::Add(1)).expect("the append landed");
    assert_eq!(app.model().count, 1);
    assert_eq!(app.model().provider_runs, 1);
    assert!(app.model().failures.is_empty());
    // The message, the durable intent, and the outcome.
    assert_eq!(
        log.read_from(&Offset::Start).unwrap().len(),
        3,
        "reconciliation must not append the same record twice"
    );
}

#[test]
fn an_unavailable_append_is_retried_once_when_the_log_is_empty() {
    let log = Arc::new(FlakyLog::new(Behavior::NotStoredThenStore));
    let mut app = boot(Arc::clone(&log));
    app.send(Msg::Add(1)).expect("the retry landed");
    assert_eq!(app.model().count, 1);
    assert_eq!(app.model().provider_runs, 1);
    assert!(app.model().failures.is_empty());
    assert_eq!(log.read_from(&Offset::Start).unwrap().len(), 3);
}

#[test]
fn a_log_that_never_answers_is_not_a_success() {
    let log = Arc::new(FlakyLog::new(Behavior::AlwaysLost));
    let mut app = boot(log);
    let error = app.send(Msg::Add(1)).expect_err("the append is unknown");
    assert!(matches!(error, AppendError::Unavailable(_)));
    assert_eq!(app.model().provider_runs, 0);
    assert!(matches!(
        app.model().failures.last(),
        Some(Failure::Unknown(AppendError::Unavailable(_)))
    ));
}

#[test]
fn a_same_triple_with_a_different_payload_is_corruption() {
    let log = Arc::new(FlakyLog::new(Behavior::StoredDifferentPayload));
    let mut app = boot(log);
    let error = app.send(Msg::Add(1)).expect_err("the triple is corrupt");
    assert!(matches!(error, AppendError::PayloadMismatch { .. }));
    assert_eq!(app.model().provider_runs, 0);
    assert!(matches!(
        app.model().failures.last(),
        Some(Failure::NotAppended(AppendError::PayloadMismatch { .. }))
    ));
}
