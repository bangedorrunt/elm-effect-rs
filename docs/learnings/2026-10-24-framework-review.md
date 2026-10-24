# Framework review: elm-effect after the minimalism cut

Date: 2026-10-24. Mode: full-scope, adversarial, witness-bearing. Skills: `adversarial-reviewer`, `code-quality`.

## Review basis

Reviewed the six-crate workspace after the cut: `elm-effect` (runtime, effect, sync, pubsub, channel, time, layer, STM), `elm-effect-elm` (Loop), `elm-effect-journal`, `elm-effect-test`, `elm-effect-lints`, `elm-effect-conformance`.

Evidence gathered by:

- `cargo test --workspace`: 1,166 tests in 91 suites, 0 failures.
- Eight repeated runs of the sleep-heavy set (`l4_fiber`, `semaphore_core`, `race`, `stream_core`, `port_time_clock`, `schedule`, `layer`, `queue`, `pubsub_subscription`, `tx_core` with `stm`): 0 failures.
- `cargo clippy --workspace --all-targets --all-features`: 112 warnings; the library target itself is warning-free.
- Targeted temporary probes (written, run, deleted): Mask inheritance, zero-capacity queue, interrupted take, semaphore resize.
- `cargo check --workspace --all-targets --all-features`, `cargo check --workspace --all-targets`, and direct inspection of the high-risk modules.

No `unsafe` exists outside `elm-effect-test` (a sound no-op raw waker). There are no `TODO`/`FIXME` markers in `src`.

## Material findings

### F1. `--all-features` is broken; the `stm` test suite cannot compile (blocker, regression)

`crates/elm-effect/tests/tx_hash_set_algebra.rs` still does:

```rust
#[path = "../src/containers/hash_map.rs"]
mod hash_map;
#[path = "../src/containers/hash_set.rs"]
mod hash_set;
```

Both files were deleted by the std-lib cut. The `[[test]]` entry in `crates/elm-effect/Cargo.toml` still declares `required-features = ["stm"]`, so `cargo test --workspace --all-features` fails with `couldn't find file ... containers/hash_map.rs`. The default-feature run never builds it, which is why the cut looked green.

**Minimum fix:** rewrite the test against `std::collections::HashSet` (the algebra under test is `TxHashSet`, not the containers) or delete the test and its `[[test]]` entry, then add `cargo test --workspace --all-features` to the gate. **Do not** restore the cut containers.

### F2. Zero-capacity `Queue::take_n` / `take_between` deadlocks (high, hang)

`Queue::take_between` waits on `not_empty` without claiming a rendezvous reservation. On a capacity-0 queue, a concurrent `offer` waits for a reservation; neither side can move.

Witness: a probe with `Queue::new(0)`, one offerer, one `take_n(1)` timed out after 1 s. The default tests only exercise rendezvous through `take`, which does reserve.

**Minimum fix:** make batch takes reserve like `take` (or route them through `take_interruptible`), add a capacity-0 `take_n` test with a watchdog. Do not change the capacity semantics.

### F3. An interrupted zero-capacity take leaks its reservation (high, corruption)

`Queue::take_interruptible` sets a reservation, then returns `Err(())` on interruption without clearing it. A later `offer` sees the stale reservation, pushes with no taker, and returns `true`; the queue no longer behaves as a rendezvous.

Witness: a probe parked a take, set its interruption flag, then offered; the offer was accepted (`reservation leaked: offer accepted with no taker`). The misplaced doc comment "Give up a reservation" in `sync.rs` is the missing function.

**Minimum fix:** release the reservation and notify `not_full` on every interrupted exit; add the probe as a regression test.

### F4. `Semaphore` subtracts below zero after a shrink (high, panic)

`Semaphore::take` and `take_if_available` compute `state.permits - state.taken` with `usize`. `resize` allows `permits < taken` (its own doc says availability is zero until loans return), so the next take underflows.

