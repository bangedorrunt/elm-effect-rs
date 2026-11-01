---
status: proposed
date: 2026-11-01
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: ADR-0002, ADR-0003, ADR-0004, docs/research/resonate-inventory.md, Resonate / Temporal / Rivet public docs (design reference)
informed: agents implementing elm-effect-rs and the three plane migrations
---

# Durable engine port plus our own single-host engine

> Carrier law stays [ADR-0002](0002-port-effect-4-into-rust-with-an-elm-loop.md).
> This record does not copy it. Point at L14 (journal), L16 (`DurableLog`),
> and the journal crate note that toron implements the store trait for
> `WorkflowEffectStore` (ADR-0002 around the journal section).

> **Amended 2026-11-01.** Cron *timing* moves into this framework as a pure
> 5-field evaluator (`CronSchedule`, `next_after_ms`,
> `latest_at_or_before_ms`, `fire_key`). Cron registration, pause/resume,
> catch-up policy, and the tick loop stay plane-side. Where this record says
> "cron stays plane-side", read that as registration and operations, not the
> timing kernel.

> **Amended 2026-11-01.** There will be no Resonate, Temporal, or Rivet
> adapter crates. Our engine replaces the vendor; those systems are design
> references, not compatibility targets. The conformance suite is the boundary
> for any future engine, not for an adapter layer.

## Context and Problem Statement

Three planes pin three different Resonate SDKs (crates.io 0.6 in flywheel and
toron-workflow; a git `main` tip in chiebukuro). They use Resonate for durable
run start, durable sleep/park, wake, cancel, and status query. Cron timing is a
small pure computation; cron registration, `resonate serve`, `/health`, groups,
and promise dashboards are plane ops around that core (see
[resonate-inventory.md](../research/resonate-inventory.md)).

ADR-0002 already rejected "Resonate as the carrier" and kept Resonate as an
engine adapter. The inventory shows the surface we actually need is small. We
ship our own single-host engine behind that surface and drop the vendor from
the framework path. We borrow the failure-mode lessons from Resonate, Temporal,
and Rivet; we do not build adapter crates for them.

How does a plane schedule a durable run without naming a vendor? How does the
framework stay free of Resonate, Temporal, and Rivet dependencies?

## Decision Drivers

* ADR-0002: Resonate was an adapter, not the carrier; ADR-0005 replaces it. No
  thunk-style middleware that starts work; journal and `DurableLog`
  vocabularies already exist.
* Inventory: only scheduling verbs and cron timing are shared; cron
  registration, server process, and dashboards are not.
* Single host (mikasa) today; multi-host and a hosted dashboard are not
  required.
* Drop the vendor without rewriting the pure plane reducers; the effect entry
  points move to the port and the plane's own commands.
* No regressions: planes keep their Resonate pins until each adoption record
  cuts them; this repo adds the port and default engine only.

## Considered Options

* Keep Resonate as the only engine; pin one SDK version in every plane.
* Write our own single-host engine with a small scheduling port, borrow the
  vendor failure-mode lessons, and retire the vendor; no adapter crates.
* Adopt Temporal (or Rivet) now as the shared engine.

## Decision Outcome

Chosen option: "Durable engine port plus our own single-host engine."

### Port (scheduling verbs only)

Crate `elm-effect-durable` defines `DurableEngine`:

| Verb | Meaning |
| --- | --- |
| `start` | Begin or re-attach a run. The run key **is** the journal key, which **is** the `DurableLog` `producer_id`. |
| `park_until` | Ask to wait until an absolute `until_ms`. Decision is `plan_park` from `elm-effect-journal` (overdue → resume; over ceiling → typed refuse; never a silent shorter sleep). |
| `wake` | Resume a parked run (scheduler or host). |
| `cancel` | Terminal cancel. |
| `query` | Read run status. |

No third vocabulary. Journal states (`Prepared` … `Parked` …) and `DurableLog`
offsets stay where ADR-0002 put them. The engine does not invent a parallel
`PromiseState` enum. A run whose outcome is `Unknown` refuses `park_until` and
`wake` with `EngineError::Uncertain`; reconcile or cancel it first.

### Default engine (ours)

* Single-host. In-memory storage ships in the crate. A storage trait lets a
  host persist rows; **prefer** toron implementing that trait on
  `WorkflowEffectStore` (ADR-0002 journal section) rather than opening SQLite
  inside this framework.
* Storage and journal are separate seams. `EngineStorage` persists runs and
  parks; `JournalStore` persists intent / claim / settle. `with_storage` binds
  the former and `with_journal` binds the latter, so a host can inject the
  journal it already persists. `commit_run` and `commit_park` are the
  transactional hooks; a host with one transaction overrides them so a crash
  cannot leave a run and its park disagreeing.
