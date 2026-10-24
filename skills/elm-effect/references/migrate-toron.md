# Migrate toron

Repo: the toron workspace. Law: ADR-0002 journal section and sibling migration "Toron".

## Gate

Green in elm-effect-rs: phase 9 (`golden_transitions`, `plan_attempt` arms, `may_settle`, `check_replay`, `plan_park`) and phase 9b (L16).

## Order

1. Re-read `crates/toron-core/src/index/workflow_effects.rs` (`EffectState`, `can_transition`), `crates/toron-workflow/src/engine/reconcile.rs` (`plan_attempt`, `may_settle`, `check_replay`), `crates/toron-workflow/src/engine/park.rs`, `crates/toron-workflow/src/engine/boundary.rs`.
2. Follow the park finding in ADR-0002's Journal section. Do not add a `can_transition` edge into `Parked`. Persist a park with `upsert_pending`'s key `(run_id, promise_id, park_generation)`. A row that is already `Parked` is `ResumeParked`, not `Failed`.
3. `plan_attempt` in the framework and in toron must match. `Prepared` dispatches. `ReconcilePending` refuses. Query beats idempotency. `ResumeParked` uses `attempt.max(1)`. `RetrySameKey` uses `attempt + 1`. A mismatch is a bug in the newer copy. Fix the newer copy.
4. Retire uses of `boundary::EffectState` by calling the core enum plus `Option` for "no row". Do not add `Parked` to the boundary enum to silence the doc comment in `park.rs`. Fix the comment.
5. Mail send is a durable command. The key is the event id. Reservation acquire is a claim: generation, lease, `may_settle`. NIP-44 stays a service at the edge.
6. Read `references/durable-log.md`. Map generation to `epoch`, attempt to `seq`, effect key to `producer_id`, resume token to `offset`.
7. Cleanup kinds stay in toron. Do not move `CaptainEvidence` or `BeadComment` into the framework crate.

## Done

- [ ] One persisted `EffectState`, the core one.
- [ ] `plan_attempt` table tests pass against toron's function and the framework's function on the same vectors.
- [ ] `PARK_MAX_WAIT_SECS` is still 86400. A longer park is a typed refusal.
- [ ] A replay digest mismatch does not mint a second effect key.
- [ ] `cargo test` in `toron-core` and `toron-workflow` for the journal and park modules.
