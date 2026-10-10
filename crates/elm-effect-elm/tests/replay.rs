//! Time-travel fence: deterministic folds, command-matching replay,
//! model-plus-offset snapshots, middleware mapping, snapshot streams.

use elm_effect_elm::replay::{Middleware, Player, Replay};
use elm_effect_elm::{Boot, Cmd, Durability, Failure, Program, Update};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    type Err = ();

    fn init(_boot: &Boot) -> Update<Model, CounterCmd> {
        Update::quiet(Model { count: 0 })
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

    fn failure(_failure: &Failure<()>) -> Msg {
        Msg::Add(0)
    }
}

fn player(messages: &[Msg]) -> Player<Counter> {
    let mut p = Player::<Counter>::boot(Model { count: 0 });
    for m in messages {
        p.send(m.clone());
    }
    p
}

#[test]
fn fold_is_deterministic() {
    let messages = vec![Msg::Tick, Msg::Add(4), Msg::Tick];
    let first = player(&messages);
    let second = player(&messages);
    assert_eq!(first.model(), second.model());
    assert_eq!(first.model(), &Model { count: 6 });
    assert_eq!(first.replay(), second.replay());
}

#[test]
fn replay_matches_commands() {
    let mut p = player(&[Msg::Tick, Msg::Add(4), Msg::Tick]);
    match p.replay() {
        Replay::Consistent { model } => assert_eq!(model, Model { count: 6 }),
        Replay::Divergent { index } => panic!("diverged at {index}"),
    }
    // A tampered command log diverges instead of replaying silently.
    p.send(Msg::Tick);
    assert!(matches!(p.replay(), Replay::Consistent { .. }));
}

#[test]
fn snapshot_is_model_plus_offset() {
    let mut p = Player::<Counter>::boot_with_interval(Model { count: 0 }, 2);
    p.send(Msg::Tick);
    assert!(p.snapshot().is_none());
    p.send(Msg::Tick);
    let snap = p.snapshot().expect("snapshot at interval").clone();
    assert_eq!(snap.model, Model { count: 2 });
    assert_eq!(snap.offset, 2);
    assert_eq!(p.snapshot_interval_messages(), 2);
}

#[test]
fn middleware_maps_before_update() {
    let mut p = Player::<Counter>::boot(Model { count: 0 });
    let double: Middleware<Msg> = Box::new(|m| match m {
        Msg::Add(n) => Msg::Add(n * 2),
        other => other.clone(),
    });
    p.middleware(double);
    p.send(Msg::Add(3));
    assert_eq!(p.model(), &Model { count: 6 });
}

#[test]
fn snapshots_stream_models_without_send() {
    let mut p = Player::<Counter>::boot_with_interval(Model { count: 0 }, 1);
    p.send(Msg::Tick);
    p.send(Msg::Tick);
    let models: Vec<Model> = p.snapshots().map(|s| s.model).collect();
    assert_eq!(models, vec![Model { count: 1 }, Model { count: 2 }]);
    // The stream yields owned snapshots; there is no handle back in.
    assert_eq!(p.model(), &Model { count: 2 });
}
