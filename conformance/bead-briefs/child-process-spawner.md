# Port ChildProcessSpawnerTest.ts's portable behavior rows

## Source
`~/.cache/effect-4.0.0/packages/effect/test/process/ChildProcessSpawnerTest.ts` — 67
`unported` rows for `packages/effect/src/process/ChildProcessSpawner.ts`.

## Rust owner
`crates/elm-effect-unstable/src/process.rs` (372 lines) already lands the surface
these rows drive: `command_array` / `command_template`, `CommandSpec`
(`cwd`/`env`/`clear_env`/`spawn`), `ChildHandle` (`id`/`wait`/`kill`),
`CommandOutput` (`code`/`stdout`/`stderr`/`is_success`/`stdout_text`),
`Pipeline`, `run`, and `ProcessError` (`Spawn`/`NonZeroExit`/`MissingEnv`).

Witness: extend `crates/elm-effect-unstable/tests/process_spawn.rs`, one test
per row, each carrying a `// ChildProcessSpawnerTest.ts:<line> — <behavior>`
comment so the flip names the row it proves.

## Method
RED first: write the witness for a row, watch it fail (or watch the capability
be missing), implement only what the row needs, then flip with
`conformance/flip-row.py`.

## Scope boundary
Rows that need capability `process.rs` does not model stay `unported` in this
unit, they are not excused here:
- combined/interspersed `.all` output, incremental stdout streaming
- stdin fed to a child, extra fd plumbing (fd3+, gaps, bidirectional)
- specific-signal kill, force-kill after a timeout, scope-close kill
- `unref` and process-group cleanup (rows carried as `it.skipIf` upstream)

Leaving at least one row open keeps `ChildProcessSpawner.ts` `unmatched`, so this
unit needs **no** `conformance/modules.toml` change. That file is under
CyanGlen's exclusive lease (reason `elm-effect-rs-azf3`); a byte changed there
by anyone else is blocked by the pre-commit guard. A later unit that closes or
excuses the remaining rows takes the lease and flips the derived status.

## Verify
`sh -c "cargo test -p elm-effect-unstable --test process_spawn"`
