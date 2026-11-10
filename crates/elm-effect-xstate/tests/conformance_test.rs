use elm_effect_xstate::prelude::*;
use serde::Deserialize;
use std::collections::BTreeSet;
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
    final_states: Option<Vec<String>>,
    #[serde(default)]
    final_data: Option<serde_json::Value>,
    #[serde(default)]
    done: Option<bool>,
    #[serde(default)]
    emitted: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    trace: Vec<String>,
}

fn action_names(list: &ActionList) -> Vec<&String> {
    match list {
        ActionList::Single(action) => vec![action],
        ActionList::Multiple(list) => list.iter().collect(),
    }
}

fn collect_rule_actions(rule: &TransitionRule, out: &mut BTreeSet<String>) {
    match rule {
        TransitionRule::Shorthand(_) => {}
        TransitionRule::Single(config) => {
            out.extend(action_names(&config.actions).into_iter().cloned());
        }
        TransitionRule::Multiple(configs) => {
            for config in configs {
                out.extend(action_names(&config.actions).into_iter().cloned());
            }
        }
    }
}

fn collect_actions(config: &MachineConfig, out: &mut BTreeSet<String>) {
    for node in config.states.values() {
        out.extend(action_names(&node.entry).into_iter().cloned());
        out.extend(action_names(&node.exit).into_iter().cloned());
        for rule in node.on.values().chain(node.after.values()) {
            collect_rule_actions(rule, out);
        }
        for transition in &node.always {
            out.extend(action_names(&transition.actions).into_iter().cloned());
        }
        if let Some(rule) = &node.on_done {
            collect_rule_actions(rule, out);
        }
    }
}

