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
mod exit;

pub use cause::{Cause, Defect, FiberId, Reason};
pub use exit::Exit;
