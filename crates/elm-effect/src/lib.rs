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

pub mod basics;
// The `data/` values a schema family speaks for: byte counts and arbitrary
// precision decimals, the chunk sequence and the regexp pattern. The rest of
// the `data/` tree stays test-only.
#[path = "data/bigdecimal.rs"]
pub mod bigdecimal;
#[path = "data/byte_size.rs"]
pub mod byte_size;
#[path = "data/chunk.rs"]
pub mod chunk;
// `Chunk::take_nonempty` answers a `NonEmpty`, so the chunk module reaches it
// as `super::nonempty` and it has to sit beside `chunk` at the crate root.
#[path = "data/nonempty.rs"]
pub mod nonempty;
#[path = "data/regexp.rs"]
pub mod regexp;
pub mod cause;
pub mod channel;
// `Logger.console*` writes through this port, so it is part of the surface.
pub mod console;
// `ChannelSchema` only exists for a schema-encoded boundary, so the feature that
// brings the schema crate in is the one that carries it.
#[cfg(feature = "schema")]
pub mod channel_schema;
#[cfg(feature = "tokio")]
mod concurrency;
mod config;
pub mod config_provider;
mod context;
mod crypto;
// `Schema.Date` speaks for `DateTime`, so the module is part of the surface
// rather than only the test targets that include it by path.
pub mod datetime;
pub mod effect;
pub mod execution_plan;
#[cfg(feature = "tokio")]
mod executor;
mod exit;
#[cfg(feature = "tokio")]
pub mod fiber;
// `Schema.File` speaks for this value, so it is part of the surface.
pub mod file;
pub mod filesystem;
mod formatter;
mod foundation;
// `Schema.Graph` speaks for this type, so the module is part of the surface
// rather than only the test targets that include it by path.
pub mod graph;
mod json;
pub mod json_patch;
mod json_pointer;
mod json_schema;
#[cfg(feature = "tokio")]
mod layer;
mod metric;
#[cfg(feature = "tokio")]
mod mock;
mod optic;
pub mod observability;
// `observability/OtlpTracer.ts`: the tracer's `layer` builds on `Layer`.
#[cfg(feature = "tokio")]
pub mod otlp_tracer;
mod partitioned_semaphore;
pub mod path;
pub mod platform_error;
#[cfg(feature = "tokio")]
mod policy;
pub mod predicate;
mod pubsub;
pub mod request;
pub mod pull;
#[cfg(feature = "tokio")]
mod runtime;
mod schedule;
pub mod scheduler;
#[cfg(feature = "schema")]
mod schema_adapter;
#[cfg(feature = "schema")]
mod schema_cause;
#[cfg(feature = "schema")]
mod schema_chunk;
#[cfg(feature = "schema")]
mod schema_data;
#[cfg(feature = "schema")]
mod schema_datetime;
#[cfg(feature = "schema")]
mod schema_declarations;
#[cfg(feature = "schema")]
mod schema_defect;
#[cfg(feature = "schema")]
mod schema_graph;
#[cfg(feature = "schema")]
mod schema_duration;
#[cfg(feature = "schema")]
mod schema_exit;
#[cfg(feature = "schema")]
mod schema_file;
#[cfg(feature = "schema")]
mod schema_parser;
#[cfg(feature = "schema")]
mod schema_regexp;
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
pub mod utils_gen;

pub use bigdecimal::Decimal;
pub use byte_size::ByteSize;
pub use chunk::Chunk;
pub use cause::{
    Cause, CauseError, Defect, Done, FiberId, Reason, Squash, is_die_reason, is_done,
    is_exceeded_capacity_error, is_fail_reason, is_illegal_argument_error, is_interrupt_reason,
    is_no_such_element_error, is_timeout_error, is_unknown_error,
};
#[cfg(feature = "tokio")]
pub use concurrency::{
    BOUNDED_RECV_TIMEOUT, Body, Concurrency, all, all_discard, all_result, all_result_record,
    all_in, all_with_deadline, find_first, find_first_filter, for_each, for_each_in, head,
    partition, partition_items,
    partition_with_deadline, reduce, validate, validate_discard, validate_discard_items,
    validate_items,
};
pub use context::{Context, ContextReference, Tag};
pub use datetime::DateTime;
pub use effect::{Effect, Requirement};
pub use graph::{Graph, Kind};
#[cfg(feature = "tokio")]
pub use effect::{Resume, Restore};
#[cfg(feature = "schema")]
pub use elm_effect_schema::{Schema, SchemaError, StructValue, Unknown};
#[cfg(feature = "tokio")]
pub use executor::{BoxFuture, Executor, TokioExecutor};
pub use exit::Exit;
pub use file::File;
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
pub use layer::{BuiltServices, Cycle, Layer, LayerMemo, LayerSpanOptions, RequirementIds};
pub use metric::{
    Attributes, Counter, CounterState, Frequency, FrequencyState, Gauge, GaugeState, Histogram,
    HistogramState, MetricRegistry, Series, SeriesState, Summary, SummaryState, linear_boundaries,
};
#[cfg(feature = "tokio")]
pub use mock::{Mock, Unimplemented, UnimplementedError};
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
    LoopOptions, LoopPredicate, Random, RandomError, RandomTag, Schedule, repeat, repeat_with,
    retry, retry_with,
};
#[cfg(feature = "schema")]
pub use schema_adapter::{
    AdapterFailure, AdapterRefusal, OPTION_MESSAGE, PROMISE_MESSAGE, RESULT_MESSAGE, SYNC_MESSAGE,
};
#[cfg(feature = "schema")]
pub use schema_cause::{
    CauseReasonSchema, CauseSchema, cause_reason_schema, cause_schema,
};
#[cfg(feature = "schema")]
pub use schema_data::{
    bigdecimal, bigdecimal_from_string, byte_size, byte_size_from_number, byte_size_from_string,
    is_between_bigdecimal, is_greater_than_bigdecimal, is_greater_than_or_equal_to_bigdecimal,
    is_less_than_bigdecimal, is_less_than_or_equal_to_bigdecimal,
};
#[cfg(feature = "schema")]
pub use schema_chunk::chunk;
#[cfg(feature = "schema")]
pub use schema_datetime::date;
#[cfg(feature = "schema")]
pub use schema_declarations::declaration_revivers;
#[cfg(feature = "schema")]
pub use schema_defect::defect_schema;
#[cfg(feature = "schema")]
pub use schema_duration::{duration_from_millis, duration_from_string};
#[cfg(feature = "schema")]
pub use schema_exit::{ExitSchema, exit_schema};
#[cfg(feature = "schema")]
pub use schema_file::file;
#[cfg(feature = "schema")]
pub use schema_graph::graph;
#[cfg(feature = "schema")]
pub use schema_parser::{ASYNC_DEFECT, decode_unknown_exit, decode_unknown_result};
#[cfg(feature = "schema")]
pub use schema_regexp::regexp;
pub use regexp::RegExp;
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
