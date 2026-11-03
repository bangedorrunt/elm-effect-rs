//! Durable retry backoff (`retry_after`): count the attempt, then park until the backoff deadline, or dead-letter when exhausted.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.
