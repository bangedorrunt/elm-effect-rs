---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, arbitrary]
bead: elm-effect-rs-port-arbitrary-ctu
created: 2026-10-15
verify: sh -c "cargo test -p elm-effect-unstable && python3 conformance/rows-port-remaining.py packages/effect/test/arbitrary/Arbitrary.test.ts"
verify_result: unit-test-verified (sha=f267598; fence green)
---

# Wave-1 verified solution: port arbitrary

The Effect v4 module `arbitrary` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-arbitrary-ctu` and closed `unit-test-verified` at `f267598`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p elm-effect-unstable && python3 conformance/rows-port-remaining.py packages/effect/test/arbitrary/Arbitrary.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
