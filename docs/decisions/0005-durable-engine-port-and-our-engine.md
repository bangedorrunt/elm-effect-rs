---
status: proposed
date: 2026-11-01
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: ADR-0002, ADR-0003, ADR-0004, docs/research/resonate-inventory.md
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

## Context and Problem Statement

Three planes pin three different Resonate SDKs (crates.io 0.6 in flywheel and
toron-workflow; a git `main` tip in chiebukuro). They use Resonate for durable
run start, durable sleep/park, wake, cancel, and status query. Cron timing is a
small pure computation; cron registration, `resonate serve`, `/health`, groups,
and promise dashboards are plane ops around that core (see
[resonate-inventory.md](../research/resonate-inventory.md)).

ADR-0002 already rejected "Resonate as the carrier" and kept Resonate as an
engine adapter. The inventory shows the adapter surface we actually need is
small. We can ship our own single-host engine behind that surface and drop the
vendor from the framework path, while leaving a door for Resonate, Temporal, or
Rivet adapters later.

How does a plane schedule a durable run without naming a vendor? How does the
framework stay free of Resonate, Temporal, and Rivet dependencies?

## Decision Drivers

* ADR-0002: Resonate is an adapter, not the carrier; no thunk-style middleware
  that starts work; journal and `DurableLog` vocabularies already exist.
* Inventory: only scheduling verbs and cron timing are shared; cron
  registration, server process, and dashboards are not.
* Single host (mikasa) today; multi-host and a hosted dashboard are not
  required.
* Swap or drop a vendor later without rewriting plane reducers.
* No regressions: planes keep their Resonate pins until each adoption record
  cuts them; this repo adds the port and default engine only.

## Considered Options

* Keep Resonate as the only engine; pin one SDK version in every plane.
* Write a durable engine port (scheduling verbs) plus our own SQLite-or-trait
  backed single-host engine as the default; vendor adapters later.
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

### Vendor adapters (later, not now)

Allowed as separate crates (`elm-effect-resonate`, `-temporal`, `-rivet`) only
after an adoption need. Rules:

* Vendor history (Temporal event history, Rivet actor state, Resonate promise
  log) is **internal** to the adapter.
* Our `DurableLog` remains the source of truth for Msgs.
* Vendor workflow / run id maps 1:1 onto our run/journal key — no second
  identity.
* Every adapter must pass the conformance suite below.

This record does **not** add those adapter crates.

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

### What we give up

* Multi-host durable execution.
* A vendor-hosted dashboard and search attributes / actor placement.
* Local-time / DST cron. Timing is UTC and has no timezone database; a host
  that needs local time converts at its edge.
* A persistent journal by default. The in-memory journal survives no process
  restart; a host injects a persisted `JournalStore` with `with_journal`, and a
  `Running` row with no journal intent is re-dispatched under the same key.

Acceptable for the current single-host stack. The port keeps the replacement
path open if we outgrow one host.

## Consequences

* Good: one pin, no Resonate in the framework; planes speak journal + port.
* Good: one cron timing kernel; planes keep their own registration tables.
* Good: the journal is a host seam; persistent history is an injection, not a
  fork of the engine.
* Good: conformance suite catches a bad adapter before a plane cutover.
* Bad: we maintain a small scheduler, a storage trait, and a cron parser.
* Neutral: the default journal is in-memory; a single-process host and the
  conformance suite use it, a durable host injects its own.
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
  SQLite inside the framework without a new decision; adapter crates now; a
  cron crate dependency or a timezone database in the decision path.
* **Migration steps**: none in this repo's siblings. Plane cutovers are
  separate records. Each plane registers its own cron jobs and calls
  `CronSchedule` + `fire_key` + `start` at its edge.

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

* Good: dependency-free default; swap path preserved; sized to inventory.
* Bad: we own the scheduler.

### Adopt Temporal / Rivet now

* Good: multi-host later.
* Bad: larger surface than we use; still a vendor pin; migrates every plane
  twice if we later want our own default.

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
  conformance, 2 cron integration), workspace `cargo test` / `clippy -D
  warnings` / `fmt --check` green.

### 2026-11-01 — what we borrowed

- Resonate: the idempotent run key and the park/wake split stay.
- Temporal: the journal is the history and wins for lifecycle; a claim is a
  fencing generation; terminal state must project onto the query; cron overlap
  is a host policy, not a tick.
- Rivet: one run key is one actor identity; one active park per key; a wake
  resumes the same identity.
- Not copied: multi-host placement, actor migration, a replay VM, timezone or
  jitter schedules.