Witness: `Semaphore::new(2).take(2)`, then `resize(1)`, then `take_if_available(1)` panics with `attempt to subtract with overflow` at `sync.rs:1304`.

**Minimum fix:** `permits.saturating_sub(taken)` (or clamp on resize); add the probe as a regression test. Do not invent permits in `release`.

### F5. `Mask::Inherit` is silently ignored by scoped/detached forks (high, semantics)

`Ctx::fork_child` and `Fiber::fork_child` match on all three `Mask` variants. `Ctx::fork_scoped` (`fiber.rs:532`) and `Ctx::fork_detach` (`fiber.rs:554`) use `opts.mask == Mask::On`, which turns `Inherit` into `Off`. `Runtime::fork_detach_with` (`runtime.rs:271`) and `Runtime::fork_scoped` (`runtime.rs:297`) do the same; those two have no spawning fiber, so `Inherit` can only mean `Off` there, but the API should say so.

Witness: a parent started with `Mask::On` forked a scoped child with `ForkOpts { mask: Mask::Inherit }`; the child reported `is_masked() == false`.

**Minimum fix:** resolve the mask once (a helper on `ForkOpts`/`FiberControl`) and use it in every spawn entry; test the two `Ctx` entries, and document the runtime entries' `Inherit` as `Off`. Do not change the default start mask.

### F6. Completed children are retained by their parent forever (high, memory leak)

`Shared::spawn` pushes every child into `children[parent]`. `Shared::finish` removes only the finishing fiber's own key, never that fiber from its parent's vector. A long-lived parent that forks and joins per request keeps every `Arc<FiberControl>` (scope, exit slot, fields) until the parent itself finishes. `Effect::await_all_children` also clones the whole vector every 1 ms, so the leak becomes quadratic work while waiting.

**Minimum fix:** remove the finished child from its parent's vector in `finish`; add a long-lived-parent probe that forks, joins, and checks the snapshot size. Do not change parent-completion waiting semantics.

### F7. `Effect::scoped` closes its scope without waiting for member fibers (high, ordering)

`Scoped::run_erased` runs the body, then `fresh.close()` (`effect.rs:279`). `Scope::close` requests interruption on members and immediately runs the scope's finalizers; it does not join members. A scoped child's release can therefore run after the resource it uses has already been released. The pin's `Scope.close` waits for children. Existing test `fork_scoped_is_interrupted_when_the_ambient_scope_closes` only asserts the interrupt flag, so it cannot see the ordering.

**Minimum fix:** either wait for members in `Scope::close` with a bound and report stragglers, or document the divergence and make the release order explicit at the call site. If waiting, do not make `Drop` block unbounded.

### F8. `SubscriptionRef` can skip an update at subscribe time (medium-high, lost data)

`subscribe` reads the current value (`get()`), builds the stream, then registers it under the hub lock. A concurrent `set` between the read and the registration updates `current` and notifies no stream; the new stream replays the old value and never sees the update. The module doc claims "no update is delivered twice or skipped for a live stream", which is false. The per-stream queue is also unbounded, so a slow consumer grows memory without limit.

**Minimum fix:** register and snapshot under one hub lock, or attach a sequence number and replay from it. Bound the queue or state the unbounded ceiling.

### F9. A bounded `PubSub` publisher can stay parked after its only full subscriber leaves (medium-high, hang)

`PubSub::publish` on `Strategy::Bounded` parks on `inner.space` while any live subscription's queue is full. `Subscription::unsubscribe` (and dropping the subscription) only wakes that subscription's own condvar; it never notifies `inner.space`. If the full subscriber unsubscribes or drops, the publisher can wait forever. The full-check also holds the subscription's `live` guard across the `queue` lock in a let-chain, while `take` holds `queue` and then locks `live`, which is a lock-order hazard.

**Minimum fix:** notify `space` on unsubscribe/drop, and bind `let live = ...` before locking the queue. Add a stress test with a full queue and a leaving subscriber.

