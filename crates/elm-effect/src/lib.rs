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

mod basics;
// Byte counts and arbitrary-precision decimals, with their schemas below. The
// rest of the `data/` tree stays test-only; these two are here because the
// `Schema.ByteSize` and `Schema.BigDecimal` families speak for them.
#[path = "data/bigdecimal.rs"]
pub mod bigdecimal;
#[path = "data/byte_size.rs"]
pub mod byte_size;
pub mod cause;
mod channel;
#[cfg(feature = "tokio")]
mod concurrency;
mod config;
mod config_provider;
mod context;
mod crypto;
mod effect;
mod execution_plan;
#[cfg(feature = "tokio")]
mod executor;
mod exit;
#[cfg(feature = "tokio")]
mod fiber;
pub mod filesystem;
mod formatter;
mod foundation;
mod graph;
mod json;
mod json_patch;
mod json_pointer;
mod json_schema;
#[cfg(feature = "tokio")]
mod layer;
mod metric;
mod optic;
mod partitioned_semaphore;
mod path;
mod platform_error;
#[cfg(feature = "tokio")]
mod policy;
pub mod predicate;
mod pubsub;
mod pull;
#[cfg(feature = "tokio")]
mod runtime;
mod schedule;
mod scheduler;
#[cfg(feature = "schema")]
mod schema_cause;
#[cfg(feature = "schema")]
mod schema_data;
#[cfg(feature = "schema")]
mod schema_defect;
#[cfg(feature = "schema")]
mod schema_duration;
#[cfg(feature = "schema")]
mod schema_exit;
#[cfg(feature = "tokio")]
mod scope;
mod service;
#[cfg(feature = "stm")]
mod stm;
#[cfg(feature = "tokio")]
mod stream;
mod subscription_ref;
#[cfg(feature = "tokio")]
mod supervision;
mod supervisor;
mod sync;
mod take;
mod time;
mod utils_gen;

pub use bigdecimal::Decimal;
pub use byte_size::ByteSize;
pub use cause::{
    Cause, CauseError, Defect, Done, FiberId, Reason, Squash, is_die_reason, is_done,
    is_exceeded_capacity_error, is_fail_reason, is_illegal_argument_error, is_interrupt_reason,
    is_no_such_element_error, is_timeout_error, is_unknown_error,
};
#[cfg(feature = "tokio")]
pub use concurrency::{
    BOUNDED_RECV_TIMEOUT, Body, Concurrency, all, all_discard, all_result, all_result_record,
    all_with_deadline, find_first, find_first_filter, for_each, head, partition, partition_items,
    partition_with_deadline, reduce, validate, validate_discard, validate_discard_items,
    validate_items,
};
pub use context::{Context, ContextReference, Tag};
pub use effect::{Effect, Requirement};
#[cfg(feature = "schema")]
pub use elm_effect_schema::{Schema, SchemaError, StructValue, Unknown};
#[cfg(feature = "tokio")]
pub use executor::{BoxFuture, Executor, TokioExecutor};
pub use exit::Exit;
#[cfg(feature = "tokio")]
pub use fiber::{Ctx, Fiber, FiberHandle, ForkOpts, Mask, StartMode};
pub use foundation::{
    Combiner, Differ, Equal, Equivalence, Filter, Hash, Matcher, Order, Predicate, Reducer, Tagged,
    absurd, array_equivalence, boolean_equivalence, boolean_order, combine_hash, combine_ordering,
    compose, const_false, const_null, const_true, constant, flip, flip_curried, hash_f64, hash_str,
    identity, match_ordering, number_equivalence, number_order, ord_order, reverse_ordering,
    string_equivalence, string_order, tagged, tuple_equivalence, tupled, untupled,
};
#[cfg(feature = "tokio")]
pub use layer::{BuiltServices, Cycle, Layer, LayerMemo, RequirementIds};
pub use metric::{
    Attributes, Counter, CounterState, Gauge, GaugeState, MetricRegistry, Series, SeriesState,
};
pub use optic::{
    Iso, Lens, OpticError, Optional, Prism, Traversal, at_map, at_vec, at_vec_exact, failure, none,
    some, success,
};
#[cfg(feature = "tokio")]
pub use policy::{BreakerConfig, Bulkhead, RateLimiter, Remote, RemoteError, RemotePolicy, hedge};
#[cfg(feature = "tokio")]
pub use runtime::{
    ManagedRuntime, Runtime, SHUTDOWN_DEADLINE, race, race_all, race_all_first, race_first,
    timeout, timeout_fail,
};
pub use schedule::{
    LoopOptions, LoopPredicate, Random, RandomTag, Schedule, repeat, repeat_with, retry, retry_with,
};
#[cfg(feature = "schema")]
pub use schema_cause::cause_schema;
#[cfg(feature = "schema")]
pub use schema_data::{
    bigdecimal, bigdecimal_from_string, byte_size, byte_size_from_number, byte_size_from_string,
    is_between_bigdecimal, is_greater_than_bigdecimal, is_greater_than_or_equal_to_bigdecimal,
    is_less_than_bigdecimal, is_less_than_or_equal_to_bigdecimal,
};
#[cfg(feature = "schema")]
pub use schema_defect::defect_schema;
#[cfg(feature = "schema")]
pub use schema_duration::{duration_from_millis, duration_from_string};
#[cfg(feature = "schema")]
pub use schema_exit::exit_schema;
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
pub use time::{
    Clock, ClockImpl, Duration, DurationJson, Parts, SleepOutcome, SystemClock, TestClock,
    is_duration,
};
