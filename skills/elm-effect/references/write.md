# Write inside elm-effect-rs

No sibling in the utterance. Implement the next phase whose law tests are red. Phase order is ADR-0002. Tests for the phase land first and fail, then the implementation.

## Pick the phase

Run the law tests. The first red phase is the work. Do not start a later phase to look busy.

| Red | Build |
| --- | --- |
| No `conformance/PIN`, or `cargo test -p elm-effect-conformance` is red | Phase 0. The ignored test stays red until phase 13 |
| L1, L2, L6, L7 | Phase 1 |
| L5, sixteen-service check | Phase 2 |
| L4, L8, L9, L10 | Phase 3 |
| `all_without_concurrency_is_sequential` | Phase 4 |
| Stream ledger rows | Phase 5 |
| Schema corpus | Phase 6 |
| Single-flight and batch | Phase 7 |
| L12, L15 | Phase 8 |
| L14, `golden_transitions` | Phase 9 |
| L16 | Phase 9b, and read `references/durable-log.md` |
| Later phases | The ADR phase with that number |

## Shape

- A library function returns `Effect`. `Runtime::run` appears only in the allow-listed crates.
- `?` carries the typed error. A panic is `Die` at the supervisor.
- `policy::remote` is the retry stack. A `for` loop that sleeps inside a service is the wrong shape.
- A long-lived loop implements `Program`. A one-shot CLI does not.
- Commands are `Serialize`. The interpreter is the only place an effect starts.

## Done

- [ ] The phase's named tests pass.
- [ ] `cargo test --no-default-features -p elm-effect` passes with no network, once that crate exists.
- [ ] The report names the phase and the tests. It does not ask which phase is next.