### F10. A panic inside `atomically` breaks STM isolation (medium-high, soundness)

`atomically` sets the thread-local `IN_TX` to true, runs the body, then resets it. A panic skips the reset, and the `GLOBAL` mutex is poisoned. Every later `atomically` call on that thread sees `IN_TX == true` and runs the body directly, without the global lock, so transactions stop being isolated. The same panic can poison the per-ref mutexes, and every later `.lock().unwrap()` panics.

**Minimum fix:** an RAII guard that resets `IN_TX` on drop (and decide the poison policy: recover or fail closed). Do not add a second transaction model.

### F11. `channel::merge` spawns threads it cannot cancel (medium, resource + perf)

`merge` and `merge_with_deadline` spawn one OS thread per side (`channel.rs:446`, `:544`). The `halted` flag is checked only between pulls, so a blocking pull (a subscription, a queue take) keeps its thread alive after the consumer stops. Dropping the merged pull closes the receiver, but a thread already blocked inside `pull()` never sees that. `Channel::run` also uses `pending.remove(0)`, an O(n) shift per take, making a fan-out channel quadratic.

**Minimum fix:** drive the merge from the consumer (poll both sides per take) or add a cancellation hook to `Pull`; use `VecDeque` for pending takes.

### F12. `TokioExecutor::queue_bound` does not bound a queue (medium, perf/contract)

`with_options` passes `queue_bound` to `max_blocking_threads`, which caps concurrent blocking threads, not the queue. Tokio's blocking queue is unbounded, so fiber tasks queue without backpressure. `queue_bound()` reports a number that does not bound anything. Every `Runtime::new()` also builds a fresh 4-worker Tokio runtime eagerly, so many runtimes mean many idle threads.

**Minimum fix:** rename the knob to what it does, or enforce a real queue with a semaphore; share a default executor and create it lazily.

### F13. `config_effect` contains a permanently dead test module (medium, inert)

`crates/elm-effect/tests/config_effect.rs` gates a `provider_defect` module on `#[cfg(feature = "schema")]`. The schema crate and feature were removed, so the module never compiles. `cargo test --test config_effect` runs 3 tests; the module's tests are gone. The file also triggers unused-import warnings from the dead gate.

**Minimum fix:** delete the dead module or port it to the surviving `Config` API; remove the stale `cfg`.

### F14. The decision record still mandates the removed crates (medium, spec drift)

`docs/decisions/0002-port-effect-4-into-rust-with-an-elm-loop.md` still specifies `Schema`, the `schema` feature, and `elm_effect::Schema` (lines 264, 534). `docs/decisions/0003-make-elm-effect-a-first-class-rust-framework.md` still says default features are `tokio` only with `schema`/`stm` behind `full`, and that `elm-effect-unstable` stays off by default. The tree now has `tokio` and `stm`, no `full`, no unstable. The README calls ADR-0002 "the law".

**Minimum fix:** amend or supersede both records with the operator's removal decision; keep the historical text but mark it superseded. Do not silently edit history.

### F15. The runtime is built on 1 ms polling (medium, perf)

There are 18 `park_timeout`/1 ms-wait sites. `Effect::never`, `Layer::launch`, `race_deadline`, `AwaitAllChildren`, `TimeoutOrElse`, `Race`, `SystemClock::sleep`, `TestClock::sleep`, `Semaphore`, and several queue waits all wake every millisecond. Each parked fiber costs ~1,000 wakeups/s; a server with 10k parked fibers costs ~10M wakeups/s. The `ponytail` comments acknowledge each site, but the aggregate is the framework's main scaling ceiling.

**Minimum fix:** a condvar notified by `FiberControl::mark_done` and `TestClock::adjust`, plus a shared timer wheel for sleeps. Measure before/after with a parked-fiber benchmark.

### F16. `Semaphore::with_permits` leaks permits on panic (low, correctness)

