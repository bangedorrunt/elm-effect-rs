---
tier: perishable
tags: [schema, fmt, git, multi-lane, rustfmt, staging]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-13
verify: sh -c "cd /Users/bangedorrunt/workspace/elm-effect-rs && [ \"$(git show HEAD:crates/elm-effect-schema/tests/schema_fields.rs | rustfmt --edition 2024 --emit stdout | md5 -q)\" != \"$(md5 -q crates/elm-effect-schema/tests/schema_fields.rs)\" ]"
verify_result: ok (exit 0; HEAD's committed file is not what this toolchain's rustfmt emits, so `cargo fmt -p elm-effect-schema` rewrites it, and twelve more like it)
---

# Running `cargo fmt -p` in a shared tree rewrites other lanes' files

## The failure

`cargo fmt -p elm-effect-schema`, one command, dirtied **15 files**. Only three of
them held content: `src/lib.rs` and `tests/schema_suspend.rs` (mine, in flight)
and `tests/schema_corpus.rs` (BoldKite's, uncommitted). The other twelve were
pure rustfmt re-wraps of code nobody had touched.

The reason is not a broken toolchain: this repo's committed style was written by
a different rustfmt, and there is no fmt gate to hold it. `cargo fmt --all
--check` already fails on `HEAD` (`crates/elm-effect/src/cause.rs:212`), so the
churn buys nothing at all, and it lands in a tree that three lanes share.

The pre-commit guard cannot save you here either: it checks attribution, bead
tokens and live reservations per path, and formatting touches no semantics.

## The check that separates churn from content

Before touching anything, hash what the formatter would produce from `HEAD`:

```sh
git show HEAD:<file> | rustfmt --edition 2024 --emit stdout | md5 -q
md5 -q <file>
```

Equal hashes mean the file's whole diff is rustfmt's output, so
`git checkout HEAD -- <file>` is provably lossless: there is no one's work in
that diff to delete. Unequal means the file holds someone's edits and the diff
must be read, not reverted.

Measured on the 15: twelve equal (reverted), three unequal (two mine, one
BoldKite's).

## The recipe actually used

1. `git checkout HEAD --` the twelve churn-only files.
2. For the two files with my own hunks: `git checkout HEAD -- <file>` and
   re-apply the hunks by hand. Fighting the formatter hunk by hunk costs more
   than retyping a 30-line change, and it cannot lose anyone's content.
3. For the sibling's file: leave it, and tell them by mail which lines are mine.
   Reverting it would have destroyed their uncommitted tests, and their own
   `git add` of that path carries the whitespace into their commit; that is
   recoverable, a deleted half-written test is not.

Committing the intended change after the revert also shrank: `src/lib.rs` went
from 138 changed lines (rustfmt'd) to 44 (the hunk alone).

## Principles

- **boundary-discipline** — a shared working tree is a boundary, and a formatter
  is a writer. A repo-wide formatter is the same class of action as
  `git add <shared-file>`: it touches paths you do not own.
- **prove-it-works** — the `rustfmt(HEAD)` hash is the proof that the revert
  deletes nothing. It is one command per file and it replaces judgement.
- **fix-root-causes** — the defect is not "one file got re-wrapped". It is
  running a repo-wide writer in a tree with three live lanes, and no fmt gate to
  make it worth doing. Format only the file you are about to commit, if at all.
