# elm-effect-rs

A Rust port of [Effect 4.0.0](https://github.com/Effect-TS/effect/releases/tag/effect@4.0.0), plus an Elm loop, for the flywheel, toron, beads, and chiebukuro stack.

The law is [ADR-0002](docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md). The steps are [skills/elm-effect](skills/elm-effect/SKILL.md).

An agent told to build an app or CLI, to port an Effect module, or to migrate flywheel, toron, beads, or chiebukuro follows the skill and does not ask whether the record is accepted. The status word stays `proposed` until the operator types accepted.
