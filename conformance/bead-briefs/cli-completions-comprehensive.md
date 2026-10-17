# Port the ComprehensiveCli integration rows of the cli completion generators

Source rows: `packages/effect/test/cli/completions/completions.test.ts`, the
`Completions integration` section — `:814` (bash), `:854` (zsh), `:885` (fish).
These are the last 3 unported rows of that file; the other 67 were closed by the
descriptor, bash, zsh and fish units.

## Oracle

The pin's `test/cli/fixtures/ComprehensiveCli.ts` (439 lines) is transcribed into
`crates/elm-effect-unstable/tests/completions_fixtures/mod.rs::comprehensive()`.

The goldens under `tests/fixtures/completions/{bash,zsh,fish}/comprehensive.*`
were produced by running the pin's **own** `Command.make` / `Command.withSharedFlags`
/ `Flag` / `Argument` API to build the fixture, then the pin's **own**
`cli/internal/completions/descriptor.ts` + `bash.ts` / `zsh.ts` / `fish.ts` to
generate the scripts. The Rust port matches that output byte for byte, which is
stricter than the pin's `assert.include` rows.

`Command.withSharedFlags` merges the shared flags into both the command's own
config and its context config, so the Rust descriptor sets both `flags` and
`context_flags` the way the pin does.

## Witness

- `crates/elm-effect-unstable/tests/cli_completions_bash.rs::generates_valid_bash_script_for_comprehensive_cli` (`:814`)
- `crates/elm-effect-unstable/tests/cli_completions_zsh.rs::generates_valid_zsh_script_for_comprehensive_cli` (`:854`)
- `crates/elm-effect-unstable/tests/cli_completions_fish.rs::generates_valid_fish_script_for_comprehensive_cli` (`:885`)

Each mirrors the pin's own assertion set, and `comprehensive` is registered in
the three `*_goldens()` lists so the byte-equality tests cover it too.

## Verify

`cargo test -p elm-effect-unstable --test cli_completions_bash --test cli_completions_zsh --test cli_completions_fish`

## Why no modules.toml change

`completions.test.ts` is linked to no module in `conformance/modules.toml`, so
flipping these rows moves no module status. Nothing to lease.
