use elm_effect::{Effect, Exit, Runtime};
use elm_effect_xstate::prelude::*;

fn id(name: &str) -> ActorId {
    ActorId::new(name)
}

fn worker() -> DynamicMachine {
    DynamicMachine::from_json_str(
        r#"{"initial":"Go","states":{
            "Go":{"on":{"FINISH":"Done"}},
            "Done":{"type":"final"}
        }}"#,
    )
    .expect("worker schema")
}

#[test]
fn invoke_starts_on_entry_and_stops_on_exit() {
    let root = DynamicMachine::builder(
        serde_json::from_str(
            r#"{"initial":"Running","states":{
                "Running":{
                    "invoke":{"id":"worker","src":"worker"},
                    "on":{"STOP":"Stopped"}
                },
                "Stopped":{}
            }}"#,
        )
        .expect("config"),
    )
    .actor("worker", ActorLogic::from_machine(worker()))
    .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    assert!(system.exists(&id("worker")));

    system
        .send("root", DynamicEvent::new("STOP"))
        .expect("send STOP");
    system.run().expect("system settles");
    assert!(!system.exists(&id("worker")));
}

#[test]
fn invoke_on_done_advances_the_owner() {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","states":{
            "Waiting":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "onDone":"Finished"
                }
            },
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let root = DynamicMachine::builder(config)
        .actor("worker", ActorLogic::from_machine(worker()))
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    system
        .send("worker", DynamicEvent::new("FINISH"))
        .expect("send FINISH");
    system.run().expect("system settles");

    assert_eq!(
        system
            .configuration(&id("root"))
            .expect("root configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Finished"]
    );
}

#[test]
fn invoke_on_error_advances_the_owner() {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","states":{
            "Waiting":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "onError":"Failed"
                }
            },
            "Failed":{}
        }}"#,
    )
    .expect("root schema");
    let root = DynamicMachine::builder(config)
        .actor(
            "worker",
            ActorLogic::from_effect(|| {
                Effect::<DynamicEvent, String>::sync(|| DynamicEvent::new("READY"))
            }),
        )
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    system.fail(&id("worker"), "boom").expect("fail worker");
    system.run().expect("system settles");

    assert_eq!(
        system
            .configuration(&id("root"))
            .expect("root configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Failed"]
    );
    assert!(!system.exists(&id("worker")));
}

#[test]
fn invoke_input_reaches_the_child_and_output_returns_in_on_done() {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","context":{"seen":null},"states":{
            "Waiting":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "input":{"value":7},
                    "onDone":{"target":"Finished","actions":["capture"]}
                }
            },
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let root = DynamicMachine::builder(config)
        .actor(
            "worker",
            ActorLogic::from_effect_with_input(|input| {
                let output = input.clone();
                Effect::<DynamicEvent, String>::sync(move || {
                    DynamicEvent::with_payload("READY", output.clone())
                })
            }),
        )
        .action("capture", |context, event, _enq| {
            context.data["seen"] = event.payload.clone();
        })
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    let task = system.take_tasks().into_iter().next().expect("worker task");
    let output = match Runtime::run(task.effect) {
        Exit::Success(event) => event.payload,
        Exit::Failure(cause) => panic!("effect failed: {cause:?}"),
    };
    assert_eq!(output, serde_json::json!({"value": 7}));
    system
        .complete(&task.actor, output)
        .expect("complete worker");
    system.run().expect("system settles");

    assert_eq!(
        system.context(&id("root")).expect("root context").data["seen"],
        serde_json::json!({"value": 7})
    );
}