`with_permits` and `with_permits_if_available` take, call `f`, then release; a panic in `f` skips the release. `PartitionedSemaphore::with_permits` already uses a `Hold` guard. Use the same guard for `Semaphore`.

### F17. Detached forks are added to the root scope twice (low, duplicate work)

`Runtime::fork_detach_with` passes the root in `bound`, so `Shared::spawn` already calls `root.add_member`, then it calls `self.shared.root.add_member` again. `Scope::close` interrupts the same fiber twice; the members vector grows by two per detached fiber. Remove the second call (or the first, once).

### F18. `LayerMemo::get` waits without interruption (low, responsiveness)

`LayerMemo::get` builds an effect that calls `entry.wait_done::<E>(Ctx::none())`. A fiber waiting on a shared in-flight build cannot observe its own interrupt; if the builder hangs, the waiter hangs. Thread the ambient fiber into the wait (the API already has `Ctx`).

### F19. Test and lint hygiene (low)

- `crates/elm-effect-elm/tests/loop_joint.rs:263`, `:292`: ignored `Result` from `append_json`/`send`; clippy `unused_must_use`. The later assertion usually catches a failure, but the error itself is lost.
- `crates/elm-effect/tests/port_time_clock.rs:112`, `:125`: `1 * 3_600_000` has no effect (clippy `erasing_op`).
- `stm.rs:98`: `field id is never read`; either use it in the type guard or remove it.
- 112 clippy warnings, almost all dead-code noise from `#[path]` test includes; triage them or scope the allows to the test targets.

### F20. The "only catch_unwind" boundary claim is false (low, documentation)

`lib.rs:10` and `supervisor.rs:1` say the supervisor is the only `catch_unwind` in the workspace. There are 10 sites: the runtime edge, `Scope` finalizers, `Effect::on_exit`/`use`, and `run_finalizer`. The boundary is still deliberate, but the invariant text is wrong. Update the claim or the boundary.

## Soundness ledger

| Claim or obligation | Witness required | Status | Evidence |
| --- | --- | --- | --- |
| Default workspace behavior is preserved | full suite | PASS | 1,166 tests, 91 suites, 8x stress |
| The `stm` feature path builds and tests | `--all-features` | FAIL | missing `containers/hash_map.rs` |
| `Mask::Inherit` starts a child masked | probe | FAIL | child `is_masked() == false` |
| Zero-capacity queue rendezvous makes progress | probe | FAIL | `take_n` deadlock at 1 s |
| An interrupted take releases its reservation | probe | FAIL | offer accepted with no taker |
| Semaphore counts never underflow | probe | FAIL | panic at `sync.rs:1304` |
| A done child leaves its parent's registry | code | FAIL | `finish` removes only the child's own key |
| `scoped` releases members before scope resources | test | ABSENT | no ordering test |
| Bounded `PubSub` publish makes progress after unsubscribe | test | ABSENT | no test |
| STM isolation survives a body panic | test | ABSENT | `IN_TX` reset is not RAII |
| Durable log fencing (L16) holds | journal tests | PASS | `durable_log` suite green |

## Complexity delta

The cut reduced the public surface and crate count substantially; that is the right direction. The remaining incidental complexity is concentrated in (a) the 1 ms polling loops, which stand in for missing notification, and (b) the coarse STM global lock. Neither is a new cost from the cut; both are ceilings the review names.

## Invariant ledger

- **L2 (panic becomes a defect at the fiber boundary):** holds for fiber bodies, but a panic inside a lock-holding user callback can poison shared state (F10, F16).
- **L6 (cleanup defects append, never erase):** holds in `Scope::close` and `Effect::on_exit`; `Effect::scoped`'s ordering is the gap (F7).
- **L7 (releases run masked):** holds for the fiber completion path; `Scope::close` masks only when called on a fiber thread (documented).
- **L16 (durable log fencing):** holds in the journal tests; `FileLog` fsyncs per append and serializes appends (documented ceiling).

