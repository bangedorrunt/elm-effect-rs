//! Latched durable signals (`signal`, `await_signal`, `take_signals`): a signal that arrives before the park is never lost.
//!
//! Extension module of ADR-0005. Inherent methods on [`OurEngine`](crate::OurEngine);
//! the `DurableEngine` port trait is unchanged.
