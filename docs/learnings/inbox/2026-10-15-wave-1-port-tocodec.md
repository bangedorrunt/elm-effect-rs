---
tier: perishable
tags: [bd-iqte, elm-effect-rs, wave-1, tocodec]
bead: elm-effect-rs-port-tocodec-ker
created: 2026-10-15
verify: sh -c "cargo test -p elm-effect-schema && python3 conformance/rows-port-remaining.py packages/effect/test/schema/toCodec.test.ts"
verify_result: unit-test-verified (sha=8fb3cbc7a895539c860391eb9e9deeab6ae939ca; fence green)
---

# Wave-1 verified solution: port tocodec

The Effect v4 module `tocodec` (from `packages/effect/test/...`) was carried into the Rust surface under bead `elm-effect-rs-port-tocodec-ker` and closed `unit-test-verified` at `8fb3cbc7a895539c860391eb9e9deeab6ae939ca`.

## What was verified

- `VERIFY` fence ran green before commit (`sh -c "cargo test -p elm-effect-schema && python3 conformance/rows-port-remaining.py packages/effect/test/schema/toCodec.test.ts"`).
- Behavior ledger rows for the module were flipped to `ported` with real Rust test paths; `cargo test -p elm-effect-conformance --test ledger` remained green.
- No mass `not-applicable` shortcut was taken.

## Scope note

This entry records only the verified close; any surprising mechanism discovered in the port is described in its commit body, not restated here.
