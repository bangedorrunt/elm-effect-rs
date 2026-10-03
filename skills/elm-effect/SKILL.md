---
name: elm-effect
description: >
  Build an application or CLI on elm-effect, port or add an Effect module to
  the framework, and migrate flywheel, toron, beads, or chiebukuro onto it.
  Use when the user says build, scaffold, or write an app, service, or CLI
  with elm-effect; port, add, or extend an Effect TS module (Cache, HttpClient,
  Stream, Schema, or any ledger row); or migrate, rewrite, integrate, or adopt
  elm-effect, the Elm loop, the effect journal, or a durable stream. The skill
  picks app versus CLI versus module port versus sibling migration. It does
  not ask whether ADR-0002 is accepted, whether Resonate is deleted, which
  crate a new module belongs in, or whether a cursor is the resume position.
---

# elm-effect

The law is `/Users/bangedorrunt/workspace/elm-effect-rs/docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md`. This file is the steps. Do not copy a law into a sibling. Point at the ADR section. The phases land in that repo even when the utterance was typed in a sibling checkout.

## Closed

Do not ask these. The ADR agent-authority section already answered them.

- Status `proposed` is not a stop. Implement.
- Resonate stays. Do not remove the git dependency in the migration change.
- Resume position is `offset`. `collapsing_cursor` is the HTTP CDN token only.
- The log is in-process. Do not add the Node or Caddy server as the runtime.
- `schema`, `stm`, and `tokio` default on. Feature `durable-streams` stays off.
- Executor defaults are 4 workers and queue bound 4096.
- Issues and gates are two streams. Gates are `.beads/gates.jsonl`.
- A migration that names no sibling: say the four names and stop. An app, a CLI, or a module port does not need a sibling.

## Branch

A named Effect module, or the word module, is `references/port-module.md` even when a sibling or a CLI is also named. Finish that file, then the row below. Otherwise the first matching row wins.

| Utterance names | Read, the whole file | Do not read |
| --- | --- | --- |
| flywheel | `references/migrate-flywheel.md` | the other migrate files, both build files |
| toron | `references/migrate-toron.md` | the other migrate files, both build files |
| beads, br, gates | `references/migrate-beads.md` | the other migrate files, both build files |
| chiebukuro, chie, resonate pipeline | `references/migrate-chiebukuro.md` | the other migrate files, both build files |
| port, add, or extend a named Effect module, or a new framework module | `references/port-module.md` | migrate files. Read one build file after, only when the utterance also names an app or a CLI |
| CLI, command, argv, subcommand | `references/build-cli.md` | `build-app.md`, migrate files |
| app, service, long-running program, fold | `references/build-app.md` | `build-cli.md`, migrate files |
| next phase, keep going, no module and no product | `references/write.md` | migrate files, both build files |
| durable stream, offset, epoch, fork, snapshot, after a branch is already chosen | that branch's file, then `references/durable-log.md` | the state protocol, the HTTP client design |

A build that resumes after a crash also reads `references/durable-log.md`. A build that names an Effect module whose ledger row is not `matched` reads `references/port-module.md` first, then the build file. A sibling branch that persists or replays does the same with `durable-log.md`. A retry wrapper does not.

## Gate

Before editing a sibling, the tests named in that sibling file are green in elm-effect-rs. Red means implement that phase in elm-effect-rs first, in the same task, then edit the sibling. Do not split the task by asking.

Phase order is ADR-0002's phase list. Do not skip a red law test to reach the sibling.

## Done

Done is the checklist in the file the branch table named, plus the `cargo test` that file names. Report the paths changed and the commands run. Do not end on a question.

## Never

- A second `EffectState` enum. The boundary mirror has no `Parked`, and adding it there hides the drift instead of retiring the mirror.
- A saga, a thunk, or a listener that starts work the fold did not return. Replay cannot see that work, so a crash runs it twice or not at all.
- `HashMap` inside a `Program::Model`. Iteration order changes the command list across processes.
- Rebuilding `.beads/gates.jsonl` from `issues.jsonl`. The issues log does not hold verdicts, and a rebuild empties them.
- `or_die` after a retry budget. The caller still needs the typed error to fall back. `catch_cause` belongs to the fiber supervisor, because a business handler that sees defects swallows bugs.
- Storing `collapsing_cursor` as the fold offset. That token is for CDN fan-in. The resume token is `offset`.
- Expiring producer epoch state on a TTL. The draft allows it. A zombie after expiry double-sends mail.
- Treating `UpToDate` as EOF. Caught up means more appends may arrive. `Closed` is the end.
