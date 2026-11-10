use elm_effect_xstate::prelude::*;
use std::sync::{Arc, Mutex};

fn id(name: &str) -> ActorId {
    ActorId::new(name)
}

fn worker() -> DynamicMachine {
    DynamicMachine::from_json_str(r#"{"initial":"Waiting","states":{"Waiting":{}}}"#)
        .expect("worker schema")
}

fn spawning_root() -> DynamicMachine {
    let config: MachineConfig =
        serde_json::from_str(r#"{"initial":"Idle","states":{"Idle":{"entry":["spawn_worker"]}}}"#)
            .expect("root schema");
    DynamicMachine::builder(config)
        .action("spawn_worker", |_context, _event, enq| {
            enq.spawn("worker", ActorLogic::from_machine(worker()));
        })
        .build()
}

#[test]
fn entry_action_spawns_a_child_actor() {
    let mut system = ActorSystem::new("root", ActorLogic::from_machine(spawning_root()));
    system.run().expect("system settles");

    let worker = id("worker");
    assert!(system.exists(&worker));
    assert_eq!(system.parent(&worker), Some(&id("root")));
    let leaves: Vec<_> = system
        .configuration(&worker)
        .expect("worker runs")
        .leaves()
        .cloned()
        .collect();
    assert_eq!(leaves, ["Waiting"]);
}

fn pinging_root() -> DynamicMachine {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Idle","states":{
            "Idle":{"entry":["spawn_worker"],"on":{"START":"Running"}},
            "Running":{"entry":["ping_worker"]}
        }}"#,
    )
    .expect("root schema");
    DynamicMachine::builder(config)
        .action("spawn_worker", |_context, _event, enq| {
            enq.spawn("worker", ActorLogic::from_machine(responding_worker()));
        })
        .action("ping_worker", |_context, _event, enq| {
            enq.send_to("worker", DynamicEvent::new("WORK"));
        })
        .build()
}

fn responding_worker() -> DynamicMachine {
    DynamicMachine::from_json_str(
        r#"{"initial":"Idle","states":{"Idle":{"on":{"WORK":"Working"}},"Working":{}}}"#,
    )
    .expect("worker schema")
}

#[test]
fn send_to_delivers_to_a_child_actor() {
    let mut system = ActorSystem::new("root", ActorLogic::from_machine(pinging_root()));
    system
        .send("root", DynamicEvent::new("START"))
        .expect("root receives START");
    system.run().expect("system settles");

    let leaves: Vec<_> = system
        .configuration(&id("worker"))
        .expect("worker runs")
        .leaves()
        .cloned()
        .collect();
    assert_eq!(leaves, ["Working"]);
}

fn stopping_root() -> DynamicMachine {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Idle","states":{
            "Idle":{"entry":["spawn_worker"],"on":{"STOP":"Stopped"}},
            "Stopped":{"entry":["stop_worker"]}
        }}"#,
    )
    .expect("root schema");
    DynamicMachine::builder(config)
        .action("spawn_worker", |_context, _event, enq| {
            enq.spawn("worker", ActorLogic::from_machine(worker()));
        })
        .action("stop_worker", |_context, _event, enq| {
            enq.stop("worker");
        })
        .build()
}

#[test]
fn stop_directive_removes_the_child() {
    let mut system = ActorSystem::new("root", ActorLogic::from_machine(stopping_root()));
    system.run().expect("system settles");
    assert!(system.exists(&id("worker")));

    system
        .send("root", DynamicEvent::new("STOP"))
        .expect("root receives STOP");
    system.run().expect("system settles");

    assert!(!system.exists(&id("worker")));
    assert!(system.exists(&id("root")));
}

#[test]
fn child_completion_notifies_the_parent() {
    let root = DynamicMachine::from_json_str(
        r#"{"initial":"Waiting","states":{
            "Waiting":{"on":{"xstate.done.actor.worker":"Finished"}},
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let child = DynamicMachine::from_json_str(
        r#"{"initial":"Go","states":{"Go":{"on":{"FINISH":"End"}},"End":{"type":"final"}}}"#,
    )
    .expect("worker schema");

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system
        .spawn(&id("root"), "worker", ActorLogic::from_machine(child))
        .expect("spawn worker");
    system
        .send("worker", DynamicEvent::new("FINISH"))
        .expect("send FINISH");
    system.run().expect("system settles");

    let leaves: Vec<_> = system
        .configuration(&id("root"))
        .expect("root runs")
        .leaves()
        .cloned()
        .collect();
    assert_eq!(leaves, ["Finished"]);
}

#[test]
fn stopping_a_parent_closes_child_scopes_last_in_first_out() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut system = ActorSystem::new("root", ActorLogic::from_machine(spawning_root()));
    system.run().expect("system settles");

    let worker = id("worker");
    for label in ["first", "second"] {
        let log = Arc::clone(&log);
        assert!(system.on_stop(&worker, move || {
            log.lock().expect("log lock").push(label.to_string());
        }));
    }

    assert!(system.stop(&id("root")));
    assert_eq!(*log.lock().expect("log lock"), ["second", "first"]);
    assert!(system.ids().next().is_none());
    assert!(system.defects().is_empty());
}

#[test]
fn transition_actor_folds_events_without_a_machine() {
    let logic = ActorLogic::from_transition(serde_json::json!(0), |value, _event| {
        serde_json::json!(value.as_u64().unwrap_or(0) + 1)
    });
    let mut system = ActorSystem::new("counter", logic);
    for _ in 0..3 {
        system
            .send("counter", DynamicEvent::new("INC"))
            .expect("send INC");
    }
    system.run().expect("system settles");

    assert_eq!(system.value(&id("counter")), Some(&serde_json::json!(3)));
}

#[test]
fn sending_to_an_unknown_actor_is_an_error() {
    let mut system = ActorSystem::new("root", ActorLogic::from_machine(worker()));
    let error = system
        .send("ghost", DynamicEvent::new("BOO"))
        .expect_err("unknown target");
    assert_eq!(error, ActorError::Unknown(id("ghost")));
}

#[test]
fn a_directive_cycle_trips_the_step_budget() {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"A","states":{"A":{"entry":["loop"],"on":{"TICK":"A"}}}}"#,
    )
    .expect("schema");
    let machine = DynamicMachine::builder(config)
        .action("loop", |_context, _event, enq| {
            enq.send_to("root", DynamicEvent::new("TICK"));
        })
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(machine)).with_max_steps(16);
    assert_eq!(system.run(), Err(ActorError::StepBudget(16)));
}
