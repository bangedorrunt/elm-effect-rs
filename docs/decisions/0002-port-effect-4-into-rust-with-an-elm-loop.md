---
status: proposed
date: 2026-10-02
decision-makers: operator of the flywheel, toron, beads, and chiebukuro stack
consulted: Effect 4.0.0 tag (source and release), effect.website v4 onboarding, the effect-oriented discipline, toron workflow journal, flywheel effect journal, chiebukuro ADR-0011, beads close policy
informed: agents implementing this repository and agents later adopting it inside the four siblings
---

# Port Effect 4.0.0 into a Rust carrier with an Elm loop

## How to read this

The decision, in one page, is the next section. Everything after it is the specification an agent implements. The constitution (the laws) is the part that has to stay true if the TypeScript library, the executor, or the sibling codebases change. The crate map and the phases say where code goes and in what order.

This record is `proposed`. That status is not a stop. The agent-authority section says when a named sibling may be edited.

## Decision in one page

Build `elm-effect`, a Rust workspace that implements the behavior of the `effect` package at git tag `effect@4.0.0` (published 2026-10-01). The carrier is a value, `Effect<A, E, R>`, interpreted by a fiber runtime we own. Tokio is the first production executor. It is not the specification.

Beside that carrier, in a separate crate, ship an Elm loop: a pure fold `(model, message) -> (model, commands)` whose commands are data, interpreted into effects at the edge. That loop is the Redux-shaped complement. A long-lived program in this stack is the loop. A one-shot program is a single effect run at the process edge. Both use the same carrier.

Parity means every public module of that tag is classified, and every behavior test under `packages/effect/test` has a Rust test with the same observable result. A module row is not done while one of those tests is still open. The inventory is generated from the tag, not hand-copied. TypeScript-only modules (`HKT`, `Pipeable`, `Unify`, `Types`, `Newtype`, `Inspectable`, `Symbol`, `Effectable`) are classified `native`: Rust already has the feature, and we do not port the polyfill. Node, Bun, and DOM adapters are classified `platform-foreign`.

The framework does not depend on flywheel, toron, beads, or chiebukuro. They will depend on it, later, each through its own adoption record. The journal vocabulary is the one toron already persists (`Prepared`, `InFlight`, `Accepted`, `Settled`, `Failed`, `Unknown`, `ReconcilePending`, `Cancelled`, `Parked`). Resonate stays an engine adapter. It is not the carrier, and this record does not remove it.

Durable park is a fifth outcome. It is suspension. It is not success, not a typed failure, not a defect, and not an in-process interrupt. The persisted transition function `can_transition` in toron-core, as read on 2026-10-02, does not admit `Parked`. The engine still plans `ResumeParked`. The port keeps those facts apart and does not "fix" the gap by collapsing park into failed or settled.

## Context and Problem Statement

Effect (TypeScript) describes a program as a value: what it returns, how it can fail, and which services it needs. Policy (retry, timeout, fallback, limit) attaches to that value. A scope releases resources on success, typed failure, and interruption. The same idea, at the speed of a long-running process, is an Elm fold: the model changes only in a pure function, and the world happens in commands the function returns.

The four sibling codebases are already half of this design, each in its own dialect.

* Flywheel's `decide` is a pure function from a snapshot to a decision (`src/decide.rs`). Its durable swarm loop is a Resonate workflow whose `tick` is a pure reducer (`src/durable.rs`, governed by ADR-0024 through ADR-0027). Its effectful leaves are supposed to pass through a host-owned journal (`src/effect_journal.rs`, governed by ADR-0032 and ADR-0035): commit a prepared intent, claim it with a fencing token, invoke the provider once, settle, fail, or record `unknown`. The executor under that journal is a hand-built pool: 4 worker threads, one Tokio runtime, a bounded queue of 4096. The module's own comment records the ceiling: a deeper nesting of submits can starve, and the submitter blocks because the engine's runners are synchronous closures.
* Toron owns the journal the host uses. `toron_core::index::workflow_effects::EffectState` is the persisted lifecycle. `can_transition` is the legal-edge function. `toron_workflow::engine::plan_attempt` is the pure dispatch decision (`DispatchNew`, `RetrySameKey`, `ReconcileFirst`, `RefuseBlindRetry`, `DuplicateTerminal`, `ResumeParked`). `plan_park` is pure over `(until_ms, max_wait_secs, now_ms)` and refuses a wait above `PARK_MAX_WAIT_SECS` (86400) instead of truncating it. A second enum, `toron_workflow::engine::boundary::EffectState`, mirrors the lifecycle without `Parked` and with an extra `None`. The park module's doc comment names `boundary::EffectState::Parked`, which is not a variant. The test that actually holds the invariant imports the core enum.
* Beads stores issues, events, and dependencies as data (`src/model/`). Close is a policy gate over that data (`src/close_policy.rs`). The issues JSONL log is a partial source of truth: it holds issues, and it does not hold gate verdicts. Rebuilding the database from that log destroys verdicts. A framework helper that "replays one log into all projections" would reintroduce that bug.
* Chiebukuro runs long pipelines as `#[resonate::function]` checkpoints (ADR-0011, `docs/decisions/0011-chiebukuro-daemon-resonate.md`). Its agent docs forbid side effects outside `resonate.run` when the work must survive a restart. The Rust SDK is a git dependency on `resonatehq/resonate-sdk-rs` branch `main`, not a crates.io pin.

What is missing is one carrier those dialects can share. Today a retry in flywheel, a park in toron, a gate in beads, and a checkpoint in chiebukuro do not compose, do not share a test clock, and do not share a type for "this failed, this was cancelled, this is waiting, this is unknown". Each new leaf re-implements a slice of the stack and gets the slice boundaries wrong: a timeout that includes a queue wait, a nested runtime, a park modelled as failure so `on_error` fires, a verdict rebuilt from the wrong log.

The question this record answers: what do we build, in this empty repository, so that a later adoption can move each sibling onto one carrier without losing the invariants they already paid for, and without declaring the port done while Effect behavior is still missing?

## Decision Drivers

* Behavioral parity with `effect@4.0.0`. The tag is the spec. `main` is not the spec.
* The four outcomes Effect keeps apart, plus the fifth outcome this stack already persists (`Parked`).
* An Elm fold as the shape of a long-lived agent, with commands as data so a log can be replayed.
* Dependency direction: this repository depends on no sibling. Siblings will depend on it.
* The async-drop hole in Rust. `Drop` is synchronous. A timeout that works by dropping a future does not run an async release. The carrier has to run finalizers itself.
* The R channel (required services) has to be a compile-time fact. A `TypeId` map as the only context is a hole in the type that Effect's whole point is to close.
* Durability the siblings already have: intent before invoke, same key on retry, unknown stays unknown, acceptance is not delivery, generation fence, lease, ordered replay.
* One vocabulary an agent can learn once and apply in all four repos.
* The port has to be implementable in phases. Phase 3 already unblocks a sibling experiment. The product is done when the witness list passes. The ledger is an index, not that finish line.
* Compile times and allocations have ceilings, written down, so a later agent does not "simplify" the R channel away or chase zero-cost before the laws exist.
* Nightly Rust, edition 2024, the toolchain the siblings already float.

## Considered Options

* Semantic port: our own fiber runtime, `Effect<A, E, R>`, Elm loop in a sibling crate, journal vocabulary taken from toron's persisted states.
* Conventions only: `Result`, Tokio, traits, and a written discipline, with no carrier.
* Futures as the carrier: `impl Future<Output = Result<A, E>>` plus generic services.
* A TypeScript sidecar or a WASM build of Effect, called from Rust.
* Resonate as the carrier.
* A line-by-line transpilation of the TypeScript sources.
* ZIO or cats-effect over FFI.

## Decision Outcome

Chosen option: "Semantic port", because it is the only option that gives the siblings a value they can retry, time out, scope, test with a fake clock, and replay, while leaving Resonate, SQLite, JSONL, and Nostr where they are.

The rest of this record is that option, in enough detail to implement.

### What "100% feature and test parity" means

Parity is a ledger, `conformance/modules.toml`, generated by `conformance/inventory.sh` from a pinned checkout of tag `effect@4.0.0`. The script lists every `packages/effect/src/**/*.ts` file outside `internal/`, greps the first 40 lines for `@stability`, and records the test file that covers it when one exists under `packages/effect/test/`. On 2026-10-02 the whole tag tree contained 507 paths matching `*.test.ts`, across every package. The script maps only `packages/effect/test/` and stores the count it measured there. CI fails if a regenerated inventory differs from the committed one while the pin is unchanged.

Each row has a class:

| Class | Meaning | Done when |
| --- | --- | --- |
| `port` | A public behavior of the tag that Rust must reproduce | A Rust test module, named in the row, fails if that behavior changes, and passes against the implementation |
| `native` | A TypeScript polyfill for a feature Rust has as a language feature | The row names the Rust feature. No module is added to ape the polyfill |
| `platform-foreign` | Node, Bun, DOM, or a JavaScript host API | The row names the host API and the Rust platform substitute if any (`std::fs`, a process service). No Node binding is written |
| `unstable-port` | A module whose own header carries `@stability unstable` | Same as `port`, and the Rust item lives in `elm-effect-unstable`. A minor release of that crate may break it |
| `experimental-port` | A module whose own header carries `@stability experimental` | Same as `port`, in `elm-effect-unstable`. A patch release of that crate may break it |

A row in `stubbed` is not done. A type that exists without the test is `stubbed`.

The spec an implementer needs is this record, plus `conformance/PIN`, plus `conformance/modules.toml`, plus `conformance/behavior-tests.tsv`, plus a checkout of tag `effect@4.0.0`. The behavior rows are not copied into this file. Their expected results are the TypeScript tests at that tag.

