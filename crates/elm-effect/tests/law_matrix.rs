//! ADR-0002's laws, pinned to witnesses, upstream sources, and verbs.
//!
//! The matrix fails when a law loses its witness, a witness is renamed, a
//! native carve-out loses its reason, an upstream citation leaves the pin, or
//! a verb points at a law that does not exist. Run with `--nocapture` for the
//! counts; the report is the tracker, not a hand-maintained ledger.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// The Effect tag this port names in ADR-0002, at the commit the ADR read.
const PIN: &str = "effect@4.0.0 commit 67ba4e46a11ccda0b6761578bfd22c04ae00167d";
const UPSTREAM_PREFIX: &str = "packages/effect/test/";

#[derive(Clone, Copy)]
enum Provenance {
    /// A test under `packages/effect/test/` at [`PIN`].
    Effect(&'static str),
    /// No Effect 4 module carries the behavior; the reason is part of the law.
    Native(&'static str),
}

#[derive(Clone, Copy)]
struct Law {
    id: &'static str,
    title: &'static str,
    provenance: Provenance,
    rust: &'static [&'static str],
    witnesses: &'static [&'static str],
    not_applicable: &'static [(&'static str, &'static str)],
}

#[derive(Clone, Copy)]
struct Verb {
    verb: &'static str,
    rust: &'static str,
    law: &'static str,
    witness: &'static str,
}

