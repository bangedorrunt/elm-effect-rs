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
//! See `docs/decisions/0005-durable-engine-port-and-our-engine.md`.

#![warn(missing_docs)]

mod cron;
mod engine;
mod runtime;
mod scheduler;
mod storage;

pub use cron::{CronError, CronSchedule, fire_key};
pub use engine::{DurableEngine, EngineError, ParkOutcome, RunPhase, RunStatus, StartOutcome};
pub use runtime::{EngineMsg, OurEngine};
pub use scheduler::{WakeIntent, due_wakes};
pub use storage::{EngineStorage, MemStorage, ParkedRow, RunRow};
