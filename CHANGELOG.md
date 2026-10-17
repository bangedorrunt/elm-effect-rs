# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Doorbell wake drops recorded on this tracker were fixed in flywheel, not in this crate. The cumulative audit counter cannot return to zero. Restart no longer waits behind a full watch queue: flywheel `82a8f9e14df7a873f08ed03a808acb45ca0e2dee` rings death on a side bell. Earlier stops are `6cbb974a2051918e358962d7de2d7ea97f0addd7` (condemned consumer stops) and `9c1bccd2fbf36d016fb8f001ec219c129c1d361d` (quiet watches do not fsync).

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
