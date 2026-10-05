# Durable log

Read `docs/guides/12-durable-log.mdx` and ADR-0002 section "Durable log". This file is the order of work, not a second spec.

Pin: Durable Streams `PROTOCOL.md` draft 1.0, commit `461b40267aabd644558f9b19dbb9507dd5f691cf`. The header date inside the file is the placeholder `2025-01-XX`.

## Order

1. Implement the in-process log in `crates/elm-effect-journal` until every L16 test passes.
2. Store `{model, offset}` as the snapshot row. The offset is a token the log returned. Reject a token the log did not mint (`hand_built_offset_is_rejected`).
3. One JSON stream per Elm program run. The interpreter appends. `update` does not take a stream handle.
4. A branch is a fork at the snapshot offset. The fork's epoch starts at 0.
5. Stop. Feature `durable-streams` (HTTP, pull-wake, `collapsing_cursor`) is out of this pass.

## Map, when touching toron

Use the vocabulary table in ADR-0002, section "Durable log". `plan_attempt`'s attempt number is not `seq`. `RetrySameKey` resends the same `(epoch, seq)`. The attempt column stays on the journal row.

`can_transition` does not grow a stream offset column by guesswork. If the row has no offset column, add one in the journal store migration and commit it in the same transaction as the stream append.

## Do not port

`packages/state/STATE-PROTOCOL.md` (insert, update, delete, `snapshot-start`, `snapshot-end`). `packages/client-rust/design.md` is a proposed HTTP client, not the kernel. The Node server and the Caddy plugin are peers.
