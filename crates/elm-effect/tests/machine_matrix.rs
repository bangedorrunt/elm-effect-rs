//! Executable coverage matrix for the 21 catalogued state machines.
//!
//! A row fails when its code path or witness disappears, a TLA-backed row
//! loses its model, config, or declared invariant, or a machine is neither
//! modeled nor exhaustively enumerated. Run with `--nocapture` for the
//! counts; this is the tracker, not a separate spreadsheet.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const EXPECTED_MACHINES: usize = 21;
const EXPECTED_MODELED: usize = 20;
const EXPECTED_TLA_MODELS: usize = 19;
const EXPECTED_EXHAUSTIVE: usize = 1;

struct ModelRef {
    tla: &'static str,
    cfg: &'static str,
    invariants: &'static [&'static str],
}

enum Coverage {
    Modeled(&'static [ModelRef]),
    Exhaustive(&'static str),
}

struct Machine {
    number: usize,
    name: &'static str,
    subsystem: &'static str,
    code: &'static [&'static str],
    witness: &'static str,
    coverage: Coverage,
}

const FLOW1: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow1_elm_loop.tla",
    cfg: "skills/verify-framework-flows/flow1_elm_loop.cfg",
    invariants: &["NoReduceWithoutJournal", "NoDispatchWhenFenced"],
};

const FLOW2: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow2_concurrency.tla",
    cfg: "skills/verify-framework-flows/flow2_concurrency.cfg",
    invariants: &["NoHangingOnStraggler", "AllFinalizersRunBeforeClose"],
};

const FLOW3: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow3_saga.tla",
    cfg: "skills/verify-framework-flows/flow3_saga.cfg",
    invariants: &["RollbackMatchesCompleted", "NoExcessCompensation"],
};

const SIGNAL_LATCH: ModelRef = ModelRef {
    tla: "skills/verify-signal-latch/signal_latch.tla",
    cfg: "skills/verify-signal-latch/signal_latch.cfg",
    invariants: &["NoResurrectOnSignal", "NoLostWakeup", "LiveWhenSignalled"],
};

const LEASE_FENCING: ModelRef = ModelRef {
    tla: "skills/verify-lease-fencing/lease_fencing.tla",
    cfg: "skills/verify-lease-fencing/lease_fencing.cfg",
    invariants: &[
        "NoSplitBrain",
        "MutualExclusionActiveLease",
        "StaleWorkerFenced",
        "MonotonicGeneration",
    ],
};

const FLOW6: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow6_statechart_microstep.tla",
    cfg: "skills/verify-framework-flows/flow6_statechart_microstep.cfg",
    invariants: &[
        "QuiescenceReached",
        "StrictOrderExitedBeforeEntered",
        "ValidActiveConfiguration",
        "NoDeadlock",
    ],
};

const FLOW7: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow7_statechart_parallel.tla",
    cfg: "skills/verify-framework-flows/flow7_statechart_parallel.cfg",
    invariants: &[
        "OrthogonalIsolation",
        "SynchronizedJoinOnDone",
        "NoPrematureCompletion",
        "NoDeadlock",
    ],
};

const FLOW8: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow8_statechart_actor.tla",
    cfg: "skills/verify-framework-flows/flow8_statechart_actor.cfg",
    invariants: &[
        "NoLeakedChildrenOnParentExit",
        "FinalizersSettledBeforeClosed",
        "NoDeadlock",
    ],
};

const FLOW9: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow9_statechart_timer.tla",
    cfg: "skills/verify-framework-flows/flow9_statechart_timer.cfg",
    invariants: &[
        "TimerInvalidatedOnEarlyExit",
        "NoStaleTimerWakeup",
        "ExclusiveTargetResolution",
        "NoDeadlock",
    ],
};

const FLOW10: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow10_effect_interpreter.tla",
    cfg: "skills/verify-framework-flows/flow10_effect_interpreter.cfg",
    invariants: &[
        "TypeOK",
        "FailUnwindsAllFrames",
        "DoneHasNoFrames",
        "NoStepWhileUnmaskedInterrupted",
        "EveryFailureHasAReason",
        "ResumeNeedsAFrame",
    ],
};