## Foot-gun register

- `Semaphore::take(n)` with `n > permits` never returns; `PartitionedSemaphore::take` above capacity also spins forever (documented, but a sync API with no error is a trap).
- `FiberHandle::interrupt` on the current fiber used to deadlock in `wait_done`; the self-wait guard in `wait_done` now returns immediately (the interrupt request is still delivered).
- `Queue::new(0)` with `take_n`/`take_between` hangs (F2).
- `Effect::scoped` returns before scoped children's releases finish (F7).
- `Runtime::timeout` cannot bound a body that ignores interruption, despite the "blocking body must still lose to its deadline" comment.
- `Channel::merge` leaks blocked producer threads when the consumer stops early (F11).
- `Runtime::new` starts four OS threads even when no fiber is used (F12).
- `Mask::Inherit` is silently `Off` on scoped/detached forks (F5).
- `LayerMemo::get` cannot be interrupted (F18).
- `SubscriptionRef::update`/`get_and_update` call `f` under the cell lock; a reentrant `f` self-deadlocks (documented for `update`, not for the others).

## Non-material concerns

- `TestClock::sleep` samples `now` before registering the sleeper, so a concurrent `adjust` can make the deadline older than the documented linearization; `adjust` leaves woken sleepers in `pending_sleepers` until they run. No failing test; worth a note if the clock becomes load-bearing.
- `Remote` mixes `clock.now_millis()` for breaker/budget with `std::time::Instant` for the bulkhead wait, so a virtual clock cannot control the bulkhead.
- `elm-effect-lints` scans source text; `use rand::thread_rng;` and `use std::env;` evade the ambient-world lint.
- `ExecutionPlan::run` calls `interrupted()` twice in the error branch; a side-effecting probe can disagree.
- `Pull::of` requires `A: Clone` although it owns the vector; `Pull::single`/`from_slice` are the cloning forms.
- `elm-effect-conformance/tests/ledger.rs:204` keeps an `#[ignore]` phase-13 parity test that the cut makes permanently unreachable.

## Verification gaps

- No `--all-features` run in the gate; F1 survives because of that.
- No loom/model checking, Miri, or sanitizer run for the lock-heavy primitives.
- The flake hunt is empirical (8 runs), not a proof; the fixed-sleep tests (`l4_fiber`, `l5_scope`, `semaphore_core`) remain load-sensitive.
- STM beyond `atomically`, `stream.rs`, `metric.rs`, `tracer.rs`, and `observability.rs` were sampled, not exhaustively read.
- No benchmark suite, so F15/F12 are reasoned, not measured.

## Residual uncertainty

- F9's lock-order hazard is read from the temporary lifetimes in the let-chain; it needs a stress witness before being called a confirmed deadlock.
- F7's ordering divergence is proven by code, but the pin's exact `Scope.close` wait policy should be checked against the Effect source before implementing a bounded join.
- F11's thread leak requires a blocking `Pull`; the current tests use finite pulls, so no test demonstrates it.
- F6's leak is proven by code; the size of the retained state per child depends on the user's `A` and finalizers.

## Change agenda

