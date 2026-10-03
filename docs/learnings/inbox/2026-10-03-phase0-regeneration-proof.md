---
tier: perishable
tags: [bd-iqte, conformance, baseline, wave-0]
bead: elm-effect-rs-epic-elm-effect-port-0c6.1
wave: 0
created: 2026-10-03
verify: cargo test -p elm-effect-conformance
verify_result: ok (1 passed, 1 ignored parity gate until phase 13; re-run 2026-10-03 exit 0)
---
# Regeneration proof beats trusting a pasted ledger

## Solution
`bash conformance/inventory.sh ~/.cache/effect-4.0.0` regenerates
`conformance/modules.toml` and `conformance/behavior-tests.tsv` byte-for-byte
(377 modules, 9921 behavior rows) from the pinned `effect@4.0.0` checkout
(commit 67ba4e46). Baseline commit `c40812be` carries ADRs, conformance,
skill, and seed so every later wave reads one committed baseline.

## Why it matters for wave-1 crews
Kernel-law beads (.2-.6) all assume the same behavior-row ledger. If the
ledger drifts from the pin, ported tests prove nothing. Re-run the inventory
script before trusting row counts.

## Principles
prove-it-works — ledger regenerates from the pin, not trusted as pasted.
sequence-verifiable-units — later waves read one committed baseline.
