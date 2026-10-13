---
tier: perishable
tags: [conformance, ledger, inventory, linking, phase-13]
bead: elm-effect-rs-unlinked-effects-tests-ezqw
created: 2026-10-12
verify: cargo test -p elm-effect-conformance --test ledger
verify_result: ok (2 passed, 1 ignored)
---

# A test file's name is not evidence of its module, and at this pin its imports are not either

## Solution

88 of the 214 test files with open rows link to no module row, carrying 1983
open rows. Three measured facts, each of which kills the obvious widening:

**1. The name predicts a path that does not exist.** 85 of 88 have no module
row at the path their name predicts, and where one exists the names disagree:
`test/cli/Arguments.test.ts` (plural) against the row
`packages/effect/src/cli/Argument.ts` (singular). The cli inventory is
complete (13 non-internal source files at the pin, 13 rows), so this is a
naming gap, not a missing row. A matcher that chases near-miss names is a
rename guess wearing a match's clothes.

**2. No import names a source module.** 0 of the 88 import anything matching
`/src/`. 46 import test helpers only (`../TestUtils/...`, `./McpConformance.ts`)
and 42 import package barrels only (`effect`, `effect/testing`,
`@effect/vitest`). Reading imports therefore links test files to helpers,
never to modules. Verify with: for each unlinked path, grep its `from "..."`,
then require a match on `/src/`.

**3. The barrel route is symbol resolution, not string work.** The barrel-only
files name their subject as a symbol: `test/schema/*` uses `Schema` (23
files), `SchemaParser` (10), `SchemaGetter` (10), plus `SchemaTransformation`,
`SchemaJITCompiler`, `VariantSchema`. Rows exist for `src/Schema.ts`,
`src/schema/SchemaJITCompiler.ts` and 139 top-level `src` modules, so the
targets are there, but mapping symbol to defining module means walking the
barrel's re-export map, which `inventory.sh` does not do today.

## Which way out

It is a ledger policy call, not a bugfix, because the options trade different
gate semantics: resolve symbols in the generator (provable, but relinks many
files and needs the red-then-green discipline of the earlier linking rounds);
link only the subset whose name does match a row (cheap, leaves the
`Schema.test.ts` family unlinked); or exclude package-level test files from the
matched/unmatched derivation (honest to what these files are, biggest change to
the phase-13 gate).

## Principles

- **prove-it-works** - a link made on a name that nearly matches is green for
  the wrong reason; the counterexample is `Arguments.test.ts`, which is not
  derived from `Argument.ts` by any rule, only by a rename someone approved.
- **dont-let-a-fallback-look-like-a-win** - the same trap as the basename
  fallback: a wider rule that claims a module turns the ledger greener without
  making the mapping true.

## Resolved (2026-10-13)

The owner took the middle option, narrowed to what the pin's paths can prove:
the name and the import must agree. `inventory.sh` gains a third route, a test
that sits in a directory named after a module it imports by that same path
(`test/ai/McpServer/**` with `from "effect/ai/McpServer"`), which links 8 of the
88. The other 80 are recorded in `conformance/unowned-tests.tsv` with the
reason each fails the route, and `conformance/link-integrity.py` now fails when
an open-row file is in neither set, when a listed file becomes linked, or when
a listed file stops carrying open rows.

Fact 2 above needs correcting: none of the 88 imports a literal `/src/` path,
which is what that check grepped for, but every one names a module through a
package subpath. Reading imports is evidence of what a test touches, never of
which module it is about, which is why the route needs the name to agree.

## Related

`2026-10-10-conformance-linking-and-loop-dispatch.md` covered the first round
(path agreement, the `FooTest.ts` convention, the non-stealing basename
fallback). This note is the next layer: the files for which no path-level rule
can be evidence at all.