Ledger contract, which `conformance/inventory.sh` implements:

* A module row's fields are `path`, `class`, `status`, `tag`, `reason`, `tests`.
* A test file links to a source file only when the path under `packages/effect/test/`, with `.test.ts` removed, equals the path under `packages/effect/src/`, with `.ts` removed. `src/ai/Prompt.ts` links to `test/ai/Prompt.test.ts` and not to `test/cli/Prompt.test.ts`. A behavior file that matches no source file stays in the behavior ledger and still blocks the port.
* A behavior row's columns, tab-separated, are `status`, `path`, `line`, `kind`, `title`, `rust`.
* `status` is `unported`, `ported`, `skipped-upstream`, or `not-applicable`. `ported` requires a non-empty `rust` column naming the Rust test, and an empty `rust` column on a `ported` row fails `cargo test -p elm-effect-conformance`. `not-applicable` is the behavior-row mirror of the module classes `native` and `platform-foreign`: it marks a call whose observable result Rust cannot express, and it carries a non-empty reason in the `rust` column saying why. An empty reason on a `not-applicable` row fails the same test, so a row can be excused only with a written reason, never silently. `still_open` counts `unported` rows only, so `not-applicable` is excluded from the gate exactly as `skipped-upstream` is. No behavior is faked: a row is `not-applicable` only when the claim itself cannot be reproduced in Rust, not when it is hard or merely not yet written.
* Regenerating keeps a behavior row's status and `rust` value when path, line, and title are unchanged. It keeps a module row's `matched` or `unmatched` status when the path still classifies as `port`, `unstable-port`, or `experimental-port`.

Behavior parity is `conformance/behavior-tests.tsv`, written by the same script. It has one row for every `it`, `it.effect`, `it.live`, `it.layer`, `it.each`, `it.skipIf`, `it.skip`, and `test` call under `packages/effect/test`, including suite helpers that are not named `.test.ts`. `it.skip` is `skipped-upstream`. Every other row is `unported` until a Rust test reproduces that call's observable result. An `it.each` row is ported only when the Rust test runs every tuple in that table. A title that is a template string, or a call inside a `for`, runs once per element that loop feeds. A module row is not `matched` while any behavior row from its linked test file is still `unported`.

The product is done when this witness list passes. The ledger stays an index of Effect 4.0.0. A module we will not carry is `not-applicable` with a reason. Matching every row is not the finish line.

* A dropped description does no I/O.
* Success, typed failure, defect, and interrupt stay apart.
* Release runs on success, typed failure, and interrupt.
* The attempt timeout does not cover the queue wait.
* The test clock moves only after the sleeper is forked.
* A layer cycle fails at construction.
* A command window has a bound. Every exit reaches the fold as a message. A durable append happens before the provider runs. A subscriber reads the model after a later send.

The ignored conformance test is a ledger health check. A red result there does not mean the product is unfinished.

Stability is per module file, not per directory. An untagged module follows strict semver. `@stability unstable` may break in a minor release. `@stability experimental` may break in a patch. `MIGRATION.md` at the tag defines both. The pages opened in the cross-check did not name an experimental module. Phase 0 still greps for the tag.

A directory barrel that mentions `@stability unstable` does not make every file under that directory unstable. `TestClock` is untagged. `TestSchema` is unstable. `Base64` and `EncodingError` are untagged. `Ndjson` is unstable. `Schema.ts` is untagged. The `schema` subpath is on the migration list of paths that stayed unstable, which does not retag `Schema.ts`. `Arbitrary` is unstable on its reference page even though the file sits at `packages/effect/src/Arbitrary.ts`. Phase 0 classifies `Arbitrary.ts` from its own header, and `conformance/overrides.toml` marks it `unstable-port` if that header has no tag, with the reason "reference title is unstable".

Headers read for this record (`Effect.ts`, `Stream.ts`, `Schema.ts`, `Layer.ts`) have `@since` and no stability tag. The v4 reference at the same commit as the tag (`67ba4e46a11ccda0b6761578bfd22c04ae00167d`) groups the package as 356 module pages. That count is a cross-check. The ledger counts source files at the tag, and the tag wins if the two ever diverge. There is no `packages/effect/src/unstable` directory at the tag, and no `effect/unstable/*` compatibility export.

`@effect/platform-node`, `@effect/platform-bun`, `@effect/sql-*` driver packages, `@effect/ai-*` provider packages, `@effect/opentelemetry`, and `@effect/vitest` are not the `effect` package. They are out of the ledger. Substitutes are specified under Platform substitutes below. They are part of the port only where a sibling needs the capability, and they are still specified here so a later agent does not invent a second HTTP stack.

### Constitution

These laws are the spec. A passing test suite that breaks a law is a failed port. Each law names the test that pins it. The tests are written in the phase that introduces the feature, before the implementation (the test file is the first commit of the phase, and it fails for lack of the type).

**L1. A description does not run.** Constructing an `Effect`, dropping it, or storing it in a `Vec` performs no I/O, no sleep, and no state change. The test `description_drop_runs_nothing` counts calls on a service and asserts zero after the value is dropped.

**L2. Five outcomes stay apart.** Success, typed failure (`Fail`), defect (`Die`), in-process interruption (`Interrupt`), and durable park (`Parked`) are five representations. A handler for typed failure does not swallow a defect or an interrupt. A park does not satisfy `is_terminal` and does not satisfy a typed-error match. The test `five_outcomes_do_not_collapse` builds one of each and asserts the classifiers. `catch` on the error channel leaves `Die` and `Interrupt` in place. `catch_cause` is the only combinator that sees all three in-process reasons, and library code does not call it. The fiber supervisor is the one place that may.

**L3. One carrier, run at the edge.** A library function returns `Effect`. The binary, or one bridge equivalent to Effect's `ManagedRuntime`, calls `run`. The test `library_crate_has_no_run` is a workspace lint: `elm_effect::Runtime::run` and `block_on` appear only in crates named in the lint's allow list (`elm-effect` itself, `elm-effect-test`, binaries under `examples/`).

**L4. Policy is a value, in one order.** The default remote stack, outermost to innermost, is: filter, fallback, cache, budget timeout, retry, circuit breaker, rate limit, bulkhead, attempt timeout, call. `policy::remote` builds that order. The test `attempt_timeout_does_not_cover_the_queue` uses a limiter that waits longer than the attempt timeout, a call that returns immediately once admitted, and asserts success. Retry wraps the smallest idempotent region. The test `retry_releases_per_attempt` acquires a resource inside the retried effect, fails twice, succeeds once, and asserts three releases and one surviving outer resource.

**L5. Services are typed, acyclic, and role-distinct.** Two uses of one capability are two service types. Providing the same service tag twice in one build is a defect. A cycle fails layer construction, and the failure is a construction error, not a per-request error. Config is a value parsed before the runtime starts. The test `cycle_fails_construction` builds `A` requiring `B` and `B` requiring `A` and asserts a typed cycle error. The test `ambiguous_tag_is_a_defect` provides one tag twice and asserts `Die`.

**L6. The scope releases on every exit, in reverse order, exactly once.** Success, typed failure, and interruption all release. A finalizer failure is appended to the cause. It does not erase the original reason. The test `release_on_three_exits` covers all three and asserts the order `[second, first]`.

**L7. Finalizers and commits are uninterruptible.** Cancellation requested during a release or a marked commit region runs the region to completion, then takes effect. The test `cancel_during_release_finishes_release` asserts the release body ran and the interrupt is visible afterwards.

**L8. Tests substitute the world.** Clock, randomness, config, and services are inputs. The test executor is single-threaded and deterministic. `TestClock` does not advance by itself. The legal sequence is fork the sleeper, adjust the clock, join the fiber. The test `adjust_before_sleep_is_scheduled_does_not_complete_it` adjusts first, then forks a sleep, and asserts the fiber is still suspended. Randomness in tests comes from `Random::with_seed`. There is no `TestRandom` service, matching 4.0.0.

**L9. Shared state has a ladder.** Immutable data first. A pure atomic update may retry and must not log, emit, or perform I/O. An update that is itself an effect takes a permit and runs once (`SynchronizedRef`). Two facts that move together are one value. Two tasks communicate through a bounded queue. The test `atomic_update_retries_without_io` fails the test if the update closure calls a service.

**L10. Every wait is bounded.** A public combinator that waits takes a timeout or documents that it waits on `TestClock` and is therefore bounded by the test. Unbounded retry is not constructible through `policy::remote`. The test `remote_policy_is_capped` asserts a finite attempt count.

**L11. Parse once, at the boundary.** Untrusted bytes become a typed value or a typed schema error before domain code sees them. Secrets implement redaction in `Debug` and in span fields. The test `secret_debug_is_redacted` asserts the secret bytes are absent from the debug string.

**L12. A long-lived program is a fold.** `update` is pure. The same model and the same message produce the same next model and the same command list. Commands are data. The interpreter is the only place an effect starts. A subscriber sees snapshots and cannot write the model. The test `fold_is_deterministic` runs the same log twice. The test `subscriber_cannot_write` is a type test: the subscription handle has no `send` into the model.

**L13. A batch is a bounded map-reduce.** Fan-out takes an explicit concurrency bound. Reduce is pure and associative, with an initial value. Fail-fast, accumulate, and partition are three combinators. The test `fanout_peak_in_flight` uses a counting service and asserts the peak is at most the bound. Cancelling the batch releases every child.

