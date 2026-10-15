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

pub mod cause;
#[cfg(feature = "tokio")]
mod concurrency;
mod context;
mod effect;
#[cfg(feature = "tokio")]
mod executor;
mod exit;
#[cfg(feature = "tokio")]
mod fiber;
mod foundation;
#[cfg(feature = "tokio")]
mod layer;
mod metric;
mod optic;
#[cfg(feature = "tokio")]
mod policy;
pub mod predicate;
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
mod supervisor;
pub use supervisor::{catch_panic, catch_panic_message};
mod sync;
mod time;

pub use cause::{
    is_exceeded_capacity_error, is_illegal_argument_error, is_no_such_element_error,
    is_timeout_error, is_unknown_error, Cause, CauseError, Defect, Done, FiberId, Reason, Squash,
    is_die_reason, is_fail_reason, is_interrupt_reason, is_done,
};
#[cfg(feature = "tokio")]
pub use concurrency::{
    Body, Concurrency, all, find_first, find_first_filter, for_each, partition, partition_items,
    reduce, validate, validate_discard, validate_discard_items, validate_items,
};
pub use context::{Context, ContextReference, Tag};
pub use effect::{Effect, Requirement};
pub use metric::{
    Attributes, Counter, CounterState, Gauge, GaugeState, MetricRegistry, Series, SeriesState,
};
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
pub use optic::{
    Iso, Lens, OpticError, Optional, Prism, Traversal, at_map, at_vec, at_vec_exact, failure, none,
    some, success,
};
#[cfg(feature = "tokio")]
pub use policy::{BreakerConfig, Bulkhead, RateLimiter, Remote, RemoteError, RemotePolicy, hedge};
#[cfg(feature = "tokio")]
pub use runtime::{ManagedRuntime, Runtime, race, race_all, race_all_first, race_first, timeout};
pub use schedule::{Random, RandomTag, Schedule, repeat, retry};
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
pub use sync::{Deferred, MutableRef, PartitionedSemaphore, Queue, QueueError, Ref, Semaphore, Strategy, SubscriptionRef, SynchronizedRef, is_dequeue, is_enqueue, is_queue};
pub use time::{Clock, ClockImpl, Duration, DurationJson, Parts, SleepOutcome, SystemClock, TestClock, is_duration};
