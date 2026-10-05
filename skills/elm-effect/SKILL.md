---
name: elm-effect
description: >
  Use the finished elm-effect framework: call its APIs, build an app or CLI,
  add a ServiceTag, Program, or DurableLog, port an Effect module, or migrate
  flywheel, toron, beads, or chiebukuro. Use when the user says use, call,
  guide, API, build, scaffold, port, add, extend, migrate, rewrite, or
  integrate elm-effect, Effect, the Elm loop, the journal, or a durable log.
  The skill sends you to docs/guides first. It does not ask you to rebuild
  the carrier, delete Resonate, or treat a cursor as the resume position.
---

# elm-effect

The carrier is in the crates. ADR-0002 is the law. ADR-0003 is adoption. The calls are `docs/guides/15-use-the-apis.mdx` and the topic guide next to it. This file only picks the branch. Do not copy an API table into a sibling.

Repo root: `/Users/bangedorrunt/workspace/elm-effect-rs`.

## Closed

- Do not rebuild a phase whose tests already pass.
- Do not ask if ADR-0002 is accepted.
- Resonate stays during a migration.
- The resume field is `offset`. `seq` is not the journal attempt number.
- Read `crates/elm-effect/Cargo.toml` for features. Do not change them in an application task.
- Gates are `.beads/gates.jsonl`. Do not rebuild them from the issues log.
- A migration with no sibling name: list flywheel, toron, beads, and chiebukuro, then stop.

## Branch

Read `docs/guides/15-use-the-apis.mdx` on every branch. Then the first matching row. A named Effect module is `references/port-module.md` even when a CLI or a sibling is also named. Finish that file, then the row.

| Utterance | Then read | Do not read |
| --- | --- | --- |
| how to call, example, API, guide | the topic file under `docs/guides/` | migrate files |
| flywheel | `references/migrate-flywheel.md` | other migrate files |
| toron | `references/migrate-toron.md` | other migrate files |
| beads, br, gates | `references/migrate-beads.md` | other migrate files |
| chiebukuro, resonate pipeline | `references/migrate-chiebukuro.md` | other migrate files |
| port or add an Effect module | `references/port-module.md` | migrate files until the port is done |
| CLI, argv | `references/build-cli.md` | `build-app.md` |
| app, long-lived program, fold | `references/build-app.md` | `build-cli.md` |
| add a service, log, or program type | `docs/guides/13-add-a-capability.mdx` | a new scheduler |
| crash must resume | `references/durable-log.md` | the state protocol |
| red test inside elm-effect-rs | `references/write.md` | a sibling |

## Gate

A sibling edit waits until `cargo test -p elm-effect` is green. The ignored conformance test stays red while behavior rows are `unported`. That red test is not a reason to rebuild the kernel. Port the rows, or leave them, and say which.

## Done

The branch file's checklist is true, and the `cargo test` it names passed. Report paths and commands. Do not end on a question.

## Never

- A second `EffectState`. Park is not a new effect-row edge.
- A saga or a listener that starts work the fold did not return.
- `HashMap` in a `Program::Model`.
- `or_die` after retry. `catch_cause` outside the supervisor.
- `tokio::time::sleep` or `tokio::time::pause` inside an effect.
- A hand-built offset. A `seq` gap. A producer id with only epoch or only seq.
- `UpToDate` treated as EOF.
