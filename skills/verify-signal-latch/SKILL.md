---
name: verify-signal-latch
description: "Formally verify the signal latch, park/wake commit-gap race, and run liveness invariants in elm-effect-durable using TLA+ and statechart."
---

# Formal Verification: Signal Latching and Park Commit-Gap Race

This skill formally model-checks the concurrent interactions between signal latching, parking, cancellation, and wakeups in [`elm-effect-durable`](../../crates/elm-effect-durable).

## Code Covered & Sync Commit

- **Sync Commit**: `2537305d233157db6c1e11462057d605332f7a81`
- **Files & Functions**:
  - [`crates/elm-effect-durable/src/signal.rs`](../../crates/elm-effect-durable/src/signal.rs):
    - `OurEngine::await_signal`
    - `OurEngine::signal`
  - [`crates/elm-effect-durable/src/engine.rs`](../../crates/elm-effect-durable/src/engine.rs):
    - `OurEngine::park_until`
    - `OurEngine::wake_checked_impl`
    - `OurEngine::cancel`
  - [`crates/elm-effect-durable/tests/signal.rs`](../../crates/elm-effect-durable/tests/signal.rs):
    - `await_signal_gap_wake_checks_liveness`

If any of these files are modified after `2537305d233157db6c1e11462057d605332f7a81`, rerun this check before merging.

## Invariants Formally Verified

1. **`NoResurrectOnSignal`** (`cancelled_in_flight => outcome /= "Signalled"`):
   If a workflow run is cancelled prior to or while waiting in `await_signal`, an incoming signal must never resurrect the run into `Signalled`. It must fail with `EngineError::Terminal`.
2. **`NoLostWakeup`** (`(signal_sent /\ ~cancelled_in_flight) => outcome /= "Parked"`):
   If an external signal arrives before or during the `park_until` commit gap, `await_signal` must detect it and resume immediately, never stranding the run in `Parked`.
3. **`LiveWhenSignalled`** (`outcome = "Signalled" => phase = "Running"`):
   Any run returning `Signalled` from `await_signal` is guaranteed to be in the `Running` phase.

## Verification Workflow

Work in an external scratch directory outside the workspace and check the model with the `/statechart` skill:

```bash
OUT_DIR=$(mktemp -d /tmp/signal-verify-XXXXXX)
~/.agents/skills/statechart/scripts/statechart skills/verify-signal-latch/signal_latch.tla \
  --config skills/verify-signal-latch/signal_latch.cfg \
  --view skills/verify-signal-latch/view.js \
  --out "$OUT_DIR"
```

The check finishes in under 2 seconds. When it passes, `/statechart` writes `chart.mmd` visualizing the parallel state regions (`Runner`, `EnginePhase`, `PendingSignals`).

## Failure Witness (Known Regression)

To confirm that the verification catches real concurrency bugs:
1. Temporarily comment out `self.require_live(key)?;` in [`await_signal`](../../crates/elm-effect-durable/src/signal.rs) within the non-empty `pending_signals` branch.
2. In `signal_latch.tla`, omit `phase = "Cancelled"` handling in `RunnerGapCheck`.
3. Run the verification command above.
4. TLC will immediately fail with code `1` and report the shortest 5-step counterexample path violating `NoResurrectOnSignal`:
   ```
   0. Runner CheckPre, EnginePhase Running, PendingSignals NoSignals
   1. runner sees no signals -> CommitPark
   2. runner commits park in storage -> GapCheck (Parked)
   3. external signal arrives -> GapCheck (Running, HasSignals)
   4. external cancel arrives -> GapCheck (Cancelled, HasSignals)
   5. runner ignores liveness in gap -> Done(Signalled) [VIOLATION]
   ```
