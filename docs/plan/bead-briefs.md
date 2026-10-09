# ADR-0002 bead briefs — seed source

The product finish line is the witness list in ADR-0002. A brief below that says "port every behavior row" is the ledger index, not that finish line. Mark a row `not-applicable` with a reason when we will not carry that Effect internal.

This file is the authored decomposition of `docs/decisions/0001-*.md` and
`docs/decisions/0002-*.md` into dispatchable beads. `seed-beads.py` parses it
into `br` (once, on a fresh tracker). The briefs are the work contract.

## Conventions (apply to every bead)

- **Spec pin**: `~/.cache/effect-4.0.0` = git tag `effect@4.0.0`, commit
  `67ba4e46a11ccda0b6761578bfd22c04ae00167d`. The tag is the spec; `main` is not.
  Ledger: `conformance/modules.toml`, `conformance/behavior-tests.tsv`.
  Regenerate with `bash conformance/inventory.sh ~/.cache/effect-4.0.0`.
- **Tests first**: the bead's law/behavior tests land red, then implementation.
- **Ledger duty**: when a module is genuinely done, set its
  `conformance/modules.toml` row to `status = "matched"`; set its behavior rows
  in `conformance/behavior-tests.tsv` to `status = "ported"` with a non-empty
  `rust` column naming the Rust test. Regen preserves both on unchanged
  path/line/title. Residual unported behaviors discovered while working become
  NEW beads — never silently skipped and never marked ported without a test.
- **One bead = one commit** with the bead id in the commit message, then a pass
  gate row, then `br close --commit-sha`. See the br close ceremony.
- **Never**: `unwrap` outside tests; `anyhow`/`Box<dyn std::error::Error>` in a
  public signature; a dependency beyond ADR-0002's list (file a follow-up ADR
  instead); a path/git dep on flywheel/toron/beads/chiebukuro/resonate; `unsafe`
  without a `SAFETY` comment; a second `EffectState` enum; `HashMap` inside a
  `Program::Model`.
- **Waves gate order**: wave-N dispatches only while no lower wave bead is
  open/in_progress. Wave labels are `wave-N`; the typed `--wave N` is set too.
- **Crates**: `crates/elm-effect` (facade + kernel), `crates/elm-effect-schema`,
  `crates/elm-effect-test`, `crates/elm-effect-elm`, `crates/elm-effect-journal`,
  `crates/elm-effect-unstable`, `crates/elm-effect-conformance`.
- Public paths: `elm_effect::Effect`, `elm_effect::Cause`, `elm_effect::Layer`,
  `elm_effect::Schedule`, `elm_effect::Stream`, `elm_effect::Schema`,
  `elm_effect::elm::Program`, `elm_effect::journal::EffectState`.

---

## epic-elm-effect-port

TITLE: elm-effect-rs: Effect 4.0.0 semantic port + Elm loop (ADR-0002)
PRIORITY: 2
TYPE: epic
LABELS: epic,adr-0002
VERIFY: cargo test -p elm-effect-conformance -- --ignored
PRINCIPLES: prove-it-works — the ledger's ignored done-check is the only definition of port-complete; sequence-verifiable-units — waves land law tests before implementations
DESCRIPTION:
The whole project: port the behavior of `effect@4.0.0` into a Rust carrier
`Effect<A, E, R>` with an owned fiber runtime, plus the Elm loop, journal, and
durable log, until `conformance/modules.toml` has every `port`,
`unstable-port`, and `experimental-port` row `matched` and every behavior row
`ported` or `skipped-upstream`.

Children are the wave beads. The law is `docs/decisions/0002-*.md`; the steps
are `skills/elm-effect/SKILL.md`.

## VERIFY

```bash
cargo test -p elm-effect-conformance -- --ignored
```

---

## w0-phase0-baseline

TITLE: Phase 0 close-out: spec checkout, regeneration proof, baseline commit
PRIORITY: 2
TYPE: task
LABELS: wave-0,phase-0,conformance,setup
VERIFY: cargo test -p elm-effect-conformance
PRINCIPLES: prove-it-works — the ledger regenerates byte-for-byte from the pin rather than trusting a pasted file; sequence-verifiable-units — every later wave reads the same committed baseline
DESCRIPTION:
ADR-0002 Phase 0 is implemented in the working tree; this bead closes it with
proof and a committed baseline.

Deliverables:
- `~/.cache/effect-4.0.0` is a checkout of tag `effect@4.0.0` at commit
  `67ba4e46a11ccda0b6761578bfd22c04ae00167d`.
- `bash conformance/inventory.sh ~/.cache/effect-4.0.0` regenerates
  `conformance/modules.toml` and `conformance/behavior-tests.tsv` byte-for-byte
  (377 modules, 9921 behavior rows).
- `cargo test -p elm-effect-conformance` is green (well-formed ledger; the
  parity test stays `#[ignore]` until phase 13).
- Baseline commit carries `Cargo.toml`, `rust-toolchain.toml`, `README.md`,
  `.gitignore`, `docs/decisions/`, `conformance/`, `skills/elm-effect/`,
  `crates/elm-effect-conformance/`, and the bead seed (`docs/plan/`).

## VERIFY

```bash
cargo test -p elm-effect-conformance
```

---

## w1-kernel-skeleton

TITLE: Kernel skeleton + Cause/Exit/Defect
PRIORITY: 1
TYPE: feature
LABELS: wave-1,phase-1,kernel
VERIFY: cargo test -p elm-effect --test l2_cause
PRINCIPLES: boundary-discipline — the cause model owns the failure vocabulary at the crate edge; type-system-discipline — Defect is not an E, the types cannot collapse
DESCRIPTION:
ADR-0002 Phase 1, "Carrier" and "Cause" sections. Laws L2 (this part),
plus the skeleton the rest of the wave builds on.

Deliverables:
- Workspace members: `crates/elm-effect` (facade, may stay monolithic until its
  `cargo check` exceeds 30 s), `crates/elm-effect-test` (skeleton only this
  bead). Edition 2024, license MIT, `publish = false`.
- `crates/elm-effect` features: `tokio`, `schema`, `stm` default-on; deps
  `thiserror` 1 at crate edges only; `tokio` 1 behind feature `tokio`.
- `cause.rs`: `Cause<E> { reasons: Vec<Reason<E>> }`, `Reason<E> =
  Fail(E) | Die(Defect) | Interrupt { fiber_id: Option<FiberId> }`; `Defect`
  carries a message and a backtrace; `Cause::empty` is an empty vector (no
  `Empty`/`Sequential`/`Parallel` variants, no misleading `is_empty`).
  Structural equality for causes. No `E = Infallible` confusion: `never` is
  `Infallible`, not `()`.
- `exit.rs`: `Exit<A, E>` = success or failure-with-cause; it is ordinary data,
  not an effect.
- Tests (land first, red): `crates/elm-effect/tests/l2_cause.rs` with
  `cause_reasons_classify`, `cause_structural_eq`, `cause_empty_has_no_reasons`,
  `defect_is_not_an_error_value`, `exit_is_data_not_effect`.

Pitfalls: no v3 cause tree; `Timeout` later becomes a typed error, not a
reason kind. Phase 1 has zero `unsafe`.

## VERIFY

```bash
cargo test -p elm-effect --test l2_cause
```

---

## w1-effect-core

TITLE: Effect core: description, succeed/fail/die/sync, map/flat_map
PRIORITY: 1
TYPE: feature
LABELS: wave-1,phase-1,kernel
VERIFY: cargo test -p elm-effect description_drop_runs_nothing
PRINCIPLES: foundational-thinking — the private state machine is the one place interpretation happens; prove-it-works — L1 pins that description is inert
DESCRIPTION:
ADR-0002 Phase 1, "Carrier". Law L1.

Deliverables:
- `Effect<A, E, R = ()>` is a value, NOT a `Future`. `A`/`E` are
  `Send + 'static`; `R` implements the sealed `Requirement` trait (declared in
  this wave as `()` only; the tuple machinery lands in phase 2).
- Private step state machine inside: success, fail with cause, sync, flat-map,
  map. User code never matches it.
- Public: `succeed`, `fail`, `die`, `sync`, `Effect::map`, `Effect::flat_map`;
  `Effect::result` (typed error to `Result`), `Effect::exit`,
  `Effect::or_die` (sets `E = Infallible`).
- Test (land first, red): `description_drop_runs_nothing` — building an effect
  whose sync closure counts calls, dropping the value, asserting the count is
  zero (no I/O, no sleep, no state change from construction or drop).
- `cargo test -p elm-effect` compiles with no Tokio runtime linked when
  `--no-default-features` is used, once `tokio` feature plumbing exists.

Pitfalls: no boxing into `BoxFuture` at the API edge; no `catch_unwind` here
(phase 1 supervisor only); no `?` swallowing `Die`.

## VERIFY

```bash
cargo test -p elm-effect description_drop_runs_nothing
```

---

## w1-fiber

TITLE: Fiber: fork/join/interrupt, interruptibility, supervisor catch_unwind
PRIORITY: 1
TYPE: feature
LABELS: wave-1,phase-1,kernel
VERIFY: cargo test -p elm-effect panic_in_child_is_die_and_sibling_lives
PRINCIPLES: boundary-discipline — the fiber supervisor is the only catch_unwind in the workspace; fix-root-causes — a panic becomes Die at the fiber boundary, never E
DESCRIPTION:
ADR-0002 Phase 1, "Fibers, interruption, and scope" (fiber part).

Deliverables:
- `Fiber` with id, interruptibility depth, scope stack, context, parent.
- `fork_child` (child dies with parent; parent waits for child finalizers),
  `fork_scoped` (caller-supplied `Scope`), `fork_in`, `fork_detach` (attaches
  to the runtime root scope; NO supervisor parameter).
