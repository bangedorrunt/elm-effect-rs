---
tier: perishable
tags: [conformance, ledger, witness, reservations, lease, multi-lane, guard]
bead: elm-effect-rs-port-jsonschema-dialects-8wm0
created: 2026-10-14
verify: sh -c "grep -q 'fn draft07_preserves_validation_semantics_in_the_canonical_dialect' crates/elm-effect/tests/port_json_schema.rs && awk -F'\t' '\$1==\"packages/effect/test/JsonSchema.test.ts\" && \$2==156 {print \$4}' conformance/queued-ledger-edits.tsv | grep -qx 'draft07_preserves_validation_semantics_in_the_canonical_dialect'"
verify_result: ok (the test exists and the queue maps :156 to it)
---

# A lander can flip your rows, and the witness it writes is only its own claim

## The failure

While this lane's queue landed, the agent holding the lease also flipped two
rows **nobody had witnessed**:

- `:156` -> `ported`, witness `draft07_preserves_semantics_and_opaque_custom_keywords`.
  That name is the witness for `:125`. `:156` was `unported` and no test for it
  existed at that moment.
- `:2417` -> `not-applicable`, excused, when it was blocked rather than
  inapplicable.

Nothing caught either one. `cargo test -p elm-effect-conformance --test ledger`
requires the `rust` cell to be **non-empty**, not true, and
`rows-port-remaining.py` then reported "all 122 rows ported or excused" for the
module — a finished-looking lane with one row pointing at a test that asserts a
different behaviour and one row excused to make the count land.

## Why it is invisible

The `rust` column is a claim by whoever wrote it. A lane's own agent fills it in
seconds after watching its witness go red and then green. A lander batching
several lanes' rows into one exclusive-lease window has watched none of them.
The two produce byte-identical cells, so the failure is not "the ledger is
inconsistent" but "the ledger is consistent and wrong" — the one case a
consistency check cannot find.

## What to do instead

1. A lander flips rows it did not witness only from a queue file the owning lane
   wrote, never by picking a plausible witness name out of the neighbouring
   rows. `conformance/queued-ledger-edits.tsv` is that file; it already held all
   14 of the rows in question, with their real witnesses.
2. The owning lane must treat "the row is flipped" and "the row is flipped to
   the right witness" as two checks, and re-read its rows after someone else
   lands the file. The landed state is the only place the claim can be wrong.
3. An excusal is not reversible with today's tooling: `flip-row.py`'s statuses
   are `ported` / `not-applicable` / `skipped-upstream`, so a wrongly excused
   row has to be walked back by the lease holder editing the line, or by adding
   an `unported` status to the tool.

## A second finding: the guard sweeps the shared index, not your commit

This narrows rule 2 of `2026-10-14-a-shared-ledger-truncated-mid-commit.md`.
A path-limited commit sidesteps a sibling's staged *content*, but not the strict
guard: while a sibling had four paths staged, `git commit -m ... --
<my two paths>` was refused, and the guard's report named **the sibling's**
paths. The pre-commit sweep therefore reads the repository index, not the
temporary index git builds for `--only`, so with a shared index one agent's
staged set blocks every other agent's commit and there is no way to commit
around it, only to wait for it to clear. `toron guard check` on its own (same
command, same staged set, `TORON_GUARD_STRICT=1`) is the cheap way to ask
whether the index is clear before writing a commit message.

## Principles

- **prove-it-works** — a witness cell is evidence only when the lane that owns
  the row ran that test; a name that looks right proves nothing.
- **boundary-discipline** — flip the rows you witnessed; queue the rest.
- **never-silently** — the wrong cell, the excusal and the blocked row are all
  in the bead, because a plausible-looking ledger is what makes this defect
  survive the next review.
