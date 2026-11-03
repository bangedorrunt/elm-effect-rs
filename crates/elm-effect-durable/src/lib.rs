//! Durable engine port and our single-host engine (ADR-0005).
//!
//! Scheduling verbs only: [`DurableEngine`] (`start`, `park_until`, `wake`,
//! `cancel`, `query`). Journal lifecycle and
//! [`DurableLog`](elm_effect_journal::durable_log::DurableLog) stay in
//! `elm-effect-journal` — this crate does not invent a third vocabulary.
//! The run key equals the journal key, which equals the log `producer_id`.
//!
//! The default engine is [`OurEngine`]: single-host, storage-trait-backed and
//! journal-injectable (in-memory shipped; a host binds its persisted store
//! with `with_storage` and its journal with `with_journal`). Pure park and
//! dispatch decisions reuse
//! [`plan_park`](elm_effect_journal::plan::plan_park) and
//! [`plan_attempt`](elm_effect_journal::plan::plan_attempt); they are not
//! reimplemented here. A clock is injected; tests advance time explicitly.
//!
//! [`CronSchedule`] is the pure 5-field cron timing kernel. The host computes a
//! tick, calls [`DurableEngine::start`] with [`fire_key`], and keeps cron
//! registration and the tick loop.
//!
//! ## Framework fit
//!
//! The engine is a synchronous kernel, like `DurableLog`, not a second
//! runtime. The pure decisions are functions ([`due_wakes`],
//! [`CronSchedule`], `plan_park`, `plan_attempt`). The host reads its
//! `Clock` / `TestClock` at the edge and passes `now_ms`; the engine never
//! sleeps, spawns, or reads the clock. Storage and journal are traits, so the
//! host supplies its store and can inject a persisted journal. Both are
//! fallible: the store returns [`StorageError`] and the journal returns
//! [`JournalError`](elm_effect_journal::store::JournalError); the engine
//! propagates them as [`EngineError::Storage`] and [`EngineError::Journal`].
//! `MemStorage` and `InMemoryStore` never fail.
//!
//! A host lifts an engine verb into an `Effect` and provides the engine through
//! its own `ServiceTag`; `tests/framework_fit.rs` is the witness. [`EngineMsg`]
//! is data for the host's Elm `Msg`, and the run key is the host's durable
//! command key and the `DurableLog` `producer_id`. Retries, timeouts, and
//! remote policy stay in the host's `Interpret` / `plan_attempt` layer.
//!
//! See `docs/decisions/0005-durable-engine-port-and-our-engine.md`.

#![warn(missing_docs)]

mod cron;
mod engine;
mod runtime;
mod scheduler;
mod storage;

pub use cron::{CronError, CronSchedule, fire_key};
pub use engine::{
    DurableEngine, EngineError, ParkOutcome, ParseRunPhaseError, RunPhase, RunStatus, StartOptions,
    StartOutcome, StorageError, engine_laws,
};
pub use runtime::{EngineMsg, OurEngine};
pub use scheduler::{WakeIntent, due_wakes};
pub use storage::{EngineStorage, MemStorage, ParkedRow, RunRow, storage_laws};
