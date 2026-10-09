# Port or add a module

Read `docs/guides/15-use-the-apis.mdx` and `docs/guides/13-add-a-capability.mdx`. One Effect module, or one new module this stack needs that Effect does not have. A product on top of a module that already exists is `build-app.md` or `build-cli.md`.

The pin is `conformance/PIN`. Read the TypeScript at that tag, not `main`. Placement and stability are ADR-0002, sections "What 100% feature and test parity means", "Crate map", and "Platform substitutes".

## Where it goes

Look up `conformance/modules.toml` by the file path.

| Ledger class | Crate | Public path |
| --- | --- | --- |
| `port` | `crates/elm-effect` | `elm_effect::<Module>` |
| `unstable-port` | `crates/elm-effect-unstable`, feature = the directory name (`http`, `sql`, `workflow`, `ai`, `cli`, …). A top-level file with an unstable tag uses feature `unstable` | `elm_effect::unstable::<module>` |
| `experimental-port` | same unstable crate. A patch may break it | same |
| `native` | nowhere | say the Rust feature in the row's reason, and stop |
| `platform-foreign` | nowhere for the host adapter | the service trait, if the module is the trait, follows the class of that trait's file |

No row, and the file exists at the pin: run `conformance/inventory.sh` on a checkout of the pin and commit the regenerated `modules.toml`. Do not type a row by hand.

No file at the pin: this is an extension, not a port. Put it in `elm-effect-unstable` under feature `ext`. Do not add a ledger row. Do not mark a fictional Effect path `matched`.

## Gate

The carrier is in the crates. Do not rebuild a phase whose tests pass. A destination crate that does not exist yet is created with the path in the table above.

## Order

1. Read the `.ts` file at the pin and the behavior rows for that file.
2. Carry a row only when the task named that behavior. A carried call becomes a Rust test with the same observable result. A carried `it.each` row runs every tuple in its table. A test that only asserts the function exists does not port the row. A row we will not carry is `not-applicable` with a reason. Do not open the next Effect file.
3. Implement until that test passes. `Equal` for domain values treats `NaN` as equal. `f64`'s `==` stays IEEE. Waits go through `Clock`. Direct `tokio::time::sleep` inside the module is a bug.
4. A service the module introduces is a new tag. It does not change the fiber state machine.
5. Helpers that exist in the TypeScript file only to simulate traits (`pipe`, HKT, `Yieldable`) are not ported. A trait is the port.
6. Set a carried row to `ported` only after its Rust test was red and then green, and put that test's path in the `rust` column. A `ported` row with an empty `rust` column fails the ledger test. Leave uncarried rows `not-applicable` with a reason.
7. `cargo test -p` the crate that received the module.

## Done

- [ ] Each behavior the task named is `ported`, and a stub fails that Rust test.
- [ ] Each behavior we will not carry is `not-applicable` with a reason.
- [ ] `cargo test -p` the destination crate passes.
- [ ] The ledger was not made green by deleting rows.
