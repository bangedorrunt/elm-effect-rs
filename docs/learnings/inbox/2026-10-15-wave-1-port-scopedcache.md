---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, scopedcache]
bead: elm-effect-rs-port-scopedcache-12d
created: 2026-10-15
verify: sh -c "cargo test -p elm-effect && python3 conformance/rows-port-remaining.py packages/effect/test/ScopedCache.test.ts"
verify_result: unit-test-verified (sha=ffff7a3ae28d9de7360ee56181508421a5c2bca4; fence green)
---

# Wave-1 verified solution: port scopedcache

The Effect v4 module `scopedcache` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-scopedcache-12d` and closed `unit-test-verified` at `ffff7a3ae28d9de7360ee56181508421a5c2bca4`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p elm-effect && python3 conformance/rows-port-remaining.py packages/effect/test/ScopedCache.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
