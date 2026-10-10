---
tier: perishable
tags: [br, close-ceremony, gates]
bead: elm-effect-rs-scoped-release-order-zbj
created: 2026-10-10
verify: flywheel ledger check elm-effect-rs-scoped-release-order-zbj --dir .
verify_result: ok (LEGAL on sha 6fb36ca)
---
# A superseded close still needs a commit whose message cites the bead

## Solution
Closing a bead as superseded looks like a paperwork close: the work landed
in someone else's commit, so the natural move is to gate-report against
that commit's sha. The guard refuses it.

Measured this session: `br close elm-effect-rs-scoped-release-order-zbj
--commit-sha 608a386` (the commit that ported the two rows the bead names)
failed with "commit 608a386... does not cite
elm-effect-rs-scoped-release-order-zbj (its message names no such bead;
the close sha must cite the bead being closed)". The gate row itself was
accepted; only the close's sha binding was rejected.

The working order, then:

1. Commit something that CITES the bead in its message. For a supersede
   that is the `.beads` state itself: `chore(beads): record the supersede
   of <id>`.
2. `br gate report <id> --gate command-verified --status pass --to closed
   --note "sha=<that sha> receipt=<path> superseded by <other sha>"`.
3. `br close <id> --commit-sha <that sha>`.

Cite only the bead being closed. A message carrying two bead ids binds
both, and the earlier lesson says a later commit that merely mentions a
bead steals its ledger binding.

## Principles
sequence-verifiable-units — the close sha must be the commit that answers
for this bead, even when that answer is "already done elsewhere".
boundary-discipline — name the bead you are closing, and only it.
