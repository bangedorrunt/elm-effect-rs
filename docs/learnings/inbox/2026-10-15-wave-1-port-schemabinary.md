---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, schemabinary]
bead: elm-effect-rs-port-schemabinary-302
created: 2026-10-15
verify: sh -c "cargo test -p elm-effect-unstable && python3 conformance/rows-port-remaining.py packages/effect/test/encoding/SchemaBinary.test.ts"
verify_result: unit-test-verified (sha=dba04bb5033ade4a1d2fa78ef63e65c851b76fbf; fence green)
---

# Wave-1 verified solution: port schemabinary

The Effect v4 module `schemabinary` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-schemabinary-302` and closed `unit-test-verified` at `dba04bb5033ade4a1d2fa78ef63e65c851b76fbf`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p elm-effect-unstable && python3 conformance/rows-port-remaining.py packages/effect/test/encoding/SchemaBinary.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
