# Migrate chiebukuro

Repo: the chiebukuro workspace. Law: ADR-0002 sibling migration "Chiebukuro" and ADR-0005 (`OurEngine` cutover in chiebukuro's adoption records).

## Gate

Green in elm-effect-rs: phase 3 for a new effect that is not durable, phase 9, phase 9b, and `elm-effect-durable` (`storage_laws`, `engine_laws`, `journal_store_all`) for daemon jobs, cron, and pipelines.

## Order

1. Re-read `src/daemon/engine_store.rs` (`SqliteEngineStorage`, `SqliteJournalStore`), `src/daemon/jobs.rs` (`HostEngine`, `enqueue_job`, `JobPhase`), `src/daemon/cron.rs` (`CronSchedule::plan_skip_tick`), and `src/daemon/monitor.rs`.
2. `OurEngine::with_journal(SqliteEngineStorage, SqliteJournalStore)` backs `HostEngine`. Use `engine.start_with(key, now_ms, &StartOptions { max_attempts, lease_ms, owner })` so intent and claim options are recorded in one journal step. Do not restore `resonate-sdk` or add a vendor adapter.
3. Map `RunPhase::Unknown` to `JobPhase::Unknown` — never collapse an uncertain run into `JobPhase::Running` or guess its exit. Use `RunPhase::as_str` / `FromStr` and `Disposition::from_str` rather than custom string match tables.
4. Search and ingest are effects. A ranking that must replay is a fold over a message that already contains the retrieved set. The fold does not call the store.
5. Memory record is a durable command. Memory recall is an effect whose result comes back as a message.
6. Use `elm_effect_durable::{child_key, step_key, fire_key, compensate_key}` for deterministic run keys and `CronSchedule::plan_skip_tick` (or `plan_cron_tick`) for cron ticks.

## Done

- [ ] `SqliteEngineStorage` and `SqliteJournalStore` pass `journal_store_all`, `storage_laws`, and `engine_laws`.
- [ ] The pipeline's fold test replays a log and emits the same command list.
- [ ] A crash-shaped test (kill between intent and settle, or lease expiry via `expire_lease` + `reconcile`) redelivers or reconciles once and does not double-commit.
- [ ] `cargo test` for the daemon engine store, jobs, cron, monitor, and the pipeline that moved.
