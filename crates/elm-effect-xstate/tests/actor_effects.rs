use elm_effect::{Clock, ClockImpl, Duration, Effect, Exit, Runtime, TestClock};
use elm_effect_xstate::prelude::*;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

fn id(name: &str) -> ActorId {
    ActorId::new(name)
}

fn waiting_parent(done_event: &str) -> DynamicMachine {
    DynamicMachine::from_json_str(&format!(
        r#"{{"initial":"Waiting","states":{{
            "Waiting":{{"on":{{"{done_event}":"Finished"}}}},
            "Finished":{{}}
        }}}}"#
    ))
    .expect("parent schema")
}

fn leaves(system: &ActorSystem, actor: &ActorId) -> Vec<String> {
    system
        .configuration(actor)
        .expect("machine actor")
        .leaves()
        .cloned()
        .collect()
}

#[test]
fn an_effect_actor_reports_done_to_its_parent() {
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(waiting_parent("xstate.done.actor.worker")),
    );
    system
        .spawn(
            &id("root"),
            "worker",
            ActorLogic::from_effect(|| {
                Effect::<DynamicEvent, String>::sync(|| DynamicEvent::new("READY"))
            }),
        )
        .expect("spawn worker");
    system.run().expect("system settles");

    let tasks = system.take_tasks();
    assert_eq!(tasks.len(), 1);
    let task = tasks.into_iter().next().expect("one task");
    let event = match Runtime::run(task.effect) {
        Exit::Success(event) => event,
        Exit::Failure(cause) => panic!("effect failed: {cause:?}"),
    };
    system
        .complete(&task.actor, event.payload)
        .expect("complete worker");
    system.run().expect("system settles");

    assert_eq!(leaves(&system, &id("root")), ["Finished"]);
    assert!(!system.exists(&id("worker")));
}

#[test]
fn an_effect_failure_carries_its_payload_to_the_parent_and_stops_the_actor() {
    let config: MachineConfig = serde_json::from_str(
        r#"{"initial":"Waiting","context":{"failure":null},"states":{
            "Waiting":{"on":{"xstate.error.actor.worker":{"target":"Finished","actions":["capture"]}}},
            "Finished":{}
        }}"#,
    )
    .expect("parent schema");
    let root = DynamicMachine::builder(config)
        .action("capture", |context, event, _enq| {
            context.data["failure"] = event.payload.clone();
        })
        .build();
    let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
    system
        .spawn(
            &id("root"),
            "worker",
            ActorLogic::from_effect(|| {
                Effect::<DynamicEvent, String>::sync(|| DynamicEvent::new("UNUSED"))
            }),
        )
        .expect("spawn worker");
    system.run().expect("system settles");
    let task = system.take_tasks().into_iter().next().expect("one task");

    system.fail(&task.actor, "boom").expect("fail worker");
    system.run().expect("system settles");

    assert_eq!(leaves(&system, &id("root")), ["Finished"]);
    assert_eq!(
        system.context(&id("root")).expect("root context").data["failure"],
        serde_json::json!({"error": "boom"})
    );
    assert!(!system.exists(&id("worker")));
}

#[test]
fn a_stream_actor_forwards_values_to_its_parent() {
    let (tx, rx) = mpsc::channel();
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(
            DynamicMachine::from_json_str(
                r#"{"initial":"Idle","states":{
                    "Idle":{"on":{"PING":"Ponged"}},
                    "Ponged":{}
                }}"#,
            )
            .expect("parent schema"),
        ),
    );
    system
        .spawn(&id("root"), "feed", ActorLogic::from_stream(rx))
        .expect("spawn stream");
    system.run().expect("system settles");

    tx.send(DynamicEvent::new("PING")).expect("send value");
    system.run().expect("system settles");

    assert_eq!(leaves(&system, &id("root")), ["Ponged"]);
}

