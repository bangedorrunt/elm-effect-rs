# Durable log and durable engine

Read `docs/guides/12-durable-log.mdx`, `docs/guides/15-use-the-apis.mdx`, ADR-0002 section "Durable log", and ADR-0005 (`docs/decisions/0005-durable-engine-port-and-our-engine.md`). This file is the order of work, not a second spec.

Pin: Durable Streams `PROTOCOL.md` draft 1.0, commit `461b40267aabd644558f9b19dbb9507dd5f691cf`. The header date inside the file is the placeholder `2025-01-XX`.

## Order: DurableLog (L16 stream)

1. Implement the in-process log in `crates/elm-effect-journal` until every L16 test (`durable_log_laws`) passes.
2. Store `{model, offset}` as the snapshot row. The offset is a token the log returned. Reject a token the log did not mint (`hand_built_offset_is_rejected`).
3. One JSON stream per Elm program run. The interpreter appends. `update` does not take a stream handle.
4. A branch is a fork at the snapshot offset. The fork's epoch starts at 0.
5. Read with `DurableLog::read_page`, not a hand-rolled loop: it returns bounded records, the last-consumed cursor, `up_to_date`, and `closed` (EOF). `Offset::Now` resolves to the last existing record so a tail catches the next append.
6. The HTTP read/write adapter is `crates/elm-effect-journal/src/http.rs`. It serves catch-up, long-poll, append, close, HEAD, and SSE. It is not a subscription transport. Pull-wake and `collapsing_cursor` remain out of scope. For auth and TLS, read `docs/guides/16-reverse-proxy.mdx`; the adapter has neither.

## Order: DurableEngine and OurEngine (ADR-0005)

1. `OurEngine` in `crates/elm-effect-durable` is a synchronous kernel, like `DurableLog`, not a second runtime. The host provides it through a `ServiceTag` (for example `Arc<Mutex<OurEngine<S, J>>>`), reads `now_ms` from its `Clock` or `TestClock` at the edge, and lifts engine calls into `Effect::sync`.
2. A durable host binds its persisted `EngineStorage` and `JournalStore` with `OurEngine::with_journal(store, journal)` and overrides `EngineStorage::commit_run` and `commit_park` so the run row and park row write atomically in one transaction.
3. Every host store implementation must pass the law suites in its test suite:
   - `elm_effect_journal::laws::journal_store_all(&mut journal)`
   - `elm_effect_journal::journal_lease_laws(&mut journal)`
   - `elm_effect_durable::storage_laws(&mut storage)`
   - `elm_effect_durable::digest_storage_laws(&mut storage)`
   - `elm_effect_durable::signal_storage_laws(&mut storage)`
   - `elm_effect_durable::engine_laws(&mut engine)`
   - `elm_effect_durable::heartbeat_laws(&mut engine)`
   - `elm_effect_durable::retry_laws(&mut engine)`
   - `elm_effect_durable::signal_laws(&mut engine)`
   - `elm_effect_durable::cancel_tree_laws(&mut engine)`
   - `elm_effect_durable::lineage_laws(&mut engine)`
   - `elm_effect_durable::guard_laws(&mut engine)`
4. The run key is the durable command key, the journal key, the `DurableLog` `producer_id`, and the external System-of-Record `Idempotency-Key`. Never mint a second vocabulary.

## Temporal and Resonate pattern map

