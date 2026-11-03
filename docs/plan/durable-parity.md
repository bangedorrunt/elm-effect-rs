# Durable engine parity: Temporal and Resonate verbs, our way

Status: in progress. Governing record: ADR-0005 (extension entries land in
its "More Information" when each verb ships). Carrier law: ADR-0002.

This plan adds the verbs a Temporal or Resonate user reaches for and our
engine lacks. It adds no replay VM, no timer thread, no server, no vendor
adapter. Every verb is a synchronous kernel call on `OurEngine` (or a pure
function), takes `now_ms` as an input, returns typed outcomes as data, and
propagates `StorageError` / `JournalError` without inventing success.

## Rules that bind every bead

1. **Additive only.** No existing public signature, enum variant, struct
   field, or law-suite case list changes. Siblings (toron, chiebukuro,
   flywheel) match `EngineError`, `ParkOutcome`, `EngineMsg` exhaustively and
   construct `Row`, `RunRow`, `StartOptions` by literal; a new variant or field
   breaks them.
2. **No new `DurableEngine` trait methods.** New verbs are inherent methods
   on `OurEngine<S, J>` in the feature's own module (`pub mod` already stubbed
   in `crates/elm-effect-durable/src/lib.rs`; do **not** edit that file, the
   integration bead owns root re-exports). Reuse the `pub(crate)` helpers in
   `runtime.rs` (`require_live`, `project_row`, `wake_checked_impl`,
   `attempt_impl`, `status_with_journal`, `phase_of`, `journal_ms`); do not
   copy their rules.
3. **New `JournalStore` / `EngineStorage` trait methods carry a loud default**
   that returns `Err(...Rejected("<verb> unsupported by this store"))`, never a
   silent no-op, so an un-adopted sibling store fails loudly only when it
   calls the new verb. The one allowed exception is a pure cleanup method
   (`clear_*`) whose default is `Ok(())` because an unsupporting store can
   never have held the data.
4. **New law suites are opt-in.** `engine_laws`, `storage_laws`,
   `journal_store_all` keep identical case lists. Each feature ships its own
   `*_laws` function with the same `(name, Result<(), String>)` shape so a
   sibling adopts it explicitly.
5. **Effect-oriented.** Kernel is synchronous. No clock read, no sleep, no
   spawn, no thread. `now_ms` is a parameter. Keep `Fail`, `Die`, `Interrupt`,
   and park apart. `BTreeMap`/ordered `Vec` wherever order is observable.
6. **Tests prove, with no real sleep and no network:** success; typed failure;
   not-found / terminal / uncertain refusal; stale-fence refusal where a fence
   exists; idempotent replay; a `FaultyStore`/`FaultyJournal` fault-injection
   case per store call (copy the pattern in
   `crates/elm-effect-durable/tests/fallible_*.rs`) proving a failed verb
   changes nothing and a retry succeeds; and `TestClock` driven time
   (fork, adjust, join). Where a section below says **live Loop**, add a
   witness that drives an Elm `Program` through `Loop::send` (pattern:
   `tests/framework_fit.rs`).
7. **Docs.** Every `pub` item has rustdoc (`missing_docs` is a warning and CI
   runs clippy with `-D warnings`). The integration bead writes guide, ADR, and
   skill text; feature beads only write rustdoc.
8. **Gate.** `cargo test -p elm-effect-durable -p elm-effect-journal`,
   `cargo clippy -p elm-effect-durable -p elm-effect-journal --all-targets -- -D warnings`,
   `cargo fmt --check`. Commit only reserved files, cite the bead id, report a
   `unit-test-verified` gate whose note contains `sha=<full sha>`, close, release.

## §1 Lease heartbeat (`elm-effect-rs-` heartbeat bead)

Temporal `activity.heartbeat`, Resonate task lease renewal. A long job must
extend its lease without moving its fence, and learn it was cancelled.

Why not `claim`: claim bumps `generation`, which would fence out the worker
that is renewing.

**Journal (`crates/elm-effect-journal`)**

- `plan.rs`: `HeartbeatRejection` (`NotFound`, `Terminal { state }`,
  `StaleGeneration { expected, actual }`, `NotRunning { state }`,
  `NotOwner { owner: Option<String> }`, `LeaseExpired { lease_deadline:
  Option<i64> }`; derive like `ClaimRejection`, `thiserror`, serde) and pure
  `check_heartbeat(state, current_owner, current_deadline, owner, now_ms,
  expected_generation, current_generation) -> Result<(), HeartbeatRejection>`.
  Check order: terminal, generation, state in `{InFlight, Accepted}`, owner,
  lease (`None` deadline or `deadline <= now_ms` is `LeaseExpired`).