**L14. Intent is recorded before the provider runs.** A retry reuses the key. `Unknown` is a stored state. A reconciliation pass is the only way out of it, and only toward `Settled`, `Failed`, `Unknown` again, or `Cancelled`. A terminal state never moves. Acceptance is not delivery: `Accepted` is its own state. A claim carries a generation and a lease. A late holder whose generation does not match cannot settle. The test `unknown_has_no_default_exit` tries to settle `Unknown` as `Settled` without a reconcile row and asserts refusal. The golden transition table is copied from `can_transition` as of 2026-10-02, and it is extended only by the park edges once their write path is identified (see Journal). The test `golden_transitions` accepts exactly the committed pairs.

**L15. Replay order is ordered.** A model that affects output uses `BTreeMap` and `BTreeSet`, or a `Vec` whose order is part of the state. `HashMap` iteration order is not part of a fold. The test `btreemap_replay_matches` is the positive case. The lint `fold_model_no_hashmap` flags `std::collections::HashMap` inside a type that implements `Program::Model`.

**L16. The message log is a durable stream.** The resume position is an opaque offset the log mints. It is lexicographically ordered. The sentinels are `-1` (start) and `now` (tail), and the log never mints those two strings as real offsets. The consumer commits `next_offset` in the same transaction as the state derived from the bytes it just applied. `UpToDate` means caught up, and more may arrive. `Closed` means EOF. An offset older than retention is `Gone`, and the consumer resets explicitly to `-1` or `now`. A soft-deleted stream is a different `Gone`. Producer identity is `(producer_id, epoch, seq)`. The same triple appends once. A lower epoch is fenced. A new epoch starts at sequence 0. Epoch state does not expire. A fork inherits bytes up to an offset and does not inherit producer epoch. A snapshot is the pair `{model, offset}` stored beside the stream. Tests: `same_epoch_same_seq_appends_once`, `stale_epoch_is_fenced`, `new_epoch_starts_at_zero`, `hand_built_offset_is_rejected`, `retention_gone_is_not_a_skip`, `soft_delete_gone_is_not_retention`, `up_to_date_is_not_closed`, `fork_drops_producer_epoch`, `snapshot_is_model_plus_offset`, `seq_gap_is_rejected`, `partial_identity_is_rejected`, `append_is_atomic`, `append_after_close_fails`, `closed_stream_stays_readable`.

### Carrier

```rust
pub struct Effect<A, E, R = ()> { /* private */ }
```

`A` and `E` are `Send + 'static`. `R` implements a sealed `Requirement` trait. `()` means no requirements. A tuple of service tags means all of them. A macro implements the trait for tuples up to 16. Beyond 16, `Cons<H, T>` is the escape. Providing a service is a type-level removal: `Effect<A, E, (Database, Clock)>` becomes `Effect<A, E, (Clock,)>` after `provide::<Database, _>`. Tags are zero-sized types with a string id (`"myapp/db/Database"`) and an associated service type.

This 16-wide tuple set is a deliberate ceiling. If `cargo check` of `crates/elm-effect/tests/sixteen_services.rs` exceeds 30 seconds on the operator's machine, stop and write a follow-up record before raising the cap or replacing the encoding. Do not replace it with a `TypeId` map to win that budget. A dynamic map may exist as an explicit `DynamicServices` service for plugin hosts (flywheel harnesses). It is opt-in, and it does not satisfy a static tag.

The value is not a `Future`. The interpreter turns it into polls at `Runtime::run`. Inside, the fiber steps a private state machine: success, fail with a cause, an async primitive (timer, I/O, queue, deferred), flat-map, provide, acquire-release, and an interruptibility marker. User code never matches that enum.

The ergonomic form of `Effect.gen` is a macro:

```rust
let program = effect! {
    let db = use_service::<Database>();
    let rows = db.query(sql).await?;
    succeed(rows)
};
```

The macro expands to `flat_map`. `?` propagates the effect's typed error only. A panic inside a fiber is caught at the fiber boundary and stored as `Die`. That `catch_unwind` is the one legal use in the workspace. It lives in the supervisor. It does not turn the panic into `E`. A sibling fiber keeps running. The process stays up. The test is `panic_in_child_is_die_and_sibling_lives`.

`Runtime::run` is the edge. `ManagedRuntime` is the bridge for a non-effect caller (a CLI handler, a test). Libraries do not call either.

### Cause

Match Effect 4.0.0's flat cause, as in `packages/effect/src/Cause.ts` at the tag:

```rust
pub struct Cause<E> {
    pub reasons: Vec<Reason<E>>,
}

pub enum Reason<E> {
    Fail(E),
    Die(Defect),
    Interrupt { fiber_id: Option<FiberId> },
}
```

`Defect` carries a message and a backtrace. It is not an `E`. Parallel failure and a finalizer failure append reasons. They do not build a tree. The v3 variants `Empty`, `Sequential`, and `Parallel` are not ported. `Cause::empty` is an empty `reasons` vector. There is no empty variant and no `is_empty` method that means something else. Equality of causes is structural. `Timeout` is a typed error (`TimeoutError`), not a new reason kind, matching `Cause.TimeoutError` at 4.0.0.

No typed error is `E = Infallible`, which is Effect's `never`. It is not `()` and it is not a unit error.

`Exit<A, E>` is the finished form: success, or failure with a cause. It is ordinary data. It is not an effect. `Effect::exit` captures that full cause, including defects and interrupts, and the resulting effect has no typed error. `Effect::result` moves only the typed error into `Result<A, E>`. Defects and interrupts stay fiber failures. `catch` handles typed errors and leaves defects and interrupts alone. `catch_defect` handles defects and leaves typed failures and interrupts alone. `catch_cause` sees the whole cause and belongs to the supervisor. `or_die` turns every typed failure into a defect and sets `E` to `Infallible`. An exhausted retry does not call `or_die`.

### Fibers, interruption, and scope

A fiber has an id, an interruptibility depth, a scope stack, a context, and a parent. `fork_child` ties the child to the parent: the child is interrupted when the parent finishes or is interrupted, and the parent waits for the child's finalizers. `fork_scoped` ties the child to a caller-supplied `Scope`. The child may outlive the parent and ends when the scope closes. `fork_in` is the same bind, with the scope passed in. `fork_detach` attaches the child to the runtime's root scope. It survives the parent and ends when it completes or the runtime shuts down. It does not take a separate supervisor argument. `fork_all` and `fork_with_error_handler` are not ported.

The default start is deferred: the child begins when the current fiber yields or completes. `start_immediately: true` starts it now. `uninterruptible: true` starts the child uninterruptible. `uninterruptible: inherit` copies the parent. The default interruptibility is the runtime default, which is interruptible.

Interruption is delivered by the runtime at the next interruptible step. User code does not poll a flag, and the runtime does not preempt a native CPU section. Inside an uninterruptible region the flag stays set and fires when the depth returns to zero. `Fiber::interrupt` waits until the target's finalizers finish, then resumes. That wait is back-pressure. `Fiber::interrupt_unsafe` does not wait. The default is the waiting form.

`timeout` is a race against `Clock::sleep`. The loser is interrupted and released before the winner's value is returned. `race` returns the first success. A failure does not beat a later success. If every side fails, the causes are concatenated. `race_first` lets any completion, including a failure, decide. `all` with no concurrency argument runs one effect at a time, and the next starts only after the previous completes. A number caps how many run at once. `Concurrency::Unbounded` exists and starts every task. `policy::remote` never selects it. The test `all_without_concurrency_is_sequential` asserts the second effect has not started when the first is still suspended. The test `race_failure_does_not_beat_later_success` fails one side immediately, succeeds the other after a clock adjust, and asserts success.

`acquire_release` pushes a finalizer on the scope. The finalizer is an effect. The runtime runs it. `Drop` of a guard is not the release path for anything that needs `.await`. Synchronous resources (memory, `std` locks) may still use `Drop`. L6 says the scope runs finalizers on success, typed failure, and interruption, in reverse order of acquisition, uninterruptible, and a finalizer's own failure is appended. The v4 concurrency guides read in the cross-check do not spell that release rule. Phase 1 reads `acquireRelease` at the pin and the test follows that source. If the source disagrees with L6, the source wins and this record is amended in the same change. `Scope::provide` lends a scope and removes `Scope` from the effect's requirements. It does not close that scope when the effect completes.

The Tokio executor is an implementation of:

```rust
pub trait Executor {
    fn spawn_boxed(&self, fut: BoxFuture<'static, ()>);
    fn spawn_blocking(&self, f: Box<dyn FnOnce() + Send>);
}
```

`TokioExecutor` is behind feature `tokio`. Kernel tests use `DeterministicExecutor` in `elm-effect-test`, which runs one ready fiber at a time in registration order and completes timers only when `TestClock` says so. `tokio::time::pause` is not the spec. A blocking job spawned on `spawn_blocking` is not aborted by dropping the waiting fiber. The job checks a cancellation flag at its own safe points, or it runs to completion. The runtime does not pretend a blocked syscall was cancelled.

### Layers and context

A `Layer<ROut, E, RIn>` builds services. `Layer::provide` wires a dependency and hides it. `Layer::provide_merge` wires it and keeps it visible to the caller. `MemoMap`, in `Layer.ts` at the tag, shares one construction of the same layer instance when a build goes through that map. A documented v4 example of two sequential `Effect.provide` calls inside one generator constructs the layer twice. A stacked `provide` then `provide` of the same layer builds once. Phase 2 pins both shapes with tests copied from those examples, and does not invent a third sharing rule. `Context.Service`'s constructor is not what gets memoized. Sharing starts once the constructor is wrapped in a layer. Application code provides the graph once, at the edge. Tests that need a fresh instance call `Layer::fresh`.

A cycle is rejected by a build-time graph walk before any constructor runs. Constructors may fail. That failure fails the scope being built.

