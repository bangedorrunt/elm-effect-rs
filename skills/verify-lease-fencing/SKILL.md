---
name: verify-lease-fencing
description: "Formally verify execution lease claims, heartbeat renewals, generation fencing, and split-brain elimination under ADR-0005 using TLA+ and statechart."
---

# Formal Verification: Lease Reclaim, Generation Fencing, and Split-Brain Elimination

This skill formally model-checks execution leases, generation fences, and concurrent worker settlement under [ADR-0005](../../docs/decisions/0005-single-host-engine.md) across [`elm-effect-durable`](../../crates/elm-effect-durable) and [`elm-effect-journal`](../../crates/elm-effect-journal).

## Code Covered & Sync Commit

- **Sync Commit**: `2537305d233157db6c1e11462057d605332f7a81`
- **Files & Functions**:
  - [`crates/elm-effect-journal/src/store.rs`](../../crates/elm-effect-journal/src/store.rs):
    - `JournalStore::claim`
    - `JournalStore::renew_lease`
    - `JournalStore::settle`
  - [`crates/elm-effect-journal/src/plan.rs`](../../crates/elm-effect-journal/src/plan.rs):
    - `check_claim`
    - `check_heartbeat`
    - `check_settle`
  - [`crates/elm-effect-durable/src/lease.rs`](../../crates/elm-effect-durable/src/lease.rs):
    - `OurEngine::heartbeat`
    - `OurEngine::query_lease`
  - [`crates/elm-effect-durable/src/runtime.rs`](../../crates/elm-effect-durable/src/runtime.rs):
    - `OurEngine::attempt_impl`

If any of these files are modified after `2537305d233157db6c1e11462057d605332f7a81`, rerun this check before merging.

## Invariants Formally Verified

1. **`NoSplitBrain`** (`~split_brain_detected /\ ~(workerA_state = "Committed" /\ workerB_state = "Committed")`):
   Two distinct workers can never both commit settlement for the same run key, even when worker pauses, network lag, or thread preemption cause overlapping execution attempts.
2. **`MutualExclusionActiveLease`** (`~((workerA_state = "Holding" /\ clock < workerA_deadline) /\ (workerB_state = "Holding" /\ clock < workerB_deadline))`):
   Two workers never simultaneously possess unexpired execution leases.
3. **`StaleWorkerFenced`** (`(worker_gen < journal_gen => worker_state /= "Committed")`):
   Any worker holding a generation strictly less than the journal's current generation is unconditionally rejected (`StaleGeneration`) on attempt, heartbeat, or settlement.
4. **`MonotonicGeneration`** (`journal_gen >= 1`):
   Generation counters move strictly forward upon every lease reclaim and never regress.

## Verification Workflow

Work in an external scratch directory outside the workspace and check the model with `/statechart`:

```bash
OUT_DIR=$(mktemp -d /tmp/lease-fencing-XXXXXX)
~/.agents/skills/statechart/scripts/statechart skills/verify-lease-fencing/lease_fencing.tla \
  --config skills/verify-lease-fencing/lease_fencing.cfg \
  --view skills/verify-lease-fencing/view.js \
  --out "$OUT_DIR"
```

The check verifies 110 states and 271 steps in under 1 second. When it passes, `/statechart` writes `chart.mmd` visualizing the parallel state regions (`Journal`, `WorkerA`, `WorkerB`, `Time`).

## Failure Witness (Known Regression)

To confirm that the verification catches real fencing violations:
1. In `lease_fencing.tla`, bypass the generation check in `WorkerAAttemptSettle`:
   ```tla
   \* Broken variant: blindly accept settlement without generation fence check:
   /\ journal_state' = "Settled"
   /\ workerA_state' = "Committed"
   ```
2. Run the verification command above.
3. TLC will immediately fail with code `1` and report the 5-step counterexample path violating `NoSplitBrain`:
   ```
   0. WorkerA claims lease at gen 2
   1. WorkerA stalls (thread freeze/network lag)
   2. clock advances past deadline
   3. WorkerB reclaims expired lease at gen 3 and commits settlement
   4. WorkerA wakes from stall and commits settlement without fence -> [SPLIT-BRAIN VIOLATION]
   ```
