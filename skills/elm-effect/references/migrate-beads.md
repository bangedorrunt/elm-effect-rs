# Migrate beads

Repo: the beads workspace. Law: ADR-0002 sibling port "Beads".

## Gate

Green in elm-effect-rs: phase 8 (the fold, L12, L15) and phase 9b if a close or a gate is persisted as a stream. Phase 3 is enough for a pure status fold with no new I/O.

## Two streams

Issue events and gate verdicts are different logs.

- Issue rows: the issues JSONL the store already exports.
- Gate rows: `.beads/gates.jsonl`, flushed from `src/sync/mod.rs`. A close with a verdict and no matching PASS row is a doctor error (`docs/decisions/0001-make-beads-the-fail-closed-work-ledger.md`).

A projection may join them for display. No function writes a gate from an issue event. No import rebuilds gates by replaying issues only. `br doctor --repair` style rebuilds that drop gate rows stay refused. Do not add a helper that does it "just for the new fold".

## Order

1. Re-read `src/model/` (`Issue`, `Event`, `Status`) and `src/close_policy.rs`.
2. Status changes are `update(issue, event) -> (issue, commands)`. `update` does not read the clock, the environment, or the database.
3. `br close` is a command. The interpreter checks for a PASS gate row bound to a commit whose message cites the bead, then returns a message. `update` flips status only on that message.
4. Close policy from `.beads/policy.yaml` is `Boot` data. Unknown fields still parse. Do not hard-fail the file on an unknown key. That failure mode already took down `br close`.
5. If gate history is tailed, it is its own durable stream with its own `offset`. Do not store the issues offset in the gates row.

## Done

- [ ] `update` is pure. A test folds the same event log twice and gets the same status and the same command list.
- [ ] A test closes with no PASS row and the status does not become closed.
- [ ] A test imports issues without `gates.jsonl` and the verdict table does not grow.
- [ ] `cargo test` for the close and sync crates that the change touched.