- Default start is deferred (child begins when the current fiber yields or
  completes); `start_immediately: true`; `uninterruptible: true | inherit`;
  default interruptibility = runtime default (interruptible).
- `join`; `Fiber::interrupt` waits until the target's finalizers finish
  (back-pressure), `Fiber::interrupt_unsafe` does not wait. Default is the
  waiting form. Interruption is delivered at interruptible steps; an
  uninterruptible region runs to completion and the flag fires when depth
  returns to zero.
- Supervisor: the one legal `catch_unwind` in the workspace, living in the
  supervisor module only. A panic stores `Die`; a sibling fiber keeps running;
  the process stays up.
- Tests (land first, red): `panic_in_child_is_die_and_sibling_lives`;
  `fork_detach_child_ends_at_runtime_shutdown` (no supervisor arg);
  `interrupt_waits_for_finalizers`.

## VERIFY

```bash
cargo test -p elm-effect panic_in_child_is_die_and_sibling_lives
```

---

## w1-scope

TITLE: Scope + acquire_release: release on every exit, reverse order, once
PRIORITY: 1
TYPE: feature
LABELS: wave-1,phase-1,kernel
VERIFY: cargo test -p elm-effect release_on_three_exits
PRINCIPLES: boundary-discipline — scope owns cleanup so no caller re-invents it; prove-it-works — L6/L7 are phase-1 merge gates
DESCRIPTION:
ADR-0002 Phase 1, "Fibers, interruption, and scope" (scope part). Laws L6, L7.

Deliverables:
- `Scope`, `acquire_release` (finalizer is an effect; runtime runs it), the
  finalizer stack, `Scope::provide` (lends a scope; does not close it).
- L6: success, typed failure, and interruption all release; reverse order of
  acquisition; exactly once; a finalizer failure is APPENDED to the cause and
  never erases the original reason.
- L7: finalizers and marked commit regions are uninterruptible: cancellation
  requested during a release runs it to completion, then takes effect.
- `Drop` of a guard is not the release path for anything needing `.await`;
  sync resources may still use `Drop`.
- Tests (land first, red): `release_on_three_exits` (order `[second, first]`),
  `cancel_during_release_finishes_release`, `finalizer_failure_is_appended`.

## VERIFY

```bash
cargo test -p elm-effect release_on_three_exits
```

---

## w1-runtime

TITLE: Runtime + deterministic executor + five outcomes
PRIORITY: 1
TYPE: feature
LABELS: wave-1,phase-1,kernel
VERIFY: cargo test -p elm-effect five_outcomes_do_not_collapse
PRINCIPLES: boundary-discipline — one edge runs descriptions; libraries never do; type-system-discipline — the four in-process outcomes stay apart under every handler
DESCRIPTION:
ADR-0002 Phase 1, "Carrier" (runtime) and the L2 law test. The `Parked`
assertion of L2 lands in the journal wave (phase 9).

Deliverables:
- `Runtime::run` (the edge), `Runtime::run_main` (owns signals and exit codes:
  success 0, typed failure 1, defect 2, interrupt 130), `ManagedRuntime` for a
  non-effect caller.
- `Executor` trait: `spawn_boxed(BoxFuture<'static, ()>)`,
  `spawn_blocking(Box<dyn FnOnce() + Send>)`. `TokioExecutor` behind feature
  `tokio` with defaults `worker_threads = 4`, `queue_bound = 4096` (overridable,
  never constants inside combinators).
- `DeterministicExecutor` in `crates/elm-effect-test`: runs one ready fiber at a
  time in registration order; timers complete only when the test clock says so;
  `tokio::time::pause` is not the spec.
- L2 test (lands red before the runtime exists): `five_outcomes_do_not_collapse`
  — builds success, `Fail`, `Die`, `Interrupt` and asserts the classifiers;
  `catch` on the error channel leaves `Die`/`Interrupt` in place;
  `catch_defect` leaves typed failures/interrupts alone; `catch_cause` is the
  only combinator that sees all three and library code does not call it.
- L3 lint target: `library_crate_has_no_run` — `Runtime::run`/`block_on` appear
  only in `elm-effect` itself, `elm-effect-test`, and `examples/` binaries.

## VERIFY

```bash
cargo test -p elm-effect five_outcomes_do_not_collapse
```

---

## w2-requirements

TITLE: Requirement channel: service tags, providing as type-level removal
PRIORITY: 1
TYPE: feature
LABELS: wave-2,phase-2,context
VERIFY: cargo test -p elm-effect --test sixteen_services
PRINCIPLES: type-system-discipline — a missing service is a compile error, not a runtime lookup; boundary-discipline — the R channel is the whole point of the carrier
DESCRIPTION:
ADR-0002 Phase 2, "Carrier" (R channel) and the L5 header. Law L5 (tags part).

Deliverables:
- Sealed `Requirement` trait; `()` = no requirements; service tags are
  zero-sized types with a string id (`"myapp/db/Database"`) and an associated
  service type. Providing removes the tag at the type level: `Effect<A, E,
  (Database, Clock)>` becomes `Effect<A, E, (Clock,)>` after
  `provide::<Database, _>`.
- Tuple impls macro up to 16; beyond 16, `Cons<H, T>` is the escape. This is a
  deliberate ceiling.
- `Effect::provide`, `Effect::provide_merge` (type-level behavior here; Layer
  wiring lands in w2-layer), `use_service::<Tag>()`, and the `effect!` macro
  expanding to `flat_map` (`?` propagates the typed error only).
- Tests: `crates/elm-effect/tests/sixteen_services.rs` (compiles; braced
  fixture), `tagged_service_resolves`, `provide_removes_requirement` (compile
  evidence), `missing_service_does_not_compile` (trybuild-style negative test
  or documented compile-fail harness).
- Acceptance extra: record `time cargo check -p elm-effect --test sixteen_services`.
  Over 30 s on the operator's machine STOPS the phase and writes a follow-up
  record; never swap in a `TypeId` map. A `DynamicServices` service may exist
  for plugin hosts only; it never satisfies a static tag.

## VERIFY

```bash
cargo test -p elm-effect --test sixteen_services
```

---

## w2-layer

TITLE: Layer: build, provide, fresh, cycle detection at construction
PRIORITY: 1
TYPE: feature
LABELS: wave-2,phase-2,context
VERIFY: cargo test -p elm-effect cycle_fails_construction
PRINCIPLES: type-system-discipline — the dependency graph is walked before constructors run; fix-root-causes — a cycle is a construction error, not a per-request failure
DESCRIPTION:
ADR-0002 Phase 2, "Layers and context" (layer part). Law L5 (cycle part).

Deliverables:
- `Layer<ROut, E, RIn>` with `build`; `Layer::provide` (wires and hides),
  `Layer::provide_merge` (wires, stays visible), `Layer::fresh` (tests get a
  fresh instance).
- Build-time graph walk rejects cycles BEFORE any constructor runs; the failure
  is a typed construction error (`Cycle`), not a per-request error.
- Constructors may fail; that failure fails the scope being built.
- Tests: `cycle_fails_construction` (A requires B and B requires A, typed
  error), `layer_constructor_failure_fails_scope`, `fresh_builds_new_instance`.

## VERIFY

```bash
cargo test -p elm-effect cycle_fails_construction
```

---

## w2-wiring

TITLE: MemoMap shapes, ambiguous tag defect, Context::Reference
PRIORITY: 1
TYPE: feature
LABELS: wave-2,phase-2,context
VERIFY: cargo test -p elm-effect ambiguous_tag_is_a_defect
PRINCIPLES: prove-it-works — both MemoMap shapes get pinned to the v4 examples; type-system-discipline — a tag provided twice is a defect, not a last-writer-wins
DESCRIPTION:
ADR-0002 Phase 2, "Layers and context" (memoization, references). Laws L5.

Deliverables:
- `MemoMap` sharing, pinned to the two v4 shapes: (a) two sequential
  `Effect.provide` calls inside one generator construct the layer twice;
  (b) a stacked `provide` then `provide` of the same layer builds once. Do not
  invent a third sharing rule. Sharing starts once the constructor is wrapped
  in a layer; `Context.Service`'s constructor alone is not the memo unit.
- `ambiguous_tag_is_a_defect`: providing the same service tag twice in one
  build asserts `Die` (defect), not an overwrite.
- `Context::Reference`: an ambient value with a default (log level, locale);
  overriding for a sub-effect is `Effect::provide_reference`. Domain code does
  not read environment variables; `Config` is parsed at startup into a value
  the layer closes over.
- Tests: `memo_map_sequential_vs_stacked`, `ambiguous_tag_is_a_defect`,
  `reference_override_is_scoped`.

## VERIFY

```bash
cargo test -p elm-effect ambiguous_tag_is_a_defect
```

---

## w3-clock

TITLE: Clock, Duration, sleep, timeout + TestClock
PRIORITY: 1
TYPE: feature
LABELS: wave-3,phase-3,time
VERIFY: cargo test -p elm-effect-test adjust_before_sleep_is_scheduled_does_not_complete_it
PRINCIPLES: boundary-discipline — time enters through the Clock service so tests substitute the world; prove-it-works — the TestClock onboarding scenario is the phase gate
DESCRIPTION:
ADR-0002 Phase 3, "Time" part. Law L8.

Deliverables:
- `Clock` service, `Duration`, `Clock::sleep`, `timeout` (a race against
  `Clock::sleep`; the loser is interrupted and RELEASED before the winner's
  value is returned).
- `TestClock` in `crates/elm-effect-test`: does not advance by itself; legal
  sequence is fork the sleeper, adjust the clock, join the fiber. The test
  executor is single-threaded and deterministic.
