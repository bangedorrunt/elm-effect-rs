---
tier: perishable
tags: [bd-iqte, kernel, cause-model, wave-1]
bead: elm-effect-rs-epic-elm-effect-port-0c6.2
wave: 1
created: 2026-10-03
verify: cargo test -p elm-effect --test l2_cause
verify_result: ok (5/5; re-run 2026-10-03 exit 0)
---
# Cause model owns the failure vocabulary at the crate edge

## Solution
`cause.rs`: `Cause<E> { reasons: Vec<Reason<E>> }`, `Reason = Fail(E) |
Die(Defect) | Interrupt`. `Cause::empty` is an empty vector (no
Empty/Sequential/Parallel variants). `Defect` carries message + backtrace and
is not an `E`. `never` is `Infallible`, not `()`. No-default-features build
stays green with no tokio; zero `unsafe`.

## Why it matters for later wave-1 beads
Fiber (.4), scope, and executor beads all consume `Cause`/`Exit`. Any new
reason kind or `E`-collapse breaks every consumer. Keep failure vocabulary
changes inside `cause.rs`, never at call sites.

## Principles
boundary-discipline — cause model owns the failure vocabulary at the crate edge.
prove-it-works — RED (exit 101 unresolved import) before GREEN, receipt filed.
