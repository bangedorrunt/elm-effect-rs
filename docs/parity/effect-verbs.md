# Effect v4 verb and primitive parity

The executable index is `crates/elm-effect/tests/law_matrix.rs`. It checks
that every law has a witness, every witness resolves to a real test, every
upstream citation stays under `packages/effect/test/` at the pin
`effect@4.0.0` (`67ba4e46...`), and every verb names a law and a witness.
This page is the readable view of that table; update both in one change.

## Laws

| Law | Claim | Witness |
|---|---|---|
| L1 | A description does not run | `l3_effect.rs::description_drop_runs_nothing` |
| L2 | Five outcomes stay apart | `l2_cause.rs::five_outcomes_do_not_collapse` |
| L3 | One carrier, run at the edge | `workspace_audit.rs::the_workspace_passes_its_own_lints` |
| L4 | Policy is a value, in one order | `remote.rs::attempt_timeout_does_not_cover_the_queue` |
| L5 | Services are typed, acyclic, and role-distinct | `layer.rs::cycle_fails_construction` |
| L6 | Release on every exit, reverse order, exactly once | `l5_scope.rs::release_on_three_exits` |
| L7 | Finalizers and commits are uninterruptible | `l5_scope.rs::cancel_during_release_finishes_release` |
| L8 | Tests substitute the world | `time_clock.rs::test_clock_sleep_delays_until_adjusted` |
| L9 | Shared state has a ladder | `l9_sync.rs::atomic_update_retries_without_io` |
| L10 | Every wait is bounded | `remote.rs::remote_policy_is_capped` |
| L11 | Parse once, at the boundary | `observability.rs::secret_debug_is_redacted` |
| L12 | A long-lived program is a fold | `replay.rs::fold_is_deterministic` |
| L13 | A batch is a bounded map-reduce | `l6_concurrency.rs::fanout_peak_in_flight` |
| L14 | Intent is recorded before the provider runs | `journal.rs::golden_transitions` |
| L15 | Replay order is ordered | `lints.rs::btreemap_in_a_model_is_clean` |
| L16 | The message log is a durable stream | `durable_log_laws.rs::same_triple_appends_once_on_the_trait` |

L16's `fork` and soft-delete clauses are native carve-outs: the in-process
`DurableLog` has no fork or soft delete, and those protocol features belong to
the HTTP adapter. Retention is witnessed on `JournalStore`, where `Gone` is a
retention miss. The matrix carries the exact reasons.

## Verbs and primitives

| Effect v4 verb / primitive | Rust item | Law |
|---|---|---|
| `succeed` | `Effect::succeed` | L1 |
| `fail` | `Effect::fail` | L2 |
| `die` | `Effect::die` | L2 |
| `interrupt` | `Effect::interrupt` | L2 |
| `sync` | `Effect::sync` | L1 |
| `suspend` | `Effect::suspend` | L1 |
| `map` | `Effect::map` | L1 |
| `flatMap` | `Effect::flat_map` | L1 |
| `catch` | `Effect::catch` | L2 |
| `catchCause` | `Effect::catch_cause` | L2 |
| `exit` | `Effect::exit` | L2 |
| `result` | `Effect::result` | L2 |
| `scoped` | `Effect::scoped` | L6 |
| `acquireRelease` | `Effect::acquire_use_release` | L6 |
| `forkChild` | `Effect::fork_child` | L6 |
| `join` | `FiberHandle::join` | L6 |
| `uninterruptible` | `Effect::uninterruptible` | L7 |
| `all` | `Effect::all` | L13 |
| `race` | `Effect::race` | L13 |
| `timeout` | `Effect::timeout_option` | L4 |
| `retry` | `Effect::retry` | L4 |
| `repeat` | `Effect::repeat` | L10 |
| `schedule` | `Schedule` | L10 |
| `Queue` | `Queue` | L9 |
| `Semaphore` | `Semaphore` | L9 |
| `SubscriptionRef` | `SubscriptionRef` | L12 |
| `TestClock` | `TestClock` | L8 |
| `Layer` | `Layer` | L5 |
| `JournalStore` | `JournalStore` | L14 |
| `DurableLog` | `DurableLog` | L16 |
| `Program` | `Program` | L12 |
| `Interpret` | `Interpret` | L12 |

## State machines

- `docs/research/state-machines-and-flows.md` catalogs the machines.
- `skills/verify-framework-flows/flow10_effect_interpreter.tla` models the
  `Step`/`Drive` trampoline, frame unwind, boundary interrupt, and mask.
- `skills/verify-framework-flows/flow11_scope_masking.tla` models LIFO
  releases and deferred cancellation.
