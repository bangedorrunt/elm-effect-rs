# Migrate toron

Repo: the toron workspace. Law: ADR-0002 journal section, ADR-0005 durable engine port, and sibling migration "Toron".

## Gate

Green in elm-effect-rs: phase 9 (`golden_transitions`, `plan_attempt` arms, `may_settle`, `check_replay`, `plan_park`), phase 9b (L16), and `elm-effect-durable` (`storage_laws`, `engine_laws`).

## Order

1. Re-read `crates/toron-core/src/kernel.rs`, `crates/toron-core/src/journal.rs`, `crates/toron-core/src/index/durable_engine.rs` (`ToronEngineStore`), `crates/toron-core/src/index/workflow_effects.rs` (`EffectState`, `can_transition`), `crates/toron-workflow/src/engine/reconcile.rs`, `crates/toron-workflow/src/engine/park.rs`, and `crates/toron-workflow/src/cron_elm.rs`.
2. Follow the park finding in ADR-0002's Journal section. Do not add a `can_transition` edge into `Parked`. Persist a park with `upsert_pending`'s key `(run_id, promise_id, park_generation)` or `ToronEngineStore::commit_park`. A row that is already `Parked` is `ResumeParked`, not `Failed`.
3. `plan_attempt` in the framework and in toron must match (`pub use toron_core::kernel::plan_attempt;` in `reconcile.rs`). `Prepared` dispatches (`DispatchNew`; `check_claim` / `may_claim` fences live leases at claim time). `ReconcilePending` refuses. Query beats idempotency. `ResumeParked` uses `attempt.max(1)`. `RetrySameKey` uses `attempt + 1`.
4. `crates/toron-core/src/kernel.rs` re-exports the `elm-effect-durable` and `elm-effect-journal` surface (`OurEngine`, `DurableEngine`, `CronPolicy`, `CronFire`, `plan_cron_tick`, `plan_join`, `plan_saga_rollback`, `check_claim`, `check_settle`, `storage_laws`, `engine_laws`, `journal_store_all`). Use `RunPhase::as_str` / `FromStr` and `Disposition::from_str` rather than local string tables.
5. Mail send is a durable command. The key is the event id. Reservation acquire is a claim: generation, lease, `may_settle` / `check_settle`. NIP-44 stays a service at the edge.
6. Read `references/durable-log.md`. Map generation to `epoch`, attempt to `seq`, effect key to `producer_id`, resume token to `offset`.
7. Cleanup kinds stay in toron. Do not move `CaptainEvidence` or `BeadComment` into the framework crate.

## Done

- [ ] One persisted `EffectState`, the core one, and `ToronEngineStore` + `ToronJournal` pass `journal_store_all`, `storage_laws`, and `engine_laws`.
- [ ] `plan_attempt` table tests pass against toron's re-export and the framework's function on all 216 vectors.
- [ ] `PARK_MAX_WAIT_SECS` is still 86400. A longer park is a typed refusal.
- [ ] A replay digest mismatch does not mint a second effect key.
- [ ] `cargo test` in `toron-core` and `toron-workflow` for the journal, durable engine store, cron, and park modules.