- `store.rs`: `JournalStore::renew_lease(&self, key, owner, lease_deadline:
  i64, now_ms: i64, expected_generation: u64) -> Result<Option<Row>,
  JournalError>`; default per rule 3. `InMemoryStore` implements it through
  `check_heartbeat`: generation unchanged, owner unchanged, `lease_deadline`
  set, offset committed like every other transition; a refused fence is
  `Ok(None)` and writes nothing.
- `laws.rs`: `journal_lease_laws(store) -> Vec<(&'static str, Result<(),
  String>)>`. Do not touch `journal_store_all`.

**Engine (`crates/elm-effect-durable/src/lease.rs`)**

- `HeartbeatOutcome { Renewed { lease_deadline: i64, row: Row }, Cancelled,
  Rejected(HeartbeatRejection) }` (serde, `#[must_use]`-free per the
  `Result` wrapping rule).
- `OurEngine::heartbeat(key, generation, owner, lease_ms, now_ms) ->
  Result<HeartbeatOutcome, EngineError>`. `lease_ms == 0` is
  `EngineError::Journal(JournalError::Rejected(..))`. On a refusal read the
  latest journal row, `project_row` it (storage never stays stale), and map:
  latest state `Cancelled` becomes `Cancelled` (the cancel piggyback);
  anything else is `Rejected(check_heartbeat error)` with the same
  fall-back-to-`StaleGeneration` rule the other verbs use.
- `heartbeat_laws(engine) -> Vec<(&'static str, Result<(), String>)>`.

**Must prove:** renew moves the deadline and keeps the generation; renewed
lease makes `expire_lease` a no-op until the new deadline, then it expires
to `Unknown`; zombie (old generation) rejected; wrong owner rejected; expired
before renew rejected; `cancel` then `heartbeat` yields `Cancelled`; terminal
row rejected; `Prepared`/`Unknown`/`ReconcilePending` rows `NotRunning`;
fault-injection on `renew_lease`/`get`/`commit_run`; live Loop witness where a
long `Interpret` command heartbeats via `TestClock`-driven `now_ms` and stops
on `Cancelled`.

## §2 Retry backoff through park

Temporal `RetryPolicy` between attempts, Resonate durable retry. Backoff must
survive a restart and must not hold a fiber.

**Journal (`src/retry.rs`)**: pure, serde, `Hash`/`Eq`.

- `BackoffPolicy { Fixed { delay_ms: u64 }, Exponential { base_ms: u64,
  factor: u32, cap_ms: u64 } }` with `delay_ms(&self, attempt: u32) -> u64`
  (attempt is 1-based, saturating, never above `cap_ms`, `factor < 2` behaves
  as fixed `base_ms`). No jitter: jitter needs randomness; the host adds it to
  the delay it passes.
- `RetryPlan { Backoff { attempt: u32, delay_ms: u64, until_ms: u64 },
  Exhausted { attempts: u32, max_attempts: u32 } }` and
  `plan_retry(attempts_recorded: u32, max_attempts: u32, policy:
  &BackoffPolicy, now_ms: u64) -> RetryPlan`. **First read
  `InMemoryStore::attempt` in `store.rs` and make the exhaustion boundary
  identical to its dead-letter rule** (a table test pins them equal for
  `max_attempts` 0..=5).

**Engine (`src/retry.rs`)**

- `RetryOutcome { Parked { attempt: u32, until_ms: u64, park: ParkOutcome },
  DeadLettered { row: Row }, Stale, Refused { asked_secs: u64, ceiling_secs:
  u64 } }`.
- `OurEngine::retry_after(key, generation, policy, max_wait_secs, now_ms,
  ceiling_secs) -> Result<RetryOutcome, EngineError>`: `attempt_impl` first
  (fenced; `None` is `Stale`); if the returned row is terminal it is
  `DeadLettered` (the journal already dead-lettered it and `project_attempt`
  cleared parks); else `plan_retry` then `park_until` to `until_ms`.
- Guarantee to document in rustdoc: the attempt is counted before the park
  commits. A crash between the two loses one backoff delay (the run retries
  immediately) and never double-dispatches or loses the attempt count. State it
  plainly; do not claim atomicity.
- `retry_laws(engine)`.

**Must prove:** exponential table; fixed; cap; exhaustion equals journal
dead-letter boundary; `retry_after` parks to the planned instant, `drain_due`
at `until_ms - 1` yields nothing and at `until_ms` yields one `Woke` with
overdue 0; over-ceiling is `Refused` and the attempt is still counted;
exhaustion dead-letters and clears parks; stale fence is `Stale` and projects;
fault injection per store call; live Loop witness: fail twice, back off twice
on `TestClock`, succeed.

