//! Example: one `Program` that replays a message log.
//!
//! A long-lived program is the fold, and the message log is its journal.
//! Replay re-folds the recorded messages and compares the commands each
//! step derived, so a divergence surfaces instead of replaying silently.
//! No provider re-executes: provider results are already messages in the
//! log.
//!
//! Run it: `cargo run -p elm-effect-elm --example program_replay`

use elm_effect_elm::replay::{InMemoryLog, MessageLog, Player, Replay, Snapshot};
use elm_effect_elm::{Boot, Cmd as CmdTrait, Durability, Failure, Program, Update};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
struct Model {
    count: i64,
}

/// Commands are data, so they serialize and replay compares them structurally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Cmd {
    Persist { count: i64 },
}

impl CmdTrait for Cmd {
    fn durability(&self) -> Durability {
        Durability::Durable { key: "count".to_string() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    Tick,
    Add(i64),
}

struct Counter;

impl Program for Counter {
    type Model = Model;
    type Msg = Msg;
    type Cmd = Cmd;
    type Err = std::convert::Infallible;

    fn init(boot: &Boot) -> Update<Model, Cmd> {
        let start: i64 = boot.get("start").and_then(|s| s.parse().ok()).unwrap_or(0);
        Update::quiet(Model { count: start })
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, Cmd> {
        match msg {
            Msg::Tick => Update::with_commands(
                Model { count: model.count + 1 },
                vec![Cmd::Persist { count: model.count + 1 }],
            ),
            Msg::Add(n) => Update::quiet(Model { count: model.count + n }),
        }
    }

    fn failure(_failure: &Failure<std::convert::Infallible>) -> Msg {
        // `Infallible` is the failure channel: this interpreter cannot fail.
        Msg::Add(0)
    }
}

fn main() {
    // The host reads its configuration before the fold starts.
    let boot = Boot::parse(vec!["start=5".to_string()]);
    let init = Counter::init(&boot);

    // Time arrives as messages. The host samples the clock and sends; the
    // commands each fold emits are the program's outgoing intents.
    let mut log = InMemoryLog::<Msg>::default();
    let mut player: Player<Counter> = Player::boot(init.model);
    for msg in [Msg::Tick, Msg::Add(4), Msg::Tick] {
        log.push(msg.clone());
        player.send(msg);
    }

    // Replay re-folds from the latest snapshot and matches every command
    // list against what the live player recorded.
    match player.replay() {
        Replay::Consistent { model } => {
            println!("replay consistent, count {}", model.count);
            // A snapshot is the model plus the offset it includes, so a later
            // replay can resume without re-reading the whole log.
            let snapshot = Snapshot { model, offset: log.len() };
            println!("{} messages recorded, snapshot at offset {}", log.len(), snapshot.offset);
        }
        Replay::Divergent { index } => {
            eprintln!("diverged at message {index}");
            std::process::exit(1);
        }
    }

    let replayed: Vec<Msg> = log.messages_from(0);
    println!("folded {} messages from the log", replayed.len());
}