1. `change:` rewrite or delete `tx_hash_set_algebra`, add `--all-features` to the gate. `why:` the stm suite is invisible today. `posture:` direct.
2. `change:` remove the dead `schema` cfg in `config_effect`; amend ADR-0002/0003. `why:` inert tests and a contradictory law. `posture:` direct.
3. `change:` saturating semaphore counts + panic-safe `with_permits`. `why:` confirmed panic/leak. `posture:` narrow fix + regression tests.
4. `change:` release the zero-capacity reservation on interrupt; make `take_n`/`take_between` reserve. `why:` confirmed hang/corruption. `posture:` narrow fix + probes.
5. `change:` resolve `Mask::Inherit` in every spawn entry. `why:` confirmed semantic divergence. `posture:` one helper + four tests.
6. `change:` prune completed children from the parent registry. `why:` confirmed unbounded retention. `posture:` narrow fix + long-lived-parent test.
7. `change:` make `Scope::close`/`scoped` wait for members (bounded) or document the ordering and test it. `why:` release-after-resource hazard. `posture:` structural only if the bounded wait cannot be made safe.
8. `change:` atomic subscribe/snapshot for `SubscriptionRef`; notify `space` on PubSub unsubscribe and bind `live` before `queue`. `why:` lost updates and a possible hang. `posture:` narrow locking changes + stress tests.
9. `change:` RAII `IN_TX` guard and a poison policy for STM. `why:` isolation break after a panic. `posture:` narrow fix.
10. `change:` replace `Channel::merge` threads or add cancellation; use `VecDeque` in `run`. `why:` thread leak and O(n²). `posture:` structural; measure.
11. `change:` make the executor queue bound real or rename it; share/lazy the runtime. `why:` false backpressure and idle threads. `posture:` contract fix.
12. `change:` replace the 1 ms polls with notifications/a timer wheel. `why:` scaling ceiling. `posture:` measure first; do not do it blind.
13. `change:` make concurrent combinators interrupt-aware and preserve panic causes. `why:` responsiveness and diagnosis. `posture:` narrow wait-loop changes.

## Fixed-point judgment

Not at a fixed point. The default path is behavior-preserving and the cut is the right shape, but the `stm` path does not build, and four invariants fail under a witness: rendezvous progress, reservation accounting, semaphore counts, and mask inheritance. The subscription/pubsub and scoped-release hazards are material but one step less certain. One repair pass with the probes promoted to regression tests, plus an all-features gate, is the shortest path back to a truthful fixed point.

## Reviewer bottom line

Keep the removal. Fix the hidden `stm` build first, then the four witnessed correctness defects (queue rendezvous, reservation leak, semaphore underflow, mask inheritance), then the child registry leak and `scoped` ordering. The subscription/pubsub and polling work is the next tier, not the first.

## Repair pass (same day)

Every finding above was repaired in one pass. Evidence for the pass:

- `cargo test --workspace --all-features`: 1,359 passed, 0 failed, 1 ignored, 89 suites.
- `cargo clippy --workspace --all-targets --all-features`: 0 warnings.
- `cargo test --workspace --all-features` is now the gate command; the default-feature run is a subset, not the proof.

