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
    },
    Stop {
        stop: String,
    },
}

#[derive(Debug, Deserialize)]
struct Expected {
    #[serde(default)]
    final_states: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    absent: Vec<String>,
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
    }
}

fn build(config: MachineConfig, configs: &BTreeMap<String, MachineConfig>) -> DynamicMachine {
    let mut names = BTreeSet::new();
    collect_actions(&config, &mut names);
    let mut builder = DynamicMachine::builder(config);
    for name in names {
        if let Some(child) = name.strip_prefix("spawn:") {
            let child = child.to_string();
            let configs = configs.clone();
            builder = builder.action(name, move |_context, _event, enq| {
                let config = configs
                    .get(&child)
                    .unwrap_or_else(|| panic!("spawn action names unknown child '{child}'"))
                    .clone();
                enq.spawn(
                    child.clone(),
                    ActorLogic::from_machine(build(config, &configs)),
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

#[test]
fn conformance_actors() {
    let content = fs::read_to_string(fixture_path()).expect("read actors fixture");
    let cases: Vec<ActorCase> = serde_json::from_str(&content).expect("parse actors fixture");

    for case in cases {
        let root = build(case.root, &case.children);
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
                } => {
                    system.send(send.as_str(), event).unwrap_or_else(|error| {
                        panic!("case '{}': send to '{send}' failed: {error}", case.name)
                    });
                    system.run().unwrap_or_else(|error| {
                        panic!("case '{}': run after send failed: {error}", case.name)
                    });
                    assert_states(&system, &expect, &case.name);
                }
                Step::Stop { stop } => {
                    assert!(
                        system.stop(&ActorId::new(&stop)),
                        "case '{}': stop '{stop}' found no actor",
                        case.name
                    );
                }
            }
        }

        assert_states(&system, &case.expected.final_states, &case.name);
        for actor in case.expected.absent {
            assert!(
                !system.exists(&ActorId::new(&actor)),
                "case '{}': actor '{actor}' should be absent",
                case.name
            );
        }
    }
}
