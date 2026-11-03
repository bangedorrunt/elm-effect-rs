//! Replay-digest guarded start (`start_guarded`): a changed re-render of a run is a typed mismatch, never a second run.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.
