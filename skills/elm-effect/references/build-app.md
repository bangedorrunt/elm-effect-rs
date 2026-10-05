# Build an application

Read `docs/guides/11-application.mdx` and `docs/guides/15-use-the-apis.mdx` before you write code. A one-shot tool is `build-cli.md`. Laws L12, L15, L14, and L16 stay in ADR-0002. If a crash must resume the work, read `references/durable-log.md` after this file.

## Crate

The application is its own crate. It depends on `elm-effect`. `elm-effect` does not depend on it. Do not put the product inside `crates/elm-effect*`.

Default layout. Keep a name that already means the same role.

```text
src/main.rs        layer graph, Runtime::run_main
src/program.rs     Program::init and Program::update
src/cmd.rs         command enum, serde
src/interpret.rs   Interpret, the only place an Effect is built
src/services.rs    service tags and the trait each tag's Shape is
```

## Order

1. Write `update` as a pure function. The same model and message produce the same model and the same command list. No `SystemTime`, `std::env`, or `thread_rng` in that function. Time and randomness arrive as messages. Parsed config arrives as `Boot`.
2. Commands are an enum that implements `Serialize` and `Deserialize`. A command the fold did not return does not run.
3. Each service the domain needs is a zero-sized tag with its own string id. Two capabilities are two tags, even when one client could do both. The layer that builds them is composed in `main`, once.
4. `interpret` turns one command into one `Effect`. Remote calls go through `policy::remote` around that call, not around the whole fold.
5. A command that must survive a restart returns `Durability::Durable { key }`. The key is stable across a retry. Then read `references/durable-log.md`. One JSON stream per program run. The snapshot stored beside it is `{model, offset}`.
6. A command that is a pure in-memory read stays `Durability::Ephemeral` and is not journaled.
7. Subscriptions receive model snapshots. The handle has no way to write the model.
8. Tests construct the program, feed a message log, and assert the next model and the command list. Clock tests fork the sleeper, adjust `TestClock`, then join. They do not sleep on the live clock.

## Done

- [ ] `update` has a replay test: the same log twice, the same commands.
- [ ] Dropping a command value performs no I/O.
- [ ] `Runtime::run` appears in `main` and in tests, not in `program.rs`.
- [ ] `cargo test` for the application crate passes with no network. Live calls are `#[ignore]`.