#[test]
fn a_stream_disconnect_stops_the_stream_actor() {
    let (tx, rx) = mpsc::channel();
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(
            DynamicMachine::from_json_str(r#"{"initial":"Idle","states":{"Idle":{}}}"#)
                .expect("parent schema"),
        ),
    );
    system
        .spawn(&id("root"), "feed", ActorLogic::from_stream(rx))
        .expect("spawn stream");
    system.run().expect("system settles");
    assert!(system.exists(&id("feed")));

    drop(tx);
    system.run().expect("system settles");

    assert!(!system.exists(&id("feed")));
    assert!(system.exists(&id("root")));
}

#[test]
fn a_callback_actor_forwards_values_to_its_parent() {
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(
            DynamicMachine::from_json_str(
                r#"{"initial":"Idle","states":{
                    "Idle":{"on":{"PING":"Ponged"}},
                    "Ponged":{}
                }}"#,
            )
            .expect("parent schema"),
        ),
    );
    system
        .spawn(
            &id("root"),
            "callback",
            ActorLogic::from_callback(|sender| {
                assert!(sender.send(DynamicEvent::new("PING")));
            }),
        )
        .expect("spawn callback");
    system.run().expect("system settles");

    assert_eq!(leaves(&system, &id("root")), ["Ponged"]);
}

#[test]
fn a_callback_sender_returns_false_after_the_system_is_dropped() {
    let captured = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&captured);
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(
            DynamicMachine::from_json_str(r#"{"initial":"Idle","states":{"Idle":{}}}"#)
                .expect("parent schema"),
        ),
    );
    system
        .spawn(
            &id("root"),
            "callback",
            ActorLogic::from_callback(move |sender| {
                *slot.lock().expect("callback slot") = Some(sender);
            }),
        )
        .expect("spawn callback");
    system.run().expect("system settles");
    let sender = captured
        .lock()
        .expect("callback slot")
        .take()
        .expect("callback sender");

    drop(system);

    assert!(!sender.send(DynamicEvent::new("PING")));
}

#[test]
fn a_callback_event_after_the_parent_stops_is_discarded() {
    let captured = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&captured);
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(
            DynamicMachine::from_json_str(r#"{"initial":"Idle","states":{"Idle":{}}}"#)
                .expect("parent schema"),
        ),
    );
    system
        .spawn(
            &id("root"),
            "callback",
            ActorLogic::from_callback(move |sender| {
                *slot.lock().expect("callback slot") = Some(sender);
            }),
        )
        .expect("spawn callback");
    system.run().expect("system settles");
    let sender = captured
        .lock()
        .expect("callback slot")
        .take()
        .expect("callback sender");

    system.stop(&id("root"));
    assert!(sender.send(DynamicEvent::new("PING")));
    system.run().expect("system settles");

    assert!(system.ids().next().is_none());
}

#[test]
fn stopping_an_actor_interrupts_its_scope_fiber() {
    let mut system = ActorSystem::new(
        "root",
        ActorLogic::from_machine(
            DynamicMachine::from_json_str(r#"{"initial":"Idle","states":{"Idle":{}}}"#)
                .expect("root schema"),
        ),
    );
    system
        .spawn(
            &id("root"),
            "worker",
            ActorLogic::from_machine(
                DynamicMachine::from_json_str(r#"{"initial":"Idle","states":{"Idle":{}}}"#)
                    .expect("worker schema"),
            ),
        )
        .expect("spawn worker");
    let scope = system.scope(&id("worker")).expect("worker scope");

    let runtime = Runtime::new();
    let sleeper: Effect<(), String> = Effect::sleep(Duration::from_secs(3_600))
        .provide::<Clock, ()>(Arc::new(TestClock::new()) as Arc<dyn ClockImpl>);
    let forked = runtime.spawn_effect(sleeper.fork_in(scope)).join();
    let fiber = match forked {
        Exit::Success(fiber) => fiber,
        Exit::Failure(cause) => panic!("fork failed: {cause:?}"),
    };

    assert!(system.stop(&id("worker")));
    assert!(matches!(fiber.join(), Exit::Failure(_)));
}
