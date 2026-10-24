# Durable streams, then the three fiber edges

Date: 2026-10-24. Sources studied: the Durable Streams Protocol
(`durable-streams/durable-streams`, draft 1.0, ElectricSQL) and the Ursula
README / architecture notes (`tonbo-io/ursula`). Scope: `DurableLog`,
`Runtime::run`, `Effect`, `Scope`.

## What the protocol actually specifies

Five ideas matter to a log, and we already had four:

- Offsets are opaque, lexicographically sortable, and the log mints them;
  `-1` (start) and tail are sentinels, never minted.
- An append is atomic and idempotent per `(producer-id, epoch, seq)`.
  Duplicate triples reply the original offset; a lower epoch is fenced; a new
  epoch starts at sequence 0; epoch state does not expire.
- Reads are catch-up or live; `Stream-Up-To-Date` means caught up (more may
  arrive) and `Stream-Closed` means EOF. They are different signals.
- Closure is durable, monotonic, idempotent, and can be atomic with a final
  append.
- A fork inherits a prefix of a source and drops producer epoch state; source
  streams are refcounted and soft-deleted while forks exist.

Our `stream.rs` already modeled status, retention-vs-soft-delete, snapshots,
and `EpochLog::fork`; `DurableLog` already had the offset and producer rules.
The gap was real closure on the log the Elm loop uses: the guide promised
`Closed` while the trait had no way to close.

## What we took

**Closure on the real log.** `DurableLog::close`, `append_json_and_close`,
`status`, `is_closed`, and `AppendError::Closed`. Closure is monotonic and
idempotent; bytes stay readable. `status` reuses the existing `stream::Status`
instead of inventing a second liveness vocabulary.

**Atomic final append.** The final record and the closed flag share one line in
`FileLog`, so a crash leaves both or neither. A bare close is its own marker
line; a reopen sees EOF. Compaction writes the closed flag into the checkpoint
and keeps the surviving records as `replayed`, so closure survives a rewrite.

**Dedupe only the closing triple.** On a closed stream, a retry of the append
that closed it replies its original offset; a replay of an earlier triple is
`Closed`. That is the protocol's distinction, and it keeps retries safe
without pretending the stream is still open.

**Laws on the trait, not one kernel.** `laws::closing()` returns the two
closure laws as functions, each of which closes its log, so a caller runs
`all()` on one log and each closing law on a fresh one. `MemLog` and `FileLog`
run the same checks.

## What we deliberately did not take

- **Fork on `DurableLog`.** `EpochLog::fork` covers the model (prefix plus
  dropped producer epochs), but a real copy-on-write fork needs refcounts,
  soft-delete, and cascading GC across both kernels. That is storage work,
  not a framework primitive; deferred.
- **TTL / expiry / soft delete on the log.** There is no URL, owner, or
  multi-tenant surface here. `stream.rs` keeps the model for a future adapter.
- **A second lexicographic `Stream-Seq`.** `(producer_id, epoch, seq)` already
  orders each writer; adding another sequence would be a second ordering rule
  to keep consistent in the same admission path, with no caller that needs it.
- **SSE / long-poll / live tail.** Transport belongs to the host. Catch-up
  reads already return whole records, and `Status` carries the EOF signal.
  (Revisited in the follow-up below: SSE and long-poll now have an adapter;
  CDN cursor collapsing still does not.)
- **Ursula's Raft groups, S3 cold tier, leader redirects, thread-per-core.**
  Those are the price of a distributed server. We are not building one; the
  framework-level steal is the contract, not the deployment. Our single-lock
  `Store` is the same "one owner per stream, no shared hot state" idea at one
  process.
- **Group commit / batched fsync.** Ursula batches flushes for throughput; our
  `FileLog` fsyncs every acknowledged append. Batching would trade the
  "returned Ok means durable" contract for speed, so it stays a `ponytail`
  note until a real workload asks.

## Fiber static edge

`Runtime::run` used to interpret on the caller's thread. It now builds a
private runtime, spawns the effect on a fiber, joins it, and shuts down any
detached child the body left. `run_main` inherits the change. The static and
managed edges now differ only in services and disposal, not in whether
structured concurrency exists.

Consequence worth naming: `Effect::never()` at the static edge now parks until
interrupted, because it really is on a fiber. Previously it died with "outside
a fiber". That is the correct Effect 4 behavior, but a test that calls
`Runtime::run(Effect::never())` will hang rather than fail.

