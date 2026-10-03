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
mod effect;
#[cfg(feature = "tokio")]
mod executor;
mod exit;
#[cfg(feature = "tokio")]
mod fiber;
#[cfg(feature = "tokio")]
mod layer;
#[cfg(feature = "tokio")]
mod runtime;
#[cfg(feature = "tokio")]
mod scope;
mod service;
#[cfg(feature = "tokio")]
mod supervisor;
mod sync;

pub use cause::{Cause, Defect, FiberId, Reason};
pub use effect::{Effect, Requirement};
#[cfg(feature = "tokio")]
pub use executor::{BoxFuture, Executor, TokioExecutor};
pub use exit::Exit;
#[cfg(feature = "tokio")]
pub use fiber::{Ctx, FiberHandle, ForkOpts, Mask, StartMode};
#[cfg(feature = "tokio")]
pub use layer::{BuiltServices, Cycle, Layer, RequirementIds};
#[cfg(feature = "tokio")]
pub use runtime::{ManagedRuntime, Runtime};
#[cfg(feature = "tokio")]
pub use scope::Scope;
pub use service::{Cons, MemoMap, Provide, Reference, ServiceTag, read_reference, use_service};
pub use sync::{Deferred, MutableRef, Queue, Ref, Semaphore, SynchronizedRef};
