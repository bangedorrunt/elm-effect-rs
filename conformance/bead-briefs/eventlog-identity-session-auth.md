Port the EventLog identity/session-auth surface from the Effect v4.0.0 pin: the
root-secret derivation behind `EventLog.Identity` and the canonical
length-framed session-auth payload with its Ed25519 sign/verify.

Two files, 9 rows:

- `packages/effect/test/eventlog/EventLogIdentityDerivation.test.ts` (3 rows) —
  already witnessed by `crates/elm-effect-unstable/tests/eventlog_identity.rs`
  (module `crates/elm-effect-unstable/src/eventlog_identity.rs`), whose work was
  left uncommitted by a prior session. This bead lands it.
- `packages/effect/test/eventlog/EventLogSessionAuth.test.ts` (6 rows) — new
  module `crates/elm-effect-unstable/src/eventlog_session_auth.rs` against
  `packages/effect/src/eventlog/EventLogSessionAuth.ts`, witnessed by
  `crates/elm-effect-unstable/tests/eventlog_session_auth.rs`.

Carried rows (`eventlog_session_auth`):

- :26 encode/decode round-trip is deterministic and framed by the context field.
- :46 Ed25519 sign produces 64 bytes and verifies.
- :66 a tampered payload verifies `false` (not an error).
- :88 a short signature is refused `InvalidSignatureLength`.
- :102 a short signing public key is refused `InvalidSigningPublicKeyLength` at
  encode time.
- :114 a truncated canonical payload is refused `InvalidPayload`.

Not carried yet, still open in the ledger and named here so the next unit does
not re-derive them: `EventLogRemote.test.ts` (3), `EventLogServerUnencrypted.test.ts`
(3) and `SqlEventLogServerUnencryptedStorage.test.ts` (5) need the remote and
storage transports, which do not exist in Rust.

## Done

`cargo test -p elm-effect-unstable --features eventlog --test eventlog_identity
--test eventlog_session_auth` passes and `rows-port-remaining.py` reports zero
open rows for both files above.
