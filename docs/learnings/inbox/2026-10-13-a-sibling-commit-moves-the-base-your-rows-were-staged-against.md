---
tier: perishable
tags: [conformance, ledger, git, staging, multi-lane, beads]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-13
verify: sh -c "git log -1 --format=%s 809cab0 | grep -q Duration && git rev-parse --verify -q 035c03f >/dev/null"
verify_result: ok (exit 0; 809cab0 is the sibling commit and 035c03f the commit that followed the rewrite)
---

# A sibling commit moves the base your rows were staged against

## The failure

The staging recipe from `2026-10-13-staging-a-shared-ledger-file-whole-carries-a-sibling-lane.md`
reads HEAD, applies your rows to that copy, and writes the result into the index.
It assumes HEAD does not move between the read and the commit. It can, and it did:

- `git show HEAD:conformance/behavior-tests.tsv > /tmp/head.tsv`, 27 rows applied,
  `git update-index --cacheinfo` — all fine.
- In those 20 seconds a sibling landed `809cab0` (Duration's value semantics, 40
  rows flipped to `ported`).
- `git diff --cached --stat` then read **134 changed lines** where 27 rows were
  expected, and the diff direction was `-ported / +unported`: the index was built
  on the *previous* HEAD, so committing it would have published a revert of
  another lane's 40 finished rows. Their work, invisible in my rows, undone by my
  commit.

The same hazard has a benign twin. The Duration commit had also swept the shared
tsv working tree whole, which is the residual the earlier learning names, so my 27
flipped rows were *already* in HEAD: the staged diff came back empty and a
`set -e` guard expecting 27 aborted the chain instead of committing a no-op.

## The recipe

Read HEAD and commit in the **same shell invocation**, and assert both the row
count and the base hash immediately before the commit:

```sh
BASE=$(git rev-parse HEAD)
git show $BASE:conformance/behavior-tests.tsv > /tmp/base.tsv
# apply only your own rows to /tmp/base.tsv
git update-index --cacheinfo 100644,$(git hash-object -w /tmp/staged.tsv),conformance/behavior-tests.tsv
ADDED=$(git diff --cached -- conformance/behavior-tests.tsv | grep -c '^+[^+]')
test "$ADDED" = "<your row count>"          # 27 rows -> 27 added lines
test "$(git rev-parse HEAD)" = "$BASE"      # nothing landed in between
git commit -m "..."
```

Two readings, two different fixes:

- **More lines than expected, in paths you do not own**: HEAD moved. Re-read it
  and redo the index; do not commit.
- **Zero lines**: check whether your rows are already in HEAD
  (`git show HEAD:<file> | grep <your witness>`) before assuming the staging
  broke. If they are, your flips were swept in by a sibling and there is nothing
  to stage for the ledger — commit only your own source files.

## A second gate on the same commit

`docs/learnings/**` needs a live reservation under your name like every other
path: the strict guard blocked this very learning with `unreserved` until one was
acquired. The message needs no bead token (a knowledge record is not a row flip),
but the path needs the reservation.

## Why not the lazier-looking options

- Trusting the first `git diff --cached` without a base check is what nearly
  published the revert.
- `git add <shared-file>` is the same class of bug in the other direction: it
  stages the sibling's rows as well as yours.

## Principles

- **boundary-discipline** — the shared ledger file is a boundary, and so is the
  index's base. Both have to be checked at the moment of crossing.
- **prove-it-works** — the staged diff, read against a freshly read HEAD, is the
  proof; the guard's pass is not, because the guard answers "may this agent touch
  this path", never "what is this index based on".
- **sequence-verifiable-units** — an empty staged diff for the ledger is a
  legitimate outcome, not an error: another lane publishing your rows does not
  make them less verified, it just moves who commits them.