* Recovery is journal-first. A recorded `Prepared` intent is completed by
  `start`; a recorded `InFlight` row re-attaches; a `Running` storage row with
  no journal intent is re-dispatched under the same key (the provider
  idempotency key is the run key). `Unknown` cannot park or wake.
* A terminal settle or a claim projects onto the run row, so `query` and the
  park set do not go stale. The drain clears orphaned parks instead of
  aborting.
* Pure scheduler core: given parked rows and `now_ms`, which keys are due. It
  **reuses** `plan_park` and `plan_attempt`; it does not reimplement them.
* Clock is injected. Tests use `elm-effect`'s `TestClock` / a plain `now_ms`
  function — never `tokio::time::sleep` inside the decision path.
* A small runtime loop turns due wakes into Msgs for the Elm fold. I/O stays
  at the boundary.

### Framework fit

The engine is a synchronous kernel, like `DurableLog`, not a second runtime.

* Pure decisions are functions: `due_wakes`, `plan_park`, `plan_attempt`,
  `CronSchedule`.
* Time is an input: the host reads its `Clock` / `TestClock` at the edge and
  passes `now_ms`; the engine never sleeps or reads the clock.
* I/O is a trait: `EngineStorage` and `JournalStore`; a host can inject a
  persisted journal and override the transactional commit hooks.
* A host lifts a verb into an `Effect` and provides the engine through its own
  `ServiceTag`. `EngineMsg` is data for the Elm `Msg`; the run key is the
  durable command key and the `DurableLog` `producer_id`.
* Retries, timeouts, and remote policy stay in the host's `Interpret` /
  `plan_attempt` layer. The engine has no fiber, timer, server, or second
  scheduler.

### Cron timing (ours)

Cron **registration** stays plane-side: which jobs exist, pause/resume,
catch-up policy, and the loop that executes a firing. The **timing kernel**
ships here so each plane does not fork a parser:

* `CronSchedule::parse` accepts 5-field vixie cron, lists, ranges, steps, and
  month / weekday names. UTC, minute resolution; no timezone database.
* `next_after_ms` is strictly after; `latest_at_or_before_ms(last, now)` is
  the one-tick catch-up after downtime.
* `fire_key(name, tick_ms)` is the run key. The host calls `start` with it, so
  the same firing re-attaches (`Already`) instead of dispatching twice. The
  tick is part of the key, so two overlapping firings are distinct runs;
  overlap policy (skip, buffer one, cancel the previous) is the host's choice,
  not a new engine verb.
* The evaluator jumps by day, hour, and minute bit-scan; a sparse schedule
  (for example February 29) does not scan every minute of the horizon.
* No clock, no I/O, no scheduler loop. The host passes `now_ms` and runs the
  tick loop at its edge.

These are pure functions, not a second scheduler and not a port verb.

### No vendor adapters

This record does not add `elm-effect-resonate`, `-temporal`, or `-rivet`
crates, and no later adoption is planned. Our engine is the engine. Vendor
history formats (Temporal event history, Rivet actor state, Resonate promise
log) are not a compatibility target; we borrow the failure-mode lessons
(journal as history, fencing generation, leases, one active park per key,
overlap as policy) and keep our own vocabulary.

If a future requirement genuinely needs a vendor, that is a new decision and a
migration, not an adapter hidden behind this port. Any future engine must pass
the conformance suite below; this record adds none.

### Conformance suite

Shipped with the engine (in `elm-effect-durable` tests and/or
`elm-effect-test` helpers). Every engine, including ours, must prove:

1. Replay never runs an effect twice (same key / attempt).
2. `Unknown` has no default exit.
3. Park is not terminal.
4. Wake on time; overdue wake reports the gap; over-ceiling park is refused.
5. Cancel is terminal.
6. A stale generation (or epoch) holder cannot settle.
7. Parking the same key to the same instant creates one row.
8. Restart recovery: a fresh engine on the same storage wakes pending parks.

The suite also pins the hardening cases: a terminal settle projects to
`query` and clears the park; a claim projects the new generation; `Unknown`
refuses park and wake but still cancels; the drain clears orphaned parks; a
`Prepared` intent is completed by `start`; a `Running` row with no journal
intent is re-dispatched under the same key; recovery re-seeds park
generations; re-parking a different instant keeps one park; a host-injected
journal re-attaches, including a journal-only run; a live host lease
re-attaches without being stolen; and a terminal journal row wins over stale
storage.

### Migration order

