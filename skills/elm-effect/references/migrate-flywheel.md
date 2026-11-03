# Migrate flywheel

Repo: the flywheel workspace. Law: ADR-0002 sibling migration "Flywheel" and ADR-0005 (`OurEngine` backed by `ToronEngineStore` and `ToronJournal`).

## Gate

Green in elm-effect-rs before the first flywheel edit: phase 3 tests (L4, L8, L9, L10) and, if the change persists or replays, phase 9, phase 9b (L14, L16), and `elm-effect-durable`.

`src/durable_body.rs` and `src/cron.rs` run on `OurEngine::with_journal(ToronEngineStore, ToronJournal)` through `toron_core::kernel`. Do not restore `resonate_sdk` or add a vendor adapter.

## Order

1. Read `src/decide.rs` `decide`, `src/effect_journal.rs`, `src/breaker.rs`, `src/durable.rs`, `src/durable_body.rs`, and `src/cron.rs`. Re-read them. Paths move.
2. Keep `decide` pure. Its `Decision` becomes `Vec<Cmd>` of a `Program` whose model is the briefing snapshot, or `update` calls `decide` and then builds the command list. The fold still does not read the clock.
3. `effect_journal::effect_runner` becomes `Interpret`. Intent, claim, one invoke, settle or `unknown` stay in that order. In `durable_body.rs`, expired claims transition to `EffectState::Unknown` via `engine.expire_lease(key, now_ms)` and recover via `engine.reconcile(key, generation, outcome)`.
4. `breaker.rs` becomes the breaker argument to `policy::remote`. Do not keep a second breaker beside it.
5. Doorbell ticks, herdr events, and mail ingress become messages. They do not call the model.
6. Use `BTreeMap` in `Model` and in `cron.rs` state maps (`consecutive` failure counters, etc.) so iteration and serialization order are deterministic. Share `ToronEngineStore` and `ToronJournal` handles across native cron fires rather than re-opening SQLite on every tick.
7. A persisted message log uses `references/durable-log.md`. The resume field is `offset`.

## Done

- [ ] `decide` has no `SystemTime`, `std::env`, or `thread_rng`.
- [ ] Effectful leaves that moved go through `Interpret`, and a dropped command value performs no I/O.
- [ ] Worker count and queue bound are still 4 and 4096.
- [ ] `durable_body.rs` and `cron.rs` use `OurEngine::with_journal` and deterministic `BTreeMap` state without `PoisonError::into_inner` recovery.
- [ ] `cargo test` for the flywheel package covers the leaf that moved: one success, one retry then success, one expired lease (`expire_lease` -> `Unknown`) reconciled to terminal state.
