---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, equal]
bead: elm-effect-rs-port-equal-l1v
created: 2026-10-15
verify: sh -c "cargo test -p elm-effect && python3 conformance/rows-port-remaining.py packages/effect/test/Equal.test.ts"
verify_result: unit-test-verified (sha=eb88ac05150393162f0309f9db3caf331ef4f600; fence green)
---

# Wave-1 verified solution: port equal

The Effect v4 module `equal` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-equal-l1v` and closed `unit-test-verified` at `eb88ac05150393162f0309f9db3caf331ef4f600`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p elm-effect && python3 conformance/rows-port-remaining.py packages/effect/test/Equal.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
