# Resonate inventory (stack usage → durable-engine port)

Date: 2026-11-01 (Australia/Melbourne). Read-only survey of the three planes
that pin a Resonate SDK today. Sizes ADR-0005 / `elm-effect-durable`: the port
covers only what is used. Carrier law stays [ADR-0002](../decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md);
this file does not restate it.

Checkouts surveyed (local on mikasa, not pushed):

| Plane | Path | SDK pin |
| --- | --- | --- |
| flywheel | `/Users/bangedorrunt/workspace/flywheel` | `resonate-sdk = "0.6.0"` (crates.io) |
| toron | `/Users/bangedorrunt/workspace/toron` | `resonate-sdk = "0.6"` + `resonate-sdk-macros = "0.1"` in `toron-workflow` and `toron-e2e` |
| chiebukuro | `/Users/bangedorrunt/workspace/chiebukuro` | `resonate-sdk` git `main` @ `65452654…` |

Three different pins. A single engine behind one port retires that split.

## Feature matrix (what is actually called)

| Feature | flywheel | toron-workflow / cli / e2e | chiebukuro | In the port? |
| --- | --- | --- | --- | --- |
| Durable function macro (`#[…::function]`) | yes (~29 registered leaves in `durable.rs`) | yes (`run_workflow`, parallel child, cron loops) | yes (~27 functions across session/cli/memory) | no — planes keep their own entry points; the port schedules runs, it does not own a proc-macro |
| `ctx.run` / nested durable child | yes (leaves) | yes (`ctx.run(run_workflow, …).spawn()`) | yes (mining → compile, etc.) | start run (idempotent key); nesting is plane composition |
| `ctx.sleep` / durable delay | yes (loop interval) | yes (delay steps, park waits, retry backoff) | rare (mostly handle `.timeout`) | park until |
| External / createable promises (approval, spawn-await) | yes (`promise_pending`) | yes (approval + spawn side-channels in `sinks.rs`) | via SDK promise records for task UI | wake / cancel / query — promise *identity* collapses to the run/journal key |
| `Resonate::schedule` (cron → durable fn) | no (cron goes through toron-workflow / own cron) | yes (`cron.rs` durable-sleep loops) | yes (`daemon/scheduler.rs`, ~9 specs) | timing only — `CronSchedule` is shared; registration and the tick loop stay plane-side; each firing is `start` |
| Register functions on a client | yes | yes | yes | no — host registers interpreters; engine stores runs |
| `Resonate::new` / `::local` / embedded store | yes | yes (Pinned / ProbedRemote / ProbedLocal / Embedded) | yes (`ResonateServer` + `::local` in tests) | our engine is the embedded default |
| Server process (`resonate serve` + sqlite `resonate.db`) | yes (`daemon::ensure_resonate`, `cmd/engine`) | probed, not spawned by workflow | yes (`daemon/server.rs`) | no — process supervision is plane ops |
| `/health` probe | yes (`resonate_healthy`, preflight) | yes (`toron doctor` / operator) | yes (server ready wait, preflight) | no — no external server when we are the engine |
| Promise list / cancel / delete / purge / tail | no | no | yes (`cli/daemon_cmd`, `daemon/monitor`) | cancel + query only; dashboard/GC UI is plane-side |
| Groups / prefix isolation | no | no | yes (`CHIEBUKURO_RESONATE_GROUP`, legacy `PREFIX`) | no — single-host; isolation is store path / project |
| Handle `.timeout` (run deadline) | via leaf budgets | yes (`no_claim_timeout` on spawn) | yes (3600 / 86400 common) | cancel at deadline is plane policy calling `cancel`; port does not invent a second timer vocabulary |
| Dependency injection on ctx | yes (`get_dependency::<RuntimeAuthority>`) | WorkflowResolver DI | limited | no — Elm services / host DI stay outside the engine |
| Promise GC / prune of Resonate DB | no | no (toron prunes its *own* `WorkflowEffectStore`) | purge local data dir; retention compact is a scheduled *job*, not SDK GC | no |
| Config / env | `resonate_url` / `RESONATE_URL`; default `http://localhost:8001`; state `resonate.db` | `RESONATE_URL` > remote `https://resonate.alpine-tilapia.ts.net` > local `:8001` > Embedded | `CHIEBUKURO_RESONATE_URL` (default `:8001`), `TOKEN`, `GROUP`/`PREFIX`, `CHIEBUKURO_RESONATE_DATA_DIR` | engine needs a store path or trait, not a URL |