## §3 Latched signals (`signal`, `await_signal`, `take_signals`)

Temporal signals with buffered delivery, Resonate resolve-before-await. Today
`wake_checked` on a `Running` run is `NotParked` and the external event is
lost if it beats the park.

**Storage (`src/storage.rs`, `src/signal.rs`)**

- `SignalRow { key, signal_id, payload: String, seq: u64 }` (serde, `Hash`,
  `Eq`). `payload` is an opaque host-serialized string; the engine has no
  JSON dependency and must not gain one.
- `EngineStorage` gains, per rule 3: `put_signal(&mut self, key, signal_id,
  payload) -> Result<bool, StorageError>` (`false` when `(key, signal_id)`
  already exists, **consumed or not**, so a late duplicate never re-latches;
  the store assigns `seq = max + 1` per key), `pending_signals(&self, key) ->
  Result<Vec<SignalRow>, StorageError>`, `take_signals(&mut self, key) ->
  Result<Vec<SignalRow>, StorageError>` (returns unconsumed rows in `seq`
  order and marks them consumed). Defaults return `StorageError::Rejected(
  "signals unsupported by this store")`. `clear_signals(&mut self, key) ->
  Result<(), StorageError>` defaults to `Ok(())`. `MemStorage` implements all
  four.
- `signal_storage_laws(store)`.

**Engine (`src/signal.rs`)**

- `SignalOutcome { Latched { wake: Option<WakeOutcome> }, Duplicate }`.
- `OurEngine::signal(key, signal_id, payload, now_ms) ->
  Result<SignalOutcome, EngineError>`: `require_live` (so `NotFound`,
  `Terminal`, `Uncertain` are typed errors); `put_signal`; `false` is
  `Duplicate` and does nothing else; otherwise, if the run is `Parked`, wake
  it with `wake_checked_impl` and return `Latched { wake: Some(..) }`, else
  `Latched { wake: None }`. A signal is always latched, even when it also wakes.
- `AwaitOutcome { Signalled(Vec<SignalRow>), Parked(ParkOutcome) }`.
- `OurEngine::await_signal(key, until_ms, max_wait_secs, now_ms,
  ceiling_secs) -> Result<AwaitOutcome, EngineError>`: `require_live`; if
  `pending_signals` is non-empty, `take_signals` and return `Signalled`
  without parking; else `park_until`; after a `Wait` commit, re-read
  `pending_signals` and, if one arrived, wake the run, take, and return
  `Signalled` (closes the park/signal interleave for hosts that share a store
  across handles). `ResumeNow`/`Refused` pass through as `Parked(..)`.
- `OurEngine::take_signals(key)` consumes without parking (the resumed run
  reads its payload after `Woke`).
- Terminal projection (`project_row` when the row is terminal) must call
  `clear_signals`; make that one-line edit in `runtime.rs` under a short
  reservation, nothing else in that file.
- **No new `EngineMsg` variant** (`let EngineMsg::Woke { .. } = ..` is an
  irrefutable pattern in siblings). The signal payload reaches the fold through
  `take_signals`, which the host maps into its own `Msg`.
- `signal_laws(engine)`.

**Must prove:** signal-before-park (the lost-wakeup case) returns `Signalled`
and never parks; signal-while-parked wakes and latches; duplicate id is
`Duplicate` before and after consumption; seq order preserved; terminal clears;
`Uncertain`/`NotFound`/`Terminal` refusals; fault injection per store call
including `put_signal` and `take_signals`; restart: a latched signal survives
`OurEngine::recover` on cloned `MemStorage`; live Loop witness: signal arrives
before park, program sees the payload message and never hangs.

## §4 Structured child cancellation (`src/tree.rs`, `src/scheduler.rs`)

Temporal `ParentClosePolicy`, structured fan-out. `plan_join` short-circuits
to `Failed` and hides which siblings still run.

- `scheduler.rs`: `plan_cancel_children(children: &[(String, RunPhase)]) ->
  Vec<String>`: keys whose phase is not terminal (`Running`, `Parked`,
  `Unknown`), in slice order.
- `tree.rs`: `CancelReport { cancelled, already_terminal, missing }` (each
  `Vec<String>`, input order, serde). `OurEngine::cancel_children(keys:
  &[String], now_ms) -> Result<CancelReport, EngineError>`: `query` each
  (`NotFound` is `missing`), plan, `cancel` each (a `Terminal` race is
  `already_terminal`); any `Storage`/`Journal` error returns at once and a
  re-run is idempotent. `cancel` clears the park (already true).