- `effect_test!` macro + layer fixture (the `@effect/vitest` substitute).
- Tests: `adjust_before_sleep_is_scheduled_does_not_complete_it` (adjust first,
  then fork a sleep, fiber still suspended), `timeout_interrupts_and_releases_loser`,
  and the onboarding scenario from `packages/effect/src/testing/TestClock.ts`
  (sleep 5 minutes, timeout 1 minute, fork, adjust 1 minute, exit is failure)
  with NO real sleep.

## VERIFY

```bash
cargo test -p elm-effect-test adjust_before_sleep_is_scheduled_does_not_complete_it
```

---

## w3-schedule

TITLE: Schedule + retry/repeat (never retry defects or interrupts)
PRIORITY: 1
TYPE: feature
LABELS: wave-3,phase-3,schedule
VERIFY: cargo test -p elm-effect --test schedule
PRINCIPLES: model-the-domain — retry policy is a value, not a loop; boundary-discipline — retry wraps the smallest idempotent region
DESCRIPTION:
ADR-0002 Phase 3, "Schedule and the remote stack" (schedule part).

Deliverables:
- `Schedule<In, Out>` as a value: decides whether to continue and how long to
  wait. `recurs(n)` = n retries after the first attempt. `max` continues while
  every member continues, waits the longer delay. `min` continues while any
  member continues, waits the shorter delay. Exponential backoff composed with
  a spaced ceiling, then jitter (from the `Random` service), then a predicate
  (e.g. `error.retryable`).
- `retry` re-runs after typed failure; `repeat` re-runs after success. They are
  different combinators. Defects and interrupts are never retried.
- Tests: `crates/elm-effect/tests/schedule.rs` covering recurs counting,
  max/min composition, jitter drawn from `Random::with_seed`, and
  `retry_never_retries_die_or_interrupt`.

## VERIFY

```bash
cargo test -p elm-effect --test schedule
```

---

## w3-refs

TITLE: Refs, Deferred, Queue, Semaphore, MutableRef
PRIORITY: 1
TYPE: feature
LABELS: wave-3,phase-3,refs
VERIFY: cargo test -p elm-effect atomic_update_retries_without_io
PRINCIPLES: separate-before-serializing-shared-state — the ladder runs immutable first; boundary-discipline — a pure atomic update must not do I/O
DESCRIPTION:
ADR-0002 Phase 3, "Layers and context" standard-library part; Law L9.

Deliverables:
- `Ref`, `MutableRef` (sync), `SynchronizedRef` (an effectful update takes a
  permit and runs once), `Deferred`, bounded `Queue`, `Semaphore`.
- L9 ladder enforced in types and docs: immutable data first; a pure atomic
  update may retry and must not log, emit, or perform I/O; two facts that move
  together are one value; two tasks communicate through a bounded queue.
- Tests: `atomic_update_retries_without_io` (fails the test if the update
  closure calls a service), `synchronized_ref_runs_once`,
  `queue_is_bounded`, `semaphore_permits_match`.

## VERIFY

```bash
cargo test -p elm-effect atomic_update_retries_without_io
```

---

## w3-policy

TITLE: policy::remote: the L4 order + defaults + bounded waits
PRIORITY: 1
TYPE: feature
LABELS: wave-3,phase-3,policy
VERIFY: cargo test -p elm-effect attempt_timeout_does_not_cover_the_queue
PRINCIPLES: model-the-domain — the remote stack is one value with named layers; boundary-discipline — attempt timeout never wraps queue wait; prove-it-works — caps are asserted, not assumed
DESCRIPTION:
ADR-0002 Phase 3, "Schedule and the remote stack" (policy part). Laws L4, L10.

Deliverables:
- `policy::remote(call)` builds the innermost-to-outermost order: filter,
  fallback, cache, budget timeout, retry, circuit breaker, rate limit, bulkhead,
  attempt timeout, call. Defaults, all overridable, none infinite: 3 retries
  after first; backoff 250 ms; ceiling 10 s; full jitter from `Random`;
  attempt timeout 30 s; budget 2 min; bulkhead 8 in flight / queue 32 / wait
  5 s; rate limit none until passed a limiter; breaker closed until passed one
  (open after 5 consecutive typed failures, half-open after 30 s, one probe).
- `BreakerOpen` is a typed error and does NOT count as a retry attempt.
  Unbounded retry is not constructible through `policy::remote`.
- Tests: `attempt_timeout_does_not_cover_the_queue` (limiter waits longer than
  attempt timeout; call returns immediately once admitted; success),
  `remote_policy_is_capped` (finite attempt count), `retry_releases_per_attempt`
  (three releases, one surviving outer resource), `breaker_open_is_typed`.

## VERIFY

```bash
cargo test -p elm-effect attempt_timeout_does_not_cover_the_queue
```

---

## w3-hedge

TITLE: policy::hedge for idempotent reads
PRIORITY: 1
TYPE: feature
LABELS: wave-3,phase-3,policy
VERIFY: cargo test -p elm-effect hedge_does_not_send_the_duplicate_when_fast
PRINCIPLES: experience-first — latency wins only when the duplicate is provably safe; prove-it-works — the fast path must send zero duplicates
DESCRIPTION:
ADR-0002 Phase 3, "Schedule and the remote stack" (hedge part).

Deliverables:
- `policy::hedge`: start the call; start one duplicate after a caller-supplied
  delay if the first is still running; take the first success; cancel the loser
  (released). For idempotent reads only; never in the default stack.
- Tests: `hedge_does_not_send_the_duplicate_when_fast` (duplicate service sees
  zero calls when the first returns before the delay),
  `hedge_cancels_loser_after_duplicate_wins`.

## VERIFY

```bash
cargo test -p elm-effect hedge_does_not_send_the_duplicate_when_fast
```

---

## w3-datetime-cron

TITLE: DateTime + Cron modules
PRIORITY: 1
TYPE: feature
LABELS: wave-3,phase-3,time
VERIFY: cargo test -p elm-effect --test datetime_cron
PRINCIPLES: model-the-domain — cron schedules are parsed values, not string checks; prove-it-works — port the tag's own behavior tests
DESCRIPTION:
ADR-0002 Phase 3 family: the remaining time-domain modules.

Deliverables:
- `DateTime.ts` and `Cron.ts` behavior ported to Rust at the pin; tests derived
  each from `packages/effect/test/DateTime.test.ts` and
  `packages/effect/test/Cron.test.ts` behavior rows.
- Ledger: flip both module rows to `matched`; every behavior row ported by this
  bead gets `status = "ported"` + `rust` naming the test. File follow-up beads
  for residual rows.
- Tests live in `crates/elm-effect/tests/datetime_cron.rs`.

## VERIFY

```bash
cargo test -p elm-effect --test datetime_cron
```

---

## w4-scalars

TITLE: Foundation: Option, Result, UndefinedOr, Boolean, Number, String, Brand
PRIORITY: 2
TYPE: feature
LABELS: wave-4,foundation-data
VERIFY: cargo test -p elm-effect --test port_scalars
PRINCIPLES: foundational-thinking — data modules land before the phases that consume them; laziness-protocol — thin helpers over std where the tag's behavior allows, full behavior where it does not
DESCRIPTION:
ADR-0002's "Streams, schema, STM, and the standard library" section owns these
modules; they are scheduled here, before phases 4+, because later waves lean on
them.

Deliverables:
- Port the behavior of `Option.ts`, `Result.ts`, `UndefinedOr.ts`,
  `Boolean.ts`, `Number.ts`, `String.ts`, `Brand.ts` at the pin. Rust std is
  the substrate; the tag's behavior (combinators, generation patterns where
  they matter) is the spec. `Option`/`Result` remain distinct from Rust's std
  enums only where the tag's API requires wrappers.
- Tests in `crates/elm-effect/tests/port_scalars.rs`, derived from the linked
  `packages/effect/test/*.test.ts` (Option, Result, Boolean, Number, String,
  Brand behavior rows).
- Ledger: flip the 7 module rows to `matched`; port the behavior rows; file
  follow-up beads for residuals.

## VERIFY

```bash
cargo test -p elm-effect --test port_scalars
```

---

## w4-logic

TITLE: Foundation: Predicate, Function, Equal, Equivalence, Order, Ordering, Hash
PRIORITY: 2
TYPE: feature
LABELS: wave-4,foundation-data
VERIFY: cargo test -p elm-effect --test port_logic
PRINCIPLES: type-system-discipline — Equal is a trait with its own law, separate from PartialEq; prove-it-works — equal_nan_is_nan pins the deliberate divergence
DESCRIPTION:
ADR-0002 "Streams, schema, STM, and the standard library" (Equal paragraph) and
the foundation family.

Deliverables:
- Port `Predicate.ts`, `Function.ts`, `Equal.ts`, `Equivalence.ts`, `Order.ts`,
  `Ordering.ts`, `Hash.ts` at the pin.
- The deliberate divergence: `Equal` treats `NaN` as equal to `NaN`; `f64`'s
  `PartialEq` stays IEEE. The two traits are different on purpose. The test
  `equal_nan_is_nan` uses `Equal`, not `==`, and lives in this target.
- Tests in `crates/elm-effect/tests/port_logic.rs`; ledger rows flipped.

## VERIFY

```bash
cargo test -p elm-effect --test port_logic
```

---

## w5-structures

