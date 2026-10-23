# Excuse clusters: what the 3670 not-applicable rows are waiting on

`python3 conformance/excuse-audit.py` reports two shapes worth acting on: 74
blanket reasons covering 1531 rows, and 2568 reasons that name no API, file or
line. This brief dispositions the clusters that can be named by their reason, so
the next unit can pick one instead of re-reading the ledger.

## How the counts were taken

One pass over `conformance/behavior-tests.tsv`, matching reason substrings; a
row lands in the first cluster that matches. Counts are rows, not tests. The
clusters below cover 686 rows. The remaining 2938 carry per-row reasons that
this pass did not re-adjudicate.

## A. Terminal: no Rust change can make the row observable (323 rows)

| Cluster | Rows | Why it is terminal |
|---|---|---|
| JS codegen / JIT emit | 227 | `toCodeDocument` (116) and its annotations file (29) assert the emitted JavaScript text; `SchemaJITCompiler` (38) asserts a generated checker. There is no emitter to produce text from. |
| `Effect.fnUntracedEager` | 41 | The row asserts a JS eager-evaluation optimization on the effect's `_tag`/`value` shape. Effects here are values evaluated by the runtime. |
| JS `Map`/`Set` equality | 24 | The row compares JS instances entry-by-entry across insertion order. No Rust `Map`/`Set` instance implements `Equal` for these rows to travel through. |
| `Function` dual/HKT surface | 31 | The rows exercise JS data-first/data-last dispatch and arity/HKT typing. Porting them would import a calling convention the Rust API does not use. |

Disposition: keep excused. No bead. If one of these ever needs revisiting, the
reason is a design boundary, not a missing module.

## B. Absent substrate: one enabler each (363 rows)

| Cluster | Rows | Missing surface | Rust owner |
|---|---|---|---|
| Effectful `SchemaParser` | 56 | Parser is Effect-driven (Deferred/Fiber/Cause); the port has only sync adapters | `crates/elm-effect/src/schema_parser.rs` |
| `Config.schema` materialization | 57 | No `Config.schema` constructor, so schema nodes are never built from provider data | `crates/elm-effect/src/config.rs` |
| Atom surface | 60 | `Atom.fn`/idleTTL/batching (33) and `Atom.swr`/refresh (27); the port has the lifecycle core only | `crates/elm-effect-unstable/src/reactivity.rs` |
| HTTP client/api/server | 80 | Transport, discovery, handler/middleware wiring | `crates/elm-effect-unstable/src/http.rs` |
| MCP transport | 54 | stdio/HTTP/SSE/session/batch; only the in-memory pair exists | `crates/elm-effect-unstable/src/mcp_server.rs` |
| Schedule policy | 31 | no clock inside the step, so `duration`/`during`/cron/window alignment are unobservable; no `toStep` metadata; no per-run state for `concat`/`concatResult` | `crates/elm-effect/src/schedule.rs` |
| Arbitrary pattern strings | 25 | Constructive generator from a regex pattern | `crates/elm-effect-unstable/src/arbitrary.rs` |

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

`Metric.test.ts` was the fifth and is half done: 12 of 32 rows are value math
and now ported (`Frequency`, `Histogram`, `Summary`, `linear_boundaries` in
`crates/elm-effect/tests/metric_instruments.rs`). The other 20 are not
instruments at all, they are runtime hooks: 12 drive `track`/
`trackErrors`/`trackDefects`, 3 measure `trackDuration`, and 5 need interval
collection, a dump, or fiber metrics. That is a runtime unit, not a metric one.

`ExecutionPlan.test.ts` was the sixth: 22 of its 23 rows are runtime plan
machinery (step layers, requirement capture, partial streams, observer events),
but the zero-attempts row only needed `NoAttempts` to carry the pin's message.
One row ported, 22 re-excused by mechanism.

## Next step

`Arbitrary` (25 rows) is the smallest cluster left in B, but it needs a pattern
parser. `Config.schema` (57) is the better value: `config.rs` already exists and
the rows only need the schema constructor wired to the provider bridge.
