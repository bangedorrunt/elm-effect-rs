---
status: proposed
date: 2026-10-04
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: ADR-0002, the elm-effect crates in this repository, the effect-oriented laws
informed: Rust programmers and agents that use elm-effect
---

# Make elm-effect a first-class Rust framework

## Context and Problem Statement

ADR-0002 specifies the carrier, the laws, and the behavior ledger. A Rust programmer does not read that record to send one retry. The programmer adds a crate, reads rustdoc, and stops when the compiler or rust-analyzer fails.

The effect-oriented laws are already the laws L1 through L16 in ADR-0002. An application extends the framework when it adds a value of a framework type. The framework runs that value. The framework does not depend on the application.

How does a Rust programmer adopt the crate? How does an application add a capability that the framework can run?

## Decision Drivers

* The compiler channel is nightly. The four Rust repositories already float nightly. A stable pin would split this crate from them.
* A short first compile. Default features that include schema and STM make `cargo add` slow.
* The ecosystem doors are `Result`, `Future`, and `tracing`.
* rust-analyzer must show the requirement parameter `R`.
* An application adds a service, a layer, a schedule, a program, or a log. It does not fork the runtime.
* Version 1.0 must name a small stable surface. The behavior ledger can grow after that.

## Considered Options

* Keep the ADR-0002 defaults: nightly, and default features `tokio`, `schema`, and `stm`. Document the crate only in ADR-0002.
* Make the public API a small semver surface. Keep nightly as the compiler. Let applications extend the crate with framework types.

## Decision Outcome

Chosen option: "Make the public API a small semver surface." The compiler stays nightly. ADR-0002 remains the law for the carrier. This record is the law for adoption and for extension.

### Public crate

* Keep `rust-toolchain.toml` on `channel = "nightly"`. Do not set `rust-version` to a stable compiler. Re-run clippy after a nightly bump, the same way the four sibling repositories do.
* Default features are `tokio` only. `schema` and `stm` stay available behind the feature `full`.
* Put `#[must_use]` on `Effect`. The compiler must report a drop of a description.
* Add `Effect::from_result` and `Effect::try_future` as the doors in. Add `Effect::into_future` as the door out. Only `main`, a server handler, and tests call `into_future` or `Runtime::run`.
* Send logs and spans through the `tracing` crate. The `Logger` service forwards to `tracing`. It does not replace it.
* Add `elm_effect::prelude` with `Effect`, `Layer`, `Schedule`, `Runtime`, `use_service`, and `policy`.
* Ship three examples: a library function that returns `Effect`, a CLI that uses `Runtime::exit_code`, and one `Program` that replays a message log.
* Ship `elm-effect-lints` as an optional crate. It reports `Runtime::run` outside `main` and tests. It reports `HashMap` in a `Program::Model`. It reports `tokio::time::sleep` in effect code. Default `cargo build` stays quiet.
* Version 1.0 covers the kernel, `policy`, `Scope`, and `TestClock`. `elm-effect-unstable` stays off by default. The behavior ledger continues after 1.0.

The `effect!` macro is part of this surface. It must put a missing-service error on the line that uses the service. rust-analyzer must show `R` after the macro. Until the macro exists, guides show `flat_map` and `use_service`.

### Effect-native extension

An application does not edit the fiber runtime to add a capability. It adds a value. The framework already knows how to run that value.

| You add | Framework type | The framework does |
| --- | --- | --- |
| A capability | `ServiceTag` plus a `Layer` or `provide` | Supplies the service. A missing tag is a compile error |
| A retry or a timeout | `Schedule` or `RemotePolicy` | Runs the policy around the call |
| A long-lived process | `Program`, `Cmd`, `Interpret` | Folds messages and runs commands |
| A durable store | `DurableLog` | Appends and reads with the L16 rules |
| A pure decision | An ordinary function | Stays outside `Effect` until a call can fail or wait |

The application crate depends on `elm-effect`. `elm-effect` does not depend on the application. Two applications can supply two `ServiceTag` values with two string ids. The same runtime runs both.

A new combinator that changes fiber steps belongs in this repository. Add a behavior row from the Effect tag, or put the module behind the `ext` feature. An application does not add that combinator in secret.

### Consequences

* Good, because a programmer can adopt the crate from rustdoc and three examples.
* Good, because an application extends the framework with types the runtime already runs.
* Bad, because default features change. A crate that relied on schema in the default build must enable `full`.
* Neutral, because ADR-0002 still owns the laws and the behavior ledger.

## Implementation Plan

* **Affected paths**: `crates/elm-effect/Cargo.toml`, `crates/elm-effect/src/lib.rs`, `crates/elm-effect/src/effect.rs`, new `crates/elm-effect-lints`, `examples/`, `docs/guides/`.
* **Dependencies**: `tracing` on the facade. No new default heavy crates.
* **Patterns to follow**: ADR-0002 laws L1 through L16. `ServiceTag` in `crates/elm-effect/src/service.rs`. `DurableLog` in `crates/elm-effect-journal/src/durable_log.rs`. `Program` in `crates/elm-effect-elm/src/lib.rs`.
* **Patterns to avoid**: A second runtime in the application. A `TypeId` map as the requirement channel. Nightly-only public API. A guide that tells the reader to learn ADR-0002 before the first program.
* **Configuration**: default feature `tokio`. Feature `full` enables `schema` and `stm`.
* **Migration steps**: Guides in `docs/guides/` are the user text. They name the types that exist today. They name the macro as future work from this record.

### Verification

- [ ] `cargo check` of `examples/` succeeds on the nightly channel in `rust-toolchain.toml`, with default features.
- [ ] A dropped `Effect` causes a `must_use` report from the compiler.
- [ ] `ServiceTag` from an external crate compiles and `provide` removes the tag.
- [ ] `MemLog` and a second `DurableLog` implement pass the same L16 tests.
- [ ] `docs/guides/README.mdx` links each guide, and the guides obey simplified technical English.

## Pros and Cons of the Options

### Keep the ADR-0002 defaults

* Good, because the port crate stays aligned with the nightly toolchain of the four Rust repositories.
* Bad, because programmers outside that toolchain cannot start.
* Bad, because the only manual is the decision record.

### Small public API on nightly, and effect-native extension

* Good, because extension is a new value, not a fork.
* Good, because version 1.0 has a boundary.
* Bad, because the macro and the lints are more work after the behavior port.

## More Information

The effect-oriented skill is the source of laws L1 through L16. This record does not copy the skill. It says where an application is allowed to extend the carrier.

User guides: `docs/guides/`.