TITLE: Foundation: Array, Chunk, Iterable, NonEmptyIterable, MutableList, Record, Struct, Tuple
PRIORITY: 2
TYPE: feature
LABELS: wave-5,foundation-data
VERIFY: cargo test -p elm-effect --test port_structures
PRINCIPLES: foundational-thinking — sequence and record shapes land before streams; laziness-protocol — Vec/BTreeMap under the hood where behavior matches
DESCRIPTION:
Foundation family (see w4-scalars preamble). Port behavior of `Array.ts`,
`Chunk.ts`, `Iterable.ts`, `NonEmptyIterable.ts`, `MutableList.ts`,
`Record.ts`, `Struct.ts`, `Tuple.ts` at the pin.

Tests in `crates/elm-effect/tests/port_structures.rs`; flip the 8 module rows;
port behavior rows per linked test file; file residual beads.

## VERIFY

```bash
cargo test -p elm-effect --test port_structures
```

---

## w5-data-match

TITLE: Foundation: Data, Match, Filter, Combiner, Reducer, Differ
PRIORITY: 2
TYPE: feature
LABELS: wave-5,foundation-data
VERIFY: cargo test -p elm-effect --test port_data_match
PRINCIPLES: model-the-domain — tagged data and matching are encoded, not string-keyed; boundary-discipline — the match combinator returns typed values
DESCRIPTION:
Foundation family. Port `Data.ts` (tagged structs/cases), `Match.ts`,
`Filter.ts`, `Combiner.ts`, `Reducer.ts`, `Differ.ts` at the pin.

Tests in `crates/elm-effect/tests/port_data_match.rs`; flip the 6 module rows;
port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test port_data_match
```

---

## w5-containers

TITLE: Foundation: HashMap, HashSet, MutableHashMap, MutableHashSet, PrimaryKey, Trie, HashRing
PRIORITY: 2
TYPE: feature
LABELS: wave-5,foundation-data
VERIFY: cargo test -p elm-effect --test port_containers
PRINCIPLES: separate-before-serializing-shared-state — persistent and mutable maps are distinct types; type-system-discipline — PrimaryKey is a key contract, not a comment
DESCRIPTION:
Foundation family. Port `HashMap.ts`, `HashSet.ts`, `MutableHashMap.ts`,
`MutableHashSet.ts`, `PrimaryKey.ts`, `Trie.ts`, `HashRing.ts` at the pin.
Persistent collections keep their behavior (structural sharing is an
implementation detail; observable behavior is the spec).

Tests in `crates/elm-effect/tests/port_containers.rs`; flip the 7 module rows;
port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test port_containers
```

---

## w5-misc

TITLE: Foundation: Graph, Utils, Formatter, JsonPointer, JsonPatch
PRIORITY: 2
TYPE: feature
LABELS: wave-5,foundation-data
VERIFY: cargo test -p elm-effect --test port_misc
PRINCIPLES: laziness-protocol — port the behavior the tag's tests pin, no speculative API beyond it
DESCRIPTION:
Foundation family. Port `Graph.ts`, `Utils.ts`, `Formatter.ts`,
`JsonPointer.ts`, `JsonPatch.ts` at the pin.

Tests in `crates/elm-effect/tests/port_misc.rs`; flip the 5 module rows; port
behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test port_misc
```

---

## w5-optic

TITLE: Foundation: Optic
PRIORITY: 2
TYPE: feature
LABELS: wave-5,foundation-data
VERIFY: cargo test -p elm-effect --test port_optic
PRINCIPLES: type-system-discipline — lens/prism/traversal composition typed, not string paths; model-the-domain — optics are values that compose
DESCRIPTION:
Foundation family. Port `Optic.ts` (optics: lenses, prisms, traversals,
composition) at the pin.

Tests in `crates/elm-effect/tests/port_optic.rs`; flip the module row; port
behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test port_optic
```

---

## w5-numeric

TITLE: Foundation: ByteSize, BigInt, BigDecimal, RegExp
PRIORITY: 2
TYPE: feature
LABELS: wave-5,foundation-data
VERIFY: cargo test -p elm-effect --test port_numeric
PRINCIPLES: laziness-protocol — smallest implementation that passes the tag's behavior; never-block-on-the-human — a genuine dependency wall is a follow-up record, not a silent skip
DESCRIPTION:
Foundation family. Port `ByteSize.ts`, `BigInt.ts`, `BigDecimal.ts`,
`RegExp.ts` at the pin.

Risk note: `BigInt`/`BigDecimal` are arbitrary-precision in TypeScript. The
ADR's dependency list is closed; if full parity needs a crate not on it, file
a follow-up record (do not add the dependency silently) and mark the bead
blocked naming the record. A reduced but test-passing implementation that
matches the observable behavior of the linked test file is acceptable only if
it genuinely passes those rows.

Tests in `crates/elm-effect/tests/port_numeric.rs`; flip the 4 module rows;
port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test port_numeric
```

---

## w6-all

TITLE: Concurrency: all/for_each, sequential default, bounded fan-out
PRIORITY: 1
TYPE: feature
LABELS: wave-6,phase-4,concurrency
VERIFY: cargo test -p elm-effect all_without_concurrency_is_sequential
PRINCIPLES: boundary-discipline — fan-out takes an explicit bound at the boundary; prove-it-works — the sequential default is asserted, not documented
DESCRIPTION:
ADR-0002 Phase 4, "Fibers, interruption, and scope" combinator part; Law L13
(fan-out part).

Deliverables:
- `all` with no concurrency argument runs one effect at a time; the next starts
  only after the previous completes. A number caps how many run at once.
  `Concurrency::Unbounded` exists and starts every task; `policy::remote` never
  selects it.
- `for_each` mirrors `all`'s bounds.
- Tests: `all_without_concurrency_is_sequential` (second effect has not started
  while the first is suspended), `all_with_bound_respects_bound`,
  `fanout_peak_in_flight` (counting service; peak <= bound).

## VERIFY

```bash
cargo test -p elm-effect all_without_concurrency_is_sequential
```

---

## w6-race

TITLE: Concurrency: race, race_first, loser released, causes concatenated
PRIORITY: 1
TYPE: feature
LABELS: wave-6,phase-4,concurrency
VERIFY: cargo test -p elm-effect race_failure_does_not_beat_later_success
PRINCIPLES: fix-root-causes — the loser is interrupted and released before the winner returns; model-the-domain — race semantics are explicit, not emergent
DESCRIPTION:
ADR-0002 Phase 4.

Deliverables:
- `race` returns the first SUCCESS; a failure does not beat a later success; if
  every side fails, the causes are concatenated. `race_first` lets any
  completion (including failure) decide.
- The loser is interrupted and released before the winner's value is returned.
- Tests: `race_failure_does_not_beat_later_success`, `race_all_fail_concatenates_causes`,
  `race_loser_is_released`.

## VERIFY

```bash
cargo test -p elm-effect race_failure_does_not_beat_later_success
```

---

## w6-partition

TITLE: Concurrency: partition and validate (read the pin first)
PRIORITY: 1
TYPE: feature
LABELS: wave-6,phase-4,concurrency
VERIFY: cargo test -p elm-effect --test partition_validate
PRINCIPLES: prove-it-works — collect-vs-fail-fast comes from the tag's source, not a guide; boundary-discipline — the three combinators have distinct contracts
DESCRIPTION:
ADR-0002 Phase 4: "`partition` and `validate` are read from the pin before they
are encoded. The v4 concurrency guides do not spell their collect-versus-fail-fast
behavior, so the source file is the spec for those two."

Deliverables:
- Read `packages/effect/src/Effect.ts` (and the concurrency guides) at the pin
  BEFORE writing the combinators. Encode exactly what the source does.
- Tests in `crates/elm-effect/tests/partition_validate.rs`, derived from the
  pin's own tests for these combinators.

## VERIFY

```bash
cargo test -p elm-effect --test partition_validate
```

---

## w6-fiber-sets

TITLE: Concurrency: FiberSet, FiberMap, FiberHandle, ExecutionPlan, Scheduler
PRIORITY: 1
TYPE: feature
LABELS: wave-6,phase-4,concurrency
VERIFY: cargo test -p elm-effect --test fiber_sets
PRINCIPLES: boundary-discipline — fiber bookkeeping stays in typed handles; model-the-domain — ExecutionPlan is a value before it is a schedule
DESCRIPTION:
ADR-0002 Phase 4: "fiber sets, ExecutionPlan".

Deliverables:
- Port `FiberSet.ts`, `FiberMap.ts`, `FiberHandle.ts`, `ExecutionPlan.ts`,
  `Scheduler.ts` behavior at the pin.
- Tests in `crates/elm-effect/tests/fiber_sets.rs`; flip the 5 module rows;
  port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test fiber_sets
```

---

## w7-stream-core

TITLE: Streams: pull-based Stream, run_fold, map/filter
PRIORITY: 1
TYPE: feature
LABELS: wave-7,phase-5,streams
VERIFY: cargo test -p elm-effect --test stream_core
PRINCIPLES: foundational-thinking — pull-based core before operators; boundary-discipline — an unbounded input is bounded chunks, never a Vec built in memory
DESCRIPTION:
ADR-0002 Phase 5, "Streams, schema, STM, and the standard library" (streams
part).

Deliverables:
- `Stream` as pull-based effects; `run_fold` consumes; `map`, `filter`, and the
  core operator set needed by the module's linked tests.
- An unbounded input is a stream of bounded chunks; never materialize an
  unbounded input into memory.
- Tests in `crates/elm-effect/tests/stream_core.rs` derived from
  `packages/effect/test/Stream.test.ts`; flip `Stream.ts` to `matched` only
  when its behavior rows are ported or explicitly fill follow-up beads (file
  them: residual rows are new beads, never silent skips).

## VERIFY

```bash
cargo test -p elm-effect --test stream_core
```

---

## w7-channel-sink

