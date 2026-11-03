//! Lineage rollover (`rollover_key`, `continue_as_new`): close generation k and start k+1 under a deterministic key, idempotently.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.
