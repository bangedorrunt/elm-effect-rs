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

// The port's shapes are higher-order by nature: an `Effect` is a boxed
// transition over boxed handlers, and a family of erased signatures is the
// pin's own model. Alias-mongering would rename the pin's types rather than
// describe them, so the complexity lint is off for this crate.
#![allow(clippy::type_complexity)]

pub mod cause;
pub mod channel;
// `Logger.console*` writes through this port, so it is part of the surface.
pub mod console;
#[cfg(feature = "tokio")]
mod concurrency;
// `Config` is the startup read: a descriptor plus `load` against a provider,
// so an application never touches the environment from domain code.
pub mod config;
// `Config.mapEffect`'s driver: the transform is an `Effect`, and `config`
// cannot name one without breaking its path-module tests, so the bridge lives
// beside it.
mod config_effect;
pub mod config_provider;
mod context;
mod crypto;
pub mod effect;
pub mod execution_plan;
#[cfg(feature = "tokio")]
mod executor;
mod exit;
#[cfg(feature = "tokio")]
pub mod fiber;
mod foundation;
#[cfg(feature = "tokio")]
mod layer;
mod metric;
#[cfg(feature = "tokio")]
mod mock;
pub mod observability;
#[cfg(feature = "tokio")]
mod policy;
mod pubsub;
pub mod pull;
#[cfg(feature = "tokio")]
mod runtime;
mod schedule;
#[cfg(feature = "tokio")]
mod scope;
mod service;
#[cfg(feature = "stm")]
mod stm;
#[cfg(feature = "tokio")]
mod stream;
pub mod subscription_ref;
#[cfg(feature = "tokio")]
mod supervision;
mod supervisor;
pub mod sync;
mod tagged;
pub mod take;
mod time;
// `Tracer.ts`: the span model the observability port already pointed at.
pub mod tracer;

pub use cause::{
    Cause, CauseError, Defect, Done, FiberId, Reason, Squash, is_die_reason, is_done,
    is_exceeded_capacity_error, is_fail_reason, is_illegal_argument_error, is_interrupt_reason,
    is_no_such_element_error, is_timeout_error, is_unknown_error,
};
pub use config::{Config, ConfigError};
#[cfg(feature = "tokio")]
pub use concurrency::{
    BOUNDED_RECV_TIMEOUT, Body, Concurrency, all, all_discard, all_result, all_result_record,
    all_in, all_with_deadline, find_first, find_first_filter, for_each, for_each_in, head,
    partition, partition_items,
    partition_with_deadline, reduce, validate, validate_discard, validate_discard_items,
    validate_items,
};
pub use context::{Context, ContextReference, Tag};
pub use effect::{Effect, Requirement};
#[cfg(feature = "tokio")]
pub use effect::{Resume, Restore};
#[cfg(feature = "tokio")]
pub use executor::{BoxFuture, Executor, TokioExecutor};
pub use exit::Exit;
#[cfg(feature = "tokio")]
pub use fiber::{Ctx, Fiber, FiberHandle, ForkOpts, Mask, StartMode};
pub use foundation::{
    Combiner, Differ, Equal, Equivalence, Filter, Hash, Matcher, Order, Predicate, Reducer, Tagged,
    absurd, apply, array_equivalence, boolean_equivalence, boolean_order, combine_hash,
    combine_ordering, compose, const_false, const_null, const_true, const_unit, constant, flip,
    flip_curried, hash_f64, hash_str, hole, identity, match_ordering, memoize, memoize_idempotent,
    number_equivalence, number_order, ord_order, reverse_ordering, sk, string_equivalence,
    string_order, tagged, tuple_equivalence, tupled, untupled,
};
#[cfg(feature = "tokio")]
pub use layer::{BuiltServices, Cycle, Layer, LayerMemo, LayerSpanOptions, RequirementIds};
pub use metric::{
    Attributes, Counter, CounterState, Frequency, FrequencyState, Gauge, GaugeState, Histogram,
    HistogramState, MetricRegistry, Metrics, Series, SeriesState, Summary, SummaryState,
    exponential_boundaries, linear_boundaries, timer,
};
#[cfg(feature = "tokio")]
pub use mock::{Mock, Unimplemented, UnimplementedError};
#[cfg(feature = "tokio")]
pub use policy::{BreakerConfig, Bulkhead, RateLimiter, Remote, RemoteError, RemotePolicy, hedge};
#[cfg(feature = "tokio")]
pub use runtime::{
    ManagedRuntime, Runtime, SHUTDOWN_DEADLINE, race, race_all, race_all_first, race_first,
    timeout, timeout_fail,
};
pub use schedule::{
    Decision, LoopOptions, LoopPredicate, Meta, Metadata, Random, RandomError, RandomTag, Schedule,
    Stepper, UpTo, repeat, repeat_with, retry, retry_with,
};
#[cfg(feature = "tokio")]
pub use scope::{Scope, ScopeTag};
pub use service::{Cons, MemoMap, Provide, Reference, ServiceTag, read_reference, use_service};
#[cfg(feature = "stm")]
pub use stm::{
    TxChangeSubscription, TxChunk, TxDeferred, TxDequeue, TxEnqueue, TxHashMap, TxHashSet,
    TxPriorityQueue, TxPubSub, TxQueue, TxReentrantLock, TxRef, TxSemaphore, TxSubscriptionRef,
    atomically, is_tx_chunk, is_tx_deferred, is_tx_hash_map, is_tx_hash_set, is_tx_priority_queue,
    is_tx_semaphore, is_tx_subscription_ref,
};
#[cfg(feature = "tokio")]
pub use stream::{Stream, from_iter, from_iterator_succeed};
#[cfg(feature = "tokio")]
pub use supervision::{ChildSnapshot, Escalation, Supervisor};
pub use sync::{
    Deferred, MutableRef, PartitionedSemaphore, Queue, QueueError, Ref, Semaphore, Strategy,
    SubscriptionRef, SynchronizedRef, is_dequeue, is_enqueue, is_queue,
};
pub use tagged::TaggedError;
pub use time::{
    Clock, ClockImpl, Duration, DurationJson, Parts, SleepOutcome, SystemClock, TestClock,
    is_duration,
};
