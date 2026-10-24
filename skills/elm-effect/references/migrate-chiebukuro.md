# Migrate chiebukuro

Repo: the chiebukuro workspace. Law: ADR-0002 sibling migration "Chiebukuro". ADR-0011 (`docs/decisions/0011-chiebukuro-daemon-resonate.md`) stays in force. This migration does not supersede it.

## Gate

Green in elm-effect-rs: phase 3 for a new effect that is not durable, phase 9 and 9b before a pipeline leaves `#[resonate::function]`.

## Order

1. Leave every existing `#[resonate::function]` registered in `src/daemon/mod.rs` `register_all` on Resonate. `resonate.db` stays the checkpoint store for those.
2. A pipeline moves only when the user named that pipeline. One pipeline per change. The move is a `Program` whose durable commands use the journal, with Resonate as one `WorkflowEngine` backend until that pipeline's checkpoints are read back from the journal equal to the old ones.
3. Search and ingest are effects. A ranking that must replay is a fold over a message that already contains the retrieved set. The fold does not call the store.
4. Memory record is a durable command. Memory recall is an effect whose result comes back as a message.
5. Side effects that must survive a restart stay inside the engine, Resonate or the journal. Do not add a write beside `resonate.run` "temporarily".
6. The git dependency on `resonate-sdk` branch `main` stays in chiebukuro. Do not add it to elm-effect-rs.

## Done

- [ ] Unnamed pipelines still have their `#[resonate::function]` and their register line.
- [ ] The named pipeline's fold test replays a log and emits the same command list.
- [ ] A crash-shaped test (kill between intent and settle) redelivers once and does not double-commit.
- [ ] `cargo test` for the daemon register path and the pipeline that moved.
