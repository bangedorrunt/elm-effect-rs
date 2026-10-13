---
tier: perishable
tags: [git, staging, multi-lane, index, commit]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-13
verify: sh -c "git show --name-only --format= 2959295 | grep -q 'crates/elm-effect-schema/tests/schema_codec.rs'"
verify_result: ok (exit 0; that path was never reserved or added by this lane)
---

# The index is shared, not just the working tree

## The failure

`2959295` was meant to carry three paths: `lib.rs`, the new
`tests/schema_fields.rs`, and 22 rows of `conformance/behavior-tests.tsv`. I
reserved those three, staged those three, verified the tsv carried exactly 22
changed rows, and committed with no pathspec.

`git show --stat` afterwards listed **four** paths. The extra one,
`crates/elm-effect-schema/tests/schema_codec.rs` (143 lines), belongs to the
toCodec lane and was still uncommitted in their working tree. I never added it.
There is one `.git/index` for the whole shared checkout, so a sibling's
`git add` (or `commit -a` on a different path) puts *their* paths into the index
that *my* pathless commit publishes. The guard passed: it answers "may this
agent touch the paths it declared", and a path already sitting in the index is
not a path I declared.

The check I had used earlier in the session, and dropped in that last commit,
is the one that catches it:

```sh
git diff --cached --name-only          # must be exactly your own set, no more
```

In the earlier commits I read `git diff --cached --stat` before committing; this
time I checked only the tsv row count and assumed the rest of the index was mine.

## Why not repair it with a reset

A shared HEAD is not a private branch. `git reset --soft HEAD~1` while a sibling
may be mid-commit can pull the ground out from under them, and the sibling's file
is not damaged by having been committed: the content is theirs, correct, and
their ledger binding is by commit *message*, which does not cite their bead. So
the repair is a note to them plus the check above, not a rewrite.

## What to read before every pathless commit

1. `git diff --cached --name-only` — every path must be one you reserved and one
   you staged.
2. `git diff --cached -- conformance/behavior-tests.tsv | grep -c '^+[^+]'` —
   your row count, when you touch the shared ledger.
3. `git rev-parse HEAD` against the base you staged from (see the sibling
   learning from today for why).

## Principles

- **boundary-discipline** — the index is a shared boundary, exactly like the
  working tree and the ledger file. A pathless commit crosses it.
- **separate-before-serializing-shared-state** — the one index serializes every
  lane's staging, so the staged set has to be read, not assumed.
- **prove-it-works** — the staged name list, read immediately before the commit,
  is the proof that only your work is in it; the guard's pass is not.
