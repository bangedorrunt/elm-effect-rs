# A red test in elm-effect-rs

The carrier from ADR-0002 and the adoption rules in ADR-0003 are in the crates. Do not re-implement a phase whose tests pass.

## Pick the failure

1. Run `cargo test -p elm-effect`.
2. If that command fails, fix the failing test. Read the guide for that API in `docs/guides/` before you edit.
3. If only `cargo test -p elm-effect-conformance -- --ignored` fails, that is the ledger index. Follow `references/port-module.md` only for a behavior the task named. Mark the rest we will not carry as `not-applicable` with a reason. Do not mark them `ported`.
4. Do not start a later phase because the ledger index is open.

## Shape

- Library code returns `Effect`. `Runtime::run` stays in `main` and tests.
- `effect!` binds with `let`. Do not write `?` after an effect in the macro.
- `Remote::call` or `retry` is the retry path. A `for` loop that sleeps in a service is the wrong shape.
- A long-lived loop implements `Program`. A CLI does not.

## Done

- [ ] `cargo test -p elm-effect` passes.
- [ ] You did not flip feature defaults in `Cargo.toml`.
- [ ] The report names the test that was red and the test that is green.
