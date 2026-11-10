---
name: verify-framework-flows
description: "Model-check and visualize the core framework flows of elm-effect-rs (Elm loop reactive cycle, structured concurrency & stragglers, and durable sagas) using TLA+ and statechart."
---

# Formal Verification: Core Framework Flows

This skill model-checks and visualizes the three primary runtime flows of `elm-effect-rs`:
1. **Flow 1 (`flow1_elm_loop.tla`)**: Elm loop reactive cycle, pre-dispatch journaling, and fenced command windows.
2. **Flow 2 (`flow2_concurrency.tla`)**: Structured concurrency, cooperative interrupts, LIFO finalizers, and bounded straggler quarantine.
3. **Flow 3 (`flow3_saga.tla`)**: Durable sagas, atomic lease claims with generation fences, and strict LIFO compensation.

## Code Covered & Sync Commit

- **Sync Commit**: `cf8abd2eab05fb69950140d875ea16cada96d2fe`
- **Crates Covered**:
  - [`crates/elm-effect-elm`](../../crates/elm-effect-elm) (`Loop`, `Program`, `Update`)
  - [`crates/elm-effect`](../../crates/elm-effect) (`Fiber`, `Scope`, `Supervisor`)
  - [`crates/elm-effect-journal`](../../crates/elm-effect-journal) (`JournalStore`, `Saga`)
  - [`crates/elm-effect-durable`](../../crates/elm-effect-durable) (`OurEngine`)

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
```

Each run executes in under 2 seconds. When TLC verifies that all invariants hold and no deadlocks exist, `chart.mmd` is generated containing the visual statechart.
