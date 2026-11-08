//! A rejected durable append reaches the fold with the command that caused
//! it, so a program can name the intent key the log refused. When no command
//! is in hand the hook falls back to `Program::failure`.

use std::sync::Arc;

use elm_effect::{Effect, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::Status;
use elm_effect_journal::durable_log::{AppendError, DurableLog, MemLog, Offset};
use serde::{Deserialize, Serialize};

/// Messages append; durable intents are refused. That isolates the command
/// path: a producer seq gap cannot be used because both share one counter.
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
    fn close(&self) -> Result<(), AppendError> {
        self.0.close()
    }
    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        self.0.append_json_and_close(envelope)
    }
    fn status(&self) -> Status {
        self.0.status()
    }
}

/// Every append is refused, before any command can run.
struct RefuseAll(MemLog);

impl DurableLog for RefuseAll {
    fn append_json(&self, _envelope: &str) -> Result<Offset, AppendError> {
        Err(AppendError::Storage("injected: log down".to_string()))
    }
    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.0.read_from(from)
    }
    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        self.0.compact_before(before)
    }
    fn close(&self) -> Result<(), AppendError> {
        self.0.close()
    }
    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        self.0.append_json_and_close(envelope)
    }
    fn status(&self) -> Status {
        self.0.status()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Model {
    /// Set when `failure_for` saw a command; the string is its durable key.
    keyed: Option<String>,
    /// Set when the loop called `failure` instead, which carries no command.
    fallback: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    Add,
    Ran,
    RejectedByKey(String),
    RejectedWithoutCommand,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Persist;

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
            Msg::Add => Update::with_commands(model.clone(), vec![Persist]),
            Msg::Ran => Update::quiet(model.clone()),
            Msg::RejectedByKey(key) => Update::quiet(Model {
                keyed: Some(key.clone()),
                ..model.clone()
            }),
            Msg::RejectedWithoutCommand => Update::quiet(Model {
                fallback: true,
                ..model.clone()
            }),
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> Msg {
        Msg::RejectedWithoutCommand
    }

    fn failure_for(cmd: Option<&Persist>, failure: &Failure<InterpretError>) -> Msg {
        match cmd.map(|cmd| cmd.durability()) {
            Some(Durability::Durable { key }) => Msg::RejectedByKey(key),
            _ => Self::failure(failure),
        }
    }
}

struct Run;

impl Interpret<App> for Run {
    fn interpret(_cmd: &Persist) -> Effect<Msg, InterpretError> {
        Effect::succeed(Msg::Ran)
    }
}

fn boot(log: Arc<dyn DurableLog>) -> Loop<App, Run> {
    Loop::<App, Run>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        log,
        Producer::new("test"),
    )
    .expect("boot")
}

#[test]
fn a_rejected_intent_reaches_the_fold_with_its_key() {
    let mut app = boot(Arc::new(RefuseIntents(MemLog::new())));
    app.send(Msg::Add).expect("the failure message appends");
    assert_eq!(app.model().keyed.as_deref(), Some("count"));
    assert!(!app.model().fallback, "the loop must pass the command");
}

#[test]
fn a_rejection_with_no_command_uses_the_default_hook() {
    let mut app = boot(Arc::new(RefuseAll(MemLog::new())));
    assert!(app.send(Msg::Add).is_err(), "the message append fails");
    assert!(app.model().fallback);
    assert_eq!(app.model().keyed, None);
}
