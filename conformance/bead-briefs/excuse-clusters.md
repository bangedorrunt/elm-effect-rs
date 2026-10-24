# Excuse clusters: what the 3608 not-applicable rows are waiting on

`python3 conformance/excuse-audit.py` reports two shapes worth acting on: 66
blanket reasons covering 1250 rows, and 402 reasons that name no API, file or
line. The numbers move as rows are swept; re-run the tool, do not trust these. This brief dispositions the clusters that can be named by their reason, so
the next unit can pick one instead of re-reading the ledger.

## How the counts were taken

One pass over `conformance/behavior-tests.tsv`, matching reason substrings; a
row lands in the first cluster that matches. Counts are rows, not tests. The
clusters below cover 639 rows. The remaining 2812 carry per-row reasons that
this pass did not re-adjudicate.

## A. Terminal: no Rust change can make the row observable (290 rows)

| Cluster | Rows | Why it is terminal |
|---|---|---|
| JS codegen / JIT emit | 227 | `toCodeDocument` (116) and its annotations file (29) assert the emitted JavaScript text; `SchemaJITCompiler` (38) asserts a generated checker. There is no emitter to produce text from. |
| `Effect.fnUntracedEager` | 41 | The row asserts a JS eager-evaluation optimization on the effect's `_tag`/`value` shape. Effects here are values evaluated by the runtime. |
| JS `Map`/`Set` equality | 11 | The rows that need two distinct-but-equal object keys in one collection, a `NaN` key or member (`f64` has no `Ord`), a homogeneous set holding both a set and a string, or a cycle. The other 13 ported once `Equal`/`Hash` covered `BTreeMap` and `BTreeSet`. |
| `Function` dual surface | 11 | Ten rows need `dual`'s runtime arity dispatch (one constructor serving both call orders, plus the `RangeError` and predicate forms). The eleventh is `absurd`, whose panic arm cannot run because `Infallible` is uninhabited. The other 20 rows ported; see C. |

Disposition: keep excused. No bead. If one of these ever needs revisiting, the
reason is a design boundary, not a missing module.

## D. Containers: rows a std collection instance reaches

The eighth sweep was `Equal.test.ts`'s JavaScript `Map`/`Set` block: 24 rows
excused for want of a container instance. The port's `Equal` now covers
`BTreeMap` and `BTreeSet` (entry-by-entry, one-to-one, order-independent), so
13 of the 24 are ported in `crates/elm-effect/tests/equal_containers.rs`. The
other 11 are excused by representability, listed in A.

## B. Absent substrate: one enabler each (360 rows)

