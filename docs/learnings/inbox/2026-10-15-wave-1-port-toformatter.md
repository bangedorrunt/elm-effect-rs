---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, toformatter]
bead: elm-effect-rs-port-toformatter-3c3
created: 2026-10-15
verify: sh -c "cargo test -p ? && python3 conformance/rows-port-remaining.py packages/effect/test/schema/toFormatter.test.ts"
verify_result: unit-test-verified (sha=5c139ce3239d6bc4407f8bdf9d5239e94dedd104; fence green)
---

# Wave-1 verified solution: port toformatter

The Effect v4 module `toformatter` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-toformatter-3c3` and closed `unit-test-verified` at `5c139ce3239d6bc4407f8bdf9d5239e94dedd104`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p ? && python3 conformance/rows-port-remaining.py packages/effect/test/schema/toFormatter.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