TITLE: Streams: Channel, Sink, Pull, Take
PRIORITY: 1
TYPE: feature
LABELS: wave-7,phase-5,streams
VERIFY: cargo test -p elm-effect --test channel_sink
PRINCIPLES: model-the-domain — channels and sinks are the stream conversation's endpoints; sequence-verifiable-units — each module's rows land with its tests
DESCRIPTION:
ADR-0002 Phase 5: "Channels and sinks sit with streams in the same crate."

Deliverables:
- Port `Channel.ts`, `Sink.ts`, `Pull.ts`, `Take.ts` behavior at the pin.
- Tests in `crates/elm-effect/tests/channel_sink.rs`; flip the 4 module rows;
  port behavior rows; file residual beads.

## VERIFY

```bash
cargo test -p elm-effect --test channel_sink
```

---

## w7-stream-concurrency

TITLE: Streams: bounded map_effect, batch peak, cancel releases children
PRIORITY: 1
TYPE: feature
LABELS: wave-7,phase-5,streams
VERIFY: cargo test -p elm-effect fanout_peak_in_flight
PRINCIPLES: prove-it-works — the bound and the release-on-cancel are asserted with a counting service; boundary-discipline — map_effect takes its concurrency bound explicitly
DESCRIPTION:
ADR-0002 Phase 5 + Law L13 (batch part).

Deliverables:
- `map_effect` takes a concurrency bound. Cancelling a batch releases every
  child.
- Tests: `fanout_peak_in_flight` (counting service; peak <= bound),
  `cancel_batch_releases_every_child`, `map_effect_smaller_bound_is_sequential`.

## VERIFY

```bash
cargo test -p elm-effect fanout_peak_in_flight
```

---

## w8-schema-core

TITLE: Schema core: decode/encode of unknown into typed values or SchemaError
PRIORITY: 1
TYPE: feature
LABELS: wave-8,phase-6,schema
VERIFY: cargo test -p elm-effect-schema --test schema_core
PRINCIPLES: type-system-discipline — untrusted bytes become typed values at the boundary; boundary-discipline — parse once, at the edge
DESCRIPTION:
ADR-0002 Phase 6, "Streams, schema, STM, and the standard library" (schema
part). Law L11 (parse part).

Deliverables:
- `crates/elm-effect-schema` crate (depends on `elm-effect` with
  `default-features = false`), re-exported from the facade so callers write
  `elm_effect::Schema`.
- Decode of `unknown` bytes yields a typed value or a typed `SchemaError`.
  Encode is the inverse. `serde` may serve the boring cases; the spec is Effect
  Schema. `SchemaError` via `thiserror` at the crate edge.
- Port `Schema.ts` core, `SchemaAST.ts`, `SchemaIssue.ts`,
  `SchemaRepresentation.ts` behavior; tests in
  `crates/elm-effect-schema/tests/schema_core.rs` derive from
  `packages/effect/test/schema/` rows.

## VERIFY

```bash
cargo test -p elm-effect-schema --test schema_core
```

---

## w8-schema-transform

TITLE: Schema: transformations, brands, accumulated parse errors
PRIORITY: 1
TYPE: feature
LABELS: wave-8,phase-6,schema
VERIFY: cargo test -p elm-effect-schema --test schema_transform
PRINCIPLES: prove-it-works — a serde derive that cannot express transformations/brands does not count as matched; model-the-domain — parse errors accumulate as data
DESCRIPTION:
ADR-0002 Phase 6: "transformations, brands, and accumulated parse errors are
part of parity".

Deliverables:
- Port `SchemaTransformation.ts`, `SchemaGetter.ts`, `SchemaParser.ts`,
  `StandardSchema.ts`, `ChannelSchema.ts` behavior at the pin.
- Accumulated parse errors are a first-class result shape.
- Tests in `crates/elm-effect-schema/tests/schema_transform.rs`; flip the 5
  module rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-schema --test schema_transform
```

---

## w8-schema-tagged

TITLE: Schema: TaggedError + catch_tag, JsonSchema
PRIORITY: 1
TYPE: feature
LABELS: wave-8,phase-6,schema
VERIFY: cargo test -p elm-effect-schema --test tagged_error
PRINCIPLES: type-system-discipline — a tagged error is matched by its discriminant, not by string equality; boundary-discipline — catch_tag narrows the typed channel only
DESCRIPTION:
ADR-0002 Phase 6: "`TaggedError` is a schema-bearing error with a `_tag`
discriminant, matched by `catch_tag`."

Deliverables:
- `TaggedError` support (it lives in `Schema.ts` at the pin; there is no
  separate TS file) + `catch_tag` combinator restricted to the typed error
  channel.
- Port `JsonSchema.ts` behavior.
- Tests in `crates/elm-effect-schema/tests/tagged_error.rs`; flip the
  `JsonSchema.ts` row; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-schema --test tagged_error
```

---

## w8-schema-corpus

TITLE: Schema: corpus gate — schema test rows move to ported
PRIORITY: 1
TYPE: feature
LABELS: wave-8,phase-6,schema
VERIFY: cargo test -p elm-effect-schema --test schema_corpus
PRINCIPLES: prove-it-works — the corpus under packages/effect/test/schema/ is the gate; sequence-verifiable-units — residual rows become beads, not silence
DESCRIPTION:
ADR-0002 Phase 6: "The schema test corpus under `packages/effect/test/schema/`
is the gate for those rows."

Deliverables:
- Walk the behavior rows whose path starts `packages/effect/test/schema/`.
  Port them (or file follow-up beads for any that need a later capability,
  naming them in the bead close report).
- `crates/elm-effect-schema/tests/schema_corpus.rs` holds the ported corpus
  tests for this wave; every `ported` row names a Rust test.
- Flip `Schema.ts` and the remaining schema-crate module rows to `matched` only
  when their linked behavior rows are ported or bead-filed.

## VERIFY

```bash
cargo test -p elm-effect-schema --test schema_corpus
```

---

## w9-tx

TITLE: STM: TxRef, TxDeferred, TxQueue, TxSemaphore, Tx collections
PRIORITY: 2
TYPE: feature
LABELS: wave-9,phase-7,stm
VERIFY: cargo test -p elm-effect --test tx_core
PRINCIPLES: separate-before-serializing-shared-state — two facts that move together are one transactional value; model-the-domain — transactions compose
DESCRIPTION:
ADR-0002 Phase 7: "Software transactional memory (`TxRef`, `TxQueue`, and the
rest of the `Tx*` modules) is feature `stm`, default on."

Deliverables:
- Port `TxRef`, `TxDeferred`, `TxQueue`, `TxSemaphore`, `TxReentrantLock`,
  `TxChunk`, `TxHashMap`, `TxHashSet`, `TxPriorityQueue`, `TxPubSub`,
  `TxSubscriptionRef` at the pin, feature `stm` default-on.
- Tests in `crates/elm-effect/tests/tx_core.rs` derived from the linked
  `packages/effect/test/tx/` rows; flip the 11 module rows; port behavior rows;
  file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test tx_core
```

---

## w9-cache-resolver

TITLE: Stdlib: Cache single-flight, RequestResolver batching, Pool, ScopedCache
PRIORITY: 2
TYPE: feature
LABELS: wave-9,phase-7,stdlib
VERIFY: cargo test -p elm-effect --test cache_resolver
PRINCIPLES: make-operations-idempotent — identical concurrent keys run the lookup once; prove-it-works — single-flight and batching asserted with counting services
DESCRIPTION:
ADR-0002 Phase 7: "Cache single-flights identical keys. A request resolver
batches distinct keys. Failures are not cached unless the caller opts in."
"Single-flight and batching tests from the resilience rules: N identical
concurrent misses run the lookup once. M distinct keys in one window produce
one downstream call."

Deliverables:
- Port `Cache.ts`, `Request.ts`, `RequestResolver.ts`, `Pool.ts`,
  `ScopedCache.ts`; pool releases when idle with an explicit idle policy.
- Tests in `crates/elm-effect/tests/cache_resolver.rs`; flip the 5 module rows;
  port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test cache_resolver
```

---

## w9-resources

