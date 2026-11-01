//! Durable engine port and our single-host engine (ADR-0005).
//!
//! Scheduling verbs only: [`DurableEngine`] (`start`, `park_until`, `wake`,
//! `cancel`, `query`). Journal lifecycle and [`DurableLog`] stay in
//! `elm-effect-journal` — this crate does not invent a third vocabulary.
//! The run key equals the journal key, which equals the log `producer_id`.
//!
//! The default engine is [`OurEngine`]: single-host, storage-trait-backed
//! (in-memory shipped; toron later binds `WorkflowEffectStore`). Pure park
//! and dispatch decisions reuse [`plan_park`] and [`plan_attempt`]; they are
//! not reimplemented here. A clock is injected; tests advance time explicitly.
//!
//! See `docs/decisions/0005-durable-engine-port-and-our-engine.md`.
//! [`DurableLog`]: elm_effect_journal::durable_log::DurableLog
//! [`plan_park`]: elm_effect_journal::plan::plan_park
//! [`plan_attempt`]: elm_effect_journal::plan::plan_attempt

#![warn(missing_docs)]

mod engine;
mod runtime;
mod scheduler;
mod storage;

pub use engine::{DurableEngine, EngineError, ParkOutcome, RunPhase, RunStatus, StartOutcome};
pub use runtime::{EngineMsg, OurEngine};
pub use scheduler::{WakeIntent, due_wakes};
pub use storage::{EngineStorage, MemStorage, ParkedRow, RunRow};
