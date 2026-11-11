use elm_effect_xstate::prelude::*;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct ActorCase {
    name: String,
    #[allow(dead_code)]
    description: Option<String>,
    root: MachineConfig,
    #[serde(default)]
    children: BTreeMap<String, MachineConfig>,
    #[serde(default)]
    invoke_inputs: BTreeMap<String, InvokeInput>,
    #[serde(default)]
    steps: Vec<Step>,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Step {
    Send {
        send: String,
        event: DynamicEvent,
        #[serde(default)]
        expect: BTreeMap<String, Vec<String>>,
        #[serde(default)]
        context: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    },
    Stop {
        stop: String,
    },
    Fail {
        fail: String,
        error: String,
        #[serde(default)]
        expect: BTreeMap<String, Vec<String>>,
        #[serde(default)]
        context: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct InvokeInput {
    #[serde(default)]
    from_context: Option<String>,
    #[serde(default)]
    from_event: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Expected {
    #[serde(default)]
    final_states: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    absent: Vec<String>,
    #[serde(default)]
    context: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("conformance")
        .join("fixtures")
        .join("actors.json")
}

fn actions(list: &ActionList) -> Vec<&String> {
    match list {
        ActionList::Single(action) => vec![action],
        ActionList::Multiple(list) => list.iter().collect(),
    }
}

fn collect_rule(rule: &TransitionRule, out: &mut BTreeSet<String>) {
    match rule {
        TransitionRule::Shorthand(_) => {}
        TransitionRule::Single(config) => {
            out.extend(actions(&config.actions).into_iter().cloned());
        }
        TransitionRule::Multiple(configs) => {
            for config in configs {
                out.extend(actions(&config.actions).into_iter().cloned());
            }
        }
    }
}

fn collect_actions(config: &MachineConfig, out: &mut BTreeSet<String>) {
    for action in actions(&config.entry) {
        out.insert(action.clone());
    }
    for action in actions(&config.exit) {
        out.insert(action.clone());
    }
    for node in config.states.values() {
        for action in actions(&node.entry) {
            out.insert(action.clone());
        }
        for action in actions(&node.exit) {
            out.insert(action.clone());
        }
        for rule in node.on.values() {
            collect_rule(rule, out);
        }
        for rule in node.after.values() {
            collect_rule(rule, out);
        }
        for transition in &node.always {
            out.extend(actions(&transition.actions).into_iter().cloned());
        }
        if let Some(rule) = &node.on_done {
            collect_rule(rule, out);
        }
        for invoke in node.invoke.clone().into_vec() {
            if let Some(rule) = &invoke.on_done {
                collect_rule(rule, out);
            }
            if let Some(rule) = &invoke.on_error {
                collect_rule(rule, out);
            }
        }
    }
}

fn build(
    config: MachineConfig,
    configs: &BTreeMap<String, MachineConfig>,
    invoke_inputs: &BTreeMap<String, InvokeInput>,
) -> DynamicMachine {
    build_with_invokes(config, configs, invoke_inputs, true)
}

fn build_with_invokes(
    config: MachineConfig,
    configs: &BTreeMap<String, MachineConfig>,
    invoke_inputs: &BTreeMap<String, InvokeInput>,
    register_invokes: bool,
) -> DynamicMachine {
    let mut names = BTreeSet::new();
    collect_actions(&config, &mut names);
    let mut builder = DynamicMachine::builder(config);
    if register_invokes {
        for (id, input) in invoke_inputs {
            let input = input.clone();
            builder = builder.invoke_input(id.clone(), move |context, event| {
                let mut data = serde_json::Map::new();
                if let Some(key) = &input.from_context {
                    data.insert(
                        key.clone(),
                        context
                            .data
                            .get(key)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    );
                }
                if let Some(key) = &input.from_event {
                    data.insert(
                        key.clone(),
                        event
                            .payload
                            .get(key)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    );
                }
                serde_json::Value::Object(data)
            });
        }
        for (child, child_config) in configs {
            builder = builder.actor(
                child.clone(),
                ActorLogic::from_machine(build_with_invokes(
                    child_config.clone(),
                    configs,
                    invoke_inputs,
                    false,
                )),
            );
        }
    }
    for name in names {
        if let Some(child) = name.strip_prefix("spawn:") {
            let child = child.to_string();
            let configs = configs.clone();
            let invoke_inputs = invoke_inputs.clone();
            builder = builder.action(name, move |_context, _event, enq| {
                let config = configs
                    .get(&child)
                    .unwrap_or_else(|| panic!("spawn action names unknown child '{child}'"))
                    .clone();
                enq.spawn(
                    child.clone(),
                    ActorLogic::from_machine(build_with_invokes(
                        config,
                        &configs,
                        &invoke_inputs,
                        false,
                    )),
                );
            });
        } else if let Some(rest) = name.strip_prefix("send:") {
            let (target, event) = rest
                .split_once(':')
                .unwrap_or_else(|| panic!("send action '{name}' needs send:<target>:<event>"));
            let target = target.to_string();
            let event = event.to_string();
            builder = builder.action(name, move |_context, _event, enq| {
                enq.send_to(target.clone(), DynamicEvent::new(event.clone()));
            });
        } else if let Some(target) = name.strip_prefix("stop:") {
            let target = target.to_string();
            builder = builder.action(name, move |_context, _event, enq| {
                enq.stop(target.clone());
            });
        } else if let Some(key) = name.strip_prefix("assign_payload:") {
            let key = key.to_string();
            builder = builder.action(name, move |context, event, _enq| {
                if let Some(data) = context.data.as_object_mut() {
                    data.insert(key.clone(), event.payload.clone());
                }
            });
        }
    }
    builder.build()
}

fn assert_states(system: &ActorSystem, expected: &BTreeMap<String, Vec<String>>, case: &str) {
    for (actor, leaves) in expected {
        let id = ActorId::new(actor);
        let actual: Vec<String> = system
            .configuration(&id)
            .unwrap_or_else(|| panic!("case '{case}': '{actor}' has no machine state"))
            .leaves()
            .cloned()
            .collect();
        assert_eq!(&actual, leaves, "case '{case}': actor '{actor}'");
    }
}

fn assert_context(
    system: &ActorSystem,
    expected: &BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    case: &str,
) {
    for (actor, expected) in expected {
        let data = &system
            .context(&ActorId::new(actor))
            .unwrap_or_else(|| panic!("case '{case}': '{actor}' has no context"))
            .data;
        for (key, value) in expected {
            assert_eq!(
                data.get(key),
                Some(value),
                "case '{case}': context '{actor}.{key}'"
            );
        }
    }
}

#[test]
fn conformance_actors() {
    let content = fs::read_to_string(fixture_path()).expect("read actors fixture");
    let cases: Vec<ActorCase> = serde_json::from_str(&content).expect("parse actors fixture");

    for case in cases {
        let root = build(case.root, &case.children, &case.invoke_inputs);
        let mut system = ActorSystem::new("root", ActorLogic::from_machine(root));
        system
            .run()
            .unwrap_or_else(|error| panic!("case '{}': initial run failed: {error}", case.name));

        for step in case.steps {
            match step {
                Step::Send {
                    send,
                    event,
                    expect,
                    context,
                } => {
                    system.send(send.as_str(), event).unwrap_or_else(|error| {
                        panic!("case '{}': send to '{send}' failed: {error}", case.name)
                    });
                    system.run().unwrap_or_else(|error| {
                        panic!("case '{}': run after send failed: {error}", case.name)
                    });
                    assert_states(&system, &expect, &case.name);
                    assert_context(&system, &context, &case.name);
                }
                Step::Stop { stop } => {
                    assert!(
                        system.stop(&ActorId::new(&stop)),
                        "case '{}': stop '{stop}' found no actor",
                        case.name
                    );
                }
                Step::Fail {
                    fail,
                    error,
                    expect,
                    context,
                } => {
                    system
                        .fail(&ActorId::new(&fail), error)
                        .unwrap_or_else(|error| {
                            panic!("case '{}': fail '{fail}' failed: {error}", case.name)
                        });
                    system.run().unwrap_or_else(|error| {
                        panic!("case '{}': run after fail failed: {error}", case.name)
                    });
                    assert_states(&system, &expect, &case.name);
                    assert_context(&system, &context, &case.name);
                }
            }
        }

        assert_states(&system, &case.expected.final_states, &case.name);
        for actor in &case.expected.absent {
            assert!(
                !system.exists(&ActorId::new(actor)),
                "case '{}': actor '{actor}' should be absent",
                case.name
            );
        }
        assert_context(&system, &case.expected.context, &case.name);
    }
}
