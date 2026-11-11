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

fn collect_rule_guards(rule: &TransitionRule, out: &mut BTreeSet<String>) {
    match rule {
        TransitionRule::Shorthand(_) => {}
        TransitionRule::Single(config) => {
            if let Some(guard) = &config.guard {
                out.insert(guard.clone());
            }
        }
        TransitionRule::Multiple(configs) => {
            for config in configs {
                if let Some(guard) = &config.guard {
                    out.insert(guard.clone());
                }
            }
        }
    }
}

fn collect_guards(config: &MachineConfig, out: &mut BTreeSet<String>) {
    for node in config.states.values() {
        for rule in node.on.values().chain(node.after.values()) {
            collect_rule_guards(rule, out);
        }
        for transition in &node.always {
            if let Some(guard) = &transition.guard {
                out.insert(guard.clone());
            }
        }
        if let Some(rule) = &node.on_done {
            collect_rule_guards(rule, out);
        }
    }
}

fn collect_actions(config: &MachineConfig, out: &mut BTreeSet<String>) {
    out.extend(action_names(&config.entry).into_iter().cloned());
    out.extend(action_names(&config.exit).into_iter().cloned());
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
    let mut guard_names = BTreeSet::new();
    collect_guards(&config, &mut guard_names);
    let mut builder = DynamicMachine::builder(config);
    for name in guard_names {
        if let Some(expected) = name.strip_prefix("event_type:") {
            let expected = expected.to_string();
            builder = builder.guard(name, move |_context, event| event.r#type == expected);
        } else if let Some(field) = name.strip_prefix("event:") {
            let field = field.to_string();
            builder = builder.guard(name, move |_context, event| {
                event
                    .payload
                    .get(&field)
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            });
        }
    }
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
        } else if let Some(key) = name.strip_prefix("assign_event:") {
            let key = key.to_string();
            builder = builder.action(name, move |context, event, _enq| {
                if let Some(value) = event.payload.get(&key)
                    && let Some(data) = context.data.as_object_mut()
                {
                    data.insert(key.clone(), value.clone());
                }
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
        let started = machine.start(machine.initial_context(), &DynamicEvent::new("xstate.init"));
        let mut done = started.done;
        let mut emitted = started.emitted;
        let mut state = MachineState {
            configuration: started.configuration,
            history: started.history,
            context: started.context,
            armed: started.armed,
        };

        for event in &case.events {
            let outcome = machine.step_state(&state, event);
            done = outcome.done;
            emitted.extend(outcome.emitted);
            state = MachineState {
                configuration: outcome.configuration,
                history: outcome.history,
                context: outcome.context,
                armed: outcome.armed,
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
fn conformance_events() {
    execute_suite("events.json");
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
    "events.json",
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
        name: "self_transition_default_no_reentry",
        fixture: "actions.json",
        cases: &["self_transition_default_no_reentry"],
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
        name: "always_initial_startup",
        fixture: "always.json",
        cases: &["initial_transient_settles_on_start"],
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
        name: "guard_event_payload_match",
        fixture: "guards.json",
        cases: &["guard_event_payload_matches"],
    },
    Feature {
        name: "guard_event_payload_skip",
        fixture: "guards.json",
        cases: &["guard_event_payload_skips"],
    },
    Feature {
        name: "guard_state_in_match",
        fixture: "guards.json",
        cases: &["guard_state_in_matches"],
    },
    Feature {
        name: "guard_state_in_interim",
        fixture: "guards.json",
        cases: &["guard_state_in_interim"],
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
        name: "history_target_parent_entry",
        fixture: "history.json",
        cases: &["history_target_enters_never_visited_parent"],
    },
    Feature {
        name: "parallel_deep_history",
        fixture: "history.json",
        cases: &["deep_history_restores_every_parallel_region"],
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
    Feature {
        name: "eventless_before_raised",
        fixture: "actions.json",
        cases: &["eventless_transition_runs_before_raised_event"],
    },
    Feature {
        name: "hierarchy_descendant_target",
        fixture: "actions.json",
        cases: &["descendant_target_keeps_common_ancestor"],
    },
    Feature {
        name: "always_parallel_regions",
        fixture: "always.json",
        cases: &["parallel_regions_settle_eventless_together"],
    },
    Feature {
        name: "final_done_action_order",
        fixture: "final_states.json",
        cases: &["nested_done_actions_run_inside_out"],
    },
    Feature {
        name: "parallel_nested_done",
        fixture: "final_states.json",
        cases: &["nested_parallel_completion_propagates"],
    },
    Feature {
        name: "after_guard_true",
        fixture: "after.json",
        cases: &["guarded_after_transition_fires"],
    },
    Feature {
        name: "after_guard_false",
        fixture: "after.json",
        cases: &["guarded_after_transition_is_skipped"],
    },
    Feature {
        name: "after_parallel_regions",
        fixture: "after.json",
        cases: &["after_timers_are_region_scoped"],
    },
    Feature {
        name: "ancestor_target_reentry",
        fixture: "actions.json",
        cases: &[
            "ancestor_target_exits_and_reenters_it",
            "ancestor_target_reenters_with_initial_first_descendant",
            "ancestor_target_reenters_with_initial_latter_descendant",
        ],
    },
    Feature {
        name: "invalid_event_no_replay",
        fixture: "actions.json",
        cases: &["invalid_event_does_not_replay_prior_actions"],
    },
    Feature {
        name: "always_assign_quiescence",
        fixture: "always.json",
        cases: &["guarded_assign_loop_reaches_quiescence"],
    },
    Feature {
        name: "history_shallow_resets_descendant",
        fixture: "history.json",
        cases: &["shallow_history_resets_a_deep_descendant"],
    },
    Feature {
        name: "history_deep_two_levels",
        fixture: "history.json",
        cases: &["deep_history_restores_two_nested_levels"],
    },
    Feature {
        name: "history_initial_child",
        fixture: "history.json",
        cases: &["history_as_initial_child_uses_fallback"],
    },
    Feature {
        name: "parallel_nested_simple",
        fixture: "parallel.json",
        cases: &["nested_parallel_simple_transition"],
    },
    Feature {
        name: "parallel_nested_complex",
        fixture: "parallel.json",
        cases: &["nested_parallel_complex_transition"],
    },
    Feature {
        name: "parallel_nested_cross_region",
        fixture: "parallel.json",
        cases: &["nested_parallel_cross_region_preserves_source"],
    },
    Feature {
        name: "final_parallel_on_done_once",
        fixture: "final_states.json",
        cases: &["parallel_on_done_runs_once"],
    },
    Feature {
        name: "always_guard_priority",
        fixture: "always.json",
        cases: &["guarded_always_first_match_wins"],
    },
    Feature {
        name: "history_internal_default",
        fixture: "history.json",
        cases: &["internal_history_target_uses_fallback"],
    },
    Feature {
        name: "history_parent_entry_restore",
        fixture: "history.json",
        cases: &["history_restore_reenters_the_parent"],
    },
    Feature {
        name: "parallel_deep_flat",
        fixture: "parallel.json",
        cases: &["deep_flat_parallel_entry"],
    },
    Feature {
        name: "after_nested_initial",
        fixture: "after.json",
        cases: &["after_timer_from_nested_initial_state"],
    },
    Feature {
        name: "after_multiple_delays",
        fixture: "after.json",
        cases: &["after_multiple_delays_select_matching"],
    },
    Feature {
        name: "final_parallel_initial_completion",
        fixture: "final_states.json",
        cases: &["nested_parallel_initial_completion"],
    },
    Feature {
        name: "always_all_guards_false",
        fixture: "always.json",
        cases: &["all_guards_false_leave_state_active"],
    },
    Feature {
        name: "history_machine_initial",
        fixture: "history.json",
        cases: &["history_as_machine_initial_uses_fallback"],
    },
    Feature {
        name: "history_domain_restore",
        fixture: "history.json",
        cases: &["history_restore_stays_inside_domain"],
    },
    Feature {
        name: "parallel_direct_final_partial",
        fixture: "final_states.json",
        cases: &["parallel_direct_final_partial_done"],
    },
    Feature {
        name: "after_cycle",
        fixture: "after.json",
        cases: &["after_cycle_through_states"],
    },
    Feature {
        name: "mixed_parallel_completion_parallel_first",
        fixture: "final_states.json",
        cases: &["mixed_parallel_completion_parallel_first"],
    },
    Feature {
        name: "mixed_parallel_completion_compound_first",
        fixture: "final_states.json",
        cases: &["mixed_parallel_completion_compound_first"],
    },
    Feature {
        name: "history_multistage_restore",
        fixture: "history.json",
        cases: &["multistage_history_restores_latest"],
    },
    Feature {
        name: "history_default_domain",
        fixture: "history.json",
        cases: &["history_default_stays_inside_domain"],
    },
    Feature {
        name: "parallel_multi_region_toggle",
        fixture: "parallel.json",
        cases: &["multi_region_word_toggles"],
    },
    Feature {
        name: "parallel_target_entry_actions",
        fixture: "parallel.json",
        cases: &["parallel_target_entry_actions"],
    },
    Feature {
        name: "parallel_simultaneous_actions",
        fixture: "parallel.json",
        cases: &["parallel_simultaneous_region_actions"],
    },
    Feature {
        name: "parallel_simultaneous_event_assign",
        fixture: "parallel.json",
        cases: &["parallel_simultaneous_event_assignment"],
    },
    Feature {
        name: "parallel_atomic_regions",
        fixture: "parallel.json",
        cases: &["parallel_atomic_regions"],
    },
    Feature {
        name: "final_compound_parent_stays_active",
        fixture: "final_states.json",
        cases: &["parallel_child_completion_does_not_complete_compound"],
    },
    Feature {
        name: "final_parallel_reversed_exit",
        fixture: "final_states.json",
        cases: &["parallel_completion_reversed_exit_order"],
    },
    Feature {
        name: "parallel_initial_entry_actions",
        fixture: "actions.json",
        cases: &["parallel_initial_entry_actions"],
    },
    Feature {
        name: "parallel_exit_document_order",
        fixture: "actions.json",
        cases: &["parallel_children_exit_reverse_document_order"],
    },
    Feature {
        name: "final_nested_parallel_completion",
        fixture: "final_states.json",
        cases: &["nested_parallel_in_parallel_completion"],
    },
    Feature {
        name: "final_parallel_compound_partial",
        fixture: "final_states.json",
        cases: &["parallel_compound_partial_completion_stays_active"],
    },
    Feature {
        name: "targetless_parallel_state",
        fixture: "parallel.json",
        cases: &["targetless_parallel_state_actions"],
    },
    Feature {
        name: "targetless_parallel_region",
        fixture: "parallel.json",
        cases: &["targetless_region_actions"],
    },
    Feature {
        name: "parallel_targetless_action_order",
        fixture: "parallel.json",
        cases: &["parallel_targetless_action_document_order"],
    },
    Feature {
        name: "parallel_targetless_nested_action_order",
        fixture: "parallel.json",
        cases: &["parallel_targetless_action_nested_order"],
    },
    Feature {
        name: "final_parallel_sequential_exit",
        fixture: "final_states.json",
        cases: &["parallel_sequential_region_completion_exit_order"],
    },
    Feature {
        name: "transient_transition_actions",
        fixture: "always.json",
        cases: &["transient_carries_transition_actions"],
    },
    Feature {
        name: "transient_event_guard",
        fixture: "always.json",
        cases: &["transient_event_preserved_in_guard"],
    },
    Feature {
        name: "transient_event_action",
        fixture: "always.json",
        cases: &["transient_event_preserved_in_actions"],
    },
    Feature {
        name: "transient_state_in_microsteps",
        fixture: "always.json",
        cases: &["transient_state_in_microsteps"],
    },
    Feature {
        name: "after_targetless_delayed",
        fixture: "after.json",
        cases: &["delayed_targetless_transition"],
    },
    Feature {
        name: "action_nested_child_parent_order",
        fixture: "actions.json",
        cases: &["nested_child_parent_action_order"],
    },
    Feature {
        name: "action_same_parent_isolation",
        fixture: "actions.json",
        cases: &["same_parent_substate_action_isolation"],
    },
    Feature {
        name: "parallel_root_cross_region",
        fixture: "parallel.json",
        cases: &["parallel_root_cross_region_preserves_source"],
    },
    Feature {
        name: "multi_target_cross_region",
        fixture: "parallel.json",
        cases: &["multi_target_enters_orthogonal_regions"],
    },
    Feature {
        name: "after_internal_target_child",
        fixture: "after.json",
        cases: &["after_internal_target_child"],
    },
    Feature {
        name: "event_wildcard_fallback",
        fixture: "events.json",
        cases: &["wildcard_event_falls_back"],
    },
    Feature {
        name: "event_wildcard_priority",
        fixture: "events.json",
        cases: &[
            "exact_descriptor_beats_wildcard",
            "exact_descriptor_beats_partial_wildcard",
            "longer_wildcard_beats_shorter",
        ],
    },
    Feature {
        name: "event_wildcard_prefix",
        fixture: "events.json",
        cases: &[
            "wildcard_prefix_matches_zero_segments",
            "wildcard_prefix_matches_many_segments",
        ],
    },
    Feature {
        name: "event_wildcard_tokenized",
        fixture: "events.json",
        cases: &["non_tokenized_wildcard_does_not_match"],
    },
    Feature {
        name: "event_wildcard_guard_fallback",
        fixture: "events.json",
        cases: &["wildcard_fallback_when_exact_guard_fails"],
    },
    Feature {
        name: "event_payload_matches",
        fixture: "events.json",
        cases: &["transition_matches_payload"],
    },
    Feature {
        name: "history_multi_target_deep",
        fixture: "history.json",
        cases: &["history_multi_target_deep_default"],
    },
    Feature {
        name: "history_multi_target_shallow",
        fixture: "history.json",
        cases: &["history_multi_target_shallow_default"],
    },
    Feature {
        name: "relative_target_parallel_region",
        fixture: "parallel.json",
        cases: &["relative_target_within_parallel_region"],
    },
    Feature {
        name: "history_relative_target",
        fixture: "history.json",
        cases: &["history_internal_relative_target"],
    },
    Feature {
        name: "after_iso8601_duration",
        fixture: "after.json",
        cases: &["after_iso8601_duration"],
    },
    Feature {
        name: "after_named_delay_expression",
        fixture: "after.json",
        cases: &["after_named_delay_expression"],
    },
    Feature {
        name: "final_state_inert",
        fixture: "final_states.json",
        cases: &["final_region_ignores_its_own_transition"],
    },
    Feature {
        name: "root_completion_exit",
        fixture: "final_states.json",
        cases: &["root_exit_runs_after_final_entry"],
    },
    Feature {
        name: "parallel_root_completion_exit",
        fixture: "final_states.json",
        cases: &["parallel_root_completion_exit_order"],
    },
    Feature {
        name: "invoke_input_output",
        fixture: "actors.json",
        cases: &["invoke_input_and_final_output_cross_the_child_boundary"],
    },
    Feature {
        name: "invoke_reentry",
        fixture: "actors.json",
        cases: &["invoke_reentering_restarts_the_child"],
    },
    Feature {
        name: "invoke_lifetime",
        fixture: "actors.json",
        cases: &["invoke_stops_when_owner_exits"],
    },
    Feature {
        name: "invoke_error",
        fixture: "actors.json",
        cases: &["invoke_error_advances_owner_and_carries_payload"],
    },
    Feature {
        name: "invoke_error_routing",
        fixture: "actors.json",
        cases: &["invoke_error_routes_to_the_matching_actor"],
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
    assert!(
        total >= 140,
        "conformance matrix regressed to {total} cases"
    );
    assert!(
        FEATURES.len() >= 132,
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
        sourced >= 92,
        "expected at least ninety-two upstream-derived vectors, found {sourced}"
    );
}