`Context::Reference` is an ambient value with a default (a log level, a locale). Overriding it for a sub-effect is `Effect::provide_reference`. Domain code does not read environment variables. `Config` is parsed at startup into a struct the layer closes over.

### Schedule and the remote stack

A `Schedule<In, Out>` is a value: it decides whether to continue and how long to wait. `Schedule::recurs(n)` allows `n` retries after the first attempt. `Schedule::max` continues while every member continues and waits the longer delay. `Schedule::min` continues while any member continues and waits the shorter delay. Exponential backoff composed with a spaced ceiling, then jitter, then a predicate on `error.retryable`, is the production retry. Defects and interrupts are never retried. `retry` runs again after typed failure. `repeat` runs again after success. They are different combinators.

`policy::remote(call)` applies L4's order with these defaults, all overridable by arguments, none of them infinite:

| Knob | Default |
| --- | --- |
| Retry attempts after the first | 3 |
| Backoff start | 250 ms |
| Delay ceiling | 10 s |
| Jitter | full, from the `Random` service |
| Attempt timeout | 30 s |
| Budget timeout | 2 min |
| Bulkhead in flight | 8 |
| Bulkhead queue | 32, queue wait 5 s |
| Rate limit | none until the caller passes a limiter |
| Breaker | closed until the caller passes one. When passed: open after 5 consecutive typed failures, half-open after 30 s, one probe |

A caller who needs a different budget passes arguments. A caller who writes a `for` loop with `sleep` inside a service has violated L4. The breaker, when open, fails with a typed `BreakerOpen` and does not count as a retry attempt.

Hedge is not in the default stack. `policy::hedge` exists for idempotent reads: start the call, start one duplicate after a caller-supplied delay if the first is still running, take the first success, cancel the loser. The test `hedge_does_not_send_the_duplicate_when_fast` asserts the duplicate service saw zero calls when the first returns before the delay.

### Streams, schema, STM, and the standard library

Streams are pull-based effects. `Stream::run_fold` consumes. `map_effect` takes a concurrency bound (L13). An unbounded input is a stream of bounded chunks, not a `Vec` built in memory. Channels and sinks sit with streams in the same crate.

Schema is a crate behind feature `schema`, re-exported from the facade so callers write `elm_effect::Schema`. Decode of `unknown` bytes yields a typed value or a typed `SchemaError`. Encode is the inverse. `TaggedError` is a schema-bearing error with a `_tag` discriminant, matched by `catch_tag`. The implementation may use `serde` for the boring cases. The spec is Effect Schema: transformations, brands, and accumulated parse errors are part of parity, and a serde derive that cannot express them does not count as `matched`. The schema test corpus under `packages/effect/test/schema/` is the gate for those rows.

Software transactional memory (`TxRef`, `TxQueue`, and the rest of the `Tx*` modules) is feature `stm`, default on. It is how two facts move together when a lock is the wrong shape. It is not an excuse to ignore L9.

The standard library of the carrier (ref, synchronized ref, subscription ref, deferred, queue, pubsub, semaphore, pool, cache, latch, request resolver, rc-map) is in the facade. Cache single-flights identical keys. A request resolver batches distinct keys. Failures are not cached unless the caller opts in. Pool and rc-map release when idle, and the idle policy is explicit.

`Equal` for domain values treats `NaN` as equal to `NaN`, matching Effect 4.0.0's structural equality. `f64`'s `PartialEq` stays IEEE. The two traits are different, on purpose. The test `equal_nan_is_nan` uses `Equal`, not `==`.

### Elm loop and the Redux complement

Crate `elm-effect-elm`. This is the complement. Effect 4.0.0 does not ship it. We do, because the siblings are long-lived folds.

```rust
pub trait Program {
    type Model: Clone + PartialEq;
    type Msg: Clone + PartialEq;
    type Cmd: Clone + PartialEq;

    fn init(boot: &Boot) -> (Self::Model, Vec<Self::Cmd>);
    fn update(model: &Self::Model, msg: &Self::Msg) -> Update<Self::Model, Self::Cmd>;
}

pub struct Update<M, C> {
    pub model: M,
    pub cmds: Vec<C>,
}
```

`Boot` is parsed configuration. It contains no live socket, no clock, and no store. Time enters as a message. The host samples the clock and sends `Msg`. `update` that calls `SystemTime::now` is a broken fold even if it compiles. The lint `fold_no_ambient_world` flags `SystemTime`, `std::env`, and `rand::thread_rng` inside an `update` function.

Commands are data and implement `serde::Serialize` and `Deserialize`. The interpreter is a separate trait:

```rust
pub trait Interpret<P: Program> {
    type R: Requirement;
    type E;
    fn interpret(cmd: &P::Cmd) -> Effect<P::Msg, Self::E, Self::R>;
}
```

The runtime loop is: fold, record durable intents, interpret each command into an effect, run it, feed the outcome back as a message. One entrance. One exit.

A command declares durability:

```rust
pub enum Durability {
    Ephemeral,
    Durable { key: String },
}
```

Ephemeral commands (an in-memory read) are not journaled. Durable commands are journaled before `interpret` runs the provider. Redelivery uses the same key.

Redux rules, encoded as types:

* One model value owns the facts the fold decides. A projection (a status string, a TUI snapshot) is derived. It is not writable.
* A middleware trait, if written, is `fn(&Msg) -> Msg`, applied before `update`. It has no access to `Cmd` or `Effect`. Sagas, thunks, and listeners that start work are not representable on this trait.
* Subscriptions are a stream of model snapshots (`SubscriptionRef` underneath). The handle cannot send into the model.
* Time travel replays the message log from the latest snapshot. The snapshot stores the model and the log offset it includes. Default snapshot interval is every 100 messages, configurable per program. Replay is a pure fold plus a comparison of the command lists. It does not re-execute providers. Provider results are already in the log as messages.

A one-shot tool (a CLI that runs one effect and exits) does not implement `Program`. L12 applies to programs that live across messages. Forcing every function through the fold is how the loop becomes a second effect system.

### Journal

Crate `elm-effect-journal`. It defines the trait and the pure decisions. It does not open toron's database. Toron, later, implements the trait for `WorkflowEffectStore`.

Persisted effect states, matching `toron_core::index::workflow_effects::EffectState`:

| State | Meaning | Terminal? |
| --- | --- | --- |
| `Prepared` | Intent recorded, provider not invoked | no |
| `InFlight` | Claimed by a generation, provider running | no |
| `Accepted` | Provider accepted the work. Delivery is not proven | no |
| `Settled` | Delivery proven, success | yes |
| `Failed` | Typed failure, proven | yes |
| `Unknown` | The outcome was not observed. No default applies | no |
| `ReconcilePending` | A reconciliation pass owns the row | no |
| `Cancelled` | Interrupted or cancelled. Not a typed failure | yes |
| `Parked` | Durable suspension until a wake time. The step has not run | no |

`Unknown` may be entered from `InFlight` or `Accepted`. It may leave only through `ReconcilePending` or `Cancelled`. `ReconcilePending` may leave toward `Settled`, `Failed`, `Unknown`, or `Cancelled`. `Settled`, `Failed`, and `Cancelled` have no outbound edge. A timeout does not delete a row.

`can_transition` in toron-core, read on 2026-10-02, encodes exactly these pairs and no others:

* `Prepared` to `InFlight`, `Failed`, `Cancelled`
* `InFlight` to `Accepted`, `Settled`, `Failed`, `Unknown`, `Cancelled`
* `Accepted` to `Settled`, `Failed`, `Unknown`, `Cancelled`
* `Unknown` to `ReconcilePending`, `Cancelled`
* `ReconcilePending` to `Settled`, `Failed`, `Unknown`, `Cancelled`

`Parked` appears in none of them. `transition_effect` returns `false` and writes nothing when `can_transition` is false. Meanwhile `plan_attempt` returns `ResumeParked` for a parked view, and `plan_park` journals an intent before the wait. The doc comment on `park.rs` points at `boundary::EffectState::Parked`, and that variant is not on the boundary enum. The boundary enum has `None` where the core uses "no row" (`plan_attempt` takes `Option<&EffectView>`).

The port's rule:

* The golden table starts as the pairs `can_transition` actually contains. The test vectors are those pairs, plus the negative cases already in toron-core's tests (`Settled` to `Unknown` refused, `Failed` to `Settled` refused, `Prepared` to `Settled` refused).
* As read in the toron checkout: nothing writes the effect row's state column to `parked`. `can_transition` has no `Parked` edge because that write does not exist. Do not add one. The park that does persist is `WorkflowEffectStore::upsert_pending` into `workflow_pending`, idempotent on `(run_id, promise_id, park_generation)`. `park_intent` builds the `ParkIntent` value before the wait. `ResumeNow` builds none. `EffectState::Parked` stays parseable: a row that arrives already `parked` is `ResumeParked`, resumable, not terminal, and not `Failed`. The framework stores a park as a pending row with that key, and it does not invent an effect-row transition to get there.
* One enum in the framework. `Absent` is "no row", and it is not a variant of the persisted state. The boundary mirror in toron is a drift to be retired by toron's own adoption record, not something the framework copies.

`plan_attempt` is pure and total. The framework ships the same function over `Option<EffectView>` where `EffectView` is `{ state, attempt, lease_live }`, plus `ProviderCapability { supports_idempotency_key, supports_reconcile_query }`. The match below is the function as read in `toron/crates/toron-workflow/src/engine/reconcile.rs` on 2026-10-02. Phase 9 encodes it as table tests. If the function has moved, the tests follow the file and this table is updated in the same change.

