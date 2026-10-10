---
tier: perishable
tags: [bd-iqte, observability, wave-12]
bead: elm-effect-rs-epic-elm-effect-port-0c6.62
wave: 12
created: 2026-10-05
verify: cargo test -p elm-effect-unstable --test schema_obs
verify_result: ok (14 passed)
---
# Verify flip counts against the commit, not the plan

## Solution
A ledger commit under time pressure carried 63 flipped rows against a
39-row plan: the extra 24 were verified sibling work with no other home.
`git show --stat` plus per-file row counts before committing would have
shown it immediately. When it happens, say so in the close reason and
the thread instead of rewriting history peers may have pulled.

## Principles
prove-it-works — every flipped row names a test.
sequence-verifiable-units — flips survive inventory regeneration.
