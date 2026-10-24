# Minimalism audit: elm-effect-rs framework surface

Date: 2026-10-24 (updated after the cut landed). Bead: `elm-effect-rs-9sw5` (audit), `elm-effect-rs-rali`, `elm-effect-rs-qqxu`, `elm-effect-rs-9j8k`, `elm-effect-rs-t5nx` (implementation). Method: `reduce` + `effect-oriented` law card, cross-checked against ADR-0002 (the parity law) and ADR-0003 (the adoption law, status `proposed`). Evidence is repo-local; no sibling repository was read.

**Outcome: the workspace went from 240,488 LOC / 8 crates to 65,250 LOC / 6 crates (-73%).** `elm-effect-schema` and `elm-effect-unstable` are removed outright (operator decision: no extraction), the Effect-TS standard-library ports are gone, and the conformance ledger is at kernel scope. Every one of the five vision elements below survives.

## 0) Scope + assumptions

- In scope: the workspace crates, `conformance/`, `docs/guides/`, `skills/elm-effect`, workspace dependency weight.
- Out of scope: flywheel, toron, beads, chiebukuro. **No adoption record for any consumer exists in this repo**, so the cut carried no compatibility debt.
- Constraints discovered:
  - ADR-0002 defines the mission as "100% feature and test parity" with `effect@4.0.0`. That mission generated the deleted bulk.
  - ADR-0003 (status `proposed`) already demanded the opposite shape: "Version 1.0 must name a small stable surface", 1.0 = "the kernel, `policy`, `Scope`, and `TestClock`".
  - The durable streams concept (DurableLog, L16, `elm-effect-journal`) is product vision and is kept. A future durable-function/execution integration (Temporal-style) is anticipated: the deterministic fold, the journal's settle-once states, and `attempt_plan.rs` (`AttemptPlan` as a value) are the seam, and that seam is preserved.

## 1) The vision preserved (the invariant)

1. **Carrier**: `Effect<A, E, R>` as a value, four outcomes apart (`Cause`/`Exit`), interpreted only at the edge (`Runtime`, `Fiber`, `Scope`).
2. **Elm/Redux fold**: `Program` + `Cmd` + `Interpret` + replay (`elm-effect-elm`): pure `update`, commands as data.
3. **TypeScript Effect idioms**: `ServiceTag` + `Layer`/`provide` (typed `R`), `Schedule`/`RemotePolicy`, scoped acquire/release, laws L1–L16.
4. **Extensibility on demand**: applications add values (a service, a layer, a schedule, a program, a log); they never fork the runtime.
5. **Easy integration**: small semver surface, `Result`/`Future`/`tracing` doors, two mandatory deps (`thiserror`, optional `tokio`).

## 2) What was cut, and where it went

| Block | Size (src+tests) | Verdict | Rationale |
| --- | --- | --- | --- |
| `elm-effect-schema` crate + core schema stack (`schema_*`, `channel_schema`, `json_schema`, gated `Config` decode) | 27.3k + 5.2k + 2.2k | deleted | `@effect/schema` exists in TypeScript because TS has no serde; the law card says parse once at the boundary |
| `elm-effect-unstable` crate (http/mcp/openapi/sql/cluster/workers/ai/cli/completions/eventlog/rpc/yaml...) | 68.9k | deleted | an application platform; ADR-0002's own non-goal is "a web framework... or a replacement for herdr"; it also violated the crate map with unconditional `clap`/`flate2`/`sha2`/`base64` |
| Effect-TS standard-library ports (`data/`, `stdlib/`, `containers/`, `platform/`, `misc/`, plus `graph`, `optic`, `predicate`, `partition`, `json*`, `datetime`, `cron`, `filesystem`, `file`, `path`, `platform_error`, `cache`, `request`, `formatter`, `utils_gen`, `basics`, `scheduler`) | ~20k + ~8-10k test-only | deleted | Rust std/itertools/serde/jiff own these; ADR-0002: "we do not port the polyfill" |
| `otlp_tracer.rs` (hand-rolled OTLP wire exporter) | 614 | deleted | ADR-0003: the tracing door "does not replace it" |
| Duplicate ports (`misc/graph.rs` vs `graph.rs`, `misc/json_patch.rs`, `misc/json_pointer.rs`) | 743 | deleted (P0) | two Rust ports of one upstream module; the `misc/` copies were dead files on disk |
| Second lint surface (`elm-effect-elm/src/lints.rs`) | ~90 + tests | merged (P0) | one lint crate (`elm-effect-lints`, 4 rules) |

## 3) What was kept, and why (the keep-list corrections)