| View | Plan |
| --- | --- |
| No row | `DispatchNew` |
| `Parked` | `ResumeParked { attempt: attempt.max(1) }` |
| `Settled`, `Failed`, or `Cancelled` | `DuplicateTerminal` |
| `Prepared` | `DispatchNew` |
| `ReconcilePending` | `RefuseBlindRetry` (the reconciler owns the row) |
| `InFlight`, `Accepted`, or `Unknown`, and `lease_live` | `RefuseBlindRetry` |
| Those three, lease expired, provider supports a query | `ReconcileFirst` |
| Those three, lease expired, no query, provider supports an idempotency key | `RetrySameKey { attempt: attempt + 1 }` |
| Those three, lease expired, neither query nor idempotency key | `RefuseBlindRetry` |

Query beats idempotency. A live lease beats both. `ReconcilePending` never falls through into a retry. `Prepared` dispatches. It does not pretend the provider has been invoked.

Two more pure functions from the same file are part of the journal crate:

* `may_settle(current_token, presented)` is true only when both sides are non-empty and equal. An empty presented token settles nothing. An unclaimed row settles for nobody.
* `check_replay(prepared_digest, redigest)` returns a mismatch when a replay would render a different action under the same run and node. The mismatch is a typed error. It does not mint a second effect key.

Cleanup kinds (`ReleaseReservations`, `CaptainEvidence`, `BeadComment`) stay in toron. Captain evidence and bead comments are not idempotent there, and a forced retry is an explicit operator capability, not an inference. The framework does not grow those variants. A sibling adoption passes them in as its own command enum.

`plan_park(until_ms, max_wait_secs, now_ms, ceiling)` is pure. `ceiling` defaults to 86400 seconds. A requested wait above the ceiling is a typed refusal, not a shorter sleep. A `until_ms` already past `now_ms` is `ResumeNow` and reports the overdue gap without wrapping. The host supplies `now_ms`. Replay may see a later `now_ms`, and the decision then moves from park to resume. That movement is the point of a wake time. The journal key is a function of `(run_id, node, until_ms)`, so parking the same step to the same instant twice is one row.

Claim: one statement sets the owner, the lease deadline, and `generation = generation + 1`, and returns the row. Settle carries the generation. A mismatched generation settles nothing. Exhausted attempts become a dead-letter row. The dead-letter state is `Failed` plus a `disposition` of dead-letter, not a tenth lifecycle state, unless phase 9 finds toron already storing it as its own column. Match toron's column. Do not invent a parallel one.

Offsets: the next offset and the state derived from it commit together. The offset moves only forward. A position older than retention returns `Gone` with `FirstReplayable`, and the caller chooses `-1` or `now`. The log does not skip silently.

### Durable log

The native log speaks the semantics of the Durable Streams protocol, draft 1.0, at commit `461b40267aabd644558f9b19dbb9507dd5f691cf` (`PROTOCOL.md`, header date `2025-01-XX`). The document is still a draft. The laws in L16 are what this framework implements. A later draft that changes a law is a new record, the same way an Effect tag bump is.

The log is in-process. It is the stream. It is not a client of the Node reference server, the Caddy plugin, or `packages/client-rust`. Those are HTTP peers. An adapter in `elm-effect-unstable`, feature `durable-streams`, may speak the HTTP wire so a browser or another process can tail the same bytes. The kernel runs with the feature off.

Vocabulary, locked. An agent uses the left-hand word in this repository.

| Word here | Protocol name | What it is |
| --- | --- | --- |
| `offset` | `Stream-Next-Offset` | Resume position. Opaque. The only token a fold stores |
| `epoch` | `Producer-Epoch` | Writer generation. Bumps when a new owner claims the effect. A lease renewal that does not change owner does not bump it |
| `seq` | `Producer-Seq` | Append index within one epoch, starting at 0. A retry of the same append resends this seq. `plan_attempt`'s `attempt` is a different column |
| `producer_id` | `Producer-Id` | Stable writer id. For a journaled command, the effect key |
| `up_to_date` | `Stream-Up-To-Date` | Caught up. More appends may arrive |
| `closed` | `Stream-Closed` | EOF. Closure is monotonic |
| `gone` | `410 Gone` | Retention miss, or a soft-deleted stream. Callers match the reason |
| `fork` | `Stream-Fork-Offset` | A branch. Same offset space. Producer epoch starts over |
| `collapsing_cursor` | `Stream-Cursor` | CDN fan-in token on the HTTP adapter only |

`collapsing_cursor` is not a resume position. Storing it as the fold's offset loses bytes.

Append rules, which L16's tests also pin:

* One append is atomic. A crash while it is being written leaves the previous offset. A read never returns a partial message. JSON mode keeps message boundaries.
* Producer identity is all three fields or none. A seq that is not `last + 1`, and is not an already accepted seq, is rejected (`seq_gap_is_rejected`, `partial_identity_is_rejected`).
* `Closed` rejects later appends. Bytes written before the close stay readable (`append_after_close_fails`, `closed_stream_stays_readable`).
* The journal row and the stream append that records that step commit in one transaction. One without the other is not a successful step.
* Each JSON message carries the schema version from the stability section. An unknown version is a typed error.
* The log is a trait, `DurableLog`, in `elm-effect-journal`. The first implementation is in-process. Toron's store and the HTTP adapter implement the same trait. Callers depend on the trait.

One Elm program run is one JSON stream (`application/json`). Appends are messages. The fold does not read the stream. The interpreter appends, then feeds the outcome back as a message. Replay reads from the snapshot's offset, or from `-1` when there is no snapshot. A time-travel branch is a fork at that offset. The fork's first producer epoch is 0, because the protocol says a fork does not inherit producer state.

The state protocol (`packages/state/STATE-PROTOCOL.md`) is a different document. It MAY wrap a run of change messages in `snapshot-start` / `snapshot-end`, and it has insert, update, and delete operations. This framework does not implement that protocol in the kernel. The Elm snapshot is `{model, offset}` beside the stream. Adopting the state protocol is a later record.

Our log is stricter than the draft in two places. The draft says a server is not required to reject a client-built offset, and that a producer whose state TTL expired is treated as new. We reject a hand-built offset, and we never expire epoch state. A zombie after a TTL is a double write on mail and on gate rows.

Pull-wake claims, webhook delivery, and CDN collapsing stay on the HTTP adapter. Phase 9b does not build them. The in-process claim is still toron's generation and lease.

### Observability

`Logger`, `Metric`, and `Tracer` are services on the stable surface. Spans wrap an effect. Logging is an effect. `tap` observes and does not change the result or the error. Secrets stay redacted. The exporter is OpenTelemetry, crate `opentelemetry_sdk` 0.28 to match chiebukuro's existing dependency, behind feature `otel` on `elm-effect-unstable` until the stable tracer trait has a matched test that does not need a collector. A test asserts the span name and the attributes. It does not open a socket.

### Platform substitutes

| Effect capability | Rust substitute | Crate / feature |
| --- | --- | --- |
| `TestClock`, `TestConsole` | `elm-effect-test`, behavior pinned to the 4.0.0 `TestClock` docs (fork, then adjust, then join). `TestClock` is untagged strict semver. `TestSchema` is unstable and stays in the schema test helpers | `elm-effect-test` |
| `@effect/vitest` (`it.effect`, `it.layer`) | `effect_test!` macro and a layer fixture | `elm-effect-test` |
| Node and Bun runtimes | `Runtime::run_main` on `TokioExecutor` | facade feature `tokio` |
| `FileSystem`, `Path`, process | `std::fs` and `std::process` behind services, so tests pass fakes | facade |
| HTTP client | a `HttpClient` service. First implementation: `ureq` 2, matching flywheel, synchronous on `spawn_blocking`. A second implementation may be `hyper` later. The service is the spec | `elm-effect-unstable` feature `http` |
| SQL | a `Sql` service. First implementation: the driver the sibling already uses (`sqlx` in toron). The framework crate does not depend on a database in its default features | `elm-effect-unstable` feature `sql` |
| CLI | clap 4, matching flywheel, behind a service so argument parsing is testable without a process | `elm-effect-unstable` feature `cli` |
| AI `LanguageModel` | a trait with generate-text, generate-object (schema), and stream. No provider in the default build | `elm-effect-unstable` feature `ai` |
| Workflow, cluster, event log, persistence, rpc | unstable crate, semantics from the 4.0.0 modules (`Activity` results stored and replayed, `DurableClock`, `DurableDeferred`, `DurableQueue`, compensation on failure and cancel only) | `elm-effect-unstable` |
| Resonate | an implementation of `WorkflowEngine` provided by the sibling that already depends on `resonate-sdk`. This repository does not take a git dependency on Resonate | sibling adoption |

### Product shape

An application or a CLI is its own crate. It depends on `elm-effect`. `elm-effect` does not depend on it.

A CLI parses argv to a typed request before any effect runs, runs one effect, and exits. A process that lives across messages is an application and implements `Program`. Exit codes are success `0`, typed failure `1`, defect `2`, interrupt `130`. Stdout is the success value. Logs go to stderr through the `Logger` service. A parse error is a typed failure. The steps for flags, help, and files are `skills/elm-effect/references/build-cli.md` and `build-app.md`. Those files do not get to pick different exit codes.

A named sibling migration whose phase gate is red implements the missing phases in this repository first, then edits that sibling, in the same task. The utterance has to name the sibling. No name means stop and list flywheel, toron, beads, and chiebukuro.

### Sibling ports

These are the seams a later adoption record extends. This record does not edit those repositories.

