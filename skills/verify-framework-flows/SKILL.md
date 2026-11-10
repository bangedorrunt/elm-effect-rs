---
name: verify-framework-flows
description: "Model-check and visualize the core framework flows of elm-effect-rs (Elm loop reactive cycle, structured concurrency & stragglers, and durable sagas) using TLA+ and statechart."
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

## Code Covered & Sync Commit

- **Sync Commit**: `010d080`
- **Crates Covered**:
  - [`crates/elm-effect-elm`](../../crates/elm-effect-elm) (`Loop`, `Program`, `Update`)
  - [`crates/elm-effect`](../../crates/elm-effect) (`Fiber`, `Scope`, `Supervisor`)
  - [`crates/elm-effect-journal`](../../crates/elm-effect-journal) (`JournalStore`, `Saga`)
  - [`crates/elm-effect-durable`](../../crates/elm-effect-durable) (`OurEngine`)
  - [`crates/elm-effect-xstate`](../../crates/elm-effect-xstate) (`Machine`, `StateNode`, `ActorSystem`, `Configuration`)

## How to Verify and Generate Statecharts

Work in an external temporary folder and run `/statechart` across each model:

```bash
# Verify Flow 1 (Elm Loop):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow1_elm_loop.tla \
  --config skills/verify-framework-flows/flow1_elm_loop.cfg \
  --view skills/verify-framework-flows/view1.js \
  --out /tmp/flow1-chart

# Verify Flow 2 (Structured Concurrency):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow2_concurrency.tla \
  --config skills/verify-framework-flows/flow2_concurrency.cfg \
  --view skills/verify-framework-flows/view2.js \
  --out /tmp/flow2-chart

# Verify Flow 3 (Durable Saga):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow3_saga.tla \
  --config skills/verify-framework-flows/flow3_saga.cfg \
  --view skills/verify-framework-flows/view3.js \
  --out /tmp/flow3-chart

# Verify Flow 6 (Statechart Microstep & LCA):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow6_statechart_microstep.tla \
  --config skills/verify-framework-flows/flow6_statechart_microstep.cfg \
  --view skills/verify-framework-flows/view6.js \
  --out /tmp/flow6-chart

# Verify Flow 7 (Parallel Orthogonal Regions):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow7_statechart_parallel.tla \
  --config skills/verify-framework-flows/flow7_statechart_parallel.cfg \
  --view skills/verify-framework-flows/view7.js \
  --out /tmp/flow7-chart

# Verify Flow 8 (Actor Supervision):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow8_statechart_actor.tla \
  --config skills/verify-framework-flows/flow8_statechart_actor.cfg \
  --view skills/verify-framework-flows/view8.js \
  --out /tmp/flow8-chart

# Verify Flow 9 (Delayed Transitions & Timer Invalidation):
~/.agents/skills/statechart/scripts/statechart skills/verify-framework-flows/flow9_statechart_timer.tla \
  --config skills/verify-framework-flows/flow9_statechart_timer.cfg \
  --view skills/verify-framework-flows/view9.js \
  --out /tmp/flow9-chart
```

Each run executes in under 2 seconds. When TLC verifies that all invariants hold and no deadlocks exist, `chart.mmd` is generated containing the visual statechart.
