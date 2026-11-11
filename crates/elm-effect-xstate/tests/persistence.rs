use elm_effect::{Duration, Effect, Exit, Runtime};
use elm_effect_xstate::prelude::*;
use std::collections::BTreeMap;

#[test]
fn snapshot_preserves_an_event_dependent_delay_without_replaying_entry() {
    let machine = DynamicMachine::builder(
        serde_json::from_str(
            r#"{"initial":"Waiting","states":{
                "Waiting":{
                    "after":{"timeout":"Done"},
                    "on":{"GO":{"target":"Waiting","reenter":true}}
                },
                "Done":{}
            }}"#,
        )
        .expect("schema"),
    )
    .delay("timeout", |_context, event| {
        event
            .payload
            .get("wait_ms")
            .and_then(serde_json::Value::as_u64)
            .map(Duration::from_millis)
    })
    .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(machine.clone()));
    let root = ActorId::new("root");
    system
        .send(
            "root",
            DynamicEvent::with_payload("GO", serde_json::json!({"wait_ms": 75})),
        )
        .expect("send GO");
    system.run().expect("system settles");
    system.take_timers();

    let restored = match system.state(&root).expect("machine state") {
        ActorState::Machine(state) => state.clone(),
        other => panic!("expected machine state, got {other:?}"),
    };
    let encoded = serde_json::to_string(&restored).expect("serialize snapshot");
    assert!(encoded.contains("75 millis"), "armed value is serialized");
    let decoded: MachineState<String, DynamicContext> =
        serde_json::from_str(&encoded).expect("deserialize snapshot");

    let mut resumed = ActorSystem::resume("root", machine, decoded);
    assert!(matches!(
        resumed.take_timers().as_slice(),
        [(actor, TimerIntent::Start { state, duration })]
            if actor == &root
                && state == "Waiting"
                && *duration == Duration::from_millis(75)
    ));
    resumed
        .fire_timer(
            &root,
            &TimerFired::new("Waiting".to_string(), Duration::from_millis(75)),
        )
        .expect("fire restored timer");
    resumed.run().expect("system settles");
    assert_eq!(
        resumed
            .configuration(&root)
            .expect("configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Done"]
    );
}

#[test]
fn actor_tree_snapshot_restores_children_without_reentry() {
    let root_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Running","states":{
            "Running":{"invoke":{"id":"worker","src":"worker"}}
        }}"#,
    )
    .expect("root schema");
    let worker = DynamicMachine::from_json_str(
        r#"{"initial":"Idle","context":{"log":[]},"states":{
            "Idle":{"entry":["mark"],"on":{"WORK":"Working"}},
            "Working":{}
        }}"#,
    )
    .expect("worker schema");
    let root = DynamicMachine::builder(root_config)
        .actor("worker", ActorLogic::from_machine(worker.clone()))
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root.clone()));
    system.run().expect("system settles");
    system
        .send("worker", DynamicEvent::new("WORK"))
        .expect("send WORK");
    system.run().expect("system settles");

    let encoded = serde_json::to_string(&system.snapshot()).expect("serialize actor tree");
    let decoded: ActorTreeSnapshot = serde_json::from_str(&encoded).expect("decode actor tree");
    assert_eq!(decoded.actors.len(), 2);

    let mut logic = BTreeMap::new();
    logic.insert(ActorId::new("root"), ActorLogic::from_machine(root));
    logic.insert(ActorId::new("worker"), ActorLogic::from_machine(worker));
    let mut restored =
        ActorSystem::restore(decoded, |id| logic.get(id).cloned()).expect("restore actor tree");
    assert_eq!(
        restored
            .configuration(&ActorId::new("worker"))
            .expect("worker configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Working"]
    );
    assert_eq!(
        restored
            .context(&ActorId::new("worker"))
            .expect("worker context")
            .log,
        ["mark"],
        "active entry actions do not run again"
    );
    restored.run().expect("restored system settles");
}

