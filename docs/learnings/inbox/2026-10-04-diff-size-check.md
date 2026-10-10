---
tier: perishable
tags: [bd-iqte, git-hygiene, wave-5]
bead: elm-effect-rs-misc-residuals-ep7
wave: 5
created: 2026-10-04
verify: cargo test -p elm-effect --test port_misc
verify_result: ok (125 passed)
---
# Check diff size before every commit on a shared tree

## What happened
A commit meant to carry ~100 lines carried ~1800: peer uncommitted work
in the same files rode along under my bead id. `git show --stat` caught
it (969-line graph rewrite I never wrote). Unwound via reset, reversed
my hunks out to isolate peer dirt, re-applied only mine, recommitted
minimal, restored peer dirt uncommitted.

## Rule
On a shared tree, `git show --stat HEAD` after every commit. If a file
moves more lines than you wrote, stop and separate before doing anything
else. Never commit чужое under your bead; never checkout-destroy theirs.