Exit settlement also moved ahead of scope close: a body that returns success
after an interrupt landed becomes an interrupt **before** its finalizers run,
so an exit-aware release is not told the body succeeded.

## Effect fan-out

`Effect::all(effects, concurrency)` and `Effect::all_discard` are the
effect-level counterpart of the runtime-level `all`. Each effect runs on a
child of the executing fiber; results keep input order; `Bounded(n)` spawns
lazily so the peak in flight respects the bound; the first failure interrupts
and joins every live sibling before it is returned. A pending parent
interrupt stops the remaining children, interrupts live ones, joins their
releases, and returns interrupted. Services provided around the fan-out and
the effect's own provisions both reach the children.

## Exit-aware scope

`ScopeExit` erases the success value and the typed error but keeps the shape:
`ScopeOutcome::{Success, Failure, Unknown}`, typed-failure count, defects, and
an interrupted flag. `Scope::add_finalizer_with_exit` registers a release that
sees it; `Scope::close_with` supplies it. `Ctx` and `FiberHandle` expose the
same registration.

Wiring: the fiber supervisor closes its scope with the settled body exit;
`Effect::scoped` closes with the body's `Result`; a direct `close` or `Drop`
reports `Unknown` because there is no body exit to report. Finalizer defects
still append after the body's reasons and never rewrite what the body
reported.

## Evidence

- `crates/elm-effect-journal/tests/durable_log_closure.rs`: 10 cases across
  `MemLog` and `FileLog` (idempotent close, readable EOF, atomic
  append-and-close, closing-triple retry, earlier-triple refusal, reopen,
  compaction, late-record corruption, torn close line).
- `crates/elm-effect-journal/tests/durable_log_laws.rs`:
  `closing_laws_hold_on_fresh_logs`.
- `crates/elm-effect/tests/scope_exit.rs`: 9 cases (scoped success, typed
  failure, defect, interrupt, fiber exit, interrupted success, direct close,
  LIFO, mixed cause, effect release, late effect release).
- `crates/elm-effect/tests/effect_fan_out.rs`: 6 cases (order, bound, fail-fast
  release, services, discard, parent interrupt).
- `crates/elm-effect/tests/l4_fiber.rs`:
  `the_static_edge_gives_with_fiber_a_real_fiber`.
- `cargo test --workspace`: 1,287 passed, 0 failed, 99 result blocks
  (the follow-ups added the page, exit-aware effect, HTTP/SSE, long-poll, and
  restart tests).
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`,
  `git diff --check`: clean.

## Follow-up: the three boundaries closed

- **Page read.** `DurableLog::read_page` returns `Page { records, next,
  up_to_date, closed }`. The cursor is the last consumed record, `Now`
  resolves to the last existing offset (or `Start`), and `limit == 0` still
  returns one record so a page never stalls. Tests: `durable_log_page.rs`.
- **Exit-aware effect release.** `Scope::add_finalizer_effect_with_exit`
  passes `&ScopeExit` to an effect finalizer; a late registration sees the
  recorded exit and returns its defect. Tests: `scope_exit.rs`.
- **HTTP/SSE.** `elm_effect_journal::http::HttpServer` serves `GET`, `HEAD`,
  and `POST /stream`, plus `?live=sse` and `?live=long-poll&timeout=<seconds>`,
  from the same page API. Long-poll resolves `now` once, then waits at the
  tail for a record, EOF, or the timeout. It maps the page fields to
  `Stream-Next-Offset`, `Stream-Up-To-Date`, and `Stream-Closed`. E2E:
  `http_stream.rs` (append, page, close, EOF, SSE catch-up, SSE from now,
  closing append, long-poll wait and timeout, and SSE resume across a
  `FileLog` restart).
- **Proxy setup.** `docs/guides/16-reverse-proxy.mdx` covers TLS, auth,
  response buffering, timeouts, and the 8 MiB body limit. The adapter still
  has no auth or TLS of its own.

## Boundaries left

- No fork or soft-delete on the concrete log (see above).
- The Elm loop does not close its log at `Done`. That is a policy decision for
  the host: a log may be reused for a later run, so the loop only exposes the
  closure through the trait.
- The HTTP adapter is single-process and unauthenticated. It has no TLS, TTL,
  delete, fork, `Stream-Seq`, chunked request body, or cursor collapsing; it
  runs one thread per connection and polls the log every 25 ms. Bind loopback
  and put a proxy in front for auth and TLS (guide 16).