const FLOW11: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/flow11_scope_masking.tla",
    cfg: "skills/verify-framework-flows/flow11_scope_masking.cfg",
    invariants: &[
        "TypeOK",
        "ReleasedOnce",
        "LifoRelease",
        "NoDuplicateReleased",
        "NoReleaseBeforeClose",
        "TerminalReleasedAll",
        "MaskedInterruptDeferred",
        "InterruptDeliveredUnmasked",
        "NormalCloseNeedsBody",
    ],
};

const SCHEDULE_STATE: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/schedule_state.tla",
    cfg: "skills/verify-framework-flows/schedule_state.cfg",
    invariants: &[
        "AttemptsBounded",
        "WaitingWithinBudget",
        "StopIsFinal",
        "WakeIsARetry",
    ],
};

const DEFERRED_LATCH: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/deferred_latch.tla",
    cfg: "skills/verify-framework-flows/deferred_latch.cfg",
    invariants: &[
        "BodyRunsOnce",
        "CompletionPermanent",
        "ValueStable",
        "WaitersBounded",
        "CompletionNeedsBody",
    ],
};

const TEST_CLOCK: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/test_clock.tla",
    cfg: "skills/verify-framework-flows/test_clock.cfg",
    invariants: &[
        "NoEarlyWake1",
        "NoEarlyWake2",
        "WakeOrder",
        "TimeBounded",
        "WokenNotSleeping",
        "Woken2NotSleeping",
    ],
};

const CRASH_RECOVERY: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/crash_recovery.tla",
    cfg: "skills/verify-framework-flows/crash_recovery.cfg",
    invariants: &[
        "ProviderNeedsIntent",
        "OutcomeNeedsProvider",
        "PendingNeverRan",
        "InFlightNotReplayed",
        "AtMostOneRun",
        "AtMostOneReconcile",
        "CrashedHasMessage",
        "RecoveredClassifies",
        "DoneSettled",
    ],
};

const DISPATCH_PLAN: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/dispatch_plan.tla",
    cfg: "skills/verify-framework-flows/dispatch_plan.cfg",
    invariants: &[
        "PlanIsTotal",
        "PreparedDispatchesNew",
        "TerminalsDuplicate",
        "ParkedResumes",
        "ReconcileRefuses",
        "LiveLeaseRefuses",
        "QueryBeatsIdempotency",
        "IdempotentRetriesSameKey",
        "NoContractRefuses",
    ],
};

const PARK_PLAN: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/park_plan.tla",
    cfg: "skills/verify-framework-flows/park_plan.cfg",
    invariants: &[
        "RefusalNeedsCeiling",
        "WaitHasFutureDeadline",
        "ResumeReachedDeadline",
        "DecisionIsTotal",
        "AtMostOneRow",
        "WaitHasOneRow",
        "RefusalHasNoRow",
    ],
};

const ENGINE_RUN_PHASE: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/engine_run_phase.tla",
    cfg: "skills/verify-framework-flows/engine_run_phase.cfg",
    invariants: &[
        "ExclusiveTerminals",
        "RunningClear",
        "ParkedClear",
        "TerminalPhase",
        "FailedPhase",
        "CancelledPhase",
        "ParkedResumable",
        "GenerationBounded",
        "ParkCountBounded",
        "GenerationLaw",
    ],
};

const LINEAGE_ROLLOVER: ModelRef = ModelRef {
    tla: "skills/verify-framework-flows/lineage_rollover.tla",
    cfg: "skills/verify-framework-flows/lineage_rollover.cfg",
    invariants: &[
        "EpochPositive",
        "EpochBounded",
        "NextEpochAdjacent",
        "PrevClosedBeforeNext",
        "RunningAfterRolloverClosedPrev",
        "HistoryBounded",
        "DoneHasClosedPrev",
        "SettledNextStillClosesPrev",
        "CrashKeepsNextAdjacent",
    ],
};