One plane at a time, each through its own adoption record (ADR-0003 style):

1. **toron-workflow** (owns the journal store).
2. **flywheel**.
3. **chiebukuro** (git SDK pin; cron + task UI; highest risk).

This repository does not edit those planes. Planes keep Resonate until their
record cuts it.

### Plane adoption checklist

A plane is ready to cut its Resonate pin when:

1. Its store implements `EngineStorage` and `JournalStore`, or it injects a
   persisted journal with `with_journal`. A transactional store overrides
   `commit_run` / `commit_park`.
2. Durable functions are rewritten as the plane's commands and Msgs. External
   approval and spawn waits map to `wake` / `cancel` under the same run key.
3. Cron registration calls `CronSchedule` + `fire_key` + `start`; the plane's
   own loop drives ticks and chooses the overlap policy.
4. It deletes `resonate serve`, `/health`, groups, the SDK pin, and the remote
   URL. Its task UI reads the plane's store.
5. Its store round-trip and atomicity tests pass, and the engine conformance
   suite still passes on the default store.

The proc-macro durable functions are the bulk of the cutover. The port
schedules runs; it does not provide the Resonate function macro or `ctx` API.

### What we give up

* Multi-host durable execution.
* A vendor-hosted dashboard and search attributes / actor placement.
* Local-time / DST cron. Timing is UTC and has no timezone database; a host
  that needs local time converts at its edge.
* A persistent journal by default. The in-memory journal survives no process
  restart; a host injects a persisted `JournalStore` with `with_journal`, and a
  `Running` row with no journal intent is re-dispatched under the same key.
* Vendor history import. A plane cutover starts fresh runs; we do not read
  Resonate promise logs, Temporal event histories, or Rivet actor state.

Acceptable for the current single-host stack. If we outgrow one host, a new
decision chooses the next engine; the port and conformance suite are the
boundary.

## Consequences

* Good: one engine, no vendor pin; planes speak journal + port.
* Good: one cron timing kernel; planes keep their own registration tables.
* Good: the journal is a host seam; persistent history is an injection, not a
  fork of the engine.
* Good: conformance suite makes the engine boundary testable; there is no
  adapter layer to maintain.
* Bad: we maintain a small scheduler, a storage trait, and a cron parser.
* Neutral: the default journal is in-memory; a single-process host and the
  conformance suite use it, a durable host injects its own.
* Neutral: no vendor history import; a cutover starts fresh runs.
* Neutral: cron registration, `/health`, and promise UIs stay plane-side until
  deleted.
* Neutral: cron timing is UTC only; a local-time host converts at the edge.
* Neutral: status stays `proposed` until operator sign-off; the
  agent-authority section below authorizes implementation meanwhile
  (ADR-0001).

## Agent authority

While status is `proposed`, an agent **may**:

* Add crate `elm-effect-durable` with `DurableEngine`, pure scheduler,
  storage trait + in-memory impl, injected clock, and wake→Msg loop.
* Add the conformance suite and run it against our engine.
* Add the pure cron timing kernel (`CronSchedule`, `next_after_ms`,
  `latest_at_or_before_ms`, `fire_key`) and its tests.
* Point workspace `Cargo.toml` / root `AGENTS.md` at the new crate.
* Amend this record under `## More Information` with verification evidence.

An agent **must not**:

* Add Resonate, Temporal, or Rivet dependencies to this repository.
* Edit flywheel, toron, beads, or chiebukuro in the name of this record.
* Invent a third lifecycle vocabulary or a second scheduler inside
  `elm-effect` itself.
* Add a cron crate dependency or a scheduler loop inside the framework.
* Add an adapter crate for a vendor engine.
* Open toron's SQLite from this crate; storage is a trait.

## Implementation Plan

* **Affected paths**: `docs/decisions/0005-…`, `docs/decisions/README.md`,
  `crates/elm-effect-durable/`, `docs/guides/15-use-the-apis.mdx`,
  `crates/elm-effect-elm/tests/guide_imports.rs`, workspace `Cargo.toml`,
  root `AGENTS.md`, conformance and cron tests.
* **Dependencies**: `elm-effect`, `elm-effect-journal`, `thiserror`; no vendor
  SDKs and no cron crate.
* **Patterns to follow**: ADR-0002 journal purity; ADR-0004 reconcile (engine
  does not settle `Unknown`); inventory sizing; cron is pure with the host
  supplying `now_ms`; vixie OR rule for restricted day fields.
* **Patterns to avoid**: thunk middleware; `tokio::time::sleep` in decisions;
  SQLite inside the framework without a new decision; adapter crates at all; a
  cron crate dependency or a timezone database in the decision path.
