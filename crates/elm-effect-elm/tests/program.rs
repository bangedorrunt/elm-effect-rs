//! Elm loop fence: Program/Update/Boot fold, durable intents,
//! interpretation feedback, subscriber read-only shape.

use elm_effect::{Effect, ManagedRuntime};
use elm_effect_elm::{Boot, Cmd, Durability, Interpret, InterpretError, Loop, Program, Subscriber, Update};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
struct Model {
    count: i64,
}

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Tick,
    Add(i64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum CounterCmd {
    Persist { count: i64 },
    Log(String),
}

impl Cmd for CounterCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Persist { .. } => Durability::Durable { key: "count".to_string() },
            Self::Log(_) => Durability::Ephemeral,
        }
    }
}

struct Counter;

impl Program for Counter {
    type Model = Model;
    type Msg = Msg;
    type Cmd = CounterCmd;

    fn init(boot: &Boot) -> Update<Model, CounterCmd> {
        let start: i64 = boot.get("start").and_then(|s| s.parse().ok()).unwrap_or(0);
        Update::quiet(Model { count: start })
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, CounterCmd> {
        match msg {
            Msg::Tick => Update::with_commands(
                Model { count: model.count + 1 },
                vec![CounterCmd::Persist { count: model.count + 1 }],
            ),
            Msg::Add(n) => Update::quiet(Model { count: model.count + n }),
        }
    }
}

struct Run;

impl Interpret<Counter> for Run {
    type Error = InterpretError;

    fn interpret(cmd: &CounterCmd) -> Effect<Msg, InterpretError> {
        // Persistence completes silently: the fold already moved the model.
        match cmd.clone() {
            CounterCmd::Persist { .. } => Effect::sync(|| Msg::Add(0)),
            CounterCmd::Log(_) => Effect::sync(|| Msg::Add(0)),
        }
    }
}

#[test]
fn boot_parses_config_without_live_handles() {
    let boot = Boot::parse(vec!["start=5".to_string(), "verbose".to_string()]);
    assert_eq!(boot.get("start"), Some("5"));
    assert_eq!(boot.args, vec!["verbose".to_string()]);
}

#[test]
fn fold_updates_model() {
    let mut app = Loop::<Counter, Run>::boot(&Boot::parse(vec!["start=5".to_string()]), ManagedRuntime::new());
    assert_eq!(app.model().count, 5);
    // Time enters as a message.
    app.send(Msg::Tick);
    assert_eq!(app.model().count, 6);
    app.send(Msg::Add(4));
    assert_eq!(app.model().count, 10);
}

#[test]
fn durable_intents_recorded_ephemeral_not() {
    let mut app = Loop::<Counter, Run>::boot(&Boot::default(), ManagedRuntime::new());
    app.send(Msg::Tick);
    let keys: Vec<&str> = app.intents().iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, vec!["count"]);
    let body = &app.intents()[0].body;
    assert_eq!(body["Persist"]["count"], serde_json::json!(1));
}

#[test]
fn interpret_feedback_runs_commands() {
    // Persist interprets to Add(0): model moves only by the Tick fold.
    let mut app = Loop::<Counter, Run>::boot(&Boot::default(), ManagedRuntime::new());
    app.send(Msg::Tick);
    assert_eq!(app.model().count, 1);
    assert_eq!(app.intents().len(), 1);
}

#[test]
fn subscriber_cannot_write() {
    fn only_reads(s: Subscriber<Model>) -> i64 {
        s.read().count
    }
    let app = Loop::<Counter, Run>::boot(&Boot::default(), ManagedRuntime::new());
    assert_eq!(only_reads(app.subscriber()), 0);
}

#[test]
fn commands_serde_round_trip() {
    let cmd = CounterCmd::Persist { count: 7 };
    let json = serde_json::to_string(&cmd).unwrap();
    let back: CounterCmd = serde_json::from_str(&json).unwrap();
    assert_eq!(cmd, back);
}