#[test]
fn invoked_machine_uses_input_as_null_context_and_emits_final_output() {
    let root_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","context":{"seen":null},"states":{
            "Waiting":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "onDone":{"target":"Finished","actions":["capture"]}
                }
            },
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let worker = DynamicMachine::from_json_str(
        r#"{"initial":"Go","states":{
            "Go":{"on":{"FINISH":"Done"}},
            "Done":{"type":"final","output":{"result":42}}
        }}"#,
    )
    .expect("worker schema");
    let root = DynamicMachine::builder(root_config)
        .actor("worker", ActorLogic::from_machine(worker))
        .invoke_input("worker", |_context, _event| serde_json::json!({ "n": 3 }))
        .action("capture", |context, event, _enq| {
            context.data["seen"] = event.payload.clone();
        })
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    assert_eq!(
        system.context(&id("worker")).expect("worker context").data,
        serde_json::json!({"n": 3})
    );
    system
        .send("worker", DynamicEvent::new("FINISH"))
        .expect("finish worker");
    system.run().expect("system settles");

    assert_eq!(
        system
            .configuration(&id("root"))
            .expect("root configuration")
            .leaves()
            .cloned()
            .collect::<Vec<_>>(),
        ["Finished"]
    );
    assert_eq!(
        system.context(&id("root")).expect("root context").data["seen"],
        serde_json::json!({"result": 42})
    );
}

#[test]
fn a_reentering_transition_restarts_the_invoke() {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Running","states":{
            "Running":{
                "invoke":{"id":"worker","src":"worker"},
                "on":{"RESTART":{"target":"Running","reenter":true}}
            }
        }}"#,
    )
    .expect("root schema");
    let root = DynamicMachine::builder(config)
        .actor("worker", ActorLogic::from_machine(worker()))
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stopped_by_finalizer = std::sync::Arc::clone(&stopped);
    assert!(system.on_stop(&id("worker"), move || {
        stopped_by_finalizer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));

    system
        .send("root", DynamicEvent::new("RESTART"))
        .expect("send RESTART");
    system.run().expect("system settles");

    assert_eq!(stopped.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(system.exists(&id("worker")));
}

#[test]
fn invoked_machine_resolves_final_output_from_context() {
    let root_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","context":{"seen":null},"states":{
            "Waiting":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "input":{"n":7},
                    "onDone":{"target":"Finished","actions":["capture"]}
                }
            },
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let worker_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Go","states":{
            "Go":{"on":{"FINISH":"Done"}},
            "Done":{"type":"final"}
        }}"#,
    )
    .expect("worker schema");
    let worker = DynamicMachine::builder(worker_config)
        .output(
            "Done",
            |context, _event| serde_json::json!({ "result": context.data["n"] }),
        )
        .build();
    let root = DynamicMachine::builder(root_config)
        .actor("worker", ActorLogic::from_machine(worker))
        .action("capture", |context, event, _enq| {
            context.data["seen"] = event.payload.clone();
        })
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    system
        .send("worker", DynamicEvent::new("FINISH"))
        .expect("finish worker");
    system.run().expect("system settles");

    assert_eq!(
        system.context(&id("root")).expect("root context").data["seen"],
        serde_json::json!({ "result": 7 })
    );
    assert!(!system.exists(&id("worker")));
}

#[test]
fn root_output_resolver_wins_over_final_state_output() {
    let root_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","context":{"seen":null},"states":{
            "Waiting":{
                "invoke":{
                    "id":"worker",
                    "src":"worker",
                    "input":{"n":7},
                    "onDone":{"target":"Finished","actions":["capture"]}
                }
            },
            "Finished":{}
        }}"#,
    )
    .expect("root schema");
    let worker_config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Go","states":{
            "Go":{"on":{"FINISH":"Done"}},
            "Done":{"type":"final","output":{"source":"final"}}
        }}"#,
    )
    .expect("worker schema");
    let worker = DynamicMachine::builder(worker_config)
        .root_output(
            |context, _event| serde_json::json!({ "source": "root", "n": context.data["n"] }),
        )
        .build();
    let root = DynamicMachine::builder(root_config)
        .actor("worker", ActorLogic::from_machine(worker))
        .action("capture", |context, event, _enq| {
            context.data["seen"] = event.payload.clone();
        })
        .build();

    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system.run().expect("system settles");
    system
        .send("worker", DynamicEvent::new("FINISH"))
        .expect("finish worker");
    system.run().expect("system settles");

    assert_eq!(
        system.context(&id("root")).expect("root context").data["seen"],
        serde_json::json!({ "source": "root", "n": 7 })
    );
    assert!(!system.exists(&id("worker")));
}
