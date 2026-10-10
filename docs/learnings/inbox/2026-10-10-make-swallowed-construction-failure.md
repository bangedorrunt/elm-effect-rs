---
tier: perishable
tags: [elm, layer, runtime, scope, drop]
bead: elm-effect-rs-98v
created: 2026-10-10
verify: cargo test -p elm-effect --test layer
verify_result: ok (9 passed)
---
# `ok()` on a fallible boundary is a fail-open, and a scope dies with the call

## Solution

`ManagedRuntime::make` did `layer.build(&scope).ok()` under a doc comment
promising "A failing layer surfaces as a defect on the built runtime". It
did the opposite: the `Cause` was dropped, `services` became `None`, and
`None` is indistinguishable from `ManagedRuntime::new()`. A services
interpreter then reported "no services" instead of the cause. Verified with
a probe before believing the doc.

The fix is the pin's shape, not a Rust convention: `ManagedRuntime.make`
there yields an `Effect`, so construction failure fails the build and the
caller cannot ignore it. Returning `Result<Self, Cause<E>>` is what keeps
that property. Seven call sites, all tests.

Second defect, same function, same root cause: `let scope = Scope::new(0);`
was a local, so it dropped at the end of `make` — and `Scope::drop` calls
`close()`, which runs every finalizer registered during the build. Any
resource a layer acquired would have been released before the runtime's
first effect ran. The runtime now owns the scope. Drop order does the rest:
`Runtime` (shutdown interrupts live fibers) drops before `build_scope`
(finalizers release), which is the only correct order.

## The candidate I was wrong about

A panic in `Interpret::interpret` runs outside `run_effects`'s
`catch_unwind`, which looked like a hole in the fold's own guarantee. I
proved it with a probe — the process panicked, the loop never survived —
and then found `loop_joint.rs:596`, a `#[should_panic]` test pinning the
opposite contract with the reason in a comment: a panic in the interpreter
is a bug *in the interpreter* and must reach the host rather than become a
message. That is the effect-oriented law (a handler for typed failures must
not swallow defects), so the pinned behavior is right and my fix was wrong.
Reverted, no change.

Two lessons, both general: a `#[should_panic]` test is a contract, not a
test to update; and a doc comment describing intent is not evidence —
`ok()` silently contradicted one here.

## Principles

prove-it-works — the bug was reproduced with a running probe before the fix,
and the probe passes after.
separate-before-serializing-shared-state — a construction failure must reach
the caller, not collapse into a runtime that looks like `new()`.
outcome-oriented-execution — a resource a layer acquired must outlive the
call that built it.