**Flywheel.** `decide` stays pure and becomes `Program::update` for the briefing snapshot, or stays a pure function that `update` calls. Its output becomes `Vec<Cmd>`. `effect_journal::effect_runner` becomes `Interpret`. The 4-worker pool and the 4096 queue bound become configuration of `TokioExecutor` (`worker_threads: 4`, `queue_bound: 4096`) so the first adoption does not change capacity. When the last leaf has moved, the hand-built pool and its nested-runtime comment are deleted. That deletion is the adoption record's verification, not this one's. `breaker.rs` becomes the breaker in `policy::remote`. `durable.rs` keeps Resonate until a `WorkflowEngine` adapter runs the same loop. `tick` is the fold. `HashMap` in `durable.rs` is an adoption hazard under L15: if iteration order changes the command list, the model switches to `BTreeMap` in that adoption. Doorbell ticks, herdr socket events, and mail ingress enter as messages. They do not call the model.

**Toron.** The store stays. The trait implementation is new code in toron. Mail send is a durable command whose key is the event id. Reservation acquire is a claim with generation and lease. NIP-44 encryption stays a service at the edge. The two `EffectState` enums are a known drift. The adoption record retires the boundary mirror in favor of the core enum plus `Option` for absence. This framework does not grow a third spelling.

**Beads.** Issue status is a fold over events. Gate verdicts are a second stream. A projection may join them for display. No helper in this framework writes a gate row from an issue event, and no helper rebuilds verdicts from the issues JSONL. `br close` is a command. The interpreter checks for a passing gate row bound to a commit whose message cites the bead, and returns a message. `update` performs the status transition only on that message. Close policy loaded from `.beads/policy.yaml` stays beads' configuration. It is `Boot` data.

**Chiebukuro.** ADR-0011 stays in force until a chiebukuro record supersedes it. Unmigrated `#[resonate::function]` pipelines keep using `resonate.db`. A migrated pipeline is a `Program` whose durable commands go through the journal, with Resonate as one `WorkflowEngine` backend during the move. Search and ingest are effects. A ranking decision that must replay is a fold over a message that already contains the retrieved set. Memory record is a durable command. Memory recall is an effect whose result re-enters as a message. The fold does not call the store.

Dependency arrow, which adoption records are not free to reverse:

```text
flywheel ──► elm-effect
toron    ──► elm-effect
beads    ──► elm-effect
chiebukuro ► elm-effect
elm-effect ─► (none of the four)
```

Flywheel already depends on toron. That edge stays. The framework does not become a third party to it.

### Stability and evolution

Workspace version is one number. Crate `elm-effect` (the facade) follows semantic versioning for untagged modules. A minor release does not break a `port` item. Crate `elm-effect-unstable` may break an `unstable` item in a minor release, and an `experimental` item in a patch. Its crate docs say so in the first paragraph. Callers who need a stable lock depend on the facade with default features and do not enable `unstable`. Forgotten `fork_detach` children keep the process alive until the runtime shuts down, the same way a suspended fiber keeps a v4 process alive without `runMain`. `run_main` is still the edge, because it owns signals and the exit code.

Durable payloads carry a schema version. An unknown version is a typed error. It is not decoded with a best guess.

Effect 4.1 or 5.0 does not flow in by renaming. A new record names the tag, the laws that moved, and the ledger rows that change class. The pin in `conformance/PIN` is `effect@4.0.0` until that record lands.

Named divergences from the TypeScript API, each with a reason:

| Divergence | Reason |
| --- | --- |
| No `HKT`, `Pipeable`, `Unify`, `Types`, `Newtype`, `Inspectable`, `Symbol`, `Effectable` modules | They exist to patch TypeScript. `Types` and `Newtype` are compile-time-only there. Rust traits, newtype structs, `?`, and `Debug` cover them. Class `native` |
| Interrupts land at effect steps, not inside native CPU code | Effect aborts between its own steps. Rust cannot preempt a tight loop without a yield. The observable rule is the same: uninterruptible regions finish |
| `Effect` is not a `Future` | Async `Drop` cannot run finalizers (L6, L7) |
| Requirement tuples capped at 16, then `Cons` | Compile-time ceiling, measured |
| `Equal` is a trait, `PartialEq` for `f64` stays IEEE | Language contract versus Effect's `NaN` rule |
| `catch_unwind` only in the fiber supervisor | A fiber defect must not abort the process, and must not become `E` |
| Test harness is `elm-effect-test`, not vitest | Cargo is the runner. Behavior of `TestClock` is still the spec |
| Park is a first-class outcome | The stack already persists it. Effect's in-process `Interrupt` does not survive a restart |
| Resonate is an adapter | It is a checkpoint engine. It has no R channel, no cause, no layer graph |
| HTTP via `ureq` 2 first | Flywheel already depends on it. The service trait is the port, the client is an impl |

### Non-goals

* Editing flywheel, toron, beads, or chiebukuro in the work this record authorizes.
* Removing Resonate, SQLite, JSONL, or Nostr.
* Porting Node, Bun, or DOM.
* A zero-cost effect encoding. The carrier boxes continuations. Fusing `map` and sync `flat_map` is allowed after a measurement shows boxing above 20% of a sibling hot path, and not before.
* Tracking Effect `main`.
* A web framework, a UI toolkit, or a replacement for herdr.
* Human-approval UX. A parked row is the data. The host that drains it already exists in toron and flywheel.
* Declaring the port done at the end of the kernel phases.

### Crate map

Edition 2024. Toolchain nightly, floating, same comment the siblings carry: re-run clippy after a toolchain bump. License MIT.

| Path | Publishes | Depends on | Default features |
| --- | --- | --- | --- |
| `crates/elm-effect` | Facade. Re-exports kernel, schedule, streams, standard concurrency, logger, metric, tracer | `elm-effect-kernel` (path, may be the same crate if phase 1 keeps it monolithic; split only when check time of the facade exceeds 30 s) | `tokio`, `schema`, `stm` |
| `crates/elm-effect-schema` | Schema, tagged errors | `elm-effect` with `default-features = false` | |
| `crates/elm-effect-test` | Deterministic executor, `TestClock`, `TestConsole`, `effect_test!` | `elm-effect` | |
| `crates/elm-effect-elm` | `Program`, `Interpret`, replay | `elm-effect`, `serde` 1 | |
| `crates/elm-effect-journal` | States, `can_transition` golden, `plan_attempt`, `plan_park`, `JournalStore` | `elm-effect`, `elm-effect-elm` | |
| `crates/elm-effect-unstable` | http, sql, cli, ai, workflow, cluster, eventlog, persistence, rpc, otel | facade | none |
| `crates/elm-effect-conformance` | Ledger test | none (reads `conformance/`) | |
| `conformance/PIN` | The string `effect@4.0.0` | | |
| `conformance/inventory.sh` | Regenerates `modules.toml` and `behavior-tests.tsv` from a checkout of that tag | | |
| `conformance/modules.toml` | Module ledger. Test links use the path key in the ledger contract | | |
| `conformance/behavior-tests.tsv` | One row per behavior call. Columns `status`, `path`, `line`, `kind`, `title`, `rust` | | |
| `examples/readme_effect.rs` | The onboarding program: succeed a string, log it, run at the edge | facade | |

Phase 1 may implement the facade as one crate. The paths above are the module boundaries even before they are separate packages. A later split moves files. It does not rename public modules. Public paths are `elm_effect::Effect`, `elm_effect::Cause`, `elm_effect::Layer`, `elm_effect::Schedule`, `elm_effect::Stream`, `elm_effect::Schema`, `elm_effect::elm::Program`, `elm_effect::journal::EffectState`.

Dependencies, pinned to the siblings' majors:

* `tokio` 1, features `rt-multi-thread`, `macros`, `time`, `sync`, only behind feature `tokio`.
* `serde` 1 with `derive`, `serde_json` 1.
* `thiserror` 1, for errors at the crate edge (`SchemaError`, `Cycle`, `TimeoutError`). Internal causes are the `Cause` enum, not `thiserror`.
* `proptest` 1, dev-dependency, for `Arbitrary` parity.
* `ureq` 2, only on `elm-effect-unstable` feature `http`.
* `opentelemetry_sdk` 0.28, only on feature `otel`.
* `clap` 4, only on feature `cli`.

Forbidden in this workspace: `anyhow` in a public signature, `Box<dyn std::error::Error>` in a public signature, a dependency on any sibling, a git dependency on Resonate, `unwrap` outside tests, `unsafe` without a `SAFETY` comment. Phase 1 has zero `unsafe`.

### Consequences

* Good, because a sibling adoption can delete a bespoke retry, a nested runtime, or a hand-rolled park without inventing the semantics again.
* Good, because an agent can see failure, requirements, and cancellation in the type before running the program.
* Good, because the journal law the stack already enforces becomes a library test instead of a comment each leaf has to remember.
* Bad, because the port is large. Schema alone is a multi-phase crate. The siblings gain nothing until phase 3 is real and someone writes an adoption record.
* Bad, because the carrier allocates. Hot paths will be slower than a hand-written future until someone measures and fuses.
* Bad, because two workflow engines (native and Resonate) will coexist, and a pipeline can be migrated halfway.
* Neutral, because Effect remains the behavioral spec and is not a build dependency.
* Neutral, because nightly drift can break clippy, the same way it can in the siblings.

### Risks

