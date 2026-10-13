---
tier: perishable
tags: [schema, multi-lane, leases, toron, module-boundaries, blocked-work]
bead: elm-effect-rs-port-schema-a2c
created: 2026-10-14
verify: sh -c "cd /Users/bangedorrunt/workspace/elm-effect-rs && grep -q 'pub fn to_lowercase' crates/elm-effect-schema/src/transform.rs && ! grep -q 'fn to_lowercase' crates/elm-effect-schema/src/lib.rs"
verify_result: ok (exit 0; the case transformations landed in transform.rs, the crate root never grew them)
---

# When a sibling holds the crate root, grow the module that owns the behavior

## The failure this avoids

A bead needed two named constructors carried (`Schema.test.ts:5613`
`toLowerCase`, `:5627` `toUpperCase`). `crates/elm-effect-schema/src/lib.rs` was
under **another lane's exclusive lease** while that lane was live and working,
and the port had grown its whole surface there. Read naively, the bead is
blocked until the lease frees.

It was not. The pin groups its surface by module and this port mirrors that
split, so each row's behavior has a *home* file, and only some homes were leased.

## What worked

- `toron reserve list --project <p>` before concluding "blocked". BoldKite held
  `src/lib.rs` and one test file exclusively; my shared leases on `src/*.rs`,
  `src/**/*.rs` and `tests/*.rs` were still live, so 19 of the crate's 20 test
  targets and every other source file stayed buildable and committable.
- Ask which module **owns** the behavior, not where the surface happens to be.
  `transform.rs` already declared itself the home of `SchemaTransformation`, so
  the two constructors went there, with the witness next to its siblings in
  `schema_transform.rs`.
- The `rust` column then names a test that exists and is green, and the ledger
  flip queues behind the tsv lease instead of the whole row waiting on it.

Cost: the witness has to reach the module by path (`elm_effect_schema::transform::…`;
`lib.rs` re-exports only `tagged`), and the committer must have a lease on the
file it lands in. Both are cheap.

## The gotcha

Do not invent a shim in an unleased module to dodge the lease: a root-level
`Schema` combinator, a new `check_holds_arg` name, or encode-side check
enforcement genuinely belongs at the crate root, and a pass-through in a sibling
module would be wrong code chosen to make a calendar problem go away. Those rows
wait for the lease. The tell that you are shimming instead of homing is that the
file you are editing has no claim to the concept.

The remaining 374 open rows for this file are mostly in that category: their
behavior is a root combinator or needs a check name this port does not implement
yet, so the unblocked slice was two rows and saying so is the honest report.
