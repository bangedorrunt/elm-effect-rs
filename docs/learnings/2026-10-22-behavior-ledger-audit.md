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

Final state: **6,135 ported, 3,770 not-applicable, 56 skipped-upstream, 9
unported.** The open rows fell 41 → 9 across the six passes; the rows those
passes ported beyond the open count had been excused, not open.
`Effect.ts` is now `matched` (every one of its rows is ported); `Arbitrary.ts`
stays `unmatched` on its two remaining rows; `Channel.ts` came back to `matched`
once its last two rows were carried.

## Second pass: the excuse surface, and the last 24 rows

A second audit checked the excuses themselves, not just their shape. The gate
is now four checks, two of them new and both objective:

- a reason that states a function count for a file must state the current one
  (**47 reasons still said `(21 pub fns)` about `effect.rs`, which has 59**, and
  one said `channel.rs` had 13 where it has 19);
- a reason may not say `Module.name` is absent from the very file it cites:
  **6 claims did** (`Effect.flip`, `Effect.timeoutOrElse`, three
  `Effect.acquireRelease`-era rows that also listed `Effect.uninterruptible`,
  and two rows naming `Effect.ensuring` inside an absence sentence).

Both are now green: the counts are gone from the reasons that carried them,
and the six false absence claims are replaced with the true blocker. Two rows
were re-statused rather than reworded, because their "missing API" was not
missing at all: `Effect.test.ts:1864` and `:1878` were excused for a
`timeoutOrElse` that exists, and the port's `timeout_or_else` **answers its
zero-duration fallback even when the source is already ready**, which is the
row's exact claim. Those two are `unported`, not excused, and the race in
`effect.rs:race_deadline` is the work behind them.

Rows carried in this pass (24):

- `fromJsonSchemaDocument` 2 (enum containing null; root `$ref` intersected
  with `allOf`) — the tests already existed, the rows had never been flipped.
- `Channel` 2 — `catch_cause_filter` was added to `channel.rs` (the port's
  channel error is the typed error, so the selector reads it directly), and
  the second-reason extraction was already asserted.