- **Kernel**: `effect`, `cause`, `exit`, `foundation`, `fiber*`, `runtime`, `scope`, `service`, `layer`, `context`, `tagged`, `schedule`, `policy`, `sync` (L9), `concurrency`, `supervision`, `supervisor`, `executor`, `lib`. `foundation.rs` is the one port-adjacent survivor: `stm` keys on its `Order`, and `Equal`/`Order`/`Hash`/`Function`/`Combiner` are carried.
- **Durable-execution seam**: `attempt_plan.rs` (`AttemptPlan` as a value), `elm-effect-elm` (deterministic fold, replay), `elm-effect-journal` (`DurableLog`, settle-once states). This is where a Temporal-style durable-function layer would sit: as a `Layer`/program over values, not a kernel fork.
- **Services kept** (the substitution surface, effect-oriented principle 8): `time.rs` (Clock/L8 + `TestClock`), `crypto.rs` (Random service + `TestCrypto`), `config` + `config_provider` + `config_effect` (startup read), `console` (Logger), `mock` (`Layer.mock`).
- **Observation law kept**: `metric`, `tracer`, `observability` (principle 11: logs/traces/metrics observe an effect). Only the OTLP wire exporter went.
- **Streams kept**: `stream`, `sink`, `channel`, `pubsub`, `pull`, `take`, `subscription_ref` (principle 14), behind `tokio`.
- **STM kept** behind the `stm` feature (unchanged posture from ADR-0003).
- Cut despite earlier keep-candidates: `chunk`/`nonempty`/`byte_size` (islands; no surviving consumer), `Schedule::cron` + `cron.rs` + `datetime.rs` (dropped `jiff`), `Duration::from_input*` (kept `from_input_string`).

## 4) Ledger outcome (kernel scope)

`conformance/behavior-tests.tsv` (rows are never deleted; the inventory is generated):

| Status | Count | Meaning now |
| --- | --- | --- |
| `ported` | 1,507 | every claim has a live witness that reaches surviving port code |
| `not-applicable` | 8,298 | 3,322 pre-existing + 4,976 reasoned cut excuses (per-family reasons naming the Rust owner) |
| `unported` | 109 | **coverage gap**: carried behaviors (Config/Duration/Order/Function/Equivalence/Combiner/Hash/Equal/Reducer) whose witnesses died with the port tests |
| `skipped-upstream` | 56 | unchanged |

`modules.toml`: 143 modules `matched` → `excused` (every linked row excused), 9 → `unmatched` (rows left unported). `inventory.sh` preserves status/reason on regen. 162 rotten reason citations (into deleted files) were repaired in place, keeping the curated per-row narratives.

Report-only (pre-existing, deferred): 2,464 narrative reasons name no API or line; 5 absence-claims name colliding definitions.

## 5) Verification (all green)

- `cargo test --workspace` — 91 suites, 1,166 tests, 0 failures.
- `sh conformance/witness.sh` — exit 0 (product finish line: the seven law witnesses).
- `python3 conformance/witness-audit.py` — OK (every ported witness exists and reaches port code).
- `python3 conformance/excuse-audit.py` — OK (shape and citation gates hold).
- `cargo test -p elm-effect-conformance` — OK (all four ledger invariants).
- `cargo test -p elm-effect-elm --test guide_imports` — OK (the guides' `use` lines compile).

## 6) Final shape

- `elm-effect` — kernel + services + streams + stm, 28,444 src (was 56,936).
- `elm-effect-elm` — the fold, 1,258 src.
- `elm-effect-journal` — DurableLog, 1,747 src.
- `elm-effect-test` — deterministic executor/TestClock, 277 src.
- `elm-effect-lints` — 4 rules, 404 src.
- `elm-effect-conformance` — ledger gate, 292 src.
- Consumer weight: `thiserror` + optional `tokio` (was `jiff`, `num-bigint`, `num-traits` mandatory).

## 7) Risks and follow-ups

- The 109 `unported` rows are a real coverage gap on carried modules; restoring witnesses is follow-up work, recorded honestly rather than papered over with excuses.
- ADR-0002 still describes the parity mission as open. The ledger now records the shrink, but the record itself should be amended (an ADR-0005 rescoping parity to kernel/streams/journal) by the operator.
- `modules.toml` edits are regen-safe (status/reason preserved), but a regen from the upstream tag will restore the `class` fields of cut modules (`port`); the `excused` status still holds the ledger gates.
- Narrative-reason cleanup (2,464 rows) and the 5 absence-claim name collisions are report-only debt in the audits.
- One flaky first run of the gate battery showed a transient test failure in the build-cache layer that did not reproduce; the clean re-run exits 0.
