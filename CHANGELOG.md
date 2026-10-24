# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-10-25

### Added

- Durable stream closure on the real log: `DurableLog::close`,
  `append_json_and_close`, `status`, and `is_closed`. `AppendError::Closed`
  refuses a later append, bytes before the close stay readable, and `FileLog`
  persists the pair in one line (a sealed append) or as a close marker. The
  closure laws in `laws::closing` run each kernel on a fresh log.
- Catch-up pages: `DurableLog::read_page` and `Page` return bounded records,
  a resume cursor that stays valid on an empty tail, `up_to_date`, and
  `closed` (EOF). `Offset::Now` resolves to the last existing record, so a
  live reader catches the next append instead of skipping it.
- An HTTP/SSE adapter over one log: `elm_effect_journal::http::HttpServer`
  serves catch-up pages, long-poll reads (`?live=long-poll&timeout=<seconds>`),
  append, close, HEAD, and a live SSE tail at `/stream`. It is dependency-free
  and binds wherever the caller asks.
- Exit-aware scope finalizers: `ScopeExit` and `ScopeOutcome`, plus
  `add_finalizer_with_exit` on `Scope`, `Ctx`, and `FiberHandle`, and
  `Scope::close_with`. A release sees success, typed-failure count, defects,
  and interruption; a direct close or `Drop` reports `Unknown`.
  `Scope::add_finalizer_effect_with_exit` gives effect releases the same view.
- `Effect::all` and `Effect::all_discard`: effect-level fan-out with ordered
  results, `Concurrency::Bounded(n)` children, and fail-fast interruption of
  live siblings.

### Removed

- The port ledger: `conformance/` (3.9 MB of TSV/TOML and 19 scripts) and the
  `elm-effect-conformance` crate. The behavior tests themselves stay; only the
  bookkeeping and its four gates are gone.
- The `stm` feature and module, with its six test files: 2,503 source lines
  and 208 public items, with no consumer outside its own tests. `Effect::tx`
  and `Effect::tx_retry` went with it.
- Public foundation types with no consumer: `Predicate`, `Matcher`, `Filter`,
  `Differ`, and `Tagged` (with `tagged`).
- The unused `durable_log::ProducerId` bundle. The log parses the same three
  fields into its admission state; no caller constructed the type.
- The test-only function helpers from `foundation`: `identity`, `constant`,
  `const_true`, `const_false`, `const_null`, `const_unit`, `flip`,
  `flip_curried`, `compose`, `tupled`, `untupled`, `absurd`, `apply`, `sk`,
  `hole`, `memoize`, `memoize_idempotent`, `reverse_ordering`, and
  `match_ordering`. `pipe!` and `flow!` stay.
- `port_*` test names, replaced with the module each one covers.

### Changed

- Workspace crates bumped 0.3.0 to 0.4.0. The public removals below are why
  this is a breaking release under 0.x.
- `Runtime::run` and `Runtime::run_main` execute the effect on a fiber, like
  the managed edge. `with_fiber`, `fork_child`, `race`, `timeout`, and
  `Effect::all` now work at the static edge instead of dying with "outside a
  fiber"; a body that returns success after interrupt lands exits interrupted,
  and its exit-aware finalizers see that interrupt.
- `config::LogLevel` and `observability::LogLevel` are one type, defined in
  `log_level` and re-exported from both paths and the crate root. The variant
  is `Warn`; `parse` accepts both `Warn` and `Warning`.
- `ExecutionPlan` is renamed `AttemptPlan`, exported with `NoAttempts` and
  `Planned` from the crate root. The local type is a bounded attempt budget,
  not Effect 4's multi-step `ExecutionPlan` (fallback layers, `while`,
  schedule, metadata), and the name no longer promises otherwise.
- `ManagedRuntime::run` now executes the effect on a fiber, so `with_fiber`,
  `fork_child`, `race`, and `timeout` work at the managed edge. `run_fork`
  accepts any requirement channel, not only `()`.
- Elm loop command windows run as fibers of the loop's runtime instead of
  scoped OS threads. A command can use the fiber-only combinators and is
  supervised like any other effect.

### Fixed

- `number_order`, `string_order`, and `boolean_order` returned `Greater` for
  equal values. That made `is_less_than_or_equal` false for a tie, excluded
  bounds from `is_between`, skipped `clamp`'s own value, and stopped
  `Order::combine` before its tie-breaker.
- The Elm loop reconciles `AppendError::Unavailable` against the log before
  retrying. A landed append with a lost acknowledgement is no longer reported
  as `NotAppended`; an answer that stays unknown reaches the fold as the new
  `Failure::Unknown`; a matching producer triple with a different payload is
  a `PayloadMismatch`, not a second append.
