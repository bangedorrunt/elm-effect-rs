# Migrate flywheel

Repo: the flywheel workspace. Law: ADR-0002 sibling port "Flywheel".

## Gate

Green in elm-effect-rs before the first flywheel edit: phase 3 tests (L4, L8, L9, L10) and, if the change persists or replays, phase 9 and phase 9b (L14, L16).

`durable.rs` stays on Resonate until a `WorkflowEngine` adapter runs the same loop. Do not delete `resonate_sdk` in the change that turns `decide` into commands.

## Order

1. Read `src/decide.rs` `decide`, `src/effect_journal.rs`, `src/breaker.rs`, `src/durable.rs` `tick`. Re-read them. Paths move.
2. Keep `decide` pure. Its `Decision` becomes `Vec<Cmd>` of a `Program` whose model is the briefing snapshot, or `update` calls `decide` and then builds the command list. The fold still does not read the clock.
3. `effect_journal::effect_runner` becomes `Interpret`. Intent, claim, one invoke, settle or `unknown` stay in that order. The 4-worker pool and the 4096 bound become `TokioExecutor` config (`worker_threads: 4`, `queue_bound: 4096`). Delete the hand-built pool only when no leaf still submits to it. If any leaf still submits, leave the pool and say so in the report.
4. `breaker.rs` becomes the breaker argument to `policy::remote`. Do not keep a second breaker beside it.
5. Doorbell ticks, herdr events, and mail ingress become messages. They do not call the model.
6. If `durable.rs` or the snapshot model uses `HashMap` and iteration order changes the command list, switch that model to `BTreeMap` in this change. If order does not change the list, leave the map and say why in the report.
7. A persisted message log uses `references/durable-log.md`. The resume field is `offset`.

## Done

- [ ] `decide` has no `SystemTime`, `std::env`, or `thread_rng`.
- [ ] Effectful leaves that moved go through `Interpret`, and a dropped command value performs no I/O.
- [ ] Worker count and queue bound are still 4 and 4096.
- [ ] `resonate_sdk` is still a dependency if `durable.rs` still uses it.
- [ ] `cargo test` for the flywheel package covers the leaf that moved: one success, one retry then success, one `unknown` that does not settle as success.
