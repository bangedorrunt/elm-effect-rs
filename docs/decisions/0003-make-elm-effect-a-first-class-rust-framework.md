---
status: accepted
date: 2026-10-04
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: ADR-0002, the elm-effect crates in this repository, the effect-oriented laws
informed: Rust programmers and agents that use elm-effect
---

# Make elm-effect a first-class Rust framework

> **Amended 2026-10-24 (minimalism audit).** There are no facade features.
> Tokio is a plain dependency, and there is no `full`, `schema`, or `stm`
> feature. There is no `elm-effect-unstable` crate. The feature paragraphs below
> describe the pre-cut state and are kept only as history.
>
> The same cut removed the port ledger: `conformance/`, the
> `elm-effect-conformance` crate, `skills/elm-effect/references/port-module.md`,
> and the `Predicate`/`Matcher`/`Filter`/`Differ`/`Tagged` foundation types.
> The tests stay; the port bookkeeping does not. References to the ledger below
> are historical.

## Context and Problem Statement

ADR-0002 specifies the carrier, the laws, and the behavior ledger. A Rust programmer does not read that record to send one retry. The programmer adds a crate, reads rustdoc, and stops when the compiler or rust-analyzer fails.

The effect-oriented laws are already the laws L1 through L16 in ADR-0002. An application extends the framework when it adds a value of a framework type. The framework runs that value. The framework does not depend on the application.

How does a Rust programmer adopt the crate? How does an application add a capability that the framework can run?

## Decision Drivers

* The compiler channel is nightly. The four Rust repositories already float nightly. A stable pin would split this crate from them.
* A short first compile. Default features that include schema and STM make `cargo add` slow.
* The ecosystem realities are `Result`, `Future`, and `tracing`. Only `Result` is wired into the kernel; `Future` and `tracing` are host concerns in this envelope.
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
* There are no facade features. Tokio is unconditional; the fiber runtime is the carrier.
* Put `#[must_use]` on `Effect`. The compiler must report a drop of a description.
* `Effect::from_result` is the Result door. `Runtime::run` and `ManagedRuntime::run` are the edges. There is no `try_future`/`into_future` adapter in the kernel envelope; an async host bridge needs an executor contract and is future work.
* Logging and spans are in-process `observability` values. There is no `tracing` dependency and no exporter in the kernel.
* `elm_effect::prelude` exists with the core carrier, policy module, and `effect!` macro. The root remains canonical.
* The guides are the examples. There is no separate `examples/` crate in this workspace.
* `elm-effect-lints` is an optional crate. It reports `Runtime::run` outside `main` and tests. It reports `HashMap` in a `Program::Model`. It reports `tokio::time::sleep` in effect code. Default `cargo build` stays quiet.
* The kernel envelope is the semver surface: `Effect`, `Scope`, `policy`, `TestClock`, streams, and the journal seam. It does not claim the full Effect standard library.

The `effect!` macro is part of this surface. It puts a missing-service error on the line that uses the service, and rust-analyzer shows `R` after expansion.

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

A new combinator that changes fiber steps belongs in this repository. Add a witness named by the law or machine matrix. An application does not add that combinator in secret.

### Consequences

* Good, because a programmer can adopt the crate from rustdoc and three examples.
* Good, because an application extends the framework with types the runtime already runs.
* Neutral, because there are no feature toggles to migrate.
* Neutral, because ADR-0002 owns the carrier laws and the kernel envelope.

## Implementation Plan

* **Affected paths**: `crates/elm-effect/Cargo.toml`, `crates/elm-effect/src/lib.rs`, `crates/elm-effect/src/effect.rs`, new `crates/elm-effect-lints`, `examples/`, `docs/guides/`.
* **Dependencies**: `thiserror` and Tokio. No `tracing` dependency, no exporter, and no new default heavy crates.
* **Patterns to follow**: ADR-0002 laws L1 through L16. `ServiceTag` in `crates/elm-effect/src/service.rs`. `DurableLog` in `crates/elm-effect-journal/src/durable_log.rs`. `Program` in `crates/elm-effect-elm/src/lib.rs`.
* **Patterns to avoid**: A second runtime in the application. A `TypeId` map as the requirement channel. Nightly-only public API. A guide that tells the reader to learn ADR-0002 before the first program.
* **Configuration**: no facade features; Tokio is a plain dependency.
* **Migration steps**: Guides in `docs/guides/` are the user text. They name the types that exist today. The macro exists and is exercised by `crates/elm-effect/tests/public_surface.rs`.

### Verification

- [ ] The guide call sites and `crates/elm-effect/tests/public_surface.rs` compile on the nightly channel in `rust-toolchain.toml`.
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
