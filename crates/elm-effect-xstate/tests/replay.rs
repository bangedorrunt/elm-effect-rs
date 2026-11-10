use elm_effect_journal::durable_log::{DurableLog, MemLog, Offset};
use elm_effect_xstate::prelude::*;

fn machine() -> DynamicMachine {
    DynamicMachine::from_json_str(
        r#"{"initial":"Idle","states":{
            "Idle":{"on":{"GO":"Working"}},
            "Working":{"on":{"DONE":"Done"}},
            "Done":{"type":"final"}
        }}"#,
    )
    .expect("schema")
}

#[test]
fn replay_reconstructs_the_configuration_without_rerunning_actions() {
    let machine = machine();
    let started = machine.enter(machine.initial_context());
    let mut state = MachineState::new(started.configuration, started.context);
    let mut records = Vec::new();

    for (step, event) in ["GO", "DONE"].into_iter().enumerate() {
        let before = state.configuration.clone();
        let outcome = machine.step_state(&state, &DynamicEvent::new(event));
        records.push(TransitionRecord::from_outcome(
            &before,
            &outcome,
            event,
            step as u64 + 1,
        ));
        state = MachineState::new(outcome.configuration, outcome.context);
    }

    let replayed = TransitionRecord::replay(&records).expect("continuous chain");
    assert_eq!(replayed, state.configuration);
    assert_eq!(
        replayed.leaves().cloned().collect::<Vec<_>>(),
        ["Done".to_string()]
    );
}

#[test]
fn a_machine_actor_resumes_from_the_journal_without_replaying_actions() {
    let machine = DynamicMachine::from_json_str(
        r#"{"initial":"A","states":{
            "A":{"entry":["enter_a"],"on":{"GO":"B"}},
            "B":{"entry":["enter_b"],"on":{"DONE":"C"}},
            "C":{"entry":["enter_c"]}
        }}"#,
    )
    .expect("schema");
    let log = MemLog::new();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(machine.clone()));
    system
        .send("root", DynamicEvent::new("GO"))
        .expect("send GO");
    system.run().expect("system settles");
    let root = ActorId::new("root");
    let state = match system.state(&root).expect("root state") {
        ActorState::Machine(state) => state.clone(),
        other => panic!("expected machine state, got {other:?}"),
    };
    let before = system.configuration(&root).expect("configuration").clone();
    let record = TransitionRecord::new(&before, &state.configuration, "GO", 1)
        .with_snapshot(serde_json::to_string(&state).expect("serialize state"));
    record
        .append(
            &log,
            TransitionStamp {
                producer_id: "replay",
                epoch: 0,
                seq: 0,
            },
        )
        .expect("append record");
    drop(system);

    let records: Vec<TransitionRecord> = log
        .read_from(&Offset::Start)
        .expect("read journal")
        .into_iter()
        .map(|(_, raw)| serde_json::from_str(&raw).expect("decode record"))
        .collect();
    assert_eq!(records.len(), 1);
    let replayed = TransitionRecord::replay(&records).expect("continuous chain");
    assert_eq!(replayed, state.configuration);

    let snapshot = TransitionRecord::latest_snapshot(&records).expect("snapshot");
    let restored: MachineState<String, DynamicContext> =
        serde_json::from_str(snapshot).expect("deserialize state");
    assert_eq!(restored.context.log, ["enter_a", "enter_b"]);

    let mut resumed = ActorSystem::resume("root", machine, restored);
    assert_eq!(
        resumed.context(&root).expect("context").log,
        ["enter_a", "enter_b"],
        "resume must not re-run active entry actions"
    );
    resumed
        .send("root", DynamicEvent::new("DONE"))
        .expect("send DONE");
    resumed.run().expect("system settles");

    assert_eq!(
        resumed.context(&root).expect("context").log,
        ["enter_a", "enter_b", "enter_c"]
    );
    assert_eq!(
        resumed
            .configuration(&root)
            .expect("configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["C"]
    );
}

#[test]
fn replay_rejects_a_gap_between_records() {
    let machine = machine();
    let started = machine.enter(machine.initial_context());
    let before = started.configuration;
    let first = machine.step_state(
        &MachineState::new(before.clone(), started.context),
        &DynamicEvent::new("GO"),
    );
    let mut records = vec![TransitionRecord::from_outcome(&before, &first, "GO", 1)];

    let mut broken = records[0].clone();
    broken.from = vec!["Elsewhere".to_string()];
    records.push(broken);

    let error = TransitionRecord::replay(&records).expect_err("gap detected");
    assert!(matches!(error, ReplayError::Discontinuous { index: 1, .. }));
}

#[test]
fn an_empty_chain_replays_to_the_empty_configuration() {
    assert!(TransitionRecord::replay(&[]).expect("empty").is_empty());
}