const LAWS: &[Law] = &[
    Law {
        id: "L1",
        title: "A description does not run",
        provenance: Provenance::Effect("packages/effect/test/Effect.test.ts"),
        rust: &["Effect::succeed", "Effect::sync", "Effect::suspend"],
        witnesses: &[
            "crates/elm-effect/tests/l3_effect.rs::description_drop_runs_nothing",
            "crates/elm-effect/tests/l3_effect.rs::combinator_descriptions_drop_runs_nothing",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L2",
        title: "Five outcomes stay apart",
        provenance: Provenance::Effect("packages/effect/test/Cause.test.ts"),
        rust: &["Cause", "Reason", "Exit"],
        witnesses: &[
            "crates/elm-effect/tests/l2_cause.rs::five_outcomes_do_not_collapse",
            "crates/elm-effect/tests/l4_fiber.rs::panic_in_child_is_die_and_sibling_lives",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L3",
        title: "One carrier, run at the edge",
        provenance: Provenance::Native(
            "native: Rust has no ambient runtime; the workspace lint enforces the edge",
        ),
        rust: &["Runtime::run", "ManagedRuntime::run"],
        witnesses: &[
            "crates/elm-effect-lints/tests/workspace_audit.rs::the_workspace_passes_its_own_lints",
            "crates/elm-effect/tests/l2_cause.rs::library_crate_has_no_run",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L4",
        title: "Policy is a value, in one order",
        provenance: Provenance::Effect("packages/effect/test/Effect.test.ts"),
        rust: &["policy::remote", "Effect::retry", "Effect::timeout"],
        witnesses: &[
            "crates/elm-effect/tests/remote.rs::attempt_timeout_does_not_cover_the_queue",
            "crates/elm-effect/tests/remote.rs::retry_releases_per_attempt",
            "crates/elm-effect/tests/remote.rs::remote_policy_is_capped",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L5",
        title: "Services are typed, acyclic, and role-distinct",
        provenance: Provenance::Effect("packages/effect/test/Layer.test.ts"),
        rust: &["Layer", "ServiceTag", "BuiltServices"],
        witnesses: &[
            "crates/elm-effect/tests/layer.rs::cycle_fails_construction",
            "crates/elm-effect/tests/l5_memo_refs.rs::double_provide_with_distinct_values_fails_at_construction",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L6",
        title: "The scope releases on every exit, in reverse order, exactly once",
        provenance: Provenance::Effect("packages/effect/test/Effect.test.ts"),
        rust: &["Scope", "Effect::scoped", "Effect::acquire_use_release"],
        witnesses: &[
            "crates/elm-effect/tests/l5_scope.rs::release_on_three_exits",
            "crates/elm-effect/tests/scope_exit.rs::exit_aware_and_plain_finalizers_share_lifo",
            "crates/elm-effect/tests/l6_concurrency.rs::all_fails_fast_and_releases_rest",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L7",
        title: "Finalizers and commits are uninterruptible",
        provenance: Provenance::Effect("packages/effect/test/Effect.test.ts"),
        rust: &[
            "Ctx::masked",
            "Effect::uninterruptible",
            "Effect::uninterruptible_mask",
        ],
        witnesses: &[
            "crates/elm-effect/tests/l5_scope.rs::cancel_during_release_finishes_release",
            "crates/elm-effect/tests/mask_guard.rs::masked_restores_depth_on_panic_and_stays_interruptible",
            "crates/elm-effect/tests/l4_fiber.rs::uninterruptible_region_completes_before_interrupt_returns",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L8",
        title: "Tests substitute the world",
        provenance: Provenance::Effect("packages/effect/test/TestClock.test.ts"),
        rust: &["TestClock", "DeterministicExecutor", "Random::with_seed"],
        witnesses: &[
            "crates/elm-effect/tests/time_clock.rs::test_clock_sleep_delays_until_adjusted",
            "crates/elm-effect/tests/time_clock.rs::test_clock_sleep_does_not_require_wall_time",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L9",
        title: "Shared state has a ladder",
        provenance: Provenance::Effect("packages/effect/test/Ref.test.ts"),
        rust: &["Ref", "SynchronizedRef", "Queue", "Semaphore"],
        witnesses: &[
            "crates/elm-effect/tests/l9_sync.rs::atomic_update_retries_without_io",
            "crates/elm-effect/tests/queue.rs::offers_and_polls_roundtrip",
            "crates/elm-effect/tests/semaphore.rs::full_take_restores_after_body",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L10",
        title: "Every wait is bounded",
        provenance: Provenance::Effect("packages/effect/test/Schedule.test.ts"),
        rust: &["Schedule", "policy::remote"],
        witnesses: &[
            "crates/elm-effect/tests/remote.rs::remote_policy_is_capped",
            "crates/elm-effect/tests/schedule.rs::recurs_counts_retries",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L11",
        title: "Parse once, at the boundary",
        provenance: Provenance::Effect("packages/effect/test/Redacted.test.ts"),
        rust: &["Redacted"],
        witnesses: &["crates/elm-effect/tests/observability.rs::secret_debug_is_redacted"],
        not_applicable: &[],
    },
    Law {
        id: "L12",
        title: "A long-lived program is a fold",
        provenance: Provenance::Native(
            "native: the Elm loop is this repository's addition, not an Effect 4 module",
        ),
        rust: &["Program", "Interpret", "SubscriptionRef"],
        witnesses: &[
            "crates/elm-effect-elm/tests/replay.rs::fold_is_deterministic",
            "crates/elm-effect-elm/tests/program.rs::subscriber_cannot_write",
            "crates/elm-effect/tests/subscription_ref_core.rs::resubscribe_notifies_each_listener_once",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L13",
        title: "A batch is a bounded map-reduce",
        provenance: Provenance::Effect("packages/effect/test/Effect.test.ts"),
        rust: &["Effect::all", "Concurrency", "Stream::map_effect"],
        witnesses: &[
            "crates/elm-effect/tests/l6_concurrency.rs::fanout_peak_in_flight",
            "crates/elm-effect/tests/l6_concurrency.rs::all_without_concurrency_is_sequential",
            "crates/elm-effect/tests/effect_fan_out.rs::first_failure_interrupts_and_releases_siblings",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L14",
        title: "Intent is recorded before the provider runs",
        provenance: Provenance::Native(
            "native: the journal lifecycle is toron's persisted vocabulary, not an Effect 4 module",
        ),
        rust: &[
            "EffectState::can_transition",
            "plan_attempt",
            "JournalStore",
        ],
        witnesses: &[
            "crates/elm-effect-journal/tests/journal.rs::golden_transitions",
            "crates/elm-effect-journal/tests/journal_store.rs::unknown_has_no_default_exit",
            "crates/elm-effect-journal/tests/plan_attempt.rs::unknown_has_no_default_exit",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L15",
        title: "Replay order is ordered",
        provenance: Provenance::Native(
            "native: replay determinism belongs to the Elm loop, not an Effect 4 module",
        ),
        rust: &["BTreeMap", "BTreeSet", "Program::Model"],
        witnesses: &[
            "crates/elm-effect-lints/tests/lints.rs::btreemap_in_a_model_is_clean",
            "crates/elm-effect-lints/tests/lints.rs::hashmap_in_a_model_fires_on_dirty_fixture",
            "crates/elm-effect-elm/tests/replay.rs::fold_is_deterministic",
        ],
        not_applicable: &[],
    },
    Law {
        id: "L16",
        title: "The message log is a durable stream",
        provenance: Provenance::Native(
            "native: Durable Streams protocol draft 1.0 at commit 461b40267aabd644558f9b19dbb9507dd5f691cf",
        ),
        rust: &["DurableLog", "JournalStore"],
        witnesses: &[
            "crates/elm-effect-journal/tests/durable_log_laws.rs::same_triple_appends_once_on_the_trait",
            "crates/elm-effect-journal/tests/durable_log_laws.rs::stale_epoch_is_fenced_on_the_trait",
            "crates/elm-effect-journal/tests/durable_log_laws.rs::seq_gap_is_rejected_on_the_trait",
            "crates/elm-effect-journal/tests/durable_log_laws.rs::new_epoch_starts_at_zero_on_the_trait",
            "crates/elm-effect-journal/tests/durable_log_append.rs::hand_built_offset_is_rejected",
            "crates/elm-effect-journal/tests/durable_log_append.rs::partial_identity_is_rejected",
            "crates/elm-effect-journal/tests/durable_log_append.rs::append_is_atomic",
            "crates/elm-effect-journal/tests/durable_log_closure.rs::bytes_before_close_stay_readable",
            "crates/elm-effect-journal/tests/durable_log_closure.rs::close_is_monotonic_and_idempotent",
            "crates/elm-effect-journal/tests/durable_log_page.rs::a_closed_page_reports_eof_only_at_the_tail",
            "crates/elm-effect-journal/tests/journal_store.rs::old_offset_returns_gone_with_first_replayable",
            "crates/elm-effect-elm/tests/replay.rs::snapshot_is_model_plus_offset",
        ],
        not_applicable: &[
            (
                "fork_drops_producer_epoch",
                "native DurableLog has no fork; Stream-Fork-Offset belongs to the HTTP adapter, and the native log's producer identity is append-only",
            ),
            (
                "soft_delete_gone_is_not_retention",
                "native DurableLog has no soft delete; JournalStore's Gone is a retention miss, and the protocol's soft-delete reason lives in the HTTP adapter",
            ),
        ],
    },
];

const VERBS: &[Verb] = &[
    Verb {
        verb: "succeed",
        rust: "Effect::succeed",
        law: "L1",
        witness: "crates/elm-effect/tests/l3_effect.rs::description_drop_runs_nothing",
    },
    Verb {
        verb: "fail",
        rust: "Effect::fail",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::five_outcomes_do_not_collapse",
    },
    Verb {
        verb: "die",
        rust: "Effect::die",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::five_outcomes_do_not_collapse",
    },
    Verb {
        verb: "interrupt",
        rust: "Effect::interrupt",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::five_outcomes_do_not_collapse",
    },
    Verb {
        verb: "sync",
        rust: "Effect::sync",
        law: "L1",
        witness: "crates/elm-effect/tests/l3_effect.rs::description_drop_runs_nothing",
    },
    Verb {
        verb: "suspend",
        rust: "Effect::suspend",
        law: "L1",
        witness: "crates/elm-effect/tests/l3_effect.rs::combinator_descriptions_drop_runs_nothing",
    },
    Verb {
        verb: "map",
        rust: "Effect::map",
        law: "L1",
        witness: "crates/elm-effect/tests/l3_effect.rs::combinator_descriptions_drop_runs_nothing",
    },
    Verb {
        verb: "flatMap",
        rust: "Effect::flat_map",
        law: "L1",
        witness: "crates/elm-effect/tests/l3_effect.rs::combinator_descriptions_drop_runs_nothing",
    },
    Verb {
        verb: "catch",
        rust: "Effect::catch",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::five_outcomes_do_not_collapse",
    },
    Verb {
        verb: "catchCause",
        rust: "Effect::catch_cause",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::five_outcomes_do_not_collapse",
    },
    Verb {
        verb: "exit",
        rust: "Effect::exit",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::exit_is_data_not_effect",
    },
    Verb {
        verb: "result",
        rust: "Effect::result",
        law: "L2",
        witness: "crates/elm-effect/tests/l2_cause.rs::exit_is_data_not_effect",
    },
    Verb {
        verb: "scoped",
        rust: "Effect::scoped",
        law: "L6",
        witness: "crates/elm-effect/tests/l5_scope.rs::release_on_three_exits",
    },
    Verb {
        verb: "acquireRelease",
        rust: "Effect::acquire_use_release",
        law: "L6",
        witness: "crates/elm-effect/tests/l5_scope.rs::release_on_three_exits",
    },
    Verb {
        verb: "forkChild",
        rust: "Effect::fork_child",
        law: "L6",
        witness: "crates/elm-effect/tests/l4_fiber.rs::child_dies_with_parent_and_parent_waits_finalizers",
    },
    Verb {
        verb: "join",
        rust: "FiberHandle::join",
        law: "L6",
        witness: "crates/elm-effect/tests/l4_fiber.rs::interrupt_waits_for_finalizers",
    },
    Verb {
        verb: "uninterruptible",
        rust: "Effect::uninterruptible",
        law: "L7",
        witness: "crates/elm-effect/tests/l4_fiber.rs::uninterruptible_region_completes_before_interrupt_returns",
    },
    Verb {
        verb: "all",
        rust: "Effect::all",
        law: "L13",
        witness: "crates/elm-effect/tests/l6_concurrency.rs::fanout_peak_in_flight",
    },
    Verb {
        verb: "race",
        rust: "Effect::race",
        law: "L13",
        witness: "crates/elm-effect/tests/race.rs::race_failure_does_not_beat_later_success",
    },
    Verb {
        verb: "timeout",
        rust: "Effect::timeout_option",
        law: "L4",
        witness: "crates/elm-effect/tests/effect_timeout.rs::timeout_option_returns_none_for_a_long_computation",
    },
    Verb {
        verb: "retry",
        rust: "Effect::retry",
        law: "L4",
        witness: "crates/elm-effect/tests/effect_repeat_retry.rs::retry_schedule_retries_according_to_the_specified_schedule",
    },
    Verb {
        verb: "repeat",
        rust: "Effect::repeat",
        law: "L10",
        witness: "crates/elm-effect/tests/schedule.rs::repeat_reruns_success",
    },
    Verb {
        verb: "schedule",
        rust: "Schedule",
        law: "L10",
        witness: "crates/elm-effect/tests/schedule.rs::recurs_counts_retries",
    },
    Verb {
        verb: "Queue",
        rust: "Queue",
        law: "L9",
        witness: "crates/elm-effect/tests/queue.rs::offers_and_polls_roundtrip",
    },
    Verb {
        verb: "Semaphore",
        rust: "Semaphore",
        law: "L9",
        witness: "crates/elm-effect/tests/semaphore.rs::full_take_restores_after_body",
    },
    Verb {
        verb: "SubscriptionRef",
        rust: "SubscriptionRef",
        law: "L12",
        witness: "crates/elm-effect/tests/subscription_ref_core.rs::resubscribe_notifies_each_listener_once",
    },
    Verb {
        verb: "TestClock",
        rust: "TestClock",
        law: "L8",
        witness: "crates/elm-effect/tests/time_clock.rs::test_clock_sleep_delays_until_adjusted",
    },
    Verb {
        verb: "Layer",
        rust: "Layer",
        law: "L5",
        witness: "crates/elm-effect/tests/layer.rs::cycle_fails_construction",
    },
    Verb {
        verb: "JournalStore",
        rust: "JournalStore",
        law: "L14",
        witness: "crates/elm-effect-journal/tests/journal_store.rs::intent_before_provider_same_key_on_retry",
    },
    Verb {
        verb: "DurableLog",
        rust: "DurableLog",
        law: "L16",
        witness: "crates/elm-effect-journal/tests/durable_log_laws.rs::same_triple_appends_once_on_the_trait",
    },
    Verb {
        verb: "Program",
        rust: "Program",
        law: "L12",
        witness: "crates/elm-effect-elm/tests/replay.rs::fold_is_deterministic",
    },
    Verb {
        verb: "Interpret",
        rust: "Interpret",
        law: "L12",
        witness: "crates/elm-effect-elm/tests/program.rs::interpret_feedback_runs_commands",
    },
];

#[test]
fn law_matrix_is_bidirectional_and_source_pinned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut errors = Vec::new();

    let expected: BTreeSet<String> = (1..=16).map(|n| format!("L{n}")).collect();
    let mut ids = BTreeSet::new();
    let mut witnesses = 0usize;

    for law in LAWS {
        if !ids.insert(law.id.to_string()) {
            errors.push(format!("{}: duplicate law id", law.id));
        }
        if law.title.trim().is_empty() {
            errors.push(format!("{}: empty title", law.id));
        }
        if law.witnesses.is_empty() && law.not_applicable.is_empty() {
            errors.push(format!("{}: no witness and no native carve-out", law.id));
        }
        if law.rust.is_empty() || law.rust.iter().any(|item| item.trim().is_empty()) {
            errors.push(format!("{}: empty Rust mapping", law.id));
        }

        match law.provenance {
            Provenance::Effect(path) => {
                if !path.starts_with(UPSTREAM_PREFIX) || !path.ends_with(".test.ts") {
                    errors.push(format!(
                        "{}: upstream path must be under {UPSTREAM_PREFIX} and end in .test.ts: {path}",
                        law.id
                    ));
                }
            }
            Provenance::Native(reason) => {
                if !reason.starts_with("native:") || reason.trim().len() <= "native:".len() {
                    errors.push(format!(
                        "{}: native provenance needs a non-empty `native:` reason",
                        law.id
                    ));
                }
            }
        }

        let mut seen = BTreeSet::new();
        for witness in law.witnesses {
            witnesses += 1;
            if !seen.insert(*witness) {
                errors.push(format!("{}: witness claimed twice: {witness}", law.id));
            }
            check_witness(&root, witness, &mut errors);
        }

        let mut carve_outs = BTreeSet::new();
        for (clause, reason) in law.not_applicable {
            if !carve_outs.insert(*clause) {
                errors.push(format!("{}: carve-out claimed twice: {clause}", law.id));
            }
            if reason.trim().is_empty() || !reason.starts_with("native ") {
                errors.push(format!(
                    "{}: carve-out {clause} needs a `native ...` reason",
                    law.id
                ));
            }
        }
    }

    for missing in expected.difference(&ids) {
        errors.push(format!("{missing}: law is absent from the matrix"));
    }
    for extra in ids.difference(&expected) {
        errors.push(format!("{extra}: law id is outside L1..L16"));
    }

    let mut verb_names = BTreeSet::new();
    for verb in VERBS {
        if !verb_names.insert(verb.verb) {
            errors.push(format!("verb {}: duplicate row", verb.verb));
        }
        if !ids.contains(verb.law) {
            errors.push(format!(
                "verb {}: names {}, which is not a law row",
                verb.verb, verb.law
            ));
        }
        if verb.rust.trim().is_empty() {
            errors.push(format!("verb {}: empty Rust mapping", verb.verb));
        }
        witnesses += 1;
        check_witness(&root, verb.witness, &mut errors);
    }

    println!(
        "law matrix: {} laws, {} witnesses/verbs, {} verbs, {} native carve-outs (pin: {PIN})",
        LAWS.len(),
        witnesses,
        VERBS.len(),
        LAWS.iter()
            .map(|law| law.not_applicable.len())
            .sum::<usize>()
    );

    assert!(
        errors.is_empty(),
        "law matrix drift:\n{}",
        errors.join("\n")
    );
}

fn check_witness(root: &Path, witness: &str, errors: &mut Vec<String>) {
    let Some((file, name)) = witness.split_once("::") else {
        errors.push(format!("{witness}: expected a `path::test_name` witness"));
        return;
    };
    if !file.starts_with("crates/") {
        errors.push(format!("{witness}: witness must live under crates/"));
        return;
    }
    if name.trim().is_empty() {
        errors.push(format!("{witness}: empty test name"));
        return;
    }

    let path = root.join(file);
    let Ok(source) = fs::read_to_string(&path) else {
        errors.push(format!(
            "{witness}: file does not exist at {}",
            path.display()
        ));
        return;
    };
    if !source.contains(&format!("fn {name}(")) {
        errors.push(format!("{witness}: no `fn {name}(` in {file}"));
    }
}
