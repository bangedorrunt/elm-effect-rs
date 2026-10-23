# Excuse clusters: what the 3670 not-applicable rows are waiting on

`python3 conformance/excuse-audit.py` reports two shapes worth acting on: 74
blanket reasons covering 1531 rows, and 2568 reasons that name no API, file or
line. This brief dispositions the clusters that can be named by their reason, so
the next unit can pick one instead of re-reading the ledger.

## How the counts were taken

One pass over `conformance/behavior-tests.tsv`, matching reason substrings; a
row lands in the first cluster that matches. Counts are rows, not tests. The
clusters below cover 766 rows. The remaining 2904 carry per-row reasons that
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

## B. Absent substrate: one enabler each (443 rows)

| Cluster | Rows | Missing surface | Rust owner |
|---|---|---|---|
| Effectful `SchemaParser` | 56 | Parser is Effect-driven (Deferred/Fiber/Cause); the port has only sync adapters | `crates/elm-effect/src/schema_parser.rs` |
| `Config.schema` materialization | 57 | No `Config.schema` constructor, so schema nodes are never built from provider data | `crates/elm-effect/src/config.rs` |
| Atom surface | 60 | `Atom.fn`/idleTTL/batching (33) and `Atom.swr`/refresh (27); the port has the lifecycle core only | `crates/elm-effect/src/atom.rs` (absent) |
| HTTP client/api/server | 80 | Transport, discovery, handler/middleware wiring | `crates/elm-effect/src/platform/` |
| MCP transport | 54 | stdio/HTTP/SSE/session/batch; only the in-memory pair exists | `crates/elm-effect/src/platform/` |
| Metric observers | 32 | Observer/reporter internals beyond the metric surface | `crates/elm-effect/src/metric.rs` |
| Schedule policy | 31 | `concat`, `tap`, `modify_delay`, `add_delay`, `concat_result`, cron; `max`/`min` exist | `crates/elm-effect/src/schedule.rs` |
| Random PRNG | 25 | Split/unsafe PRNG internals | `crates/elm-effect/src/misc/` |
| Arbitrary pattern strings | 25 | Constructive generator from a regex pattern | `crates/elm-effect-unstable/src/arbitrary.rs` |
| ExecutionPlan telemetry | 23 | Attempt/event telemetry beyond the attempt budget | `crates/elm-effect/src/execution_plan.rs` |

Disposition: each is a machinery bead when scheduled, with a dependency on its
enabler; none may carry a wave label until then. `Schedule` is the cheapest
first unit: pure data, no I/O, and half the surface already exists.

## C. Coarse blanket: rows that are portable but excused with their siblings

The test is whether the reason describes what the row asserts. Two texts failed
it on `TxQueue.test.ts` and were swept in `elm-effect-rs-vejn`: the "one-shot
iterable across retries" text also covered plain drain rows, and the "typed
error" text also covered item-count and state rows. Sixteen rows were read
individually; eleven ported, five re-excused with their own blocker (error
channel or parking).

`Schedule.test.ts` (31 rows, counted in B) has the same tell: its blanket says
the schedule surface does not represent the rows, while `max`, `min`, `recurs`
and `during` already exist. Split it before scheduling: portable-with-existing
surface versus the six missing combinators.

## Next step

Sweep `Schedule.test.ts` first: it is the one cluster where part of the work is
a ledger edit, not new machinery. Then cut the `Metric` enabler, the smallest
substrate bead in B.
