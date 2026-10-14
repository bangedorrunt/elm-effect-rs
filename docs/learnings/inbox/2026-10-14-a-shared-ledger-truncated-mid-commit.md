---
tier: perishable
tags: [git, ledger, index, conformance, multi-lane, recovery]
bead: elm-effect-rs-port-jsonschema-dialects-8wm0
created: 2026-10-14
verify: sh -c "wc -l < conformance/behavior-tests.tsv | grep -q '^9973$' && git show HEAD:conformance/behavior-tests.tsv | awk -F'\t' '\$2==\"packages/effect/test/JsonSchema.test.ts\" && \$1==\"ported\"' | wc -l | grep -q '^60$'"
verify_result: ok (9973 rows on disk, 60 of them this lane's, matching the committed ledger)
---

# A shared ledger can be truncated mid-commit, and the index is the only copy left

## The failure

`conformance/behavior-tests.tsv` is one file every lane edits. I flipped 60
rows with `flip-row.py`, staged the file, and committed. The commit printed the
guard's JSON and reported success, but `git log -1` showed a *sibling's*
commit, and my own commit was nowhere:

- my paths were still staged, so the commit never happened;
- `COMMIT_EXIT=0` was a lie, because the command was
  `git commit ... 2>&1 | tail -3` and `$?` was `tail`'s status.

Then the worse discovery: the ledger on disk was **0 bytes**, mtime 15:29:52,
while HEAD's copy was 9973 lines. A concurrent writer had truncated the shared
file between my `git add` and my commit. Anyone else's uncommitted flips to that
file were gone with it, and I could not tell whose.

## Why the index saved it

`git add` had copied the file's content into `.git/index` a few seconds
earlier, so the staged blob was the newest good copy in existence:

```sh
git show :conformance/behavior-tests.tsv | wc -l      # 9973, with my 60 rows
git show HEAD:conformance/behavior-tests.tsv | wc -l  # 9973, without them
git checkout-index -f -- conformance/behavior-tests.tsv
```

The restore is safe to reason about because the delta against HEAD can be
printed before anything is written:

```sh
diff <(git show HEAD:conformance/behavior-tests.tsv) conformance/behavior-tests.tsv \
  | grep -E '^[<>]' | grep -v JsonSchema.test.js   # must be only your rows
```

Exactly 60 removed and 60 added lines, all mine, so the restore neither invented
nor dropped a sibling's row. Note `git checkout` (from HEAD) would have thrown my
60 rows away; the index, not HEAD, is the recovery source.

## The two rules this adds

1. **Never read a commit's exit status through a pipe.** `cmd | tail` reports
   `tail`. Capture `rc=$?` on the commit itself, or write the message to a file
   and commit bare. A guard's JSON on stdout is not proof the commit landed;
   `git log -1 --format=%h` is.
2. **Commit with an explicit pathspec when the index is shared.**
   `git commit -F msg -- <paths>` publishes the working-tree content of those
   paths and sidesteps whatever a sibling staged or committed in the meantime.
   The pathless form trusts an index nobody owns.

## Principles

- **separate-before-serializing-shared-state** — one ledger, one index, many
  writers; the staged copy is a snapshot, so it is also the fallback.
- **prove-it-works** — the row count plus the diff against HEAD is the proof of
  a clean restore; "the file is non-empty again" is not.
- **boundary-discipline** — a path-limited commit is the fence that keeps this
  lane's bytes inside this lane's commit.
