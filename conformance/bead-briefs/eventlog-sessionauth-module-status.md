One derived line in `conformance/modules.toml` cannot be committed by the agent
who caused it: `packages/effect/src/eventlog/EventLogSessionAuth.ts` is now
fully ported (all 6 rows, landed in commit `7d55395`,
`elm-effect-rs-port-eventlog-identity-session-auth-75xz`), so
`conformance/inventory.sh` derives `status = matched` for it. Left `unmatched`,
`cargo test -p elm-effect-conformance --test ledger` fails with
"EventLogSessionAuth.ts: unmatched with every behavior row already ported".

The line is already applied in the worktree, which keeps the tree green, but it
is uncommitted:

    [[module]]
    path = "packages/effect/src/eventlog/EventLogSessionAuth.ts"
    class = "unstable-port"
    status = "matched"   # was "unmatched"

`toron reserve acquire` for the file is refused — CyanGlen holds the exclusive
lease (reason `elm-effect-rs-azf3`), and the same worktree also holds that
agent's uncommitted FileSystem.test-utils `tests = [...]` edit, which must not
be swept into someone else's commit. `force-release` is operator-only and was
not used. The holder was asked via `toron reserve request-release` (message
`8876fb56…`) and the request is also recorded as a comment on `elm-effect-rs-azf3`.

## Done

Whichever way the lease resolves: `conformance/modules.toml` carries
`status = "matched"` for `EventLogSessionAuth.ts`, committed, and
`cargo test -p elm-effect-conformance --test ledger` passes with a clean
worktree for that file.