| Temporal / Resonate pattern | `elm-effect-durable` / `elm-effect-journal` call |
| --- | --- |
| Workflow + Activity (`execute_activity` / `ctx.run`) | Build deterministic key with `step_key(run, step)`. Call `engine.start_with(&key, now_ms, &StartOptions { max_attempts, lease_ms, owner })`. Execute the effect only on `StartOutcome::Started { generation }`; on `StartOutcome::Already { .. }`, skip re-execution. Finish with `engine.settle_checked(&key, generation, EffectState::Settled)` (or `Failed`). |
| Replay-digest non-determinism detection | Call `engine.start_guarded(&key, now_ms, &opts, digest)`. On `GuardedStart::Started`, run first execution; on `GuardedStart::Already`, re-attach; on `GuardedStart::Mismatch`, refuse non-deterministic replay without mutating state. |
| Durable timer / sleep (`workflow.sleep` / `ctx.sleep`) | Call `engine.park_until(key, until_ms, max_wait_secs, now_ms, None)` -> `ParkOutcome::Wait` or `ResumeNow`. At the host edge, call `engine.drain_due(now_ms)` and feed each `EngineMsg::Woke { key, overdue_ms, park_generation }` into `Loop::send`. Never call `tokio::time::sleep` in a decision path. Absolute deadlines support long-horizon (30-day) sleep without clock drift. |
| Latched durable signal (`defineSignal` / `ctx.promise`) | Call `engine.signal(key, signal_id, payload, now_ms)` (`SignalOutcome::Latched`). Resumed runs call `engine.await_signal(key, until_ms, max_wait, now_ms, ceiling)` (`AwaitOutcome::Signalled` or `Parked`). Signals arriving before park never sleep; `engine.take_signals(key)` drains unconsumed signals into Elm `Msg`. |
| Lease heartbeat + cancel piggyback | Call `engine.heartbeat(key, generation, owner, lease_ms, now_ms)` -> `HeartbeatOutcome::Renewed { lease_deadline }`. If the workflow was cancelled during execution, returns `HeartbeatOutcome::Cancelled` so the worker aborts immediately. |
| Durable retry backoff through park | Call `engine.retry_after(key, generation, &policy, max_wait, now_ms, ceiling)` with `BackoffPolicy::Fixed` or `Exponential`. Increments attempts in journal first, then parks the run until the backoff deadline. Dead-letters cleanly to `DeadLettered` when attempts exceed budget. |
| Parallel fan-out / child runs (`executeChild` + `Promise.all` / `.spawn()`) | Build child keys with `child_key(parent, step, idx)`, start each child, collect `(key, engine.query(&key)?.phase)`, and fold with `plan_join(&children)` -> `JoinOutcome::Pending { remaining }`, `AllSettled`, or `Failed { failed, cancelled, unknown }` (terminal failure short-circuits pending). |
| Structured child tree cancellation | Call `plan_cancel_children(&children)` to collect live phases, then call `engine.cancel_children(&keys, now_ms)` -> `CancelReport { cancelled, already_terminal, missing }`. Cancels non-terminal children deterministically in input order without error on already-terminal races. |
| Lineage rollover (`continue_as_new`) | Call `engine.continue_as_new(from_key, generation, &to_key, now_ms, &opts)` using `rollover_key(base, epoch)`. Starts next epoch before settling previous epoch; idempotent re-run finishes cleanly. Keeps recurring workflow history strictly bounded. |
| Saga compensation (`defer` stack / reverse `completed` loop) | Record each forward step as `SagaStep { step_key: step_key(run, name), compensate_key: compensate_key(run, name), status }`. On failure, call `plan_saga_rollback(&steps)` -> `SagaPlan { compensations, all_compensated }` (strict LIFO over `Completed` steps, skipping `Compensated`, `Failed`, `Skipped`) and run each compensation key through `engine.start`. |
| Cron schedule (`resonate.schedule` / Temporal Schedule) | Parse 5-field UTC cron with `CronSchedule::parse(expr)`. Evaluate ticks with `plan_cron_tick(name, &schedule, policy, last_fired_ms, now_ms)` using `CronPolicy::Latest` (one-tick catch-up; fresh schedules anchor at `now - 60s`) or `CronPolicy::Skip { grace_ms }` (skip stale ticks past grace window). Start `fire.key` (`fire_key(name, tick_ms)`) through `engine.start`. |
| External System of Record, heartbeat timeout, and reconciliation | Pass the run/step key as the external `Idempotency-Key`. Count provider retries with `engine.attempt(key, generation)`. Expire dead claims with `engine.expire_lease(key, now_ms)` (`InFlight` / `Accepted` past `lease_deadline` -> `EffectState::Unknown`). Query the SoR and converge with `engine.reconcile(key, generation, outcome)`. |

## Map, when touching toron

Use the vocabulary table in ADR-0002, section "Durable log". `plan_attempt`'s attempt number is not `seq`. `RetrySameKey` resends the same `(epoch, seq)`. The attempt column stays on the journal row.

`can_transition` does not grow a stream offset column by guesswork. If the row has no offset column, add one in the journal store migration and commit it in the same transaction as the stream append.

## Do not copy

`packages/state/STATE-PROTOCOL.md` (insert, update, delete, `snapshot-start`, `snapshot-end`). `packages/client-rust/design.md` is a proposed HTTP client, not the kernel. The Node server and the Caddy plugin are peers. Do not add Resonate, Temporal, or Rivet adapter crates (ADR-0005).