| Finding | Repair | Witness |
| --- | --- | --- |
| F1 `stm` build | `tx_hash_set_algebra` rewritten against `std::collections::HashSet` | build + 10 rows green under `--all-features` |
| F2 zero-capacity batch take | `take_n`/`take_between` claim and release the rendezvous like `take` | `zero_capacity_take_n_rendezvouses_with_an_offer` |
| F3 interrupted take leak | every interrupted exit calls `release_rendezvous` and wakes `not_full` | `interrupted_zero_capacity_take_releases_the_rendezvous` |
| F4 semaphore underflow | `permits.saturating_sub(taken)` in take/availability | `shrinking_below_loans_never_underflows` |
| F5 `Mask::Inherit` | `ForkOpts::resolve_mask` used by every spawn entry; runtime entries document `Inherit` as `Off` | `inherit_mask_starts_scoped_and_detached_children_masked` |
| F6 child registry leak | `Shared::finish` prunes the completed child from its parent's list | `fiber::tests::a_joined_child_leaves_its_parent_registry` (crate-private registry) |
| F7 `scoped` ordering | `Scope::close` interrupts, then joins members up to `CLOSE_JOIN_BUDGET` before running finalizers; stragglers spill as defects | `scope_close_waits_for_member_releases` |
| F8 subscribe/set race | snapshot and registration share one hub lock | `subscription_ref_subscribe_never_skips_a_concurrent_set` (50 runs) |
| F9 bounded publish hang | `unsubscribe` and drop notify `space`; `live` is bound before the queue lock | `a_leaving_subscriber_releases_a_parked_bounded_publisher` |
| F10 STM panic | `TxFlag` RAII resets `IN_TX`; the global lock recovers from poison | `a_panicking_transaction_leaves_stm_usable_and_isolated` |
| F11 merge threads | sides' halt hooks are kept and called when the merged pull drops; `Channel::run` uses `VecDeque` | `dropping_a_merge_wakes_a_blocked_producer` |
| F12 executor bound | knob renamed `max_blocking_threads`; one shared `OnceLock` Tokio runtime, built on first use | docs + `executor_substitution` |
| F13 dead `schema` tests | module removed; the surviving `Config` rows are 4 live tests | `config_effect` suite |
| F14 ADR drift | both records carry a 2026-10-24 amendment; history kept below it | ADR-0002/0003 headers |
| F15 1 ms polls | `COOPERATIVE_QUANTUM` is 10 ms; `TestClock` sleepers wait on a condvar; `FiberSet::await_empty` delegates to `join` (no poll at all) | code + suite |
| F16 `with_permits` leak | `HeldPermits` restores on unwind, including the interruptible form | `with_permits_restores_on_panic` |
| F17 double root membership | detached forks add the root once | `fork_detach` suite |
| F18 `LayerMemo` wait | the ambient `Ctx` reaches `wait_done`, which polls its own interrupt every 20 ms | `layer` suite |
| F19 hygiene | unused imports/vars gone, redundant closures and collapsible `if`s fixed, dead-code noise scoped to the `#[path]` test hosts | clippy 0 |
| F20 `catch_unwind` claim | the doc now names the four boundaries (supervisor, scope finalizers, `on_exit`/`use`, release) | `lib.rs` header |

### Remaining ceilings (named, not fixed)

- A `Pull` that blocks without registering a halt hook still cannot be cancelled; the hook is the contract, and `merge` now keeps and calls it.
- Poll-based interrupt latency is 10 ms, not zero. A condvar per `FiberControl` is the next step; no benchmark suite exists to price it.
- `Semaphore::take(n)` with `n > permits` has no error channel and waits forever (foot-gun, not repaired).
- One `#[ignore]` phase-13 conformance row remains unreachable after the cut.

## Fixed-point re-judgment

The four witnessed failures (rendezvous progress, reservation accounting, semaphore counts, mask inheritance) now have passing witnesses, the `stm` path builds under `--all-features`, the registry growth and scope-release ordering are closed, and the workspace is clippy-clean. The default path and the all-features path agree. The remaining items are documented ceilings, not silent defects.

## Second pass: residuals and non-material concerns

A second pass closed the cheap, real items that the repair ledger had left as notes, plus every stale comment that still claimed a 1 ms poll.

