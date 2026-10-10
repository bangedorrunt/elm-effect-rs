---
tier: perishable
tags: [bd-iqte, journal, wave-9]
bead: elm-effect-rs-epic-elm-effect-port-0c6.48
wave: 9
created: 2026-10-04
verify: cargo test -p elm-effect-journal --test durable_log_gone_fork
verify_result: ok (6 passed)
---
# Gone has reasons; the cursor is not an offset

## Solution
Retention-Gone and soft-delete-Gone are separate variants callers match
explicitly; a skip is never a delete. Snapshots sit beside the stream as
{model, offset}. `collapsing_cursor` stays an HTTP-only token and is
never stored: resume reads `offset`, full stop.

## Principles
prove-it-works — all six fence names run.
foundational-thinking — two Gone reasons never conflated.
