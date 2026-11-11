---
name: verify-framework-flows
description: "Model-check and visualize the core framework flows of elm-effect-rs (Elm loop, structured concurrency, durable sagas, the Step/Drive interpreter, and scope masking) using TLA+ and statechart."
---

# Formal Verification: Core Framework Flows

This skill model-checks and visualizes the core runtime flows of `elm-effect-rs`:

1. **Flow 1 (`flow1_elm_loop.tla`)**: Elm loop reactive cycle, pre-dispatch journaling, and fenced command windows.
2. **Flow 2 (`flow2_concurrency.tla`)**: Structured concurrency, cooperative interrupts, LIFO finalizers, and bounded straggler quarantine.
3. **Flow 3 (`flow3_saga.tla`)**: Durable sagas, atomic lease claims with generation fences, and strict LIFO compensation.
4. **Flow 6 (`flow6_statechart_microstep.tla`)**: Statechart microstep quiescence, LCA resolution, and Law L1 execution ordering.
5. **Flow 7 (`flow7_statechart_parallel.tla`)**: Parallel orthogonal regions, concurrent dispatch, and synchronized `onDone` completion.
6. **Flow 8 (`flow8_statechart_actor.tla`)**: Hierarchical actor supervision, scoped lifecycles, and recursive scope teardown.
7. **Flow 9 (`flow9_statechart_timer.tla`)**: Virtualized delayed transitions (`after`) and preemptive timer invalidation.
8. **Flow 10 (`flow10_effect_interpreter.tla`)**: The private `Step`/`Drive` trampoline, continuation frames, failure unwind, boundary interrupt checks, and mask deferral.
9. **Flow 11 (`flow11_scope_masking.tla`)**: LIFO release order, uninterruptible release regions, and cancellation delivery only at an unmasked boundary.

Flows 10 and 11 close the carrier gap: Flow 2 proves fiber/scope lifecycle, and these two prove the interpreter and release protocol underneath it.

Eight dedicated machine models cover the remaining catalog entries: `schedule_state.tla`, `deferred_latch.tla`, `test_clock.tla`, `crash_recovery.tla`, `dispatch_plan.tla`, `park_plan.tla`, `engine_run_phase.tla`, and `lineage_rollover.tla`.

## Code Covered & Sync Commit

- **Sync Commit**: `14574bf`
- **Crates Covered**:
  - [`crates/elm-effect-elm`](../../crates/elm-effect-elm) (`Loop`, `Program`, `Update`)
  - [`crates/elm-effect`](../../crates/elm-effect) (`Effect`, `Step`, `Drive`, `Fiber`, `Scope`, `Supervisor`)
  - [`crates/elm-effect-journal`](../../crates/elm-effect-journal) (`JournalStore`, `Saga`)
  - [`crates/elm-effect-durable`](../../crates/elm-effect-durable) (`OurEngine`)
  - [`crates/elm-effect-xstate`](../../crates/elm-effect-xstate) (`Machine`, `StateNode`, `ActorSystem`, `Configuration`)

## How to Verify and Generate Statecharts

Run all 21 checks locally, including generated TLA and all 19 models:

```bash
./scripts/verify-formal.sh
cargo test -p elm-effect --test machine_matrix -- --nocapture
```

The `formal` CI job runs TLC directly on all 19 models. The full local script
also generates the statecharts and checks the generated xstate export; those
steps need the host-local statechart skill. The machine matrix records 20
model-backed machines and 1 exhaustive state table; it has no unmodeled row it
will accept, and fails on a missing model, invariant, code path, or witness.

Or verify one flow in an external temporary folder:

```bash
# Flow 1 (Elm Loop):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow1_elm_loop.tla \
  --config skills/verify-framework-flows/flow1_elm_loop.cfg \
  --view skills/verify-framework-flows/view1.js \
  --out /tmp/flow1-chart

# Flow 2 (Structured Concurrency):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow2_concurrency.tla \
  --config skills/verify-framework-flows/flow2_concurrency.cfg \
  --view skills/verify-framework-flows/view2.js \
  --out /tmp/flow2-chart

# Flow 3 (Durable Saga):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow3_saga.tla \
  --config skills/verify-framework-flows/flow3_saga.cfg \
  --view skills/verify-framework-flows/view3.js \
  --out /tmp/flow3-chart

# Flow 10 (Step/Drive interpreter):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow10_effect_interpreter.tla \
  --config skills/verify-framework-flows/flow10_effect_interpreter.cfg \
  --view skills/verify-framework-flows/view10.js \
  --out /tmp/flow10-chart

# Flow 11 (Scope masking):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow11_scope_masking.tla \
  --config skills/verify-framework-flows/flow11_scope_masking.cfg \
  --view skills/verify-framework-flows/view11.js \
  --out /tmp/flow11-chart
```

Each flow runs in a few seconds. When TLC verifies that all invariants hold and no deadlocks exist, `chart.mmd` is generated containing the visual statechart.
