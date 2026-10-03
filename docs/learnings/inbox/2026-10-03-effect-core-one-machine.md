---
tier: perishable
tags: [bd-iqte, kernel, effect-core, wave-1]
bead: elm-effect-rs-epic-elm-effect-port-0c6.3
wave: 1
created: 2026-10-03
verify: cargo test -p elm-effect description_drop_runs_nothing
verify_result: ok (15/15 across seams; re-run 2026-10-03 exit 0)
---
# One private state machine interprets effects; Exit stays data

## Solution
Effect core: single private state machine as the one interpretation place;
`Exit<A, E>` is ordinary data, not an effect. 15/15 tests green (7 at
run_sync seam + 5 l2 + 3 l3), no-default-features green, clippy/fmt clean,
zero `unsafe`.

## Why it matters for later wave-1 beads
Fiber fork/join/interrupt (.4) and scope beads build on this seam. A second
interpretation path would split semantics silently. Route new operators
through the one machine or say why in the bead.

## Principles
foundational-thinking — private state machine is the one interpretation place.
prove-it-works — RED E0583 before GREEN, receipt filed.
