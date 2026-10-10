---
tier: perishable
tags: [elm, services, interpret, send, port]
bead: elm-effect-rs-uqh
created: 2026-10-10
verify: cargo test -p elm-effect-elm --test loop_joint
verify_result: ok (14 passed)
---
# A static interpreter needs a seam, not a global

## Solution

`Interpret::interpret` is static, so it has no `self` and no parameters a
host could use to hand it the world. That made `use_service` inside one a
dead end: the tag could never be closed, because `provide` needs a *value*
and the trait gives none, and a runtime built by `ManagedRuntime::make` had
its layers read back through `context()` for tests and never consulted at a
run. `Loop::run_effects` did reach `ManagedRuntime::run`, but that method
delegated to the static `Runtime::run` and threw its own `services` away.

Three pieces close it, all backwards compatible:

- `Effect::provide_from(&BuiltServices)` closes tag `T` out of the runtime's
  built services instead of out of a value in hand. Fail-closed: a tag the
  layer never built is `missing service: <id>` at the run edge, never a
  default value.
- `Effect::map_err(f)` translates the typed failure channel via the existing
  `Cause::map`, so `use_service` (always `Infallible`) can join the `Err` a
  `Program` declares. Defects and interrupts pass through untouched.
- `Interpret::interpret_in_env(cmd, Option<&BuiltServices>)` with a default
  that ignores the services and calls `interpret`, so every existing
  interpreter keeps working unchanged. `Loop::run_window` passes its own
  `context()`.

## The Send question, settled

`docs/learnings/inbox/2026-10-04-elm-send-bounds.md` records the `Send`
bounds as deliberate, and they stay. A `!Sync` world does not need them
lifted: it needs to ride as a *service value*, which must be
`Clone + Send + Sync`. An `Arc<Mutex<store>>` handle is all three even when
the store is not, which is what `06-services.mdx`'s "put a socket in an Arc"
was already pointing at. beads hit this porting a CLI over a
`RefCell`-backed `rusqlite` store: `&SqliteStorage` is `Send` only if the
store is `Sync`, so no closure could capture it — but a handle can be
provided, and that is the door.

## One claim worth checking before repeating

Providing one tag twice with distinct values is *not* reachable through the
public type-level API, so there is no witness to write for it. It looks
reachable: `e.provide::<T>(x).provide_from::<T>(&built)` reads like it
should compile. It does not — `provide_from` requires `R: Provide<T, Rest>`,
and `()` implements `Provide` for no tag at all, so the chain is a compile
error rather than a poisoned run. The `ambiguous tag` branch stays as
defence in depth for the internal merge paths, with no public witness.
Verified by a scratch crate, not by reasoning about the bounds.

## Principles

prove-it-works — every deliverable names a running test.
boundary-discipline — the interpreter is handed the world rather than
reaching for it.
separate-before-serializing-shared-state — a missing service tag is a named
defect, never a default value.
