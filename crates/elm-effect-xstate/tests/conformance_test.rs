use elm_effect_xstate::prelude::*;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct TestCase {
    name: String,
    #[allow(dead_code)]
    description: Option<String>,
    machine: MachineConfig,
    events: Vec<DynamicEvent>,
    expected: ExpectedOutcome,
}

#[derive(Debug, Deserialize)]
struct ExpectedOutcome {
    final_state: String,
    #[serde(default)]
    final_data: Option<serde_json::Value>,
    #[serde(default)]
    trace: Vec<String>,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("conformance")
        .join("fixtures")
}

fn execute_suite(fixture_name: &str) {
    let path = fixture_dir().join(fixture_name);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading fixture {}: {e}", path.display()));
    let cases: Vec<TestCase> = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("failed parsing fixture {}: {e}", path.display()));

    for case in cases {
        let machine = DynamicMachine::builder(case.machine).build();
        let mut curr_state = machine.resolve_leaf(machine.initial_state());
        let mut curr_ctx = machine.initial_context().clone();

        for event in &case.events {
            let outcome = machine.step(&curr_state, &curr_ctx, event);
            curr_state = outcome.current_state;
            curr_ctx = outcome.context;
        }

        assert_eq!(
            curr_state, case.expected.final_state,
            "final state mismatch in case '{}'",
            case.name
        );
        assert_eq!(
            curr_ctx.log, case.expected.trace,
            "action trace mismatch in case '{}'",
            case.name
        );

        if let Some(expected_data) = case.expected.final_data
            && let Some(expected_map) = expected_data.as_object()
        {
            for (key, val) in expected_map {
                assert_eq!(
                    curr_ctx.data.get(key),
                    Some(val),
                    "context key '{}' mismatch in case '{}'",
                    key,
                    case.name
                );
            }
        }
    }
}

#[test]
fn conformance_actions() {
    execute_suite("actions.json");
}

#[test]
fn conformance_always() {
    execute_suite("always.json");
}

#[test]
fn conformance_hierarchy() {
    execute_suite("hierarchy.json");
}

#[test]
fn conformance_guards() {
    execute_suite("guards.json");
}

#[test]
fn conformance_final_states() {
    execute_suite("final_states.json");
}
