//! Lease heartbeat (`heartbeat`): a live worker extends its claim without moving the fence; a zombie or cancelled holder learns it must stop.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.
