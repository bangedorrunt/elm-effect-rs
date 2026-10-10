---
tier: perishable
tags: [bd-iqte, schema, wave-6]
bead: elm-effect-rs-epic-elm-effect-port-0c6.32
wave: 6
created: 2026-10-04
verify: cargo test -p elm-effect-schema --test schema_transform
verify_result: ok (26 passed)
---
# Tree records need append-aware init; scans need instant anchors

## Solution
`makeTreeRecord` must create arrays (not objects) when the next segment
is an index or `[]`, and duplicate array leaves concatenate. Same-path
scalar duplicates are last-wins. Fold-safe stepping from wave-3 applies
unchanged to cron-over-zone logic; wall re-resolves collapse second
occurrences and loop.

## Principles
prove-it-works — TS append/nested cases ported verbatim, failures read first.
sequence-verifiable-units — flips survive inventory regeneration.