const MACHINES: &[Machine] = &[
    Machine {
        number: 1,
        name: "Fiber Lifecycle",
        subsystem: "elm-effect",
        code: &["crates/elm-effect/src/fiber.rs"],
        witness: "crates/elm-effect/tests/l4_fiber.rs::child_dies_with_parent_and_parent_waits_finalizers",
        coverage: Coverage::Modeled(&[FLOW2]),
    },
    Machine {
        number: 2,
        name: "Scope LIFO Stack",
        subsystem: "elm-effect",
        code: &["crates/elm-effect/src/scope.rs"],
        witness: "crates/elm-effect/tests/l5_scope.rs::release_on_three_exits",
        coverage: Coverage::Modeled(&[FLOW2, FLOW11]),
    },
    Machine {
        number: 3,
        name: "Schedule State",
        subsystem: "elm-effect",
        code: &["crates/elm-effect/src/schedule.rs"],
        witness: "crates/elm-effect/tests/schedule.rs::recurs_counts_retries",
        coverage: Coverage::Modeled(&[SCHEDULE_STATE]),
    },
    Machine {
        number: 4,
        name: "Deferred Latch",
        subsystem: "elm-effect",
        code: &["crates/elm-effect/src/sync.rs"],
        witness: "crates/elm-effect/tests/deferred_memoization.rs::complete_runs_its_body_exactly_once",
        coverage: Coverage::Modeled(&[DEFERRED_LATCH]),
    },
    Machine {
        number: 5,
        name: "TestClock Time",
        subsystem: "elm-effect",
        code: &["crates/elm-effect/src/time.rs"],
        witness: "crates/elm-effect/tests/time_clock.rs::test_clock_sleep_delays_until_adjusted",
        coverage: Coverage::Modeled(&[TEST_CLOCK]),
    },
    Machine {
        number: 6,
        name: "Loop Fold Core",
        subsystem: "elm-effect-elm",
        code: &["crates/elm-effect-elm/src/lib.rs"],
        witness: "crates/elm-effect-elm/tests/replay.rs::fold_is_deterministic",
        coverage: Coverage::Modeled(&[FLOW1]),
    },
    Machine {
        number: 7,
        name: "Crash Recovery",
        subsystem: "elm-effect-elm",
        code: &["crates/elm-effect-elm/src/lib.rs"],
        witness: "crates/elm-effect-elm/tests/crash_recovery.rs::crash_at_each_boundary_recovers_exactly_once",
        coverage: Coverage::Modeled(&[CRASH_RECOVERY]),
    },
    Machine {
        number: 8,
        name: "EffectState Registry",
        subsystem: "elm-effect-journal",
        code: &["crates/elm-effect-journal/src/lib.rs"],
        witness: "crates/elm-effect-journal/tests/journal.rs::golden_transitions",
        coverage: Coverage::Exhaustive(
            "all 9x9 state pairs are checked against the 18-edge golden table",
        ),
    },
    Machine {
        number: 9,
        name: "DispatchPlan Attempt",
        subsystem: "elm-effect-journal",
        code: &["crates/elm-effect-journal/src/plan.rs"],
        witness: "crates/elm-effect-journal/tests/plan_attempt.rs::unknown_has_no_default_exit",
        coverage: Coverage::Modeled(&[DISPATCH_PLAN]),
    },
    Machine {
        number: 10,
        name: "ParkPlan Evaluation",
        subsystem: "elm-effect-journal",
        code: &["crates/elm-effect-journal/src/plan.rs"],
        witness: "crates/elm-effect-journal/tests/plan_park.rs::plan_park_refusal_no_wrap_due_now",
        coverage: Coverage::Modeled(&[PARK_PLAN]),
    },
    Machine {
        number: 11,
        name: "Saga LIFO Rollback",
        subsystem: "elm-effect-journal",
        code: &["crates/elm-effect-journal/src/saga.rs"],
        witness: "crates/elm-effect-journal/tests/saga.rs::rollback_is_strict_lifo_over_completed_steps",
        coverage: Coverage::Modeled(&[FLOW3]),
    },
    Machine {
        number: 12,
        name: "Engine RunPhase",
        subsystem: "elm-effect-durable",
        code: &["crates/elm-effect-durable/src/engine.rs"],
        witness: "crates/elm-effect-durable/tests/conformance.rs::settle_projects_terminal_phase_and_clears_park",
        coverage: Coverage::Modeled(&[ENGINE_RUN_PHASE]),
    },
    Machine {
        number: 13,
        name: "Lease Heartbeat",
        subsystem: "elm-effect-durable",
        code: &[
            "crates/elm-effect-durable/src/runtime.rs",
            "crates/elm-effect-durable/src/lease.rs",
        ],
        witness: "crates/elm-effect-durable/tests/heartbeat.rs::renew_moves_the_deadline_and_keeps_the_fence",
        coverage: Coverage::Modeled(&[LEASE_FENCING]),
    },
    Machine {
        number: 14,
        name: "Signal Latching",
        subsystem: "elm-effect-durable",
        code: &["crates/elm-effect-durable/src/signal.rs"],
        witness: "crates/elm-effect-durable/tests/signal.rs::await_without_signals_parks_and_drains_on_time",
        coverage: Coverage::Modeled(&[SIGNAL_LATCH]),
    },
    Machine {
        number: 15,
        name: "Lineage Rollover",
        subsystem: "elm-effect-durable",
        code: &["crates/elm-effect-durable/src/lineage.rs"],
        witness: "crates/elm-effect-durable/tests/lineage.rs::happy_path_closes_prev_and_starts_next",
        coverage: Coverage::Modeled(&[LINEAGE_ROLLOVER]),
    },
    Machine {
        number: 16,
        name: "Microstep Engine",
        subsystem: "elm-effect-xstate",
        code: &["crates/elm-effect-xstate/src/machine.rs"],
        witness: "crates/elm-effect-xstate/tests/always_micro_steps.rs::eventless_always_transitions_cascade_until_quiescence",
        coverage: Coverage::Modeled(&[FLOW6]),
    },
    Machine {
        number: 17,
        name: "Parallel Coordinator",
        subsystem: "elm-effect-xstate",
        code: &["crates/elm-effect-xstate/src/configuration.rs"],
        witness: "crates/elm-effect-xstate/tests/parallel_regions.rs::one_event_advances_every_region_that_handles_it",
        coverage: Coverage::Modeled(&[FLOW7]),
    },
    Machine {
        number: 18,
        name: "Actor Hierarchy System",
        subsystem: "elm-effect-xstate",
        code: &["crates/elm-effect-xstate/src/actor.rs"],
        witness: "crates/elm-effect-xstate/tests/actors.rs::stopping_a_parent_closes_child_scopes_last_in_first_out",
        coverage: Coverage::Modeled(&[FLOW8]),
    },
    Machine {
        number: 19,
        name: "Delayed Transition Scheduler",
        subsystem: "elm-effect-xstate",
        code: &[
            "crates/elm-effect-xstate/src/dynamic.rs",
            "crates/elm-effect-xstate/src/types.rs",
        ],
        witness: "crates/elm-effect-xstate/tests/after_timers.rs::leaving_a_delayed_state_cancels_it_and_a_stale_fire_does_nothing",
        coverage: Coverage::Modeled(&[FLOW9]),
    },
    Machine {
        number: 20,
        name: "Effect Interpreter",
        subsystem: "elm-effect",
        code: &["crates/elm-effect/src/effect.rs"],
        witness: "crates/elm-effect/tests/effect_stack_safety.rs::deeply_nested_map_continuations_run",
        coverage: Coverage::Modeled(&[FLOW10]),
    },
    Machine {
        number: 21,
        name: "Scope Masking",
        subsystem: "elm-effect",
        code: &[
            "crates/elm-effect/src/scope.rs",
            "crates/elm-effect/src/fiber.rs",
        ],
        witness: "crates/elm-effect/tests/l5_scope.rs::cancel_during_release_finishes_release",
        coverage: Coverage::Modeled(&[FLOW11]),
    },
];