- `cancel_tree_laws(engine)`.

**Must prove:** mixed phases (Running, Parked, Unknown, Settled, Failed,
Cancelled, absent); parked child is cancelled and its park cleared;
`Unknown` child cancels; re-run is a no-op; pairs with `plan_join`:
fail-fast scenario cancels exactly the still-pending siblings; fault
injection; ordering stable.

## §5 Lineage rollover (`src/lineage.rs`)

Temporal `continue_as_new`, Resonate tail `detached`. Bounded history for
recurring runs.

- `rollover_key(base: &str, epoch: u64) -> String` = `"{base}:epoch:{epoch}"`.
- `ContinueOutcome { Continued { started: StartOutcome, closed: Row },
  Rejected(SettleRejection) }`.
- `OurEngine::continue_as_new(from_key, generation, to_key, now_ms, opts:
  &StartOptions) -> Result<ContinueOutcome, EngineError>`. Order is the
  safety rule: (1) fence-check `from_key` with the journal row and
  `check_settle(.., Settled)` and return `Rejected` **before** starting
  anything; (2) `start_with(to_key)` (idempotent: `Already` is fine); (3)
  settle `from_key` to `Settled`. Crash after (2) leaves both live and a
  re-run completes (3); a re-run where `from_key` is already `Settled` at the
  same generation and `to_key` exists is `Continued` (idempotent). Never settle
  first.
- `lineage_laws(engine)`.

**Must prove:** happy path; idempotent re-run after a simulated crash between
(2) and (3) (use a `FaultyJournal` that fails the settle once); stale
generation rejected without starting `to_key`; terminal `from_key` rejected;
`to_key` already terminal still closes `from_key`; key helper format; fault
injection; live Loop witness: a recurring program rolls over three epochs
without growing one run's history.

## §6 Replay-digest guarded start (`src/guard.rs`, `src/storage.rs`)

Temporal non-determinism detection. `elm_effect_journal::plan::check_replay`
already exists (string digests); nothing in the engine carries a digest.

- `EngineStorage` gains `get_digest(&self, key) -> Result<Option<String>,
  StorageError>` and `set_digest(&mut self, key, digest) ->
  Result<(), StorageError>`, loud defaults per rule 3; `MemStorage`
  implements. `digest_storage_laws(store)`.
- `GuardedStart { Started { generation }, Already { status }, Mismatch(
  ReplayMismatch) }`. `OurEngine::start_guarded(key, now_ms, opts, digest)
  -> Result<GuardedStart, EngineError>`: read the stored digest; if present
  and different, **and** a journal row or run row exists, return `Mismatch`
  (no new key, nothing written); if present and different but no row exists
  (a crash after digest write, before intent), overwrite; if absent,
  `set_digest` **before** starting (so a crash cannot start an unrecorded
  run); then `start_with`. Equal digests behave exactly like `start_with`.
- `guard_laws(engine)`.

**Must prove:** first start records; replay with same digest re-attaches;
changed digest on a live and on a terminal run is `Mismatch` and mints no key;
crash-after-digest recovers; a store without digest support fails loudly and
leaves nothing started; fault injection.

## §7 Long-horizon sleep proof (`tests/long_sleep.rs`)

Temporal `workflow.sleep("30 days")`. Believed already supported:
`park_until` stores the absolute `until_ms` and `max_wait_secs` only caps the
reported `wait_ms`. This bead **proves or refutes** it; it is not a feature.

**Must prove (`TestClock`, no real sleep):** a 30-day park with
`max_wait_secs = 3600` is `Wait` with `until_ms` absolute and `wait_ms`
capped; `drain_due` one ms early is empty; at the deadline one `Woke` with
overdue 0; the park survives an engine restart on cloned storage; a
`max_wait_secs` above the ceiling is still `RefusedOverCeiling`; re-parking
to a different instant keeps one park. If any of that is false, stop and mail
the captain with the failing assertion before changing `src/`.

## §8 Integration (after §1 to §7)

Root `pub use` of every new public item in `crates/elm-effect-durable/src/lib.rs`
and `crates/elm-effect-journal/src/lib.rs` doc update; `framework_fit.rs`
live-Loop witnesses that compose the verbs (signal then heartbeat then retry
then cancel tree then rollover); guide sections in
`docs/guides/15-use-the-apis.mdx` with compiled `use` lines added to
`crates/elm-effect-elm/tests/guide_imports.rs`; ADR-0005 "More Information"
entry; `skills/elm-effect/references/durable-log.md` pattern-map rows; the
Temporal/Resonate map gains the new verbs.
