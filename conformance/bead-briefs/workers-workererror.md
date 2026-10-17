# Port the workers WorkerError rows

Source rows: `packages/effect/test/workers/WorkerError.test.ts` (`:16`, `:27`,
`:40`, `:55`, `:66`, `:77` — 6 of 7; `:93` left unported).

## What changed

`crates/elm-effect-unstable/src/workers.rs` now models the pin's
`WorkerError` as a wrapper carrying a `WorkerErrorReason` (four tagged variants
`WorkerSpawnError`/`WorkerSendError`/`WorkerReceiveError`/`WorkerUnknownError`,
each with `message` and optional `cause`), with `message()`/`cause()`
delegation, `is_spawn`/`is_send`/`is_receive`, `Display`, JSON codecs matching
the pin's `{ _tag, message, cause? }` and `{ _tag: "WorkerError", reason }`
shapes, and a TypeId-based `is_worker_error` guard. `serde/derive` added to the
crate's deps. `enc_test_work.rs` updated to the new constructor shape.

## Witness

`crates/elm-effect-unstable/tests/worker_error.rs` — one test per row.

## Verify

`cargo test -p elm-effect-unstable --test worker_error --test enc_test_work`

## Boundary

`:93` (reports postMessage failures as WorkerSendError) needs the worker
platform send/run buffering model, which is not built in this unit, so it stays
`unported` and `WorkerError.ts` stays `unmatched` (no modules.toml change).