| Risk | What we will do |
| --- | --- |
| The R channel makes `cargo check` unusable | 16-service fixture, 30 second ceiling, stop and write a record. Do not swap in a `TypeId` map |
| Finalizers are wrong and we ship the async-drop hole again | L6 and L7 are phase-1 gates. Later phases do not merge with those tests red |
| Parity theater: types without tests | Ledger status `matched` requires a named test. `stubbed` fails the done-check |
| Tracking Effect `main` and churning forever | `conformance/PIN` is the tag. Updates are records |
| A sibling imports the framework and grows a cycle | The dependency arrow is a lint in this repo (no path dependency on the siblings) and a review rule in the adoption records |
| Park edges invented to look tidy | No effect-row write to `parked` exists. The pending-table key is the persisted park. `can_transition` does not grow a `Parked` edge to match a comment |
| Beads verdicts rebuilt from issues | No such helper. L16 is per stream. Issues and gates are two streams |
| An agent asks which crate, whether to delete Resonate, or whether a cursor is an offset | Closed by the agent-authority section and by `skills/elm-effect`. The agent proceeds |
| `HashMap` in a fold silently changes commands | L15 lint |
| Resonate removed too early | Explicit non-goal. ADR-0011 stands |

Revisit this record when any of these is true: Effect publishes a release that changes `Cause`, `MemoMap`, or the meaning of `Schedule.recurs`; the 16-service check exceeds 30 seconds; a sibling adoption cannot express `plan_attempt` on `JournalStore`; a measured hot path spends more than 20% of samples inside the carrier's box; `can_transition` grows `Parked` edges (then the golden table's `intended` rows become `persisted`).

## Implementation Plan

Work is phased. A phase's tests land first, red, then the implementation, then the ledger rows that phase owns move to `matched`. Do not start phase N+1 with phase N's law tests red. The operator may run phases as separate reviews. An agent implements one phase per review.

* **Affected paths**: the crate map above for framework phases. A sibling path only when the utterance names that sibling and the phase gate in the agent-authority section is met. The missing phases are implemented here first, in that same task.
* **Dependencies**: the list under Crate map. No others without a follow-up record.
* **Patterns to follow**: laws L1 through L16. Effect 4.0.0 source at the pin, read before writing a combinator. Toron `can_transition`, `plan_attempt`, `plan_park`, and `upsert_pending` read before writing journal code. `thiserror` only at crate edges. `BTreeMap` in fold models.
* **Patterns to avoid**: implementing the carrier as a `Future`. `catch_unwind` outside the supervisor. `tokio::time::pause` as the test clock. A `TypeId` context as the R channel. `unwrap` on I/O. A saga trait. A helper that folds issues into gate rows. A git dependency. Editing a sibling the utterance did not name. Editing a named sibling while its phase gate is red.
* **Configuration**: feature flags `tokio`, `schema`, `stm` on the facade (default on). Unstable features off by default. Executor defaults `worker_threads = 4`, `queue_bound = 4096`, overridable, not constants inside combinators.
* **Migration steps**: there is no production caller. Migration of siblings is a later record per sibling, after phase 9 at the earliest for anything durable, and after phase 3 for a pure experiment that only needs retry, timeout, and scope.

### Phases

**Phase 0. Pin and ledger.** `conformance/PIN`, `conformance/inventory.sh`, `conformance/modules.toml`, `crates/elm-effect-conformance`. Classify every file from that file's own header, never from its directory. The script is the classifier: no `@stability` tag means `port`, `@stability unstable` means `unstable-port`, `@stability experimental` means `experimental-port`, the native list in this record means `native`, and a header that names Node, Bun, or a DOM event listener means `platform-foreign`. A human corrects a row by editing `conformance/overrides.toml`, with a reason, rather than by patching the script's output by hand. The first override is `Arbitrary.ts` to `unstable-port` if its header is untagged.

**Phase 1. Kernel.** `Cause`, `Exit`, `Effect` (succeed, fail, die, sync, flat-map, map), `Fiber` (`fork_child`, join, interrupt), `Scope` (`acquire_release`), `Runtime` on the deterministic executor, supervisor `catch_unwind`. Tests: L1, L2, L6, L7, `panic_in_child_is_die_and_sibling_lives`, `description_drop_runs_nothing`.

**Phase 2. Context.** `Requirement`, service tags, `Layer`, memoization, cycle detection, `provide`, `Layer::fresh`. Tests: L5, the 16-service check timed and printed, `ambiguous_tag_is_a_defect`.

**Phase 3. Time, schedule, refs, policy.** `Clock`, `Duration`, `TestClock`, `Random::with_seed`, `Schedule`, `retry`, `repeat`, `timeout`, `Ref`, `SynchronizedRef`, `Deferred`, `Queue`, `Semaphore`, `policy::remote`. Tests: L4, L8, L9, L10, `adjust_before_sleep_is_scheduled_does_not_complete_it`. This is the first phase a sibling could experimentally import. It is not the port being done.

**Phase 4. Concurrency combinators.** `all` (sequential unless a bound is passed), `for_each`, `race`, `race_first`, fiber sets, `ExecutionPlan`. `partition` and `validate` are read from the pin before they are encoded. The v4 concurrency guides do not spell their collect-versus-fail-fast behavior, so the source file is the spec for those two. Tests: L13, `all_without_concurrency_is_sequential`, `race_failure_does_not_beat_later_success`, cause concatenation when every side fails, loser of a race is interrupted and released.

**Phase 5. Streams.** `Stream`, `Channel`, `Sink`, bounded `map_effect`, `run_fold`. Tests from the stream rows of the ledger.

**Phase 6. Schema.** `elm-effect-schema`, tagged errors, `catch_tag`. Gate: the schema test corpus rows move to `matched`.

**Phase 7. STM and the rest of the standard library.** `Tx*`, cache, pool, pubsub, request resolver, rc-map, layer map. Single-flight and batching tests from the resilience rules: N identical concurrent misses run the lookup once. M distinct keys in one window produce one downstream call.

**Phase 8. Elm.** `Program`, `Interpret`, snapshot replay, middleware type, subscription handle, lints L12 and L15.

**Phase 9. Journal.** Golden `can_transition` with no invented `Parked` edge. Park persists through `upsert_pending`'s key `(run_id, promise_id, park_generation)`. `plan_attempt` table tests cover every arm in the Journal table (including `Prepared` dispatching, `ReconcilePending` refusing, query beating idempotency, `ResumeParked` using `attempt.max(1)`, and `RetrySameKey` using `attempt + 1`), `may_settle`, `check_replay`, `plan_park`, `JournalStore`, generation fence, offset committed with derived state. Tests: L14, `golden_transitions`, `unknown_has_no_default_exit`, `empty_token_never_settles`, `replay_digest_mismatch_mints_nothing`, park refusal above 86400 seconds, past `until_ms` does not wrap, a second pending upsert with the same key writes nothing.

**Phase 9b. Durable log.** In-process log in `elm-effect-journal` (`offset`, `epoch`, `seq`, `producer_id`, fork, snapshot record). Tests named on L16. HTTP wire, pull-wake, and `collapsing_cursor` wait for feature `durable-streams` and are not part of this phase. A sibling migration that persists a message log does not start before these tests pass.

**Phase 10. Observability.** Logger, metric, tracer services. Redaction test L11. Otel exporter behind `otel` as `unstable-port`.

**Phase 11. Platform.** Filesystem, process, stdio, terminal, config, as services with fakes.

**Phase 12. Unstable modules.** Workflow (activity replay, durable clock, durable deferred, durable queue, compensation on failure and cancel only), event log, persistence, rpc, http, sql, cli, ai, cluster. Each feature flag maps to ledger rows. Cluster is last.

**Phase 13. Ledger index.** `cargo test -p elm-effect-conformance -- --ignored` reports rows that are still `unported`. That report is the index. It is not the product finish line. Mark a row `not-applicable` with a reason when we will not carry that Effect internal. The default `cargo test -p elm-effect-conformance` stays green once both ledgers are well-formed.

### Verification

- [ ] `conformance/PIN` contains exactly `effect@4.0.0`.
- [ ] `conformance/inventory.sh` regenerates `modules.toml` byte-for-byte when pointed at that tag.
- [ ] `cargo test -p elm-effect-conformance` passes the well-formed ledger test.
- [ ] `sh conformance/witness.sh` exits 0. That script is the product finish line.
- [ ] `cargo test -p elm-effect-conformance -- --ignored` is an index. A row we will not carry is `not-applicable` with a reason. It does not have to be `ported`.
- [ ] Tests named in L1 through L15 exist and pass.
- [ ] `cargo test` with `--no-default-features` of `elm-effect` passes without a Tokio runtime and without a network.
- [ ] `crates/elm-effect/tests/sixteen_services.rs` compiles. Its `cargo check` time is recorded in the phase-2 review. A time over 30 seconds stops the phase.
- [ ] `rg unwrap crates --glob '!**/tests/**' --glob '!**/*test*.rs'` finds no `unwrap` outside tests. `expect` is allowed only with a reason that names an invariant.
- [ ] `rg 'catch_unwind' crates` finds matches only under the fiber supervisor module.
- [ ] `rg 'unsafe ' crates` finds no matches in phase 1 through phase 4. Later matches have a `SAFETY` comment.
- [ ] The workspace `Cargo.toml` has no path or git dependency whose target is flywheel, toron, beads, chiebukuro, or resonate.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes on the floating nightly the siblings use, and `cargo fmt --check` passes.
- [ ] `examples/readme_effect.rs` builds a description, drops a second copy without logging, runs one copy, and exits 0.
- [ ] The onboarding scenario from `packages/effect/src/testing/TestClock.ts` (sleep 5 minutes, timeout 1 minute, fork, adjust 1 minute, exit is failure) passes under `elm-effect-test` without a real sleep.
- [ ] `all` with no concurrency argument does not start the second effect while the first is suspended.
- [ ] `race` returns a later success when the other side has already failed.
- [ ] `fork_detach` has no supervisor parameter, and the child ends when the runtime shuts down.
- [ ] Journal golden test accepts every pair listed under Journal and rejects `Settled` to `Unknown`, `Failed` to `Settled`, and `Prepared` to `Settled`.
- [ ] `plan_attempt` table tests cover every arm in the Journal section, including `Prepared` to `DispatchNew` and `ReconcilePending` to `RefuseBlindRetry`.
- [ ] `may_settle` rejects an empty presented token, and `check_replay` rejects a digest mismatch without allocating a new effect key.
- [ ] `Parked` is not classified as terminal, is not equal to `Failed`, and is not equal to `Settled`.
- [ ] No public function in `elm-effect-journal` takes issue events and returns gate rows.
- [ ] L16 tests pass, including `hand_built_offset_is_rejected` and `stale_epoch_is_fenced`.
- [ ] A grep of `crates/` for `collapsing_cursor` used as a fold position finds none. The resume field is `offset`.