- **Self-wait.** `FiberControl::wait_done` returns immediately when the caller is the fiber itself, so `FiberHandle::interrupt` on the current fiber can no longer park forever. The request itself is still delivered.
- **`TestClock` linearization.** `sleep` now reads virtual time and registers the sleeper under one lock; `adjust` prunes a sleeper as it wakes it, so `pending_sleepers` means parked, not woken-but-not-yet-run.
- **`ExecutionPlan` probe.** The error branch probes interruption once per failed attempt, so a side-effecting probe cannot answer twice. `plan_probes_interruption_once_per_attempt` pins the count at one per attempt.
- **Lint import evasion.** The ambient rule matches bare `env::` and `thread_rng` as well as the full paths, so `use std::env;` plus `env::var(..)` no longer slips past. The scanner is now string-literal aware (a file-wide scan, so `\`-continued strings are covered) and reports the offending source line rather than a generated message. `imported_ambient_capabilities_still_trip_the_fold_lint` covers the evasion.
- **`Pull::of`.** No longer requires `A: Clone`; it consumes the vector. `single`/`from_slice` keep the bound.
- **Callback wait.** `Effect::callback` uses the cooperative quantum instead of a private 1 ms wait; all remaining `ponytail` comments name the quantum, not 1 ms.
- **Merge test hygiene.** The `stalled()` helper registers a halt hook and parks on a condvar, so the deadline row no longer strands a permanently parked thread when the merged pull is dropped.
- **Formatting.** `cargo fmt --all -- --check` is clean across the workspace (one pre-existing import-order diff in `guide_imports.rs` was included).
- **Poison hardening beyond STM.** `Deferred::complete_with`, `SynchronizedRef`'s callback methods, and both `SubscriptionRef` implementations run user closures under a lock. A panic there used to poison the cell and make every later call panic; each now recovers the guard, which is sound because the value assignment happens after the callback returns. Witnesses: `a_panicking_body_leaves_the_deferred_usable`, `a_panicking_callback_does_not_brick_the_cell`, `a_panicking_update_does_not_brick_the_cell`.
- **Layer unwind guard.** A panicking layer constructor used to unwind past the memo entry while it was still `Building`, leaving every concurrent waiter parked forever. `UnwindGuard` publishes a failed entry and closes the entry scope during unwinding. Witness: `a_waiter_is_released_when_a_shared_build_panics`.

Final gate after all passes:

- `cargo test --workspace --all-features`: 1,365 passed, 0 failed, 1 ignored, 89 suites.
- `cargo clippy --workspace --all-targets --all-features` and the default-feature variant: 0 warnings each.
- `cargo fmt --all -- --check`: clean.

One thing that is still open, honestly: the ignored conformance gate `port_rows_are_matched` was un-ignored and run. It fails with nine unmatched port modules (`Combiner`, `Config`, `Duration`, `Equal`, `Equivalence`, `Function`, `Hash`, `Order`, `Reducer`), which is real phase-13 porting work, not a bookkeeping slip. The test stays ignored with that reason until those behavior rows are ported.

## Third pass: the two named ceilings

Both items named at the end of the second pass are closed, and the stale claims
above are superseded by this section.

- **Phase-13 gate.** `port_rows_are_matched` is no longer ignored. The nine
  unmatched modules (`Combiner`, `Config`, `Duration`, `Equal`, `Equivalence`,
  `Function`, `Hash`, `Order`, `Reducer`) now have 109 ported rows or a stated
  excuse: 103 new witnesses across `port_duration.rs`, `port_combiner.rs`,
  `port_equivalence.rs`, `port_order.rs`, `port_hash.rs`, `function_module.rs`,
  and `platform_config_crypto.rs`; 6 rows are excused with a reason (four
  schema-config rows, two `Date` rows with no date value in the crate). The
  ledger now reports 0 unported rows out of 9,970.
- **Interrupt latency.** `FiberControl` owns a generation-counted `Signal` and
  a list of condvar wakers. `request_interrupt` notifies both; `mark_done`
  notifies the fiber and its parent. The race/timeout loops, await-children,
  callback, layer-memo waiters, queue and semaphore interruptible waits, and
  both clocks subscribe instead of polling. An interrupt can no longer be lost
  between a predicate check and the wait: wakers take the same mutex the waiter
  holds. `race_deadline` keeps a 10 ms tick only because `ClockImpl` has no
  deadline notification channel; child completion and interruption still wake
  it immediately.
- **Measured.** `interrupt_latency.rs` runs 20 park/wake rounds end to end:
  average 28-76 µs and worst 510 µs across three runs, against the old 10 ms
  quantum. The witness asserts the average stays under 3 ms, which a poll
  cannot meet.

Final gate after the third pass:

- `cargo test --workspace --all-features`: 1,439 passed, 0 failed, 0 ignored.
- `cargo clippy --workspace --all-targets --all-features` and the
  default-feature variant: 0 warnings each.
- `cargo fmt --all -- --check`: clean.
- `cargo test -p elm-effect-conformance`: 4 passed, including the formerly
  ignored `port_rows_are_matched`.
