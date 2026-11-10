#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=== 1/11 Verifying Generated Statechart TLA Export ==="
cargo test -p elm-effect-xstate --test tla_export

STATECHART="${HOME}/.agents/skills/statechart/scripts/statechart"
if [[ ! -x "$STATECHART" ]]; then
  STATECHART="${HOME}/.gemini/skills/statechart/scripts/statechart"
fi

if [[ ! -x "$STATECHART" ]]; then
  echo "Error: statechart script not found in ~/.agents/skills/statechart or ~/.gemini/skills/statechart" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d /tmp/formal-verify-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "=== 2/11 Verifying Generated Statechart Module Under TLC ==="
cargo run -q -p elm-effect-xstate --example emit_tla > "$TMP_DIR/GeneratedStatechart.tla"
cat > "$TMP_DIR/GeneratedStatechart.cfg" <<'CFG'
INIT Init
NEXT Next
CHECK_DEADLOCK FALSE
CFG
"$STATECHART" "$TMP_DIR/GeneratedStatechart.tla" \
  --config "$TMP_DIR/GeneratedStatechart.cfg" \
  --out "$TMP_DIR/generated_statechart"

echo "=== 3/11 Verifying Signal Latch (skills/verify-signal-latch) ==="
"$STATECHART" skills/verify-signal-latch/signal_latch.tla \
  --config skills/verify-signal-latch/signal_latch.cfg \
  --view skills/verify-signal-latch/view.js \
  --out "$TMP_DIR/signal_latch"

echo "=== 4/11 Verifying Elm Loop Flow (skills/verify-framework-flows/flow1) ==="
"$STATECHART" skills/verify-framework-flows/flow1_elm_loop.tla \
  --config skills/verify-framework-flows/flow1_elm_loop.cfg \
  --view skills/verify-framework-flows/view1.js \
  --out "$TMP_DIR/flow1"

echo "=== 5/11 Verifying Concurrency & Stragglers Flow (skills/verify-framework-flows/flow2) ==="
"$STATECHART" skills/verify-framework-flows/flow2_concurrency.tla \
  --config skills/verify-framework-flows/flow2_concurrency.cfg \
  --view skills/verify-framework-flows/view2.js \
  --out "$TMP_DIR/flow2"

echo "=== 6/11 Verifying Saga Rollback Flow (skills/verify-framework-flows/flow3) ==="
"$STATECHART" skills/verify-framework-flows/flow3_saga.tla \
  --config skills/verify-framework-flows/flow3_saga.cfg \
  --view skills/verify-framework-flows/view3.js \
  --out "$TMP_DIR/flow3"

echo "=== 7/11 Verifying Lease Fencing & Split-Brain Elimination (skills/verify-lease-fencing) ==="
"$STATECHART" skills/verify-lease-fencing/lease_fencing.tla \
  --config skills/verify-lease-fencing/lease_fencing.cfg \
  --view skills/verify-lease-fencing/view.js \
  --out "$TMP_DIR/lease_fencing"

echo "=== 8/11 Verifying Statechart Microstep & LCA Resolution (Flow 6) ==="
"$STATECHART" skills/verify-framework-flows/flow6_statechart_microstep.tla \
  --config skills/verify-framework-flows/flow6_statechart_microstep.cfg \
  --view skills/verify-framework-flows/view6.js \
  --out "$TMP_DIR/flow6"

echo "=== 9/11 Verifying Parallel Orthogonal Regions & Join Completion (Flow 7) ==="
"$STATECHART" skills/verify-framework-flows/flow7_statechart_parallel.tla \
  --config skills/verify-framework-flows/flow7_statechart_parallel.cfg \
  --view skills/verify-framework-flows/view7.js \
  --out "$TMP_DIR/flow7"

echo "=== 10/11 Verifying Actor Supervision & Scoped Lifecycles (Flow 8) ==="
"$STATECHART" skills/verify-framework-flows/flow8_statechart_actor.tla \
  --config skills/verify-framework-flows/flow8_statechart_actor.cfg \
  --view skills/verify-framework-flows/view8.js \
  --out "$TMP_DIR/flow8"

echo "=== 11/11 Verifying Delayed Transitions & Timer Invalidation (Flow 9) ==="
"$STATECHART" skills/verify-framework-flows/flow9_statechart_timer.tla \
  --config skills/verify-framework-flows/flow9_statechart_timer.cfg \
  --view skills/verify-framework-flows/view9.js \
  --out "$TMP_DIR/flow9"

echo ""
echo "All 11 formal verification checks passed without violation or deadlock."
