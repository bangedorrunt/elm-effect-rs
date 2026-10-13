---
tier: perishable
tags: [conformance, ledger, excuses, rows, case-sensitive]
bead: elm-effect-rs-9zv1
created: 2026-10-13
verify: python3 conformance/rows-port-remaining.py packages/effect/test/http/FindMyWay/case-insensitive.test.ts
verify_result: ok (exit 0; "all 12 rows ported or excused", after 3 rows left `not-applicable`)
---

# A `not-applicable` row is a claim about the port, and it expires

## The case

`elm-effect-rs-port-regex-jxf8` excused three rows of
`http/FindMyWay/case-insensitive.test.ts` (lines 79, 90, 101) with the reason
"this port's FindMyWay has no case-insensitive mode". That was **true when
written** and **false one bead later**: the Router slice has carried a real
config since `elm-effect-rs-4gxr`, and the pin's own default is
`caseSensitive: false`, so the excuse described a capability gap that had
closed. Flipping the three rows needed a config field and a case-folding
comparison, not a rewrite.

## Why it is not a one-off

The ledger holds **1136 `not-applicable` rows, and 990 of them justify
themselves with a phrase that says a capability is missing** ("no …", "not
carried", "unsupported", "does not …"). That 990 is a *word* count, not 990
proofs: it flags the rows whose excuse page is a capability claim, and those are
exactly the rows that expire when the capability arrives. The other 146 rest on
something that does not move (a row asserting a JS-only object identity, a
`WeakRef` liveness check).

So the repair is a habit, not a script: **when a bead adds a capability, re-read
the excused rows that named its absence** before calling the module done. This
bead was that re-read for one file; a module-sized one has the same shape.

## What the pin actually does with the flag

Worth writing down, because "case-insensitive" is two different rules that look
like one:

- `internal/router.ts:250` lowercases the **pattern** before the duplicate
  check, so `/foo` and `/FOO` are one route, and `:148` lowercases the static
  node paths it builds.
- `find` lowercases the **path** only when it is not "clean" (`:319`, clean
  means no uppercase, `%`, `?`, `;`, `#`, and no `//`), and it slices captures
  out of `originPath` (`:~380`).

Match on folded text, report captures as written. A router that lowercases the
request path and captures from it passes the "folds" rows and silently
lowercases every parameter value.

## A small trap that cost a compile

`cargo clippy` reports `unused import: match_path` **by line**, and the file had
six byte-identical `use ...::{match_path, match_path_ci};` lines, of which only
five were unused. I read the warning list through `head -80`, mapped five
warnings onto six identical strings, and patched by string: two of the tests
lost an import they call. One `.rs` line each, caught by the next `cargo test`.

Patch a warning **at its line number**, never by matching the text it quotes,
and do not truncate the list you are patching from.

## Principles

- **boundary-discipline** — an excused row is a boundary claim about the port's
  surface. It is not a statement about the pin, and it is not permanent.
- **prove-it-works** — the excuse was believed for one bead because nobody
  re-ran it; the row's own state is the only authority, and re-running it is
  what turned a "cannot be represented here" into a five-test config change.
