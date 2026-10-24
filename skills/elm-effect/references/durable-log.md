# Durable log

Read `docs/guides/12-durable-log.mdx` and ADR-0002 section "Durable log". This file is the order of work, not a second spec.

Pin: Durable Streams `PROTOCOL.md` draft 1.0, commit `461b40267aabd644558f9b19dbb9507dd5f691cf`. The header date inside the file is the placeholder `2025-01-XX`.

## Order

1. Implement the in-process log in `crates/elm-effect-journal` until every L16 test passes.
2. Store `{model, offset}` as the snapshot row. The offset is a token the log returned. Reject a token the log did not mint (`hand_built_offset_is_rejected`).
3. One JSON stream per Elm program run. The interpreter appends. `update` does not take a stream handle.
4. A branch is a fork at the snapshot offset. The fork's epoch starts at 0.
5. Read with `DurableLog::read_page`, not a hand-rolled loop: it returns bounded records, the last-consumed cursor, `up_to_date`, and `closed` (EOF). `Offset::Now` resolves to the last existing record so a tail catches the next append.
6. The HTTP read/write adapter is `crates/elm-effect-journal/src/http.rs`. It serves catch-up, long-poll, append, close, HEAD, and SSE. It is not a subscription transport. Pull-wake and `collapsing_cursor` remain out of scope. For auth and TLS, read `docs/guides/16-reverse-proxy.mdx`; the adapter has neither.

## Map, when touching toron

Use the vocabulary table in ADR-0002, section "Durable log". `plan_attempt`'s attempt number is not `seq`. `RetrySameKey` resends the same `(epoch, seq)`. The attempt column stays on the journal row.

`can_transition` does not grow a stream offset column by guesswork. If the row has no offset column, add one in the journal store migration and commit it in the same transaction as the stream append.

## Do not copy

`packages/state/STATE-PROTOCOL.md` (insert, update, delete, `snapshot-start`, `snapshot-end`). `packages/client-rust/design.md` is a proposed HTTP client, not the kernel. The Node server and the Caddy plugin are peers.