#[test]
fn actor_tree_snapshot_restarts_a_host_effect_from_recorded_input() {
    let root_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Running","states":{
            "Running":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "input":{"n":5},
                    "onDone":"Finished"
                }
            },
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let worker = || {
        ActorLogic::from_effect_with_input(|input| {
            let n = input.get("n").cloned().unwrap_or(serde_json::Value::Null);
            Effect::<DynamicEvent, String>::sync(move || {
                DynamicEvent::with_payload("READY", serde_json::json!({ "n": n }))
            })
        })
    };
    let root = DynamicMachine::builder(root_config)
        .actor("worker", worker())
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root.clone()));
    system.run().expect("system settles");
    assert_eq!(system.take_tasks().len(), 1);

    let encoded = serde_json::to_string(&system.snapshot()).expect("serialize actor tree");
    let decoded: ActorTreeSnapshot = serde_json::from_str(&encoded).expect("decode actor tree");
    let worker_entry = decoded
        .actors
        .iter()
        .find(|entry| entry.id == ActorId::new("worker"))
        .expect("worker snapshot");
    assert_eq!(worker_entry.input, serde_json::json!({"n": 5}));

    let mut logic = BTreeMap::new();
    logic.insert(ActorId::new("root"), ActorLogic::from_machine(root));
    logic.insert(ActorId::new("worker"), worker());
    let mut restored =
        ActorSystem::restore(decoded, |id| logic.get(id).cloned()).expect("restore actor tree");

    let tasks = restored.take_tasks();
    assert_eq!(tasks.len(), 1, "restore re-emits the host task");
    let task = tasks.into_iter().next().expect("one task");
    let event = match Runtime::run(task.effect) {
        Exit::Success(event) => event,
        Exit::Failure(cause) => panic!("effect failed: {cause:?}"),
    };
    assert_eq!(event.payload, serde_json::json!({"n": 5}));
    restored
        .complete(&task.actor, event.payload)
        .expect("complete worker");
    restored.run().expect("restored system settles");

    assert_eq!(
        restored
            .configuration(&ActorId::new("root"))
            .expect("root configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Finished"]
    );
}

#[test]
fn actor_tree_snapshot_restarts_a_callback_with_recorded_input() {
    let root_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Running","context":{"count":0},"states":{
            "Running":{"on":{"PING":{"actions":["bump"]}}}
        }}"#,
    )
    .expect("root schema");
    let root = DynamicMachine::builder(root_config)
        .action("bump", |context, _event, _enq| {
            if let Some(data) = context.data.as_object_mut() {
                let count = data
                    .get("count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                data.insert("count".into(), serde_json::json!(count + 1));
            }
        })
        .build();
    let callback = || {
        ActorLogic::from_callback(|sender| {
            assert!(sender.send(DynamicEvent::new("PING")));
        })
    };

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root.clone()));
    system
        .spawn(&ActorId::new("root"), "callback", callback())
        .expect("spawn callback");
    system.run().expect("system settles");

    let encoded = serde_json::to_string(&system.snapshot()).expect("serialize actor tree");
    let decoded: ActorTreeSnapshot = serde_json::from_str(&encoded).expect("decode actor tree");
    let mut logic = BTreeMap::new();
    logic.insert(ActorId::new("root"), ActorLogic::from_machine(root));
    logic.insert(ActorId::new("callback"), callback());
    let mut restored =
        ActorSystem::restore(decoded, |id| logic.get(id).cloned()).expect("restore actor tree");
    restored.run().expect("restored system settles");

    assert_eq!(
        restored
            .context(&ActorId::new("root"))
            .expect("root context")
            .data["count"],
        serde_json::json!(2),
        "restore runs the callback again"
    );
}

#[test]
fn resume_rearms_active_delays_and_fires_after_restore() {
    let machine = DynamicMachine::from_json_str(
        r#"{"initial":"Waiting","states":{
            "Waiting":{"after":{"500":"Done"}},
            "Done":{}
        }}"#,
    )
    .expect("schema");

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(machine.clone()));
    system.run().expect("system settles");
    let root = ActorId::new("root");
    assert!(matches!(
        system.take_timers().as_slice(),
        [(actor, TimerIntent::Start { state, duration })]
            if actor == &root
                && state == "Waiting"
                && *duration == Duration::from_millis(500)
    ));

    let restored = match system.state(&root).expect("machine state") {
        ActorState::Machine(state) => state.clone(),
        other => panic!("expected machine state, got {other:?}"),
    };
    let encoded = serde_json::to_string(&restored).expect("serialize snapshot");
    let decoded: MachineState<String, DynamicContext> =
        serde_json::from_str(&encoded).expect("deserialize snapshot");

    let mut resumed = ActorSystem::resume("root", machine, decoded);
    assert!(matches!(
        resumed.take_timers().as_slice(),
        [(actor, TimerIntent::Start { state, duration })]
            if actor == &root
                && state == "Waiting"
                && *duration == Duration::from_millis(500)
    ));

    resumed
        .fire_timer(
            &root,
            &TimerFired::new("Waiting".to_string(), Duration::from_millis(500)),
        )
        .expect("fire restored timer");
    resumed.run().expect("system settles");

    assert_eq!(
        resumed
            .configuration(&root)
            .expect("configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Done"]
    );
}
