# elm-effect-rs

Effects, structured concurrency, and an Elm loop for Rust. An `Effect<A, E>`
is a description until `Runtime::run` runs it. Failures are values in
`Cause<E>`. Fibers are structured. Long-lived programs use the Elm loop;
one-shots are a single effect at the process edge.

This repository depends on **no** sibling (flywheel / toron / beads /
chiebukuro). They adopt it later through their own records. Carrier law:
[ADR-0002](docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md).
Adoption/extension: [ADR-0003](docs/decisions/0003-make-elm-effect-a-first-class-rust-framework.md).

Agent path: [docs/guides/14-agents.mdx](docs/guides/14-agents.mdx) →
[15-use-the-apis.mdx](docs/guides/15-use-the-apis.mdx) → the topic guide.
Migrate/build skill: [skills/elm-effect/SKILL.md](skills/elm-effect/SKILL.md)
(also linked from `~/.agents/skills/elm-effect`). That skill is also the
repo's Rust practice pointer for this workspace.
Formal flows & statecharts: [docs/research/state-machines-and-flows.md](docs/research/state-machines-and-flows.md),
[skills/verify-framework-flows/SKILL.md](skills/verify-framework-flows/SKILL.md),
and [skills/verify-signal-latch/SKILL.md](skills/verify-signal-latch/SKILL.md).

## Crates

| Crate | Role |
|-------|------|
| `elm-effect` | Effects, fibers, scopes, layers, services, schedules, sync, clocks |
| `elm-effect-elm` | `Program`, `Cmd`, `Interpret` — pure fold |
| `elm-effect-journal` | Durable logs for commands that must survive restart |
| `elm-effect-durable` | Durable engine port, our single-host engine, and pure cron timing (ADR-0005) |
| `elm-effect-test` | `TestClock` and helpers |
| `elm-effect-lints` | Optional lints |

## Commands

Nightly edition 2024 (`rust-toolchain.toml`). Prefer `mbx` when present
(sibling stack convention); CI uses bare `cargo`.

```bash
mbx test --workspace          # or: cargo test --workspace
mbx clippy --workspace --all-targets -- -D warnings
mbx fmt --check
```

## Effect-oriented rules

Binding skill: `~/.agents/skills/effect-oriented/SKILL.md`. Carrier law is
ADR-0002 — do not ask whether it is accepted.

- Return `Effect` from library code. Call `Runtime::run` only in `main` or a test.
- Keep reducers pure: `Program::update` / fold takes `(model, msg)` and returns
  `(model, commands)`. Commands are data; the interpreter turns them into effects
  at the boundary.
- I/O belongs at the edge. When an effect finishes, its outcome comes back as a
  `Msg` into the fold — never mutate the model from inside an effect.
- Keep `Fail`, `Die`, `Interrupt`, and park apart.
- The `can_transition` table has exactly 18 directed edges; `Parked` has 0 table edges.
- Put services in tags. Call `provide` at the edge.
- Use `policy` or `Schedule` for retry — do not sleep in a loop.
- `TestClock` order: fork → adjust → join.
- A durable append uses one `epoch` and one `seq` for one retry of the same append.
- Do not rebuild `.beads/gates.jsonl` from the issues log (beads invariant).
- A sibling removes Resonate only through its own adoption record. This repo's
  engine replaces it (ADR-0005); do not restore a vendor adapter or add a new
  one.
- A module earns its place when a caller uses it: type/function + failing test
  + guide entry. Do not mirror another library's file layout.
- Extension from an app: add a `ServiceTag`, a `Program`, or a `DurableLog` —
  not a second scheduler. Full rule:
  [13-add-a-capability.mdx](docs/guides/13-add-a-capability.mdx).

## Effect law for tests

A change is unfinished until determinism and interpreter proofs cover success,
typed failure, hang, and cancellation without real sleep or live network.

## Do not resurrect

Tokio is a plain dependency (no `tokio` feature flag). Do not bring back
`elm-effect-unstable`, the conformance ledger, or the `schema` / `stm`
features from ADR historical text without a new decision. Proofs live under
`crates/*/tests/`.
