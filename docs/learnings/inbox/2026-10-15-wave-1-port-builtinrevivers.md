---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, builtinrevivers]
bead: elm-effect-rs-port-builtinrevivers-tih
created: 2026-10-15
verify: sh -c "cargo test -p ? && python3 conformance/rows-port-remaining.py packages/effect/test/schema/representation/builtInRevivers.test.ts"
verify_result: unit-test-verified (sha=972b95c5a790b6e8d652db4a85ad7c0f5028766c; fence green)
---

# Wave-1 verified solution: port builtinrevivers

The Effect v4 module `builtinrevivers` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-builtinrevivers-tih` and closed `unit-test-verified` at `972b95c5a790b6e8d652db4a85ad7c0f5028766c`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p ? && python3 conformance/rows-port-remaining.py packages/effect/test/schema/representation/builtInRevivers.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
