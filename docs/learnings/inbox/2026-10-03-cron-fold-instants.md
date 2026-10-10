---
tier: perishable
tags: [bd-iqte, cron, dst, wave-3]
bead: elm-effect-rs-epic-elm-effect-port-0c6.15
wave: 3
created: 2026-10-03
verify: cargo test -p elm-effect --test datetime_cron
verify_result: ok (124 passed; re-run exit 0)
---
# Step instants through folds, never re-resolve walls

## Solution
The cron stepper scans in instant space and reads each instant's wall in
the zone. Rebuilding an instant from wall parts (`Compatible`) inside a
DST fold collapses the second occurrence back to the first and the scan
loops the fold forever (Berlin sequence stalled at 20k iterations).
Fold policy mirroring TS `adjustDst`: forward jumps repeated walls by the
drift (once-mode), backward snaps to the first occurrence.

## Companion trap
`*/2` derives day/weekday `and`; `1-31/2` stays OR; a leading bare `*`
voids every comma add (`*,5` is a wildcard). Same value sets, different
flags: read the parser, not the set.

## Principles
foundational-thinking — one instant-correct stepper with TS drift semantics.
prove-it-works — the Berlin fold test caught the loop, not review.