* **Migration steps**: none in this repo's siblings. Plane cutovers are
  separate records. Each plane implements the store/journal traits, registers
  its own cron jobs, calls `CronSchedule` + `fire_key` + `start` at its edge,
  and deletes the Resonate SDK, server, `/health`, and promise UI.

### Verification

- [x] `cargo test -p elm-effect-durable` and workspace `cargo test` /
      `clippy -D warnings` / `fmt --check` are green.
- [x] Conformance cases listed above exist and pass on the in-memory engine.
- [x] Cron cases cover strict next, one-tick catch-up, vixie Sunday 0/7,
      restricted day-of-month OR day-of-week, names, leap day, day/hour
      rollovers, impossible dates, parse errors, and a firing that starts once
      through `OurEngine`.
- [x] Hardening cases cover journal injection, prepared-intent completion,
      running-without-journal re-dispatch, live-lease re-attach, terminal
      journal over stale storage, terminal/claim projection, orphaned parks,
      one-park re-parking, park-generation reseeding, and `Unknown` refusing
      park/wake.
- [x] A `ServiceTag` + `Effect` + `Runtime` witness calls `start` at the
      framework edge (`crates/elm-effect-durable/tests/framework_fit.rs`).
- [x] `cargo test -p elm-effect-elm --test guide_imports` compiles the cron
      `use` line the guide publishes.
- [x] No `resonate`, `temporal`, `rivet`, or `cron` crate appears in this
      workspace's `Cargo.toml` files.
- [x] README index lists ADR-0005.

## Pros and Cons of the Options

### Keep Resonate only

* Good: no new code.
* Bad: three SDK pins; framework still vendor-shaped; ADR-0002 already rejected
  Resonate as carrier.

### Port + our engine (chosen)

* Good: dependency-free engine; no adapter layer; sized to inventory.
* Bad: we own the scheduler and the store/journal contract.

### Adopt Temporal / Rivet now

* Good: multi-host later.
* Bad: larger surface than we use; still a vendor pin; the plane rewrite
  remains.

## More Information

### 2026-11-01 — crate landed on `feat/durable-engine`

- Inventory: `docs/research/resonate-inventory.md`.
- Crate: `elm-effect-durable` with `DurableEngine`, `OurEngine`, `MemStorage`,
  `due_wakes`, `EngineMsg`. No Resonate/Temporal/Rivet deps in the workspace.
- Conformance: `crates/elm-effect-durable/tests/conformance.rs` (8 cases) plus
  two pure scheduler unit tests. Workspace `cargo test` / `clippy -D warnings`
  / `fmt --check` green after the landing.

### 2026-11-01 — cron timing kernel; crate merged

- The crate is on `main` at `c123947` (the four `feat/durable-engine` commits
  are merged). The earlier entry is retained as history.
- `crates/elm-effect-durable/src/cron.rs`: `CronSchedule`,
  `next_after_ms`, `latest_at_or_before_ms`, `fire_key`. 5-field vixie cron,
  UTC, minute resolution, no clock, no I/O, no cron crate, no timezone
  database.
- `crates/elm-effect-durable/tests/cron.rs`: one cron tick starts once through
  `OurEngine`; the second start is `Already`. The unit cases pin strict next,
  one-tick catch-up, Sunday 0/7, restricted day OR weekday, names, leap day,
  impossible dates, and parse errors.
- `docs/guides/15-use-the-apis.mdx` publishes the calls;
  `crates/elm-effect-elm/tests/guide_imports.rs` compiles the new `use` line.
- Verification: `cargo test -p elm-effect-durable`,
  `cargo test -p elm-effect-elm --test guide_imports`, workspace
  `cargo test` / `clippy -D warnings` / `fmt --check` green.
- No flywheel, toron, beads, or chiebukuro file changed. Their Resonate pins
  stay until each plane's adoption record cuts them.
- Status remains `proposed`; ADR-0001 reserves `accepted` for operator
  sign-off.

### 2026-11-01 — engine hardening and cron jump search

- `OurEngine` is generic over `JournalStore`; `with_journal` injects a
  persisted journal, so a restart re-attaches instead of guessing.
  `EngineStorage::commit_run` and `commit_park` are the transactional seams a
  host overrides when its store can write the pair atomically.
- `start` is journal-first: it completes a `Prepared` intent, re-attaches to
  every other recorded row, re-attaches without stealing a live host lease,
  and re-dispatches a `Running` row with no journal intent under the same key.
