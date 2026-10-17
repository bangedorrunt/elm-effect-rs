# elm-effect-rs

A Rust port of [Effect 4.0.0](https://github.com/Effect-TS/effect/releases/tag/effect@4.0.0), plus an Elm loop, for the flywheel, toron, beads, and chiebukuro stack.

The law is [ADR-0002](docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md). Adoption is [ADR-0003](docs/decisions/0003-make-elm-effect-a-first-class-rust-framework.md). The steps are [skills/elm-effect](skills/elm-effect/SKILL.md). The user guides are [docs/guides](docs/guides/README.mdx).

An agent told to build an app or CLI, to port an Effect module, or to migrate flywheel, toron, beads, or chiebukuro follows the skill and does not ask whether the record is accepted. The status word stays `proposed` until the operator types accepted.

## License

MIT, see [LICENSE](LICENSE). elm-effect-rs is a port of [Effect](https://github.com/Effect-TS/effect) (MIT, Copyright (c) 2023 Effectful Technologies Inc); the upstream notice is kept in [NOTICE](NOTICE).
