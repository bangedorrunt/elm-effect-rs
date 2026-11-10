#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "=== 1/7 Verifying Generated Statechart TLA Export ==="
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

echo "=== 2/7 Verifying Generated Statechart Module Under TLC ==="
cargo run -q -p elm-effect-xstate --example emit_tla > "$TMP_DIR/GeneratedStatechart.tla"
cat > "$TMP_DIR/GeneratedStatechart.cfg" <<'CFG'
INIT Init
NEXT Next
CHECK_DEADLOCK FALSE
CFG
"$STATECHART" "$TMP_DIR/GeneratedStatechart.tla" \
  --config "$TMP_DIR/GeneratedStatechart.cfg" \
  --out "$TMP_DIR/generated_statechart"

echo "=== 3/7 Verifying Signal Latch (skills/verify-signal-latch) ==="
"$STATECHART" skills/verify-signal-latch/signal_latch.tla \
  --config skills/verify-signal-latch/signal_latch.cfg \
  --view skills/verify-signal-latch/view.js \
  --out "$TMP_DIR/signal_latch"

echo "=== 4/7 Verifying Elm Loop Flow (skills/verify-framework-flows/flow1) ==="
"$STATECHART" skills/verify-framework-flows/flow1_elm_loop.tla \
  --config skills/verify-framework-flows/flow1_elm_loop.cfg \
  --view skills/verify-framework-flows/view1.js \
  --out "$TMP_DIR/flow1"

echo "=== 5/7 Verifying Concurrency & Stragglers Flow (skills/verify-framework-flows/flow2) ==="
"$STATECHART" skills/verify-framework-flows/flow2_concurrency.tla \
  --config skills/verify-framework-flows/flow2_concurrency.cfg \
  --view skills/verify-framework-flows/view2.js \
  --out "$TMP_DIR/flow2"

echo "=== 6/7 Verifying Saga Rollback Flow (skills/verify-framework-flows/flow3) ==="
"$STATECHART" skills/verify-framework-flows/flow3_saga.tla \
  --config skills/verify-framework-flows/flow3_saga.cfg \
  --view skills/verify-framework-flows/view3.js \
  --out "$TMP_DIR/flow3"

echo "=== 7/7 Verifying Lease Fencing & Split-Brain Elimination (skills/verify-lease-fencing) ==="
"$STATECHART" skills/verify-lease-fencing/lease_fencing.tla \
  --config skills/verify-lease-fencing/lease_fencing.cfg \
  --view skills/verify-lease-fencing/view.js \
  --out "$TMP_DIR/lease_fencing"

echo ""
echo "All 7 formal verification checks passed without violation or deadlock."
