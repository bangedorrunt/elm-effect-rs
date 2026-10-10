---
tier: perishable
tags: [bd-iqte, journal, wave-9]
bead: elm-effect-rs-epic-elm-effect-port-0c6.47
wave: 9
created: 2026-10-04
verify: cargo test -p elm-effect-journal stale_epoch_is_fenced
verify_result: ok (plus 4/4 fence)
---
# Fence at the append edge; dedupe before staleness

## Solution
Same-triple dedupe must be checked before sequence staleness: a repeat
of an older triple is a duplicate (original offset), only unseen older
sequences are stale. New epochs reset the baseline unconditionally.
No TTL field exists on purpose.

## Principles
prove-it-works — all fence names run.
foundational-thinking — fencing at the append edge.