TITLE: Stdlib: RcMap, RcRef, ScopedRef, LayerMap, LayerRef, Latch, Resource
PRIORITY: 2
TYPE: feature
LABELS: wave-9,phase-7,stdlib
VERIFY: cargo test -p elm-effect --test stdlib_resources
PRINCIPLES: boundary-discipline — reference-counted resources release on scope, never on GC; laziness-protocol — reuse scope/acquire_release rather than a second lifecycle
DESCRIPTION:
ADR-0002 Phase 7 standard library list ("pool, cache, latch, request resolver,
rc-map").

Deliverables:
- Port `RcMap.ts`, `RcRef.ts`, `ScopedRef.ts`, `LayerMap.ts`, `LayerRef.ts`,
  `Latch.ts`, `Resource.ts`; rc-map releases when idle with an explicit idle
  policy.
- Tests in `crates/elm-effect/tests/stdlib_resources.rs`; flip the 7 module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test stdlib_resources
```

---

## w9-pubsub

TITLE: Stdlib: PubSub, SubscriptionRef, PartitionedSemaphore
PRIORITY: 2
TYPE: feature
LABELS: wave-9,phase-7,stdlib
VERIFY: cargo test -p elm-effect --test pubsub_subscription
PRINCIPLES: boundary-discipline — subscribers read snapshots and cannot write the source; model-the-domain — bounded capacity is a parameter, not a constant
DESCRIPTION:
ADR-0002 Phase 7 standard library list.

Deliverables:
- Port `PubSub.ts` (bounded), `SubscriptionRef.ts` (snapshot stream),
  `PartitionedSemaphore.ts`.
- Tests in `crates/elm-effect/tests/pubsub_subscription.rs`; flip the 3 module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test pubsub_subscription
```

---

## w10-elm-program

TITLE: Elm: Program/Update/Boot, Interpret, Durability, serde commands
PRIORITY: 2
TYPE: feature
LABELS: wave-10,phase-8,elm
VERIFY: cargo test -p elm-effect-elm --test program
PRINCIPLES: model-the-domain — a long-lived program is a fold, commands are data; boundary-discipline — the interpreter is the only place an effect starts
DESCRIPTION:
ADR-0002 Phase 8 main body. Law L12 (structure part).

Deliverables:
- `crates/elm-effect-elm` (depends on `elm-effect`, `serde` 1).
- `Program` trait as specified: `Model: Clone + PartialEq`, `Msg`, `Cmd`;
  `init(&Boot)`; pure `update(&Model, &Msg) -> Update<Model, Cmd>`.
- `Boot` is parsed configuration; no live socket, clock, or store. Time enters
  as a message; the host samples the clock and sends `Msg`.
- `Interpret<P: Program>` trait; commands `Serialize + Deserialize`; each
  command declares `Durability::{Ephemeral, Durable { key }}`.
- Runtime loop: fold, record durable intents, interpret each command into an
  effect, run it, feed the outcome back as a message. One entrance, one exit.
- Tests in `crates/elm-effect-elm/tests/program.rs`; `subscriber_cannot_write`
  is a type-level test (subscription handle has no send into the model).

## VERIFY

```bash
cargo test -p elm-effect-elm --test program
```

---

## w10-elm-replay

TITLE: Elm: replay, snapshot {model, offset}, middleware, subscriptions
PRIORITY: 2
TYPE: feature
LABELS: wave-10,phase-8,elm
VERIFY: cargo test -p elm-effect-elm fold_is_deterministic
PRINCIPLES: prove-it-works — the same log twice yields the same models and commands; boundary-discipline — replay never re-executes providers
DESCRIPTION:
ADR-0002 Phase 8 Redux rules + L12. The durable log lands in wave 12; this
bead's replay works against an in-memory log behind the same trait once wave 12
exists, and against a minimal in-crate log until then (do not fork the trait).

Deliverables:
- Time travel replays the message log from the latest snapshot; the snapshot
  stores the model and the log offset it includes. Default snapshot interval
  every 100 messages, configurable per program. Replay is a pure fold plus a
  comparison of the command lists; it does not re-execute providers (their
  results are already messages).
- Middleware (if written) is `fn(&Msg) -> Msg`, applied before `update`; no
  access to `Cmd`/`Effect`; sagas/thunks/listeners are not representable.
- Subscriptions are a stream of model snapshots (SubscriptionRef underneath);
  the handle cannot send into the model.
- Tests: `fold_is_deterministic` (same log twice), `replay_matches_commands`,
  `snapshot_is_model_plus_offset`.

## VERIFY

```bash
cargo test -p elm-effect-elm fold_is_deterministic
```

---

## w10-elm-lints

TITLE: Elm: lints — fold_no_ambient_world, fold_model_no_hashmap
PRIORITY: 2
TYPE: feature
LABELS: wave-10,phase-8,elm
VERIFY: cargo test -p elm-effect-elm --test lints
PRINCIPLES: encode-lessons-in-structure — the two fold hazards become lint failures; fix-root-causes — lint the class, not one occurrence
DESCRIPTION:
ADR-0002 Phase 8. Laws L12 (ambient-world part), L15.

Deliverables:
- Lint `fold_no_ambient_world`: flags `SystemTime`, `std::env`,
  `rand::thread_rng` inside an `update` function.
- Lint `fold_model_no_hashmap`: flags `std::collections::HashMap` inside a type
  that implements `Program::Model`.
- Implementation shape: a small `cargo`-runnable lint harness (dylint-style or
  a source scan test) in `crates/elm-effect-elm/tests/lints.rs` that fails on
  fixtures containing the hazards and passes on clean fixtures. It must be
  runnable by `cargo test` without extra tooling, or the bead close report
  names the exact lint command and CI wiring.
- Tests: `fold_no_ambient_world`, `fold_model_no_hashmap`.

## VERIFY

```bash
cargo test -p elm-effect-elm --test lints
```

---

## w11-journal-golden

TITLE: Journal: EffectState + golden can_transition table (no invented Parked edge)
PRIORITY: 2
TYPE: feature
LABELS: wave-11,phase-9,journal
VERIFY: cargo test -p elm-effect-journal golden_transitions
PRINCIPLES: prove-it-works — the golden table is copied from toron's can_transition, not imagined; fix-root-causes — the Parked gap stays visible instead of being papered over
DESCRIPTION:
ADR-0002 Phase 9, "Journal". Law L14 (registry part).

Deliverables:
- `crates/elm-effect-journal` (depends on `elm-effect`, `elm-effect-elm`).
- One `EffectState` enum: `Prepared`, `InFlight`, `Accepted`, `Settled`,
  `Failed`, `Unknown`, `ReconcilePending`, `Cancelled`, `Parked`. `Absent` is
  "no row", NOT a variant. No second enum, no boundary mirror.
- `can_transition` golden table exactly as toron-core has it (read
  `toron/crates/toron-core/src/index/workflow_effects.rs` as of 2026-10-02):
  Prepared→{InFlight,Failed,Cancelled}; InFlight→{Accepted,Settled,Failed,
  Unknown,Cancelled}; Accepted→{Settled,Failed,Unknown,Cancelled};
  Unknown→{ReconcilePending,Cancelled}; ReconcilePending→{Settled,Failed,
  Unknown,Cancelled}. `Parked` has NO effect-row edge; do not add one.
- Tests: `golden_transitions` accepts exactly those pairs and rejects
  `Settled`→`Unknown`, `Failed`→`Settled`, `Prepared`→`Settled`;
  `parked_is_not_terminal_not_failed_not_settled`.

## VERIFY

```bash
cargo test -p elm-effect-journal golden_transitions
```

---

## w11-journal-plan

TITLE: Journal: plan_attempt, may_settle, check_replay
PRIORITY: 2
TYPE: feature
LABELS: wave-11,phase-9,journal
VERIFY: cargo test -p elm-effect-journal --test plan_attempt
PRINCIPLES: prove-it-works — the dispatch table is encoded as table tests from the toron source; boundary-discipline — query beats idempotency, live lease beats both
DESCRIPTION:
ADR-0002 Phase 9, "Journal" (plan_attempt, may_settle, check_replay).

Deliverables:
- Pure, total `plan_attempt` over `Option<EffectView>` with `EffectView =
  { state, attempt, lease_live }` and `ProviderCapability
  { supports_idempotency_key, supports_reconcile_query }`, exactly the table in
  ADR-0002 (read `toron/crates/toron-workflow/src/engine/reconcile.rs` again —
  it moves; if moved, tests follow the file and the ADR table is updated in the
  same change).
- `may_settle(current_token, presented)`: true only when both non-empty and
  equal; empty presented token settles nothing; an unclaimed row settles for
  nobody.
- `check_replay(prepared_digest, redigest)`: mismatch is a typed error and
  mints NO second effect key.
- Tests: `crates/elm-effect-journal/tests/plan_attempt.rs` covering every arm —
  `Prepared`→`DispatchNew`, `ReconcilePending`→`RefuseBlindRetry`,
  query-beats-idempotency, `ResumeParked` attempt.max(1), `RetrySameKey`
  attempt+1; plus `empty_token_never_settles`,
  `replay_digest_mismatch_mints_nothing`, `unknown_has_no_default_exit`.

## VERIFY

```bash
cargo test -p elm-effect-journal --test plan_attempt
```

---

## w11-journal-park

TITLE: Journal: plan_park ceiling + pending upsert key
PRIORITY: 2
TYPE: feature
LABELS: wave-11,phase-9,journal
VERIFY: cargo test -p elm-effect-journal plan_park
PRINCIPLES: boundary-discipline — an over-ceiling wait is refused, never truncated; make-operations-idempotent — the pending key makes a repeated park one row
DESCRIPTION:
ADR-0002 Phase 9, "Journal" (park part).

Deliverables:
- `plan_park(until_ms, max_wait_secs, now_ms, ceiling)` pure; `ceiling` defaults
  to 86400 s; a wait above ceiling is a TYPED REFUSAL, not a shorter sleep; a
  past `until_ms` is `ResumeNow` reporting the overdue gap without wrapping.
  The host supplies `now_ms`; replay may move park→resume; that movement is the
  point.
- Park persists as a pending row: `upsert_pending` idempotent on
  `(run_id, promise_id, park_generation)`; a second upsert with the same key
  writes nothing. `ResumeNow` builds no intent; `park_intent` builds the value
  before the wait. No effect-row write to `parked`; `Parked` stays parseable
  and non-terminal.
- Tests: `plan_park` (refusal above 86400, no wrap, due-now), 
  `second_pending_upsert_writes_nothing`, `parked_row_is_resumable_not_terminal`.

## VERIFY

```bash
cargo test -p elm-effect-journal plan_park
```

---

## w11-journal-store

TITLE: Journal: JournalStore trait, generation fence, atomic offset+state
PRIORITY: 2
TYPE: feature
LABELS: wave-11,phase-9,journal
VERIFY: cargo test -p elm-effect-journal --test journal_store
PRINCIPLES: make-operations-idempotent — a stale generation settles nothing; boundary-discipline — the trait is the contract, toron implements it later
DESCRIPTION:
ADR-0002 Phase 9, "Journal" (claim/settle/offsets).

Deliverables:
- `JournalStore` trait: intent before provider, same key on retry, `Unknown`
  has no default exit (reconcile-only toward `Settled`/`Failed`/`Unknown`/
  `Cancelled`), terminal states never move, `Accepted` is its own state,
  claim sets owner + lease deadline + `generation + 1` atomically and returns
  the row, settle carries the generation and a mismatch settles nothing,
  exhausted attempts become a dead-letter row (`Failed` + `disposition`,
  matching toron's column — do not invent a parallel one).
- The next offset and the state derived from it commit together; offsets move
  only forward; an offset older than retention returns `Gone` with
  `FirstReplayable` (the caller chooses `-1` or `now`; no silent skip).
- Tests: `crates/elm-effect-journal/tests/journal_store.rs`; no public function
  in this crate takes issue events and returns gate rows.

## VERIFY

```bash
cargo test -p elm-effect-journal --test journal_store
```

---

## w12-log-append

TITLE: Durable log: DurableLog trait, offset minting, atomic append, seq rules
PRIORITY: 2
TYPE: feature
LABELS: wave-12,phase-9b,durable-log
VERIFY: cargo test -p elm-effect-journal --test durable_log_append
PRINCIPLES: boundary-discipline — the log is a trait; the kernel runs in-process; make-operations-idempotent — same triple appends once
DESCRIPTION:
ADR-0002 Phase 9b, "Durable log". Law L16 (append part). Read
`skills/elm-effect/references/durable-log.md` before coding.

Deliverables:
- `DurableLog` trait in `elm-effect-journal`; first implementation in-process
  (the log IS the stream; it is not a client of the Node server).
- Offsets: opaque, lexicographically ordered, minted by the log; sentinels `-1`
  (start) and `now` (tail) are never minted as real offsets; a hand-built
  offset is rejected.
- Append is atomic (crash leaves the previous offset; a read never returns a
  partial message; JSON mode keeps boundaries). Producer identity is
  `(producer_id, epoch, seq)` all-or-none; a seq that is not `last + 1` and not
  an already accepted seq is rejected. Each JSON message carries the schema
  version; an unknown version is a typed error.
- Tests: `crates/elm-effect-journal/tests/durable_log_append.rs` —
  `hand_built_offset_is_rejected`, `seq_gap_is_rejected`,
  `partial_identity_is_rejected`, `append_is_atomic`,
  `unknown_schema_version_is_typed_error`.

## VERIFY

```bash
cargo test -p elm-effect-journal --test durable_log_append
```

---

## w12-log-epoch

TITLE: Durable log: epoch fencing, same-seq dedupe, no epoch TTL
PRIORITY: 2
TYPE: feature
LABELS: wave-12,phase-9b,durable-log
VERIFY: cargo test -p elm-effect-journal stale_epoch_is_fenced
PRINCIPLES: fix-root-causes — a zombie after a TTL double-writes mail; epoch state never expires here; make-operations-idempotent — the same triple appends once
DESCRIPTION:
ADR-0002 Phase 9b. Law L16 (epoch part).

Deliverables:
- The same `(producer_id, epoch, seq)` appends once; a lower epoch is fenced; a
  new epoch starts at sequence 0; epoch state does NOT expire (stricter than
  the draft's TTL — say so in the module docs).
- A fork inherits bytes up to an offset and does NOT inherit producer epoch;
  the fork's first epoch is 0.
- Tests: `same_epoch_same_seq_appends_once`, `stale_epoch_is_fenced`,
  `new_epoch_starts_at_zero`, `fork_drops_producer_epoch`.

## VERIFY

```bash
cargo test -p elm-effect-journal stale_epoch_is_fenced
```

---

## w12-log-gone

TITLE: Durable log: Gone kinds, close, snapshot record, collapsing_cursor is not an offset
PRIORITY: 2
TYPE: feature
LABELS: wave-12,phase-9b,durable-log
VERIFY: cargo test -p elm-effect-journal --test durable_log_gone_fork
PRINCIPLES: boundary-discipline — retention Gone and soft-delete Gone are different answers; prove-it-works — up_to_date is not EOF and the tests say so
DESCRIPTION:
ADR-0002 Phase 9b. Law L16 (Gone/close part).

Deliverables:
- `UpToDate` means caught up (more may arrive); `Closed` is EOF (closure
  monotonic; appends after close fail; bytes before close stay readable); an
  offset older than retention is `Gone` with `FirstReplayable`, and a
  soft-deleted stream is a DIFFERENT `Gone` (callers match the reason).
- A snapshot is the pair `{model, offset}` stored beside the stream.
- `collapsing_cursor` is the HTTP CDN fan-in token only; it is never stored as
  a fold position (the resume field is `offset`).
- Tests: `up_to_date_is_not_closed`, `append_after_close_fails`,
  `closed_stream_stays_readable`, `retention_gone_is_not_a_skip`,
  `soft_delete_gone_is_not_retention`, `snapshot_is_model_plus_offset`.

## VERIFY

```bash
cargo test -p elm-effect-journal --test durable_log_gone_fork
```

---

## w13-observability

TITLE: Observability: Logger, Metric, Tracer, redaction
PRIORITY: 2
TYPE: feature
LABELS: wave-13,phase-10,observability
VERIFY: cargo test -p elm-effect secret_debug_is_redacted
PRINCIPLES: boundary-discipline — logging is an effect and secrets stay redacted at the type; prove-it-works — L11 asserts absence of bytes, not presence of labels
DESCRIPTION:
ADR-0002 Phase 10, "Observability". Law L11 (redaction part).

Deliverables:
- `Logger`, `Metric`, `Tracer` services on the stable surface; `tap` observes
  without changing result or error; spans wrap an effect; logging is an effect.
- Port `LogLevel.ts`, `References.ts`, `ErrorReporter.ts`, `Logger.ts`,
  `Metric.ts`, `Tracer.ts`, `Redacted.ts`, `Redactable.ts` behavior at the pin.
- Secrets implement redaction in `Debug` and in span fields.
- Tests: `secret_debug_is_redacted` (secret bytes absent from the debug string),
  `span_name_and_attributes` (no socket), `tap_does_not_change_result`.

## VERIFY

```bash
cargo test -p elm-effect secret_debug_is_redacted
```

---

## w13-otel

TITLE: Observability: OpenTelemetry exporter behind feature otel
PRIORITY: 2
TYPE: feature
LABELS: wave-13,phase-10,observability
VERIFY: cargo test -p elm-effect-unstable --test otel_exporter
PRINCIPLES: boundary-discipline — the exporter is an adapter behind an unstable feature; prove-it-works — the test asserts span name/attributes without a collector
DESCRIPTION:
ADR-0002 Phase 10: "The exporter is OpenTelemetry, crate `opentelemetry_sdk`
0.28 to match chiebukuro's existing dependency, behind feature `otel` on
`elm-effect-unstable` until the stable tracer trait has a matched test that
does not need a collector."

Deliverables:
- `crates/elm-effect-unstable` crate with feature `otel` (off by default);
  `opentelemetry_sdk` 0.28 only under that feature.
- Tests in `crates/elm-effect-unstable/tests/otel_exporter.rs`: span name and
  attributes asserted without opening a socket. No default-build dependency.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test otel_exporter
```

---

## w14-fs-path

TITLE: Platform: FileSystem and Path services with fakes
PRIORITY: 2
TYPE: feature
LABELS: wave-14,phase-11,platform
VERIFY: cargo test -p elm-effect --test platform_fs_path
PRINCIPLES: boundary-discipline — std::fs/std::process live behind services so tests pass fakes; type-system-discipline — no ambient filesystem in domain code
DESCRIPTION:
ADR-0002 Phase 11, "Platform substitutes": FileSystem, Path.

Deliverables:
- `FileSystem.ts`, `Path.ts`, `PlatformError.ts` behavior ported behind
  services (`std::fs` implementations), tests pass fakes.
- Tests in `crates/elm-effect/tests/platform_fs_path.rs`; flip module rows;
  port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test platform_fs_path
```

---

## w14-io

TITLE: Platform: Stdio, Terminal, Console services
PRIORITY: 2
TYPE: feature
LABELS: wave-14,phase-11,platform
VERIFY: cargo test -p elm-effect --test platform_io
PRINCIPLES: boundary-discipline — I/O devices are services; prove-it-works — fakes observe writes without a terminal
DESCRIPTION:
ADR-0002 Phase 11 / Product shape: stdout is the success value, logs go to
stderr through `Logger`.

Deliverables:
- `Stdio.ts`, `Terminal.ts`, `Console.ts` behavior ported behind services with
  fakes; exit-code discipline (0/1/2/130) documented against `run_main`.
- Tests in `crates/elm-effect/tests/platform_io.rs`; flip module rows; port
  behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test platform_io
```

---

## w14-config-crypto

TITLE: Platform: Config, ConfigProvider, Crypto services
PRIORITY: 2
TYPE: feature
LABELS: wave-14,phase-11,platform
VERIFY: cargo test -p elm-effect --test platform_config_crypto
PRINCIPLES: type-system-discipline — config is a value parsed before the runtime starts; boundary-discipline — crypto randomness is a service, seeded in tests
DESCRIPTION:
ADR-0002 Phase 11 / "Layers and context": "Config is parsed at startup into a
struct the layer closes over. Domain code does not read environment variables."

Deliverables:
- `Config.ts`, `ConfigProvider.ts`, `Crypto.ts` behavior ported behind
  services; config parsed once; crypto substitutes `getrandom` behind a
  service so tests are deterministic.
- Tests in `crates/elm-effect/tests/platform_config_crypto.rs`; flip module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect --test platform_config_crypto
```

---

## w15-workflow

TITLE: Unstable: workflow module (Activity replay, DurableClock/Deferred/Queue, compensation)
PRIORITY: 2
TYPE: feature
LABELS: wave-15,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test workflow_module
PRINCIPLES: make-operations-idempotent — activity results are stored and replayed; boundary-discipline — compensation fires on failure and cancel ONLY
DESCRIPTION:
ADR-0002 Phase 12, "Platform substitutes": "Workflow, cluster, event log,
persistence, rpc — unstable crate, semantics from the 4.0.0 modules (`Activity`
results stored and replayed, `DurableClock`, `DurableDeferred`, `DurableQueue`,
compensation on failure and cancel only)."

Deliverables:
- Port every ledger row under `packages/effect/src/workflow/` (9 modules)
  behind feature `workflow` (off by default) with semantics from the pin.
- Tests in `crates/elm-effect-unstable/tests/workflow_module.rs`; flip the 9
  module rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test workflow_module
```

---

## w15-eventlog

TITLE: Unstable: eventlog module
PRIORITY: 2
TYPE: feature
LABELS: wave-15,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test eventlog_module
PRINCIPLES: model-the-domain — the event log is an append-only data contract, not a cache; sequence-verifiable-units — module rows land with their tests
DESCRIPTION:
ADR-0002 Phase 12, "event log" under the unstable crate.

Deliverables:
- Port every ledger row under `packages/effect/src/eventlog/` (15 modules)
  behind feature `eventlog` (off by default).
- Tests in `crates/elm-effect-unstable/tests/eventlog_module.rs`; flip the 15
  module rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test eventlog_module
```

---

## w15-persistence

TITLE: Unstable: persistence module
PRIORITY: 2
TYPE: feature
LABELS: wave-15,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test persistence_module
PRINCIPLES: boundary-discipline — the store is a service, the model is data; make-operations-idempotent — replay-safe writes
DESCRIPTION:
ADR-0002 Phase 12, "persistence" under the unstable crate.

Deliverables:
- Port every ledger row under `packages/effect/src/persistence/` (8 modules)
  behind feature `persistence` (off by default).
- Tests in `crates/elm-effect-unstable/tests/persistence_module.rs`; flip the
  8 module rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test persistence_module
```

---

## w15-rpc

TITLE: Unstable: rpc module
PRIORITY: 2
TYPE: feature
LABELS: wave-15,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test rpc_module
PRINCIPLES: boundary-discipline — serialization at the RPC boundary, typed errors inside; model-the-domain — schemas define the wire
DESCRIPTION:
ADR-0002 Phase 12, "rpc" under the unstable crate.

Deliverables:
- Port every ledger row under `packages/effect/src/rpc/` (13 modules) behind
  feature `rpc` (off by default).
- Tests in `crates/elm-effect-unstable/tests/rpc_module.rs`; flip the 13 module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test rpc_module
```

---

## w16-http

TITLE: Unstable: HTTP client (ureq 2 first), http-api, net
PRIORITY: 2
TYPE: feature
LABELS: wave-16,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test http_stack
PRINCIPLES: boundary-discipline — HttpClient is the service, ureq is one impl on spawn_blocking; laziness-protocol — first client is the dependency the stack already carries
DESCRIPTION:
ADR-0002 Phase 12 + "Platform substitutes": "HTTP client: a `HttpClient`
service. First implementation: `ureq` 2 ... synchronous on `spawn_blocking`. A
second implementation may be `hyper` later. The service is the spec."

Deliverables:
- Port every ledger row under `packages/effect/src/http/` (33), `http-api/`
  (14), `net/` (4) behind feature `http` (off by default). `ureq` 2 only under
  that feature.
- Tests in `crates/elm-effect-unstable/tests/http_stack.rs`; flip the 51
  module rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test http_stack
```

---

## w16-sql-socket

TITLE: Unstable: SQL service and socket modules
PRIORITY: 2
TYPE: feature
LABELS: wave-16,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test sql_socket
PRINCIPLES: boundary-discipline — the driver the sibling already uses is one service impl; the framework does not depend on a database in default features
DESCRIPTION:
ADR-0002 Phase 12 + "Platform substitutes": "SQL: a `Sql` service. First
implementation: the driver the sibling already uses (`sqlx` in toron)."

Deliverables:
- Port every ledger row under `packages/effect/src/sql/` (10) and `socket/`
  (3) behind features `sql` and `socket` (off by default); no default-build DB
  dependency. Driver choice stays behind the service; tests use fakes unless
  the row's behavior requires a real driver (then document the test DB).
- Tests in `crates/elm-effect-unstable/tests/sql_socket.rs`; flip the 13 module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test sql_socket
```

---

## w16-cli-ai

TITLE: Unstable: CLI (clap 4) and AI LanguageModel trait
PRIORITY: 2
TYPE: feature
LABELS: wave-16,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test cli_ai
PRINCIPLES: boundary-discipline — argv parses to a typed request before any effect runs; laziness-protocol — no AI provider in the default build
DESCRIPTION:
ADR-0002 Phase 12 + "Platform substitutes": CLI behind clap 4 as a service
(testable without a process); AI `LanguageModel` trait with generate-text,
generate-object (schema), stream; no provider in the default build.

Deliverables:
- Port every ledger row under `packages/effect/src/cli/` (13) and `ai/` (21)
  behind features `cli` and `ai` (off by default); `clap` 4 only under `cli`.
- Tests in `crates/elm-effect-unstable/tests/cli_ai.rs`; flip the 34 module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test cli_ai
```

---

## w16-enc-test-work

TITLE: Unstable: encoding, testing, process, devtools, reactivity, workers
PRIORITY: 2
TYPE: feature
LABELS: wave-16,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test enc_test_work
PRINCIPLES: sequence-verifiable-units — the small subpaths land as one coherent sweep with their own rows; boundary-discipline — host adapters stay behind features
DESCRIPTION:
ADR-0002 Phase 12. These subpaths are already-unstable in the tag; the port
keeps them in `elm-effect-unstable` behind feature flags mapping to their
ledger rows (off by default).

Deliverables:
- Port every ledger row under `packages/effect/src/encoding/` (11),
  `testing/` (4; `TestClock`-linked rows live in `elm-effect-test` where the
  class is `port`), `process/` (3), `devtools/` (5), `reactivity/` (9),
  `workers/` (5).
- Tests in `crates/elm-effect-unstable/tests/enc_test_work.rs`; flip the module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test enc_test_work
```

---

## w16-schema-obs

TITLE: Unstable: schema subpath and observability subpath
PRIORITY: 2
TYPE: feature
LABELS: wave-16,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test schema_obs
PRINCIPLES: boundary-discipline — the schema subpath does not retag Schema.ts; sequence-verifiable-units — subpath rows land as their own sweep
DESCRIPTION:
ADR-0002 Phase 12 note: "The `schema` subpath is on the migration list of
paths that stayed unstable, which does not retag `Schema.ts`."

Deliverables:
- Port every ledger row under `packages/effect/src/schema/` (9) and
  `observability/` (9) behind their feature flags (off by default).
- Tests in `crates/elm-effect-unstable/tests/schema_obs.rs`; flip the 18 module
  rows; port behavior rows; file residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test schema_obs
```

---

## w17-cluster-core

TITLE: Unstable: cluster core (last of the unstable family, part 1)
PRIORITY: 2
TYPE: feature
LABELS: wave-17,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test cluster_core
PRINCIPLES: sequence-verifiable-units — cluster is last by design; model-the-domain — membership and sharding are data
DESCRIPTION:
ADR-0002 Phase 12: "Cluster is last."

Deliverables:
- Port the core ledger rows under `packages/effect/src/cluster/` (first ~20):
  membership, sharding, entity routing primitives at the pin, behind feature
  `cluster` (off by default).
- Tests in `crates/elm-effect-unstable/tests/cluster_core.rs`; flip rows as
  tests land; port behavior rows; file residuals for the rest.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test cluster_core
```

---

## w17-cluster-rest

TITLE: Unstable: cluster rest (workflows, singleton, pubsub surfaces)
PRIORITY: 2
TYPE: feature
LABELS: wave-17,phase-12,unstable
VERIFY: cargo test -p elm-effect-unstable --test cluster_rest
PRINCIPLES: sequence-verifiable-units — the remaining cluster rows close the phase; prove-it-works — rows flip only with tests
DESCRIPTION:
ADR-0002 Phase 12: remaining cluster ledger rows.

Deliverables:
- Port every remaining ledger row under `packages/effect/src/cluster/` at the
  pin, behind feature `cluster`.
- Tests in `crates/elm-effect-unstable/tests/cluster_rest.rs`; flip every
  remaining cluster module row to `matched`; port behavior rows; file
  residuals.

## VERIFY

```bash
cargo test -p elm-effect-unstable --test cluster_rest
```

---

## w18-ledger-done

TITLE: Phase 13: ledger done-check — every port row matched, every behavior ported
PRIORITY: 2
TYPE: feature
LABELS: wave-18,phase-13,conformance
VERIFY: cargo test -p elm-effect-conformance -- --ignored
PRINCIPLES: prove-it-works — the ignored test is the definition of done, not a percentage; sequence-verifiable-units — residuals must be closed as beads before this passes
DESCRIPTION:
ADR-0002 Phase 13. No features.

Deliverables:
- Every `port`, `unstable-port`, `experimental-port` module row in
  `conformance/modules.toml` is `matched`.
- Every behavior row in `conformance/behavior-tests.tsv` is `ported` (with a
  non-empty `rust`) or `skipped-upstream`.
- `cargo test -p elm-effect-conformance -- --ignored` fails while any row is
  open and passes when none is. The default `cargo test -p
  elm-effect-conformance` stays green throughout.
- `index.ts` and `Arbitrary.ts` rows are closed (facade re-export surface +
  unstable row reasons) as part of this wave.

## VERIFY

```bash
cargo test -p elm-effect-conformance -- --ignored
```
