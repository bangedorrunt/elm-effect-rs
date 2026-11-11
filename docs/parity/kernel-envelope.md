# Kernel envelope

Status: accepted 2026-11-11. This page is the boundary of the framework's
Effect v4 claim. It is deliberately smaller than a full port.

## Claim

The executable claim is:

- **16 laws**, mapped to real witnesses by `crates/elm-effect/tests/law_matrix.rs`.
- **32 curated verbs**, not the whole public API. `Effect` exposes roughly 70
  public functions and `Stream` roughly 55 methods.
- **21 machines**, tracked by `crates/elm-effect/tests/machine_matrix.rs`.
- **19 TLA+ models** and one exhaustive enumeration, run by
  `scripts/verify-formal.sh`.
- **11 Effect-sourced laws** and **5 native laws**: L3, L12, L14, L15, and L16.
  L16 also carries two native carve-outs. Native laws are framework laws, not
  claims about upstream Effect behavior.

The law matrix passes with:

```text
law matrix: 16 laws, 81 witnesses/verbs, 32 verbs, 2 native carve-outs
machine matrix: 21 machines, 20 modeled rows, 19 TLA models, 1 exhaustive, 21 witnesses
```

## Included public surface

The kernel exposes the carrier and the primitives the framework actually runs:

- `Effect`, `Exit`, `Cause`, `Defect`, `Runtime`, `ManagedRuntime`
- fibers, `FiberHandle`, scopes, interruption, `TestClock`
- `Layer`, `ServiceTag`, `provide`, typed service requirements
- `Schedule`, `policy`, `Remote`, breaker, bulkhead, rate limiting, hedging
- `Ref`, `SynchronizedRef`, `FiberRef`, `Deferred`, `Queue`, `Semaphore`, `PartitionedSemaphore`
- `Cache` and `ScopedCache` (single-flight, TTL, scope-cleared entries)
- `Pool` (bounded reuse with an RAII guard) and `Latch` (fiber-aware gate)
- `Request`, `Resolver`, and `Entry` (automatic batching, grouping, delay,
  caching, persistence, and cancellation)
- `RcMap` and `RcRef` (keyed reference-counted resources, idle TTL, capacity,
  and generation-safe invalidation)
- `Stream`, `Pull`, `Take`, `Channel`, `SubscriptionRef`, `Sink`
- `Stream::from_change_stream` and `Stream::from_pubsub` as native bridges to
  the existing event primitives
- `FiberSet`, `FiberMap`
- `Config`, `ConfigProvider`
- `Crypto`, `TestCrypto`, `SystemCrypto`
- `PubSub` and `Subscription` as **thread primitives**, not fiber-native ones
- `observability` logs, spans, and the small atomic counter service
- `elm-effect-elm`, `elm-effect-journal`, `elm-effect-durable`, and
  `elm-effect-xstate` as native crates, not Effect parity claims

`PubSub` blocks a thread in `Subscription::take` and does not close with an
`Effect` scope or observe interruption. Use `Queue` or `SubscriptionRef` when
the subscriber is a fiber.

`Cache` and `Pool` are synchronous handles too: run them at the process edge or
inside `Effect::sync` when the lookup or resource creation blocks. `Latch`
observes the current fiber's interruption while `await_open` is parked.
`Deferred::wait_exit` observes the current fiber's interruption and returns the
whole recorded `Exit`; the older `wait` keeps its success-only shape but now
ends on an interrupt instead of parking forever. `FiberRef` is fiber-local and
inherited at fork; it is mutable, unlike `Reference`, and is not a global
`Ref`. The witnesses for these synchronous primitives live in
`crates/elm-effect/tests/cache_pool_latch.rs`. The `Resolver` batch queue and
`RcMap` leases are synchronous handles with the same edge rule; their witnesses
live in `crates/elm-effect/tests/request_core.rs`,
`crates/elm-effect/tests/request_batch.rs`,
`crates/elm-effect/tests/rc_map.rs`, and
`crates/elm-effect/tests/fiber_ref.rs`.

## Excluded

These are outside the envelope. Their absence is a scope decision, not a hidden
promise:

- `Schema`, `STM`, `elm-effect-unstable`
- the full Effect verb vocabulary and the full `Stream` surface
- the full platform packages, OTLP exporter, and tracing backend
- `Effect::try_future` and `Effect::into_future`
- full upstream module and behavior parity

`Runtime::run`, `Runtime::run_main`, and `ManagedRuntime::run` are the process
edges. There is no async `Future` adapter in the kernel.

## The 109 unported rows

The last ledger snapshot on 2026-10-24 recorded **109 unported** carried-behavior
rows in Config, Duration, Order, Function, Equivalence, Combiner, Hash, Equal,
and Reducer. The ledger was retired. Those rows have no CI tracker today.

They are declared outside the kernel envelope. This is an exclusion, not a pass.
If a row is brought back into scope, add a named witness to the law matrix or
the machine matrix in the same change. Do not cite the retired ledger as proof.

## Evidence limits

The current evidence proves different things:

- The law matrix proves that the declared witnesses exist and that the suite is
  green. It does not prove that a witness encodes upstream behavior.
- The TLA+ models prove invariants of the models. They do not prove that Rust
  executions refine those models; there is no trace validation.
- The tests prove selected behaviors against this implementation. There is no
  vendored pinned Effect v4 corpus and no differential execution against it.

Therefore the defensible claim is a **formally tracked Effect-shaped kernel**,
not "100% Effect v4 behavior parity." Restoring the larger claim requires a
vendored corpus, a differential suite, and Rust-to-model trace validation.

## Adoption rule

A sibling may adopt the public kernel through its own adoption record. That
record names this envelope, not the withdrawn 100% mission. A new capability is
added as a `ServiceTag`, a `Program`, or a `DurableLog`; it is not a second
scheduler. A new behavior claim needs a witness.
