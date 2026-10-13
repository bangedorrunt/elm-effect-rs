---
tier: perishable
tags: [beads, conformance, ledger, git, staging, multi-lane]
bead: elm-effect-rs-port-params-collisions-fz2g
created: 2026-10-13
verify: sh -c "[ \$(git show 460c483 -- conformance/behavior-tests.tsv | grep -c '^[+-][^+-]') -eq 20 ]"
verify_result: ok (exit 0; the working tree held 122 changed lines in that file at the same moment)
---

# Staging a shared file whole carries a sibling lane's uncommitted work

## The failure, twice in one day

Two lanes share `conformance/behavior-tests.tsv` and `.beads/*.jsonl`, and both
files are edited *while you work*: a row flip here, an auto-flush there. `git
add <file>` stages the file, not your rows.

Measured cases:

1. `elm-effect-rs-port-txsubscriptionref-eiif` staged
   `crates/elm-effect/src/lib.rs` whole and carried a sibling's uncommitted
   export line whose definitions lived only in their still-dirty `sync.rs`. That
   revision **did not compile**.
2. `elm-effect-rs-port-params-collisions-fz2g` found 51 rows of another lane's
   in-flight `Queue.test.ts` flips already in the working tree tsv, and another
   lane's uncommitted close (`elm-effect-rs-supervise-preflight-blocked-0e9d`,
   gate rows 85-87) in `.beads/`. Staging either whole publishes work whose
   tests and gate rows are not in the commit.

The pre-commit guard cannot see this: it checks attribution, bead tokens, and
live reservations per path, and both occurrences passed it. It answers "may
this agent touch this path", never "is this staged content yours".

## The recipe

Stage HEAD's content plus only your own rows, and commit the index with no
`-a` and no pathspec, so nothing else can ride:

```sh
git show HEAD:conformance/behavior-tests.tsv > /tmp/head.tsv
# rewrite only your rows in /tmp/head.tsv (same edit you made in the tree)
git update-index --cacheinfo 100644,$(git hash-object -w /tmp/head.tsv),conformance/behavior-tests.tsv
git add <your own new files>
git diff --cached --stat        # read this before committing
```

For an append-only ledger, appending just your own rows to HEAD's copy is the
same move. `git diff --cached` is the check that it worked: 20 changed lines
(10 rows) where the working tree had 122.

## Why not the lazier-looking options

- `git checkout HEAD -- <file>` deletes the sibling's edits.
- `git stash` / `git add -p` / `git restore --staged` move *their* working-tree
  state to solve *your* staging problem. The blob route touches only the index.

## Residual

Your rows stay in the shared working tree after the commit, so a sibling's
later `git commit -a` on that path re-adds them. That is harmless (they are
already in HEAD) but their message must not cite your bead id: one bead id per
commit message, because the ledger binds a bead to a commit by searching
messages, and a message that merely mentions it steals the binding.

A second trap sits in the same commit message: citing the bead id in a
*follow-up* commit moves the ledger binding. After
`elm-effect-rs-port-params-collisions-fz2g` closed legally at `460c483`, the
`chore(beads): close ...` flush (`.beads/` only, message citing the bead) left
`flywheel ledger check` LEGAL, but a later `docs(learnings): ... — <bead-id>`
turned it into "no legal verdict bound to sha <that commit>": the check resolves
the bead to the newest citing revision, and a `.beads/`-only commit is excused
from that. A closed bead has no legal gate transition, so the repair is to stop
citing it (amend the follow-up message) rather than to re-gate.

## Principles

- **boundary-discipline** — a shared file is a boundary. Commit your rows
  across it, never the file, exactly as you would never commit another agent's
  module.
- **prove-it-works** — the staged diff, read before the commit, is the proof
  that only your work is in it; the guard's pass is not.
- **fix-root-causes** — the earlier defect was recorded as "did not compile",
  which is the symptom. The cause is staging a shared path whole, and the same
  cause produced a second, non-compiling-free occurrence here.
