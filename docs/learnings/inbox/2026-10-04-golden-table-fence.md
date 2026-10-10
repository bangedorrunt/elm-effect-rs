---
tier: perishable
tags: [bd-iqte, journal, wave-9]
bead: elm-effect-rs-epic-elm-effect-port-0c6.42
wave: 9
created: 2026-10-04
verify: cargo test -p elm-effect-journal golden_transitions
verify_result: ok (plus 3/3 fence)
---
# Golden tables get fenced pair-for-pair against the source file

## Solution
`can_transition` as a `const fn` over matches, with a test asserting
every one of the 18 edges plus the key rejections (terminal sinks,
Prepared skips, failure settling, all Parked pairs). Absent stays "no
row", never a variant. Cross-checked against toron-core line by line
before writing; the test then guards drift.

## Principles
prove-it-works — table fenced pair-for-pair against the source file.
foundational-thinking — one enum, Absent is no row.
