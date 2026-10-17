---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, schema]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-15
verify: sh -c "cargo test -p elm-effect-schema && python3 conformance/rows-port-remaining.py packages/effect/test/schema/Schema.test.ts"
verify_result: unit-test-verified (sha=ec2c5ecea9fd6a262e4a1fdcb12e31a94099ee3b; fence green)
---

# Wave-1 verified solution: port schema

The Effect v4 module `schema` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-schema-a2c` and closed `unit-test-verified` at `ec2c5ecea9fd6a262e4a1fdcb12e31a94099ee3b`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p elm-effect-schema && python3 conformance/rows-port-remaining.py packages/effect/test/schema/Schema.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