- `settle` and `claim` project onto the run row; terminal and unknown settles
  clear parks; the drain clears orphaned parks; `Unknown` refuses park and wake
  but still cancels; a terminal journal row wins over stale storage; park
  generations are re-seeded on recovery; the redundant in-engine pending store
  was removed.
- Cron search jumps by day, hour, and minute bit-scan instead of scanning every
  minute; plane schedules and day/hour rollovers are pinned.
- Verification: `cargo test -p elm-effect-durable` (13 cron unit, 20
  conformance, 2 cron integration, 1 framework fit), workspace `cargo test` /
  `clippy -D warnings` / `fmt --check` green.

### 2026-11-01 — what we borrowed

- Resonate: the idempotent run key and the park/wake split stay.
- Temporal: the journal is the history and wins for lifecycle; a claim is a
  fencing generation; terminal state must project onto the query; cron overlap
  is a host policy, not a tick.
- Rivet: one run key is one actor identity; one active park per key; a wake
  resumes the same identity.
- Not copied: multi-host placement, actor migration, a replay VM, timezone or
  jitter schedules.
- No adapter crates: the vendor is replaced, not wrapped.

### 2026-11-01 — no vendor adapters; integration review

- Decision revised: our engine replaces Resonate. There is no
  `elm-effect-resonate`, `-temporal`, or `-rivet` adapter crate and none is
  planned. Those systems are design references, not compatibility targets.
- Integration review against `docs/research/resonate-inventory.md`:
  - Engine verbs: `start`, `park_until`, `wake`, `cancel`, `query`, and
    `drain_due`. `OurEngine::settle` / `claim` project the journal fence onto
    the run row for a host that holds the execution lease.
  - Host seams: a plane implements `EngineStorage` and `JournalStore` on its
    store, or injects a persisted journal with `with_journal`. A transactional
    store overrides `commit_run` / `commit_park`.
  - Runtime: the host calls `drain_due(now_ms)` and feeds `EngineMsg::Woke`
    into its Elm fold. This repository does not spawn a timer, a server, or a
    second scheduler.
  - Cron: registration stays plane-side; the timing kernel is
    `CronSchedule` + `next_after_ms` / `latest_at_or_before_ms` + `fire_key`.
    Five-field UTC expressions only.
  - Durable functions: the ~29 flywheel and ~27 chiebukuro functions plus
    toron-workflow's entries remain a plane rewrite. The port schedules runs;
    it does not provide the Resonate proc-macro or `ctx` API.
  - Removal: deleting `resonate serve`, `/health`, groups, the SDK pin, and the
    remote URL is plane work. The engine adds no server process.
- Cutover limitations: single-host only; the default journal is in-memory;
  cron is 5-field UTC at minute resolution; no vendor history import; no
  multi-host placement, dashboard, or actor migration.
- Store/journal traits are synchronous and infallible in this record. A host
  with fallible I/O adapts at its boundary or opens a new decision to make the
  traits fallible. This is the main pre-production gap.
- `park_until` stores the absolute `until_ms`; `max_wait_secs` caps the
  reported `wait_ms`, not the stored deadline. A host that needs an earlier
  wake schedules it and calls `wake`, then re-parks.
- A terminal run key is not reused; a plane that must run again after a
  terminal state uses a new key (or its own retry key).
- Readiness: the engine, store/journal seams, cron kernel, and conformance
  suite are ready for a toron-workflow spike on a synchronous store. The
  fallible-I/O trait decision and the durable-function rewrite are the
  remaining gates before a production cutover.
- Verification: `cargo test --workspace` (1,328 tests, 0 failed), including 13
  cron unit, 20 conformance, 2 cron integration, and 1 framework-fit test;
  workspace `clippy -D warnings` / `fmt --check` green.

### 2026-11-01 — framework fit witness

- `crates/elm-effect-durable/tests/framework_fit.rs` holds `OurEngine` in a
  host `ServiceTag` service, resolves it with `use_service`, calls `start`
  through `Effect::sync`, and runs the description with `Runtime::run`. The
  engine stays synchronous and spawns nothing.
- The crate docs, guide, and ADR now state the layering: pure decisions
  (`due_wakes`, `plan_park`, `plan_attempt`, `CronSchedule`), `now_ms` as an
  input from the host's `Clock` / `TestClock`, storage/journal as traits,
  `EngineMsg` as Elm fold data, and the run key as the durable command key /
  `DurableLog` `producer_id`. Retries, timeouts, and remote policy stay in the
  host's `Interpret` / `plan_attempt` layer.
- The `library_crate_has_no_run` audit now treats `tests/` as the edge, so a
  downstream test may run the service/effect witness. Library `src` stays
  run-free.