#[test]
fn machine_matrix_is_complete_and_model_backed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut errors = Vec::new();
    let mut numbers = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut models = BTreeSet::new();
    let mut modeled = 0usize;
    let mut exhaustive = 0usize;

    for machine in MACHINES {
        let label = format!("#{} {}", machine.number, machine.name);
        if !numbers.insert(machine.number) {
            errors.push(format!("{label}: duplicate machine number"));
        }
        if machine.number < 1 || machine.number > EXPECTED_MACHINES {
            errors.push(format!("{label}: number outside 1..={EXPECTED_MACHINES}"));
        }
        if !names.insert(machine.name) {
            errors.push(format!("{label}: duplicate machine name"));
        }
        if machine.subsystem.trim().is_empty() {
            errors.push(format!("{label}: empty subsystem"));
        }
        if machine.code.is_empty() {
            errors.push(format!("{label}: no code location"));
        }

        for code in machine.code {
            if !code.starts_with("crates/") {
                errors.push(format!(
                    "{label}: code path must live under crates/: {code}"
                ));
                continue;
            }
            if !root.join(code).is_file() {
                errors.push(format!("{label}: code path does not exist: {code}"));
            }
        }
        check_witness(&root, machine.witness, &label, &mut errors);

        match machine.coverage {
            Coverage::Modeled(refs) => {
                modeled += 1;
                if refs.is_empty() {
                    errors.push(format!("{label}: modeled row has no model"));
                }
                for model in refs {
                    models.insert(model.tla);
                    check_model(&root, &label, model, &mut errors);
                }
            }
            Coverage::Exhaustive(reason) => {
                exhaustive += 1;
                if !reason.starts_with("all ") || reason.trim().len() <= "all ".len() {
                    errors.push(format!(
                        "{label}: exhaustive coverage needs an `all ...` reason"
                    ));
                }
            }
        }
    }

    for missing in 1..=EXPECTED_MACHINES {
        if !numbers.contains(&missing) {
            errors.push(format!("#{missing}: machine is absent from the matrix"));
        }
    }

    let actual = [
        ("machines", MACHINES.len(), EXPECTED_MACHINES),
        ("modeled rows", modeled, EXPECTED_MODELED),
        ("TLA models", models.len(), EXPECTED_TLA_MODELS),
        ("exhaustive rows", exhaustive, EXPECTED_EXHAUSTIVE),
    ];
    for (kind, found, expected) in actual {
        if found != expected {
            errors.push(format!(
                "{kind}: found {found}, expected {expected}; update the matrix and its baseline together"
            ));
        }
    }

    println!(
        "machine matrix: {} machines, {} modeled rows, {} TLA models, {} exhaustive, {} witnesses",
        MACHINES.len(),
        modeled,
        models.len(),
        exhaustive,
        MACHINES.len()
    );

    assert!(
        errors.is_empty(),
        "machine matrix drift:\n{}",
        errors.join("\n")
    );
}

