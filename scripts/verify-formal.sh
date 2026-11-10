#!/usr/bin/env bash
set -euo pipefail

STATECHART="${HOME}/.agents/skills/statechart/scripts/statechart"
if [[ ! -x "$STATECHART" ]]; then
  STATECHART="${HOME}/.gemini/skills/statechart/scripts/statechart"
fi

if [[ ! -x "$STATECHART" ]]; then
  echo "Error: statechart script not found in ~/.agents/skills/statechart or ~/.gemini/skills/statechart" >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TMP_DIR="$(mktemp -d /tmp/formal-verify-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "=== 1/5 Verifying Signal Latch (skills/verify-signal-latch) ==="
"$STATECHART" skills/verify-signal-latch/signal_latch.tla \
  --config skills/verify-signal-latch/signal_latch.cfg \
  --view skills/verify-signal-latch/view.js \
  --out "$TMP_DIR/signal_latch"

echo "=== 2/5 Verifying Elm Loop Flow (skills/verify-framework-flows/flow1) ==="
"$STATECHART" skills/verify-framework-flows/flow1_elm_loop.tla \
  --config skills/verify-framework-flows/flow1_elm_loop.cfg \
  --view skills/verify-framework-flows/view1.js \
  --out "$TMP_DIR/flow1"

echo "=== 3/5 Verifying Concurrency & Stragglers Flow (skills/verify-framework-flows/flow2) ==="
"$STATECHART" skills/verify-framework-flows/flow2_concurrency.tla \
  --config skills/verify-framework-flows/flow2_concurrency.cfg \
  --view skills/verify-framework-flows/view2.js \
  --out "$TMP_DIR/flow2"

echo "=== 4/5 Verifying Saga Rollback Flow (skills/verify-framework-flows/flow3) ==="
"$STATECHART" skills/verify-framework-flows/flow3_saga.tla \
  --config skills/verify-framework-flows/flow3_saga.cfg \
  --view skills/verify-framework-flows/view3.js \
  --out "$TMP_DIR/flow3"

echo "=== 5/5 Verifying Lease Fencing & Split-Brain Elimination (skills/verify-lease-fencing) ==="
"$STATECHART" skills/verify-lease-fencing/lease_fencing.tla \
  --config skills/verify-lease-fencing/lease_fencing.cfg \
  --view skills/verify-lease-fencing/view.js \
  --out "$TMP_DIR/lease_fencing"

echo ""
echo "All 5 formal verification models passed without violation or deadlock."
