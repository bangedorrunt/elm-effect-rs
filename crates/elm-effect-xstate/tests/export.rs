use elm_effect_journal::durable_log::{DurableLog, MemLog, Offset};
use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Player {
    Active,
    Idle,
    Playing,
    Stopped,
}

#[derive(Debug, Clone)]
enum Signal {
    Play,
}

impl std::fmt::Display for Player {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

fn machine() -> Machine<Player, Signal, (), ()> {
    Machine::builder(Player::Active, ())
        .compound(Player::Active, Player::Idle, |node| node)
        .state(Player::Idle, |node| {
            let play = node
                .transition(Player::Playing)
                .on(|e| matches!(e, Signal::Play))
                .label("PLAY");
            node.with_parent(Player::Active).on(play)
        })
        .state(Player::Playing, |node| {
            let stop = node.transition(Player::Stopped);
            node.with_parent(Player::Active)
                .after(elm_effect::Duration::from_millis(250), stop.clone())
                .on(stop)
        })
        .state(Player::Stopped, |node| {
            let idle = node.transition(Player::Idle);
            node.always(idle)
        })
        .build()
}

#[test]
fn mermaid_renders_hierarchy_and_edges() {
    let diagram = machine().to_mermaid();

    assert!(diagram.starts_with("stateDiagram-v2\n"));
    assert!(diagram.contains("    [*] --> Active\n"), "{diagram}");
    assert!(diagram.contains("    state Active {\n"), "{diagram}");
    assert!(diagram.contains("        [*] --> Idle\n"), "{diagram}");
    assert!(
        diagram.contains("        Idle --> Playing : PLAY\n"),
        "{diagram}"
    );
    assert!(
        diagram.contains("        Playing --> Stopped : after 250ms\n"),
        "{diagram}"
    );
    assert!(
        diagram.contains("    Stopped --> Idle : always\n"),
        "{diagram}"
    );
}

#[test]
fn a_transition_record_round_trips_through_the_durable_log() {
    let machine = machine();
    let before = machine.initial_configuration();
    let outcome = machine.step(&Player::Active, &(), &Signal::Play);
    let record = TransitionRecord::from_outcome(&before, &outcome, "PLAY", 9);

    assert_eq!(record.from, ["Idle"]);
    assert_eq!(record.to, ["Playing"]);
    assert_eq!(record.event, "PLAY");
    assert_eq!(record.generation, 9);

    let log = MemLog::new();
    record
        .append(
            &log,
            TransitionStamp {
                producer_id: "chart",
                epoch: 1,
                seq: 0,
            },
        )
        .expect("the log accepts one record");

    let records = log.read_from(&Offset::Start).expect("the log reads back");
    assert_eq!(records.len(), 1);
    let decoded: TransitionRecord =
        serde_json::from_str(&records[0].1).expect("the record decodes");
    assert_eq!(decoded, record);
}
