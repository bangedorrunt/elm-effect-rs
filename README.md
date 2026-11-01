# elm-effect-rs

Effects, structured concurrency, and an Elm loop for Rust.

An `Effect<A, E>` is a description: it does nothing until `Runtime::run` runs
it. Failures are values in `Cause<E>`, not panics. Fibers are structured, so a
scope waits for its children and interruption is a real outcome.

## Crates

| Crate | What it is |
| --- | --- |
| `elm-effect` | Effects, fibers, scopes, layers, services, schedules, sync primitives, clocks |
| `elm-effect-elm` | `Program`, `Cmd`, `Interpret`: a message log folded into a model |
| `elm-effect-journal` | Durable logs for commands that must survive a restart |
| `elm-effect-durable` | Durable engine port, single-host engine, and pure cron timing (ADR-0005) |
| `elm-effect-test` | `TestClock` and test helpers |
| `elm-effect-lints` | Optional lints for effect code |

## Start here

Read [docs/guides/15-use-the-apis.mdx](docs/guides/15-use-the-apis.mdx), then
the topic guide beside it. [docs/guides/14-agents.mdx](docs/guides/14-agents.mdx)
is the same path for an agent.

The design records are
[ADR-0002](docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md)
(the carrier and its laws),
[ADR-0003](docs/decisions/0003-make-elm-effect-a-first-class-rust-framework.md)
(adoption and extension), and
[ADR-0005](docs/decisions/0005-durable-engine-port-and-our-engine.md)
(the durable engine, cron timing, and vendor removal).

## License

MIT, see [LICENSE](LICENSE). The carrier derives from
[Effect](https://github.com/Effect-TS/effect) (MIT, Copyright (c) 2023
Effectful Technologies Inc); the upstream notice is kept in [NOTICE](NOTICE).
