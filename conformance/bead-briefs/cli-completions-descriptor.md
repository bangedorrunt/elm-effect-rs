# Port the cli completion descriptor rows

Source rows: `packages/effect/test/cli/completions/descriptor.test.ts` (20 rows).

The pin's descriptor (`cli/internal/completions/descriptor.ts`) flattens a
command tree into plain data for shell completion generation. The port adds
`crates/elm-effect-unstable/src/cli_completions.rs`: `DescriptorCommand` in,
`CommandDescriptor` out, reusing `cli::primitive::PathKind` and the pin's
`Primitive` tag names rather than inventing a second value model.

## Witness

`crates/elm-effect-unstable/tests/cli_completions.rs` — one test per row, each
carrying its upstream line.

## Scope boundary

Only the descriptor. The shell script generators
(`cli/internal/completions/{bash,zsh,fish}.ts`, 70 further rows) and the
`Completions.generate` dispatch stay unported; the module doc names them.

## Why no modules.toml change

`descriptor.test.ts` is linked to no module in `conformance/modules.toml`, so
flipping these rows moves no module status. Nothing to lease.
