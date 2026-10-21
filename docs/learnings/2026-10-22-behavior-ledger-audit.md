---
title: Behavior ledger audit and repair, 2026-10-22
status: repaired
---

# The behavior ledger was an inventory, not a completion certificate

`conformance/behavior-tests.tsv` covers the `effect@4.0.0` pin. Before this
audit it read **6,076 ported, 3,838 not-applicable, 56 skipped-upstream, 0
unported**. Zero open rows was a claim about the port, not a fact about it.

The audit (below) found claims in both directions: rows marked done that no
test asserted, and rows excused on reasons the code had since invalidated. All
155 rows it named are repaired.

Final state: **6,077 ported, 3,781 not-applicable, 56 skipped-upstream, 56
unported.** Three modules dropped from `matched` to `unmatched`, because their
linked rows now include open work: `Arbitrary.ts`, `Channel.ts`, `Effect.ts`.

## What was falsified

**Ported rows no test proved (32).** 30 cited a witness that does not exist.
Five `Duration` rows cited `max_ms` and friends, which are `pub fn`s in
`crates/elm-effect/src/basics.rs`, not tests. Eight `Channel` rows carried
labels (`channel_unwrap_reason`) instead of test names; one of the eight,
*extracts second reason type*, had no group test that asserted it and was
demoted. Eleven `fromJsonSchemaDocument` rows named
`json_schema_document_literal_refinement_rows_carry_their_code_and_type`; the
test has no "literal". The last two, `Arbitrary.test.ts:1627` and `:1817`,
pointed at a file-local helper that re-implements the contract in plain float
math, so the witness passed without the crate.

Separately, `TxHashMap.test.ts:57` had a witness that existed but read `"a"`
and never a missing key. The missing read was added to `tx_hash_map_crud`
rather than demoting the row.

A retraction belongs here. `array_std_substrate`
(`crates/elm-effect/tests/port_structures.rs:99`) opens with a page of
`vec![1, 2] == vec![1, 2]` assertions, and it was reported as a tautology
during this audit. It is not: from `array::take_right` onward the same test
calls the port, and 29 of the 141 `Array.test.ts` rows have their own real
witnesses besides. The 112 rows on that group witness stay `ported`; they are
weak evidence, not false evidence.

**Excused rows on a premise the code contradicts (123).**

- `fromJson.test.ts` (23) said "no fromJson representation decoder".
  `crates/elm-effect-schema/src/representation_doc.rs:292` is that decoder,
  and `schema_from_representation.rs` tests it. Eight rows had a direct
  counterpart and became `ported`: *decodes Void* → `revives_void`, *Never*,
  *ObjectKeyword*, *Reference*, *Suspend*, *Filter*, *FilterGroup*, and
  *requires representation on persisted Filters* →
  `requires_representation_when_persisting_a_filter`.
- `Effect.test.ts` (12) said `onInterrupt`, `forever`, `callback`,
  `uninterruptibleMask` are "absent from `effect.rs`". They are at :1808,
  :1555, :1140, :1577.
- `builtInRevivers.test.ts` (12) said `Schema.Option` / `Result` / `Redacted` /
  `Cause` / `Exit` do not exist and that there is no reviver registry. `Schema::option`
  is `lib.rs:4638`, `::result` :4746, `::redacted` :7734, and
  `built_in_revivers.rs` is the registry.
- `SchemaParser.test.ts` (56) said "not implemented as a separate parser".
  `crates/elm-effect/src/schema_parser.rs` exists with its own tests.
- `toFormatter.test.ts` (20) said `Schema.Option` / `Result` / `Redacted`
  do not exist. They do.

**The excuse surface.** 80 blanket reasons covered 1,681 rows; 2,613 reasons
still name no API, no file, no line. 83 test files have zero ported rows.

## What the repair did

155 rows, each decided by evidence:

- **33 → `ported`** with the witness that actually asserts the row. Nine of
  them are the `literal refinements` group (`fromJsonSchemaDocument.test.ts`
  1807-1851): the test
  `json_schema_document_literal_refinements_filter_through_both_orders` was
  extended with the minLength/maxLength/pattern/integer/multipleOf/min/max/
  exclusive rows, so the claim is now true rather than merely renamed.
- **56 → `unported`**: a real gap, not an impossibility. These carry no text.
  They are the twenty `toFormatter` rows that need Option/Result rendering
  (`to_formatter` exists at `representation.rs:257`, the element formatters
  do not), the twelve `Effect` rows whose excuse named combinators that now
  exist, the twelve `builtInRevivers` rows (registry exists, node kinds
  missing), six `fromJson` rows with no validation path, the two `Arbitrary`
  rows, the two `Channel` rows, and the two `fromJsonSchemaDocument` rows.
- **66 excuses rewritten** to the true cause, status unchanged:
  - `SchemaParser` (56): "no effectful SchemaParser: `schema_parser.rs`
    carries only the sync `decodeUnknownExit`/`Result` adapters, and the pin's
    parser is Effect-driven".
  - `fromJson` (9): the genuine domain gaps (`undefined`, symbol and bigint
    have no `Unknown` variant; template parts fold to an opaque marker;
    Declaration revival needs the pin's JS predicate; `toJSON` is a JS
    protocol hook).
  - `Graph.test.ts:250` (1): node indexes are `usize`, so the pin's
    `MAX_SAFE_INTEGER` exhaustion boundary has no Rust counterpart.

## What is still true, and what is still weak

- Every `ported` row now names a test that exists and reaches port code
  (`python3 conformance/witness-audit.py` exits 0).
- 56 rows are open. That is the honest number, and `matched` is derived from
  it, so the three demoted modules cannot claim parity while those rows are
  open.
- 2,613 excuses are narrative: they state a cause but name no API, file or
  line. They were not re-adjudicated here and should not be read as verified.
- A group witness is still one test standing behind many rows. The audit
  proves the test exists and reaches port code; it cannot prove every row's
  assertion is in it. `TxHashMap.test.ts:57` was that failure mode.
- The remaining excuse blocks (JS codegen, no JS object model, the effectful
  parser, the absence of `Schema.Date`/`URL`/`RegExp` constructors) still rest
  on one reading per file, and a later change to the crates can falsify them
  again. `witness-audit.py` is the check that catches the API-landed case.

## Why it rotted

Nothing re-read a reason. `flip-row.py` refuses a `ported` row with no
witness, but the column is free text, so a stale witness name passes every
gate the repo ran. `ledger.rs` checks shape only, and the phase-13 gate is
`#[ignore]`d. Two mechanical checks now cover the gap:

    python3 conformance/witness-audit.py            # witnesses and excuse claims
    python3 conformance/excuse-audit.py             # shape, citations, blankets

Reproduce either against another ledger with `--ledger PATH`.