## Pros and Cons of the Options

### Semantic port

An owned fiber runtime, a typed carrier, an Elm crate, a journal crate, and a ledger against the Effect tag.

* Good, because finalizers run on cancellation, which bare futures do not do.
* Good, because the R channel makes a missing service a compile error.
* Good, because the Elm fold is the shape flywheel's `decide` and `tick` already have, so adoption is a move, not a rewrite of the decisions.
* Good, because the journal vocabulary is the one toron persists, so adoption does not translate states.
* Bad, because we take on a runtime, a schema crate, and a long ledger.
* Bad, because allocations are structural until measured fusion.

### Conventions only

Document the discipline. Keep `Result`, Tokio, and traits. Do not ship a carrier.

* Good, because there is nothing to build and the siblings already compile.
* Good, because it matches the default Rust advice for application code that only needs one request and one response.
* Bad, because the siblings have already outgrown it. The journal, the park, the nested 4-thread executor, and the Resonate checkpoints are bespoke carriers that do not compose.
* Bad, because a convention cannot make a missing service fail at compile time, and it cannot run an async finalizer when a future is dropped.

### Futures as the carrier

`impl Future<Output = Result<A, E>>` with services passed as generics.

* Good, because it looks like ordinary async Rust and has no interpreter.
* Bad, because dropping a future does not run async release, so timeout and scope are wrong exactly where the stack is most careful.
* Bad, because providing a service does not change a type unless every function is generic in a way that does not compose across a fiber spawn boundary.
* Bad, because `TestClock` cannot see `tokio::time::sleep` unless every wait already goes through a clock service, which is the carrier again.

### TypeScript sidecar or WASM

Run the real Effect runtime and call it from Rust.

* Good, because parity would be the upstream test suite, literally.
* Bad, because cancellation, crashes, and the journal would cross a process or a WASM boundary on the path flywheel already simplified to avoid nested runtimes.
* Bad, because a sibling could not implement `Program` as ordinary Rust data.

### Resonate as the carrier

Treat `#[resonate::function]` as the framework.

* Good, because chiebukuro and flywheel's durable loop already use it, and checkpoint-on-replay is real.
* Bad, because Resonate does not provide a typed R channel, a flat cause, a layer graph, streams, or schema.
* Bad, because the SDK dependency is a floating git branch. Putting that inside the framework would make every sibling float with it.
* Neutral, because it remains the right engine adapter for pipelines that are already checkpointed.

### Line-by-line transpilation

Generate Rust from the TypeScript sources.

* Good, because the mapping looks mechanical and the file tree matches.
* Bad, because the TypeScript encodes HKT, structural types, and yield-star generators. The output fights the borrow checker and still needs the semantic design this record writes down.
* Bad, because parity would be textual. A behavior change in a `.ts` file would churn generated Rust without a law to judge it.

### ZIO or cats-effect over FFI

Import a JVM effect system.

* Good, because those systems are the prior art Effect itself comes from, and they are older than 4.0.0.
* Bad, because none of the siblings run on a JVM, and an FFI boundary is a second runtime on the journal path.
* Bad, because the Elm loop and the toron journal would still have to be written here.

## More Information

### Sources read for this record

* Effect release `effect@4.0.0`, published 2026-10-01T01:47:13Z. https://github.com/Effect-TS/effect/releases/tag/effect@4.0.0
* Effect onboarding. https://effect.website/docs/v4/onboarding
* `packages/effect/src/Cause.ts` at the tag. `Cause` is `{ reasons }`. `Fail` holds `error`. `Interrupt` holds `fiberId`. `Die` is a separate interface in the same file.
* `packages/effect/src/testing/TestClock.ts` at the tag. Sleep suspends until the clock is adjusted. The documented order is fork, then adjust, then join.
* `packages/effect/src/Layer.ts` at the tag. `MemoMap` memoizes layer construction so one layer instance is built once.
* Subpath `index.ts` files for ai, cli, cluster, devtools, encoding, eventlog, http, http-api, net, observability, persistence, process, reactivity, rpc, schema, socket, sql, workflow, workers, and testing, at the tag. Each carried `@stability unstable`.
* Git tree of the tag. 507 paths matching `*.test.ts`. Top-level modules enumerated from `packages/effect/src/*.ts`.
* Flywheel `src/effect_journal.rs` (4 workers, queue bound 4096, intent then claim then invoke once then settle or unknown). `src/durable.rs` (Resonate loop, pure `tick`). `src/decide.rs` (`decide` is pure). `Cargo.toml` edition 2024, `tokio` 1, `serde` 1, `ureq` 2, `clap` 4.
* Toron `crates/toron-core/src/index/workflow_effects.rs` (`EffectState`, `can_transition`, `transition_effect`). `crates/toron-workflow/src/engine/reconcile.rs` (`plan_attempt`, `ResumeParked`). `crates/toron-workflow/src/engine/park.rs` (`plan_park`, `PARK_MAX_WAIT_SECS` of 86400, refuse rather than truncate, key from run, node, and wake time). `crates/toron-workflow/src/engine/boundary.rs` (second `EffectState`, no `Parked`, has `None`).
* Chiebukuro ADR-0011 and `Cargo.toml` (`resonate-sdk` git branch `main`, `opentelemetry_sdk` 0.28).
* Beads `src/model/` (issues and events as data), `src/close_policy.rs` (close is a gate), and `src/sync/mod.rs` (`.beads/gates.jsonl` is the verdict sidecar). The issues JSONL does not hold verdicts.
* Durable Streams `PROTOCOL.md` draft 1.0 at commit `461b40267aabd644558f9b19dbb9507dd5f691cf`, plus `docs/concepts.md` and `packages/state/STATE-PROTOCOL.md` at that commit. A second automated research pass failed its own verifier and is not a source.

The effect-oriented discipline (defer execution, four in-process outcomes, policy on the value, typed acyclic dependencies, scope-owned cleanup, uninterruptible commits, substituted tests, the state ladder, bounded waits, parse at the boundary, libraries return effects, the Elm fold, bounded map-reduce, journal intent before the world moves) is the constitution's source. Where this stack already persisted a fifth outcome, park, the constitution follows the stack.

### Cross-check on 2026-10-02

A deep-research pass against the tag and the v4 docs confirmed the flat cause, the fork renames, `TestClock`'s fork-then-adjust order, per-module stability, and the absence of `src/unstable`. The tag commit is `67ba4e46a11ccda0b6761578bfd22c04ae00167d`, the same commit the reference pages link. This record was amended the same day where it had overclaimed:

* A whole subpath is not unstable because its barrel file contains the word. Classification is per file.
* `fork_detach` attaches to the root scope. It does not take a supervisor service.
* `all` is sequential unless the caller passes a bound.
* `MemoMap` sharing and in-generator `provide` are two different shapes, and phase 2 tests both.
* L6's release-on-every-exit rule is not spelled in the concurrency guides that were opened. Phase 1 confirms it against `acquireRelease` at the pin.

The pass did not open all 356 module pages. It is not a second ledger. `conformance/inventory.sh` still is.

### Agent authority (2026-10-04)

The operator asked for an agent to migrate, rewrite, or integrate this framework into a named sibling without a human in the loop. That sentence is permission to implement. It is not a flip of this record's status to `accepted`. An agent does not ask whether the record is accepted.

Closed, so the agent does not ask:

* Facade features `schema` and `stm` default on. `tokio` defaults on. `durable-streams` defaults off.
* First HTTP client is `ureq` 2. A second client implements the same service trait.
* Executor defaults are 4 workers and a queue bound of 4096.
* Resonate stays. Chiebukuro ADR-0011 stays. The migration does not remove the git dependency.
* The resume token is an `offset`. A `collapsing_cursor` is not stored as one.
* The native log is in-process. The HTTP server is an adapter.
* Issues and gates are two streams. Gates are `.beads/gates.jsonl`.
* If the utterance names no sibling, name the four (flywheel, toron, beads, chiebukuro) and stop. Do not guess.
* If the utterance names a sibling and the phase gate in `skills/elm-effect` is red, implement the missing phases in this repository, then edit the sibling. Do not stop to ask which to do first.

The procedure lives in `skills/elm-effect`. This record stays the law. The skill stays the steps. An agent that copies a law into a new place has created a second source.

### Locked defaults

* `schema`, `stm`, and `tokio` are default features.
* `ureq` 2 is the first HTTP implementation.
* Executor defaults are 4 and 4096.
* Protocol pin is commit `461b40267aabd644558f9b19dbb9507dd5f691cf`.

### What a later agent reads first

1. `skills/elm-effect/SKILL.md`, then the one reference it names for the branch.
2. This record's constitution, L16, and the durable-log section, when the branch persists or replays.
3. `conformance/PIN` and `conformance/modules.toml` once phase 0 exists.
4. The Effect file named by the ledger row, at the pin, not at `main`.
5. For journal work, the three toron files named above, read again. They move.