fn check_model(root: &Path, label: &str, model: &ModelRef, errors: &mut Vec<String>) {
    let Some(tla) = read(root, model.tla, label, "model", errors) else {
        return;
    };
    let Some(cfg) = read(root, model.cfg, label, "config", errors) else {
        return;
    };

    if !cfg.contains("INIT Init") {
        errors.push(format!("{label}: {} has no `INIT Init`", model.cfg));
    }
    if !cfg.contains("NEXT Next") {
        errors.push(format!("{label}: {} has no `NEXT Next`", model.cfg));
    }
    for invariant in model.invariants {
        if !cfg.contains(invariant) {
            errors.push(format!(
                "{label}: {} does not declare invariant {invariant}",
                model.cfg
            ));
        }
        if !tla.contains(invariant) {
            errors.push(format!(
                "{label}: {} does not define invariant {invariant}",
                model.tla
            ));
        }
    }
}

fn read(
    root: &Path,
    path: &str,
    label: &str,
    kind: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    match fs::read_to_string(root.join(path)) {
        Ok(source) => Some(source),
        Err(error) => {
            errors.push(format!("{label}: {kind} {path} is unreadable: {error}"));
            None
        }
    }
}

fn check_witness(root: &Path, witness: &str, label: &str, errors: &mut Vec<String>) {
    let Some((file, name)) = witness.split_once("::") else {
        errors.push(format!("{label}: expected a `path::test_name` witness"));
        return;
    };
    if !file.starts_with("crates/") {
        errors.push(format!("{label}: witness must live under crates/: {file}"));
        return;
    }
    if name.trim().is_empty() {
        errors.push(format!("{label}: empty test name in witness"));
        return;
    }
    match fs::read_to_string(root.join(file)) {
        Ok(source) if source.contains(&format!("fn {name}(")) => {}
        Ok(_) => errors.push(format!("{label}: no `fn {name}(` in {file}")),
        Err(error) => errors.push(format!(
            "{label}: witness file {file} is unreadable: {error}"
        )),
    }
}