fn build_machine(config: MachineConfig) -> DynamicMachine {
    let mut names = BTreeSet::new();
    collect_actions(&config, &mut names);
    let mut builder = DynamicMachine::builder(config);
    for name in names {
        if let Some(event) = name.strip_prefix("raise:") {
            let event = event.to_string();
            builder = builder.action(name, move |_context, _event, enq| {
                enq.raise(DynamicEvent::new(event.clone()));
            });
        } else if let Some(payload) = name.strip_prefix("emit:") {
            let payload = payload.to_string();
            builder = builder.action(name, move |_context, _event, enq| {
                enq.emit(payload.clone());
            });
        }
    }
    builder.build()
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
        let machine = build_machine(case.machine);
        let entered = machine.enter(machine.initial_context());
        let mut done = entered.done;
        let mut emitted = Vec::new();
        let mut state = MachineState {
            configuration: entered.configuration,
            history: entered.history,
            context: entered.context,
        };

        for event in &case.events {
            let outcome = machine.step_state(&state, event);
            done = outcome.done;
            emitted.extend(outcome.emitted);
            state = MachineState {
                configuration: outcome.configuration,
                history: outcome.history,
                context: outcome.context,
            };
        }

        assert_eq!(
            state.configuration.primary(),
            Some(&case.expected.final_state),
            "final state mismatch in case '{}'",
            case.name
        );
        if let Some(expected_leaves) = &case.expected.final_states {
            assert_eq!(
                state.configuration.leaves().cloned().collect::<Vec<_>>(),
                *expected_leaves,
                "active leaves mismatch in case '{}'",
                case.name
            );
        }
        assert_eq!(
            state.context.log, case.expected.trace,
            "action trace mismatch in case '{}'",
            case.name
        );

        if let Some(expected_done) = case.expected.done {
            assert_eq!(done, expected_done, "done mismatch in case '{}'", case.name);
        }
        if let Some(expected_emitted) = &case.expected.emitted {
            assert_eq!(
                &emitted, expected_emitted,
                "emitted mismatch in case '{}'",
                case.name
            );
        }

        if let Some(expected_data) = case.expected.final_data
            && let Some(expected_map) = expected_data.as_object()
        {
            for (key, val) in expected_map {
                assert_eq!(
                    state.context.data.get(key),
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

#[test]
fn conformance_parallel() {
    execute_suite("parallel.json");
}

#[test]
fn conformance_history() {
    execute_suite("history.json");
}

#[test]
fn conformance_after() {
    execute_suite("after.json");
}

const FIXTURES: &[&str] = &[
    "actions.json",
    "actors.json",
    "after.json",
    "always.json",
    "final_states.json",
    "guards.json",
    "hierarchy.json",
    "history.json",
    "parallel.json",
];

struct Feature {
    name: &'static str,
    fixture: &'static str,
    cases: &'static [&'static str],
}

const FEATURES: &[Feature] = &[
    Feature {
        name: "action_l1_order",
        fixture: "actions.json",
        cases: &[
            "strict_l1_execution_order",
            "multiple_entry_and_exit_actions_declaration_order",
            "nested_exit_and_entry_are_topological",
        ],
    },
    Feature {
        name: "self_transition_reentry",
        fixture: "actions.json",
        cases: &["self_transition_exits_and_reenters"],
    },
    Feature {
        name: "internal_target",
        fixture: "actions.json",
        cases: &["internal_target_keeps_source_active"],
    },
    Feature {
        name: "raised_event",
        fixture: "actions.json",
        cases: &["a_raised_event_is_consumed_in_the_same_macrostep"],
    },
    Feature {
        name: "emitted_payload",
        fixture: "actions.json",
        cases: &["emitted_payloads_reach_inspection"],
    },
    Feature {
        name: "raised_event_order",
        fixture: "actions.json",
        cases: &["parallel_raised_events_execute_in_order"],
    },
    Feature {
        name: "actor_spawn",
        fixture: "actors.json",
        cases: &["entry_action_spawns_a_child"],
    },
    Feature {
        name: "actor_send",
        fixture: "actors.json",
        cases: &["send_to_reaches_a_child"],
    },
    Feature {
        name: "actor_stop",
        fixture: "actors.json",
        cases: &["stop_directive_removes_the_child"],
    },
    Feature {
        name: "actor_done_event",
        fixture: "actors.json",
        cases: &["child_completion_notifies_the_parent"],
    },
    Feature {
        name: "actor_peer_send",
        fixture: "actors.json",
        cases: &["peer_actors_exchange_events"],
    },
    Feature {
        name: "actor_unhandled_done",
        fixture: "actors.json",
        cases: &["unhandled_completion_is_ignored"],
    },
    Feature {
        name: "actor_stop_subtree",
        fixture: "actors.json",
        cases: &["stopping_a_child_removes_its_descendants"],
    },
    Feature {
        name: "after_timer_fire",
        fixture: "after.json",
        cases: &["delayed_transition_fires_on_its_timer_event"],
    },
    Feature {
        name: "after_timer_cancel",
        fixture: "after.json",
        cases: &["delayed_transition_ignored_after_the_state_is_left"],
    },
    Feature {
        name: "after_stale_timer",
        fixture: "after.json",
        cases: &["a_stale_timer_from_an_exited_state_is_ignored"],
    },
    Feature {
        name: "after_zero_delay",
        fixture: "after.json",
        cases: &["a_zero_delay_fires_on_its_timer_event"],
    },
    Feature {
        name: "always_cascade",
        fixture: "always.json",
        cases: &["cascading_microsteps_to_quiescence"],
    },
    Feature {
        name: "always_guarded",
        fixture: "always.json",
        cases: &["cascading_microsteps_to_quiescence"],
    },
    Feature {
        name: "always_on_entry",
        fixture: "always.json",
        cases: &["immediate_transient_transition_on_entry"],
    },
    Feature {
        name: "always_guard_fallback",
        fixture: "always.json",
        cases: &["a_false_guard_falls_through_to_the_next_always"],
    },
    Feature {
        name: "always_action_order",
        fixture: "always.json",
        cases: &["eventless_transition_actions_run_after_exit_actions"],
    },
    Feature {
        name: "always_region_scoped",
        fixture: "always.json",
        cases: &["eventless_transition_stays_inside_its_parallel_region"],
    },
    Feature {
        name: "final_terminal",
        fixture: "final_states.json",
        cases: &["final_state_is_terminal"],
    },
    Feature {
        name: "final_on_done",
        fixture: "final_states.json",
        cases: &["compound_final_takes_on_done"],
    },
    Feature {
        name: "final_ignores_events",
        fixture: "final_states.json",
        cases: &["final_state_ignores_later_events"],
    },
    Feature {
        name: "guard_first_match",
        fixture: "guards.json",
        cases: &["guarded_transition_matching_branch"],
    },
    Feature {
        name: "guard_fallback",
        fixture: "guards.json",
        cases: &["guarded_transition_fallback"],
    },
    Feature {
        name: "guard_priority",
        fixture: "guards.json",
        cases: &["the_first_true_guard_wins"],
    },
    Feature {
        name: "guard_self_transition",
        fixture: "guards.json",
        cases: &["guarded_self_transition_reenters"],
    },
    Feature {
        name: "hierarchy_initial",
        fixture: "hierarchy.json",
        cases: &["initial_compound_state_resolution"],
    },
    Feature {
        name: "hierarchy_same_parent",
        fixture: "hierarchy.json",
        cases: &["transition_within_same_compound_preserves_parent"],
    },
    Feature {
        name: "hierarchy_bubbling",
        fixture: "hierarchy.json",
        cases: &[
            "event_bubbling_to_compound_parent",
            "three_level_event_bubbling",
        ],
    },
    Feature {
        name: "hierarchy_lca",
        fixture: "hierarchy.json",
        cases: &["sibling_compound_transition_stays_under_the_lca"],
    },
    Feature {
        name: "hierarchy_parent_reentry",
        fixture: "hierarchy.json",
        cases: &["parent_self_transition_reenters_its_child"],
    },
    Feature {
        name: "history_shallow",
        fixture: "history.json",
        cases: &["shallow_history_restores_the_child_then_its_initial"],
    },
    Feature {
        name: "history_deep",
        fixture: "history.json",
        cases: &[
            "deep_history_restores_the_exact_leaf",
            "deep_history_remembers_the_latest_visit",
        ],
    },
    Feature {
        name: "history_default",
        fixture: "history.json",
        cases: &["history_without_a_record_uses_the_fallback"],
    },
    Feature {
        name: "history_transient_update",
        fixture: "history.json",
        cases: &["transient_reentry_updates_the_history_record"],
    },
    Feature {
        name: "history_parent_entry",
        fixture: "history.json",
        cases: &["history_default_enters_the_parent_and_its_fallback"],
    },
    Feature {
        name: "parallel_region_entry",
        fixture: "parallel.json",
        cases: &["parallel_regions_enter_and_broadcast"],
    },
    Feature {
        name: "parallel_broadcast",
        fixture: "parallel.json",
        cases: &["parallel_regions_enter_and_broadcast"],
    },
    Feature {
        name: "parallel_region_isolation",
        fixture: "parallel.json",
        cases: &[
            "parallel_region_without_a_handler_keeps_its_leaf",
            "transition_targeting_one_region_keeps_the_sibling",
        ],
    },
    Feature {
        name: "parallel_target_region",
        fixture: "parallel.json",
        cases: &["transition_targeting_one_region_keeps_the_sibling"],
    },
    Feature {
        name: "parallel_cross_region",
        fixture: "parallel.json",
        cases: &["cross_region_transition_keeps_the_source_region"],
    },
    Feature {
        name: "parallel_all_regions_done",
        fixture: "parallel.json",
        cases: &[
            "parallel_node_completes_when_all_regions_are_final",
            "parallel_completes_only_after_the_last_region",
        ],
    },
    Feature {
        name: "parallel_partial_done",
        fixture: "parallel.json",
        cases: &["a_finished_region_does_not_complete_the_parallel_early"],
    },
    Feature {
        name: "parallel_nested_flat",
        fixture: "parallel.json",
        cases: &["nested_flat_parallel_expands_every_region"],
    },
    Feature {
        name: "parallel_final_region_inert",
        fixture: "parallel.json",
        cases: &["final_region_under_parallel_ignores_events"],
    },
];

fn fixture_values(name: &str) -> Vec<serde_json::Value> {
    let path = fixture_dir().join(name);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed reading {}: {error}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed parsing {}: {error}", path.display()))
}

fn fixture_case_names(name: &str) -> Vec<String> {
    fixture_values(name)
        .iter()
        .map(|case| {
            case["name"]
                .as_str()
                .unwrap_or_else(|| panic!("case in {name} has no string name"))
                .to_string()
        })
        .collect()
}

fn fixture_sources(name: &str) -> Vec<(String, Option<String>)> {
    fixture_values(name)
        .iter()
        .map(|case| {
            let case_name = case["name"]
                .as_str()
                .unwrap_or_else(|| panic!("case in {name} has no string name"))
                .to_string();
            let source = case["source"].as_str().map(str::to_string);
            (case_name, source)
        })
        .collect()
}

#[test]
fn conformance_feature_matrix() {
    let mut claimed = BTreeSet::new();
    for feature in FEATURES {
        let cases = fixture_case_names(feature.fixture);
        assert!(
            !feature.cases.is_empty(),
            "feature '{}' has no proving cases",
            feature.name
        );
        for case in feature.cases {
            assert!(
                cases.iter().any(|candidate| candidate == case),
                "feature '{}' cites missing case '{case}' in {}",
                feature.name,
                feature.fixture
            );
            claimed.insert((feature.fixture.to_string(), (*case).to_string()));
        }
    }

    let mut total = 0;
    for fixture in FIXTURES {
        for case in fixture_case_names(fixture) {
            total += 1;
            assert!(
                claimed.contains(&((*fixture).to_string(), case.clone())),
                "case '{case}' in {fixture} is missing from the feature matrix"
            );
        }
    }
    assert!(total >= 52, "conformance matrix regressed to {total} cases");
    assert!(
        FEATURES.len() >= 49,
        "feature matrix regressed to {} features",
        FEATURES.len()
    );
    println!(
        "conformance feature matrix: {total} cases, {} features, {} categories",
        FEATURES.len(),
        FIXTURES.len()
    );
}

#[test]
fn upstream_derived_vectors_cite_the_pinned_source() {
    let mut sourced = 0;
    for fixture in FIXTURES {
        for (case, source) in fixture_sources(fixture) {
            if let Some(source) = source {
                sourced += 1;
                assert!(
                    source.starts_with("xstate@6.0.0-beta.1 commit d56483f"),
                    "case '{case}' cites an unpinned source: {source}"
                );
                assert!(
                    source.contains("packages/core/test/"),
                    "case '{case}' does not name its upstream test file: {source}"
                );
            }
        }
    }
    assert!(
        sourced >= 9,
        "expected at least nine upstream-derived vectors, found {sourced}"
    );
}