- `fromJson` 5 — one real fix (`property_name` accepted a number-typed key;
  the pin's `PropertyKey` is a string or a symbol, so it now refuses) and four
  rows whose tests already existed.
- `Effect` 6 — callback sync/async, `forever` interruptibility, callback
  cleanup on interrupt, the deferred interrupt after a restored section, and
  the two throwing-finalizer rows. The last two needed a real change:
  `on_exit` now catches a panicking finalizer and merges the defect into the
  cause (`[Interrupt, Die]`, `Exit.die` on success) instead of letting the
  panic unwind past the caller's `exit()`.
- One excused row (`Effect.test.ts:2274`) has no `AbortSignal` to abort: the
  port's callback gets no signal token, interruption is the fiber flag.

## Third pass: the Effect combinators, and one absence claim that was false

15 more rows are ported: nine that were excused as "absent", four that were open
without a cause, and two that were real runtime bugs. Open rows: 41 → 34. A
fourth pass then took the `toFormatter` group down by eleven more (34 → 23).

`Effect.acquireUseRelease` was the one real gap in this pass, and it is now in
`effect.rs` as `Effect::acquire_use_release`, ported phase by phase rather than
as a wrap: the acquire and the release are masked at the pin and the use is not,
so an interrupt that lands during the acquire still completes it, skips the use,
and then runs the release (row 315). A caller that wraps the whole thing in
`uninterruptible` defers that interrupt past the use instead (row 370). The
release is handed `(resource, &Exit)` like the pin's `(a, exit)`, which is how
the two "use throws" rows read the defect out of a panicking body. It takes the
exit by reference because the effect the callback builds must be `'static`, so a
release that wants the exit must read it while building, not capture it.

Nine rows flipped on that: 315, 342, 370, 2189, 2209, 2899, 2912, 2922, 2935.

**Four `fork` rows were excused for absence that was not absence.** 2598, 2624,
2650 and 2668 were open for `forkDetach`/`forkIn`/`forkScoped` "absent from
effect.rs". All four effect-level constructors are at `effect.rs:1534-1590`, and
their tests (`fork_child_immediate_dies_with_its_parent`,
`fork_detach_outlives_its_parent`, `fork_in_is_interrupted_when_its_scope_closes`,
`fork_scoped_is_interrupted_when_the_ambient_scope_closes`) already existed and
passed. The rows had simply never been flipped. That is the second time this
audit found "absent" written against code that defines the `fn`, and the check
that catches it (`excuse-audit.py`'s absent-but-defined gate) was added after
those rows were written.

**The two timeout rows are closed too, and they were a real bug.**
`race_deadline` polled the child's completion before the deadline, so a ready
source lost a zero-duration tie, and a source that landed just after the
deadline could still win. The deadline is now read first (the source has lost
once it fires), and a value-level source (`Succeed`/`Fail`) answers inline
before the race starts, which is the pin's "a ready source wins the tie" rule
without depending on a poll interval. A `Sync` step is deliberately not in that
fast set: a thunk may block, and a blocking body must still lose to its
deadline, which is what `timeout_or_else_can_die_on_its_own_terms` pins down.

## Fourth pass: the "value domain" diagnosis was wrong

The 32 `toFormatter`/`builtInRevivers` rows were open on a theory: that they
needed new `Unknown` variants because the formatter takes `&Unknown` and can only
receive a JSON value. I started that change — seven variants, `to_json`/
`from_json` arms, formatter arms — and the compiler disagreed with the theory in
the useful direction: four exhaustive matches, and the build was clean again.

What those rows actually needed was smaller, and not a value model at all:

- **constructor formatters.** `Schema::option`, `Schema::result` and
  `Schema::redacted` built no formatter, so `formatter()` fell through to the
  kind walk and printed the wire tag object. They now print `some(...)` /
  `none()`, `success(...)` / `failure(...)` and `<redacted>` /
  `<redacted:label>`, each recursing into the wrapped schema's own formatter.
- **`tuple_of` had no formatter either**, so a tuple position holding an Option
  printed its tag object; each position now renders through its element schema.
- **`optional_key_shape` dropped the wrapped formatter**, so an `optionalKey`
  field rendered as a tag object rather than through its inner schema, and an
  absent one rendered as `null` instead of being left out (the pin prints `{}`).
- **`record2` filtered keys on decode/encode but not when formatting**, so
  `Record(String.check(isPattern("^a")), ...)` printed the keys its own key
  check refuses. Row 334 is that fix.

Eleven rows flipped on that: 218, 225, 290, 307, 328, 334, 497, 557, 563, 582,
587. Their witnesses (`schema_formatter_values.rs`) build the wire value through
the schema's own `try_encode` and then format it, so the claim is end to end
rather than a hand-written fixture.

The general lesson is the one this audit keeps re-learning: "the value model is
too narrow" was a plausible story that nobody had tested, and the four compile
errors that came back were the whole cost of testing it.

## Fifth pass: four constructors and an `undefined`

Six more rows, and this time the missing pieces really were missing, so they were
built rather than worked around:

- **`Schema::readonly_set`, `Schema::readonly_map`, `Schema::hash_map`.** The
  pin builds all three as a declaration whose codec is *linked to an array*
  (`Schema.ts:9142`: decode array, encode the set's values), so an array is the
  shape a formatter ever sees. The port keeps that array as the value and
  attaches the pin's own rendering: `ReadonlySet(0) {}` when empty,
  `ReadonlySet(n) { item, item }` otherwise, `k => v` for map entries. Their
  witnesses format a *recursive* schema, which is what rows 510 and 518 assert:
  `ReadonlySet(1) { ReadonlySet(1) { ReadonlySet(0) {} } }`.
- **`Unknown::Undefined` and `Schema::undefined_or`.** `undefined` is the one
  value the pin has and JSON does not. It rides as its own variant because
  collapsing it into `Null` would make a schema that refuses null accept an
  undefined. `to_json` writes null for it, and the form encoding writes nothing,
  both documented at the site.

Rows: 298, 510, 518, 529, 569, 575.

**The excuse audit caught this pass immediately, which is the point.** Three
reviver rows were excused with "Schema.ReadonlyMap/HashMap/ReadonlySet does not
exist on the Rust schema surface". Adding the constructors made those reasons
false, and `witness-audit.py` failed on them within the same run. They now name
the real blocker, the absent declaration-reviver registry. That is the rot this
audit exists to stop, caught in minutes rather than months.

## Sixth pass: the reviver registry, and a struct that formats its fields

Three passes of work were already sitting in this file as open rows. This pass
built the registry they needed and a formatter that was simply missing.

- **The declaration-reviver registry exists.** `built_in_revivers.rs` still
  refuses a `Declaration` node when reviving without a reviver list, but
  `from_representation_with_declarations` takes the list, and
  `elm_effect_schema::builtin_declaration_revivers()` supplies the schema
  crate's own (`Option`, `Result`, `Redacted`, `ReadonlySet`, `ReadonlyMap`,
  `HashMap`, `Json`). The crate that owns a value type composes its own on top:
  `elm_effect::declaration_revivers()` adds `ByteSize`, `CauseReason`, `Cause`
  and `Exit`. Each reviver checks its arity before building, so a shifted
  `typeParameters` list is refused by id rather than built into the wrong
  schema.
- **A `Field`-based struct had no formatter.** `struct_of` rendered through its
  children, but `struct_shape` fell through to the kind walk, whose `Object` arm
  has no child schemas: a field holding an `Option` printed its tag object. It
  now renders through each declared field, skipping a key the value does not
  carry, which is the pin's `Object.hasOwn` test. That is row 316 (`optional`
  field: a present `undefined` prints as `undefined`, an absent key prints
  nothing) and it also fixes every `pick`/`omit`/`partial` derivation, which
  build through the same constructor.

Rows: 316, 849, 867, 966. The witnesses are `schema_declarations.rs` (a full
convert → `to_json` → `from_json` → revive → convert round trip, not a name
check) and `schema_formatter_values.rs` (format the wire value the schema itself
encoded).

Still open (9), each with its cause rather than a story:

- **2 `toFormatter` rows.** `:635` and `:644` need an `instanceOf`/class model,
  which the port does not have at all.
- **5 `builtInRevivers` rows** revive constructors that do not exist here:
  `Graph`, `Chunk`, `RegExp`, `Date`, `File`. The registry is no longer the
  blocker — the value types are. `Chunk`, `RegExp` and `Date` have parked port
  modules (`data/chunk.rs`, `data/regexp.rs`, `datetime.rs`) that `lib.rs` does
  not export; `Graph` is the whole `graph.rs` surface; `File` has no type in the
  crate at all.
- **2 `Arbitrary` rows** (NaN bounds, infinite interval under `isFinite`) are
  rejected by a test-local predicate in `arbitrary_gen.rs`, not by the port.
  They are back to `unported` on purpose: `witness-audit.py` caught them as a
  ported row whose witness never reaches port code, which is the exact
  tautology this ledger was repaired to stop.

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
- 9 rows are open. That is the honest number, and `matched` is derived from
  it, so `Arbitrary.ts` cannot claim parity while those rows are open.
- 2,615 excuses are narrative: they state a cause but name no API, file or
  line. They were not re-adjudicated here and should not be read as verified.
  What the second pass did verify is the part a file can answer: every cited
  line exists, no reason states a stale function count, and no reason claims
  absent a `fn` that sits in the file it cites.
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
`#[ignore]`d. Two mechanical checks now cover the gap, and the second one now
gates the four things a file can answer:

    python3 conformance/witness-audit.py            # witnesses and excuse claims
    python3 conformance/excuse-audit.py             # shape, citations, stale
                                                    # counts, absent-but-defined

The count and absence gates are the ones that would have caught this rot on
the day it happened: `(21 pub fns)` was true when written, and `Effect.flip`
was absent when that reason was written. Both went false without anyone
touching the reason, which is what a free-text column does when nothing
re-reads it.

Reproduce either against another ledger with `--ledger PATH`.
