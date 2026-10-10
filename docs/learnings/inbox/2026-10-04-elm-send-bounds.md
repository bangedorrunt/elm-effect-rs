---
tier: perishable
tags: [bd-iqte, elm, wave-8]
bead: elm-effect-rs-epic-elm-effect-port-0c6.39
wave: 8
created: 2026-10-04
verify: cargo test -p elm-effect-elm --test program
verify_result: ok (6 passed)
---
# Effects at the edge need Send bounds up front

## Solution
`Effect<A, E>` requires `A/E: Send + 'static` at the struct level, so
any trait producing effects (Program Msg, Interpret Error) must carry
those bounds. `Runtime::run` is an associated function; holding a
`Runtime` field for it only earns an unused-field warning.

## Principles
prove-it-works — every deliverable names a running test.
foundational-thinking — one entrance send, effects at the edge.