## Behaviours that must be preserved

Shared across planes (laws already in ADR-0002 / journal):

1. **Idempotent start** — the same run key does not double-dispatch a completed or in-flight attempt (`plan_attempt` / L14).
2. **Durable sleep / park** — `plan_park`: overdue → resume with gap; over ceiling → typed refuse (not a shorter sleep); same `(run, promise/node, until)` → one pending row.
3. **Park is not terminal** — resume, do not fire `on_error` / settle-as-failed.
4. **Wake on time and after restart** — a fresh process on the same storage must wake pending parks whose `until_ms <= now`.
5. **Cancel is terminal** — cancelled runs do not resume.
6. **Stale generation / epoch cannot settle** — fencing already on journal claims and `DurableLog` producers.
7. **Unknown has no default exit** — reconcile owns it (ADR-0004); the engine must not invent Settled.

Plane-specific (stay plane-side during migration; port must not block them):

- **toron:** approval and spawn-await side-channels (`sinks.rs`); `WorkflowEffectStore` pending rows; doctor `/health` until Resonate is gone.
- **flywheel:** leaf registration set in `durable.rs`; `ensure_resonate` autostart of loopback `resonate serve` until cutover; swarm loop interval sleeps.
- **chiebukuro:** cron `schedule()` registrations; task/promise CLI; group isolation; `BoundedNetwork` stall bound (SDK HTTP had no timeout).

## Tests that pin those behaviours today

| Plane | Pins |
| --- | --- |
| toron-workflow | `engine/park.rs` unit tests (resume at T, overdue, ceiling refuse, redrive key); `engine/tests.rs` (`durable_delay_completes`, approval park/resume, spawn-await, invoke≡trigger, live `resonate serve` park); URL resolution unit tests; e2e `sc16`–`sc24` (concurrency, DAG, spawn, cron) |
| toron-cli | `check_resonate_tests` (GS-037: `/health` must be HTTP 2xx, no invent-green) |
| flywheel | `durable.rs` / `daemon.rs` unit tests; ADR-0010 source gates (`adr0010_d0`…); `captain_watch_preflight` (no live resonate required for that path); cron/engine health-shaped probes |
| chiebukuro | `tests/durable_wait_bounds.rs` (stalled server must error; explicit deadline ends wait); `daemon/scheduler.rs` ensure_schedule idempotency; `daemon/monitor.rs` promise-state classifiers |
| elm-effect-journal | `plan_park.rs`, `plan_attempt.rs`, `journal_store.rs`, durable-log law tests — **these stay the source of truth**; the engine reuses them, it does not fork them |

## What the port covers

Scheduling verbs and pure cron timing, on top of `elm-effect-journal`
vocabulary and `DurableLog`:

- `start` (run key = journal key = `producer_id`)
- `park_until` (delegates to `plan_park`)
- `wake`
- `cancel`
- `query`
- cron timing (`CronSchedule`, `next_after_ms`, `latest_at_or_before_ms`,
  `fire_key`); registration stays plane-side

Pure scheduler core: given parked rows + `now_ms`, which keys are due. Storage trait with an in-memory impl; toron later binds `WorkflowEffectStore` (ADR-0002 L342). A host can inject a persisted journal with `with_journal` and override `commit_run` / `commit_park` with a transaction. No SQLite inside the framework unless a future adoption record requires it — today every plane that needs durable bytes already has a store (Resonate's db or toron's).

## What the port deliberately does not cover

- Proc-macro durable functions and SDK `Context` / `ctx.run` shape
- Cron registration, pause/resume, and the tick loop (`schedule`)
- Spawning or probing an external `resonate serve`
- `/health`, groups, tokens, promise dashboards, DB purge UIs
- Multi-host placement, search attributes, actor addressing
- A third lifecycle vocabulary (no parallel "PromiseState" enum in the framework)

Those remain plane concerns until each plane's adoption record deletes its Resonate pin. No vendor adapter crate is planned; our engine replaces the vendor (ADR-0005).

## Open follow-ups (not decisions for this inventory)

- Migration order (toron-workflow → flywheel → chiebukuro) belongs in ADR-0005.
- Whether chiebukuro's cron firings become toron-workflow cron or direct `start` calls is a chiebukuro adoption question.
- Cutting `ensure_resonate` / doctor `/health` is per-plane once the engine is default.
- No adapter crate and no vendor history import; a plane cutover starts fresh runs.