| Cluster | Rows | Missing surface | Rust owner |
|---|---|---|---|
| Effectful `SchemaParser` | 45 | The concurrency, interruption, middleware, and mixed-cause rows need `Schema<T>` to carry an effect step. The port resolves each step before attaching it, so `requires_effect` marks a marker, not a runnable step. Enabler: a schema redesign, not the parser file | `crates/elm-effect-schema/src/lib.rs` |
| `Config.schema` materialization | 54 | No `Config.schema` constructor, so schema nodes are never built from provider data | `crates/elm-effect/src/config.rs` |
| Atom surface | 60 | `Atom.fn`/idleTTL/batching (33) and `Atom.swr`/refresh (27); the port has the lifecycle core only | `crates/elm-effect-unstable/src/reactivity.rs` |
| HTTP client/api/server | 80 | Transport, discovery, handler/middleware wiring | `crates/elm-effect-unstable/src/http.rs` |
| MCP transport | 54 | stdio/HTTP/SSE/session/batch; only the in-memory pair exists | `crates/elm-effect-unstable/src/mcp_server.rs` |
| ~~Schedule policy~~ | ~~31~~ 0 | **Ported** in `elm-effect-rs-wb8g`: a schedule now owns its own metadata counters (`attempt`/`start`/`previous`, the pin's `metadataFn`), steps are timestamped, and `Decision::Done` carries the pin's done value. `during`, `duration`, `cron`, `windowed`, `fixed`'s window alignment, `concat`/`concatResult`, `tap`/`modifyDelay`/`addDelay`, `while`, and `upTo(options)` all exist. All 23 remaining rows are in `crates/elm-effect/tests/schedule.rs`. | `crates/elm-effect/src/schedule.rs` |

Disposition: each is a machinery bead when scheduled, with a dependency on its
enabler; none may carry a wave label until then. `Schedule` looked cheapest and
was not (see C); the remaining clusters each need runtime or transport work.

## C. Coarse blanket: rows that are portable but excused with their siblings

The test is whether the reason describes what the row asserts. Two texts failed
it on `TxQueue.test.ts` and were swept in `elm-effect-rs-vejn`: the "one-shot
iterable across retries" text also covered plain drain rows, and the "typed
error" text also covered item-count and state rows. Sixteen rows were read
individually; eleven ported, five re-excused with their own blocker (error
channel or parking).

`Schedule.test.ts` (31 rows) was the third: its blanket said the surface did
not represent the rows, and 8 of the 31 were portable once the missing
constructors existed. They are now ported (`fixed`, `once`, `identity`,
`up_to_times`, and `jitter` moved to the pin's 80%-120% band); the other 23 are
excused by cause: 15 need a clock in the step, 3 need `toStep` metadata, 4 need
per-run state for sequencing, and 1 is the `upTo({})` options record.

The lesson from three sweeps: a blanket reason is trustworthy only when it
names the mechanism the row needs. "The surface does not represent it" is not a
mechanism.

`Random.test.ts` was the fourth and cleanest: 25 rows, all portable. The port's
`Random` grew `next_boolean`, `next_int`, `next_between`, both `next_int_between`
forms, `shuffle`, `choice` with the pin's `NoSuchElement` message, string seeds,
and a `from_draws` source that stands in for the pin's injected service; all 25
are ported in `crates/elm-effect/tests/random.rs`.

`Metric.test.ts` was the fifth and is now finished but for four rows: value
math went to `crates/elm-effect/tests/metric_instruments.rs`, and the exit
hooks are ported as `Effect::track`/`track_errors`/`track_defects`/
`track_duration` over a `Metrics` handle in
`crates/elm-effect/tests/metric_track.rs` (16 rows, including `trackDuration`
against a `TestClock` and its monotonic reading). The hook shape differs from
the pin in one way only: the registry is an explicit handle, because a
description here has no fiber to own one. `Metric.dump` landed as
`MetricRegistry::dump` (a padded table per series, sorted by id then attribute
text because a hash map has no insertion order to inherit). The three left need
an interval collector, fiber runtime metrics, and an OTLP exporter.

`ExecutionPlan.test.ts` was the sixth: 22 of its 23 rows are runtime plan
machinery (step layers, requirement capture, partial streams, observer events),
but the zero-attempts row only needed `NoAttempts` to carry the pin's message.
One row ported, 22 re-excused by mechanism.

`Config.test.ts` was the seventh and a correction: its 57-row `Config.schema`
blanket is genuinely a bridge (no schema constructor, no StringTree
materialization), but 3 of the 57 rows were misfiled under it. `Config.option`,
`Config.option` over a group, and `Config.LogLevel`'s default-versus-invalid
rule need no schema at all; they are ported in
`crates/elm-effect/tests/config_absence.rs`. The other 54 keep the bridge
reason, which is accurate.

`Arbitrary.test.ts` was the eighth and the one that needed machinery rather
than re-reading: 25 rows wanted a pattern parser and a constructive generator.
`elm-effect-schema/src/pattern.rs` now parses the ECMAScript subset, matches,
analyses reachable UTF-16 lengths, constructs a value at a chosen length, and
shrinks structurally. 23 rows are ported in
`crates/elm-effect-unstable/tests/arbitrary_pattern.rs`. Two stay excused for a
declaration the derivation cannot see: `Schema.RegExp` reports the generic
`Object` kind (no `source`/`flags` fields to construct), and `Schema.URL` has
no declaration arbitrary at all.

`Function.test.ts` was the ninth: 31 rows under one "arity/HKT/cache parity"
text. Twenty were portable once the surface existed: `apply`, `const_unit`
(both `constUndefined` and `constVoid`), `compose` as a `pipe` step, the
`pipe!`/`flow!` macros, `hole`, `sk`, `memoize`, and `memoize_idempotent`, all
witnessed in `crates/elm-effect/tests/function_module.rs` and each proven red
under a stubbed implementation. Eleven stay excused by mechanism: ten `dual`
rows need runtime arity dispatch, and `absurd` has no caller because
`Infallible` is uninhabited.

`SchemaParser.test.ts` was the tenth: 56 rows under "no effectful parser".
Eleven were adapters that only needed the facade, and they are witnessed in
`crates/elm-effect/tests/schema_parser_adapters.rs`: `make` and `makeOption`
report a mixed constructor cause, the sync adapter refuses an issue, the effect
adapter fails with the issue and is eager for a sync schema, and a `Suspend`
factory stays lazy, memoized, mutually recursive, and retried after a failure.
The effectful engine exists now: `Schema::effect_codec` carries both
directions of a step, `resolve` walks the step graph under the parse's
concurrency limit, and the effect adapter reports the whole cause. A second
pass read the eleven mixed-cause rows individually, and all eleven are ported in
`crates/elm-effect/tests/schema_parser_mixed_causes.rs`: the sync, promise,
option, result, guard and assertion refusals, plus the union rule that a
candidate which died is not one that refused. The guard and assertion adapters
are `type_guard` (the pin's `SchemaParser.is`, spelled out because `is` is a
Rust keyword) and `asserts`. The rest of the file's rows are excused by their
own named mechanism (prototype chain, missing sentinel, lazy compilation).

## Next step

No cheap cluster is left. `Config.schema` is **done** (the bridge in
`crates/elm-effect/src/config.rs`: `schema_wires` walks the provider into the
codec's wire shape, `has_schema_input` decides absence versus refusal; 166 of
its 169 rows are ported and the last three are terminal: module copies, an
ambient provider service, and a `Result` arm on `Value`). `Schedule` is done
too. What remains: `Atom`/`MCP`/`HTTP` need runtime or transport work. Pick
one substrate and build it end to end.

A claim-absent API that resolves is usually a name collision, not a stale
excuse: `Channel.forever` resolves to an effect combinator, `MutableList.Empty`
to a schema declaration, and `Effect.fromNullishOr` to `Array.from_nullish_or`.
Read the cited line before re-opening a row.
