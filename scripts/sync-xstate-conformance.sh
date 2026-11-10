#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

UPSTREAM_REPO="https://github.com/statelyai/xstate"
PINNED_REF="d56483f"
FIXTURES_DIR="crates/elm-effect-xstate/tests/conformance/fixtures"

echo "=== Validating the XState v6 Conformance Matrix (local vectors) ==="
echo "Upstream Reference: ${UPSTREAM_REPO} (${PINNED_REF})"
echo "Fixtures Directory: ${FIXTURES_DIR}"
echo ""

TOTAL_CASES=0
for fixture in "${FIXTURES_DIR}"/*.json; do
  COUNT=$(python3 -c "import json; data=json.load(open('${fixture}')); print(len(data))" 2>/dev/null || jq 'length' "${fixture}")
  echo "  - $(basename "${fixture}"): ${COUNT} test cases"
  TOTAL_CASES=$((TOTAL_CASES + COUNT))
done

echo ""
echo "Total declarative behavioral test vectors: ${TOTAL_CASES}"
echo ""
echo "=== Executing Rust Conformance Test Runners ==="
cargo test -p elm-effect-xstate --test conformance_test --test conformance_actors

echo ""
echo "All XState v6 conformance test vectors verified successfully."
