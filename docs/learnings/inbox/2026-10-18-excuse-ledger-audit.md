---
tier: perishable
tags: [elm-effect-rs, conformance, excuses, audit]
bead: elm-effect-rs-uryb
created: 2026-10-18
verify: sh -c "cargo test -p elm-effect-conformance && cargo test -p elm-effect --test rc_map_excuses && python3 conformance/excuse-audit.py"
verify_result: unit-test-verified (sha=e50df87aef9c138566c32540b8d5991bf6c06ef9; fence green)
---

# The 4,162 excuses were never audited; 371 of them were open work

A `not-applicable` row is a claim that a behavior cannot be represented in Rust.
Nothing checked those claims, so they drifted: three of the four largest clusters
named a blocker that is already in the tree. The audit that found this is now
`conformance/excuse-audit.py`, and its two gates are the cheap half.

## What was wrong

- 26 rows in `schema/toIso.test.ts` carried a duplicated line column.
  `Behavior::load` split with `splitn(6, '\t')`, so an extra field was absorbed
  and every tool read `title = "it"` with the reason glued onto the rust column.
  The ledger test passed the whole time.
- `Schema.test.ts` excused 222 rows with one text: "needs schema AST walk
  (lib.rs lacks visitor)". `SchemaKind::visit` is
  `crates/elm-effect-schema/src/representation.rs:121` and its test was green.
  The rows under that text are BigDecimal, ByteSize, Duration, Class, tagged
  unions, JSON stringify.
- `Request.test.ts` said the Request surface was absent while `Request` and
  `RequestResolver` sit in `crates/elm-effect/src/cache.rs:736` and `:784`.
- `persistence/RateLimiter.test.ts` said there is no persistent RateLimiter
  while `RateLimiter` is `crates/elm-effect-unstable/src/persistence.rs:261`.
- `RcMap.test.ts` said RC internals are not exposed while two of its rows were
  already witnessed by green tests in `crates/elm-effect/tests/stdlib_resources.rs`.

## The rule that came out of it

A reason that describes work to do is not an excuse; it is an open row. A row is
`not-applicable` only when the behavior cannot exist here (JS object model,
codegen, browser reactivity, host transport) or asserts nothing at runtime.
Everything else is `unported`, and `inventory.sh` then stops deriving `matched`
for the module: five modules (Effect, Pool, Request, Schema, persistence/RateLimiter)
stopped claiming parity.

Carrying is still carrying: five RcMap rows flipped to `ported` with real
witnesses, three of them new tests in `crates/elm-effect/tests/rc_map_excuses.rs`.

## Checks worth keeping

- `python3 conformance/excuse-audit.py` gates on row shape and on every
  `crates/<path>.rs:<line>` a reason cites resolving. It reports blanket texts,
  claimed-absent APIs that have a definition, and reasons that name no API.
- `the_declared_behavior_count_matches_the_parsed_rows` pins the `# behaviors N`
  header to the parsed count, so a row cannot vanish quietly.
- `link-integrity.py`'s staleness arm read only `unported`, so it reported 25
  stale entries once every remaining row was excused. An excused row is
  uncarried work too.

## Residual

- 666 reasons name no API or line ("internals not exposed"), on modules that
  exist: Layer 39, Metric 32, Schedule 31, Tracer 31, TxReentrantLock 27.
- The 222-row Schema cluster is open work now, not an excuse; the largest
  single-row-count port campaign left.
- `matched` still has no floor against an excuse-heavy test file: a module
  reads matched with one ported row and twenty-eight excused. A `ported >
  excused` rule would un-match 37 modules, so it is an operator call.