- Doorbell wake drops recorded on this tracker were fixed in flywheel, not in this crate. The cumulative audit counter cannot return to zero. Restart no longer waits behind a full watch queue: flywheel `82a8f9e14df7a873f08ed03a808acb45ca0e2dee` rings death on a side bell. Earlier stops are `6cbb974a2051918e358962d7de2d7ea97f0addd7` (condemned consumer stops) and `9c1bccd2fbf36d016fb8f001ec219c129c1d361d` (quiet watches do not fsync).

[0.4.0]: https://github.com/bangedorrunt/elm-effect-rs/compare/v0.3.0...v0.4.0

## [0.3.0] - 2026-10-24

The conformance ledger is closed. The phase-13 gate (`port_rows_are_matched`)
now runs un-ignored: every port-like module with a pin test is `matched`, every
module the ledger cannot speak for is named in `modules-without-rows.tsv`, and
no behavior row is `unported` (9,970 rows: 1,507 ported, 8,298
not-applicable, 56 skipped-upstream). The 0.2.0 claim of behavior parity was
not enforced by this gate; it is now.

### Added

- `Duration`: `max`, `min`, `clamp`, `equals`, `between`, the four relational
  predicates, and `combiner_max`/`combiner_min`/`reducer_sum`.
- `Equivalence::struct_` and `Order::struct_`: named-field comparisons that
  ignore extra keys. `Order::combine` now runs its tie-breaker only when the
  first criterion ties.
- `Fiber::wait_interrupted`, plus a per-fiber wake signal. `request_interrupt`
  and `mark_done` notify it, so a parked fiber is released by the event itself.

### Changed

- Fibers no longer poll their interrupt flag every 10 ms. The race/timeout
  loops, await-children, callback, layer-memo waiters, queue and semaphore
  interruptible waits, and both clocks subscribe to the fiber's notification.
  A raw thread outside a fiber keeps the bounded wait because it has no source
  to subscribe to. `race_deadline` keeps a 10 ms tick only for the clock
  deadline, which exposes no notification channel.
- Workspace crates bumped 0.2.0 → 0.3.0 (conformance stays 0.0.0).

### Fixed

- `Order::combine` evaluated its second criterion even when the first decided,
  so a side-effecting tie-breaker ran on every comparison.

[0.3.0]: https://github.com/bangedorrunt/elm-effect-rs/compare/v0.2.0...v0.3.0

## [0.2.0] - 2026-10-17

Behavior parity with the Effect v4.0.0 test suite is complete
(`conformance/behavior-tests.tsv`: 0 unported rows, witness and ledger green).

### Added

- MCP server runtime: in-memory transport, version negotiation, initialize
  handshake, capability advertisement, ping (`mcp_server`), plus tool,
  prompt, resource, logging, and sampling handler registries
  (`mcp_handlers`) and server assembly wiring them together
  (`mcp_assembly`), with idempotent `call_tool_with_id` dedupe.
- JSON Schema compiler: `compile`/`compile_multi` over `SchemaKind` with
  `$defs` hoisting, constraints, unions, tuples, and multi-document shared
  definitions; OpenAPI generation (`openapi.rs`) with reference policy,
  version gating, and spec caching.
- First-class `SchemaKind::TemplateLiteral` with conversion, revival,
  compilation to `pattern`, and template-keyed `patternProperties`.
- FileSystem port: temp dirs/files, scoped cleanup guard, access checks,
  write flags (`w`/`r+`/`r`/`a`/`wx`), and non-clobbering copy.
- Effect runtime hardening: `MaskGuard` for panic-safe masking,
  self-masking `Scope::close`, bounded shutdown with straggler ids,
  `merge_with_deadline`, fiber supervision tree with dump, fenced journal
  and cluster claims, order-insensitive `Provide` with
  construction-time double-provision errors.
- Test world: `TestRandom` (seeded + scripted), `random_layer`,
  `TestContext`, `TestLive`; `Decode.collect` now accumulates at depth;
  single fallible `try_encode` (panicking `encode` removed).
- Facade wiring: orphan modules (`channel`, `cache`, `graph`, …) now
  compile as part of the `elm-effect` lib crate.

### Fixed

- MCP `call_tool` re-ran handlers on resent JSON-RPC ids; now single-flight.
- `Scope::close` ran finalizers interruptible except at one call site.
- Journal/cluster claims could be stolen or resurrected; now fenced by
  generation and lease checks.
- SQL rollback failure discarded the original error; now preserved with
  the store marked unknown.

### Changed

- Hand-rolled Base64 tables replaced with the `base64` crate.
- Removed dead code: `GetIdentityRootSecretMaterial` memoizer,
  `decode_base64_url` helper, `file_size_from_input`/`oversized_ok` and
  their keep-alive shims.
- Workspace crates bumped 0.1.0 → 0.2.0 (conformance stays 0.0.0,
  internal tool).

[0.2.0]: https://github.com/bangedorrunt/elm-effect-rs/compare/v0.1.0...v0.2.0
