# Fiber-first edge: the Elm loop and ManagedRuntime

Date: 2026-10-24. Scope: `crates/elm-effect/src/runtime.rs`,
`crates/elm-effect-elm/src/lib.rs`, `crates/elm-effect/src/attempt_plan.rs`.

> **Later on 2026-10-24:** the three boundaries below were closed. The static
> `Runtime::run` now runs on a fiber, `Effect::all`/`all_discard` are the
> effect-level fan-out, and `Scope` finalizers can see the exit. See
> `2026-10-24-durable-streams-and-fiber-edge.md`.

## The problem

Effect 4 runs every effect in a fiber. This crate's default managed edge did
not: `ManagedRuntime::run` called `effect.run_sync_with` on the caller's
thread, and `Loop::run_effects` ran commands on scoped OS threads. The
structured combinators (`with_fiber`, `fork_child`, `race`, `timeout`) read
the ambient `FiberTag`, so they compiled at those call sites and then failed
with "outside a fiber". The runtime had fibers; the two places that mattered
did not use them.

## What changed

- `ManagedRuntime::run` now delegates to `run_fork(effect).join()`. The
  effect gets a fiber, a supervisor, and finalizers. `run_fork` accepts any
  requirement channel `R`, not only `()`.
- `Loop::run_effects` forks every command in a window with `run_fork`, then
  joins the handles in command order. A command can use the fiber-only
  combinators, and its panic becomes a defect on that command's exit.
- `ExecutionPlan` is `AttemptPlan`. The local type is a bounded attempt
  budget; Effect 4's multi-step `ExecutionPlan` (fallback layers, `while`,
  schedule, metadata) is not ported. `NoAttempts` now reports the local name.
- An `AppendError::Unavailable` answer is reconciled against the log before
  a retry. A landed append with a lost acknowledgement is accepted; a
  matching producer triple with a different payload is `PayloadMismatch`.
  An answer that stays unknown reaches the fold as `Failure::Unknown`, not
  as the false claim `NotAppended`.

## Evidence

- `crates/elm-effect/tests/effect_conversions.rs`:
  `managed_runtime_run_gives_the_effect_a_fiber`,
  `managed_runtime_run_supports_race_at_the_edge`.
- `crates/elm-effect-elm/tests/loop_fiber.rs`: every command in a window has
  a fiber, and a command can fork and join a child.
- `crates/elm-effect-elm/tests/loop_append_reconcile.rs`: lost ack, retry
  after a miss, permanently unknown, and payload mismatch.
- `cargo test --workspace`: 1,243 passed, 0 failed, 94 suites.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check` and `git diff --check`: clean.

## Boundaries and deferred work

- `Runtime::run` (the static convenience) still runs on the calling thread
  with no fiber. Use `ManagedRuntime::run` when the effect needs structured
  concurrency. The guide now says so.
- `Scope` finalizers still do not receive the exit. `Effect::acquire_use_release`
  already does, and no current resource needs the scope-level version.
- `Effect::all` is still not an effect combinator. The runtime-level
  `concurrency::all` / `all_result` cover edge fan-out; add an effect-level
  version only when a real batch needs it.
- `Context` / `ContextReference` remain a ported, tested primitive. The live
  ambient mechanism is `Reference`; cutting `Context` is a separate surface
  decision.
- `Program` still has no `Sub`. The host-pumped boundary is deliberate; add
  declarative subscriptions only if the loop is meant to own long-lived
  inputs.
