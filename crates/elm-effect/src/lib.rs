//! `elm-effect`: Effect 4.0.0 semantics as a Rust carrier plus the Elm loop.
//!
//! Law L2 lives here: [`Cause`] owns the failure vocabulary at the crate
//! edge, and [`Exit`] is ordinary data, not an effect.
//!
//! Conventions (ADR-0002 Phase 1):
//! - `never` is [`std::convert::Infallible`], not `()`. A channel that cannot
//!   fail is `E = Infallible`.
//! - A panic becomes [`Reason::Die`] at the fiber supervisor (Phase 1
//!   supervisor is the only `catch_unwind` in the workspace). Library code
//!   never catches.
//! - Phase 1 has zero `unsafe`.

mod cause;
mod concurrency;
mod effect;
#[cfg(feature = "tokio")]
mod executor;
mod exit;
#[cfg(feature = "tokio")]
mod fiber;
mod foundation;
#[cfg(feature = "tokio")]
mod layer;
mod optic;
#[cfg(feature = "tokio")]
mod policy;
#[cfg(feature = "tokio")]
mod runtime;
mod schedule;
#[cfg(feature = "tokio")]
mod scope;
mod service;
#[cfg(feature = "tokio")]
mod supervisor;
mod sync;
mod time;

pub use cause::{Cause, Defect, FiberId, Reason};
pub use concurrency::{Body, Concurrency, all, for_each, partition, validate, validate_discard};
pub use effect::{Effect, Requirement};
#[cfg(feature = "tokio")]
pub use executor::{BoxFuture, Executor, TokioExecutor};
pub use exit::Exit;
#[cfg(feature = "tokio")]
pub use fiber::{Ctx, FiberHandle, ForkOpts, Mask, StartMode};
pub use foundation::{
    Combiner, Differ, Equal, Equivalence, Filter, Hash, Matcher, Order, Predicate, Reducer, Tagged,
    array_equivalence, boolean_equivalence, boolean_order, combine_hash, combine_ordering, compose,
    const_false, const_true, constant, flip, hash_f64, hash_str, identity, match_ordering,
    number_equivalence, number_order, ord_order, reverse_ordering, string_equivalence,
    string_order, tagged, tuple_equivalence,
};
#[cfg(feature = "tokio")]
pub use layer::{BuiltServices, Cycle, Layer, RequirementIds};
pub use optic::{
    Iso, Lens, OpticError, Optional, Prism, Traversal, at_map, at_vec, at_vec_exact, failure, none,
    some, success,
};
#[cfg(feature = "tokio")]
pub use policy::{BreakerConfig, Bulkhead, RateLimiter, Remote, RemoteError, RemotePolicy, hedge};
#[cfg(feature = "tokio")]
pub use runtime::{ManagedRuntime, Runtime, race, race_first, timeout};
pub use schedule::{Random, RandomTag, Schedule, repeat, retry};
#[cfg(feature = "tokio")]
pub use scope::Scope;
pub use service::{Cons, MemoMap, Provide, Reference, ServiceTag, read_reference, use_service};
pub use sync::{Deferred, MutableRef, Queue, Ref, Semaphore, SynchronizedRef};
pub use time::{Clock, ClockImpl, Duration, SleepOutcome, SystemClock, TestClock};
