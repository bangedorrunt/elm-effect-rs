//! An [`Exit`] is the outcome of running an effect: ordinary data holding a
//! success value or the [`Cause`] of failure. It runs nothing.

use crate::Cause;
use crate::Defect;

/// The outcome of an effect that already ran: success with a value, or failure
/// with a cause. Matching on it performs no I/O and needs no runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit<A, E> {
    /// The effect succeeded with this value.
    Success(A),
    /// The effect failed for this reason.
    Failure(Cause<E>),
}

impl<A, E> Exit<A, E> {
    /// A successful outcome.
    pub fn succeed(value: A) -> Self {
        Self::Success(value)
    }

    /// A typed-failure outcome.
    pub fn fail(error: E) -> Self {
        Self::Failure(Cause::fail(error))
    }

    /// A defect outcome. Never a typed error value.
    pub fn die(defect: Defect) -> Self {
        Self::Failure(Cause::die(defect))
    }

    /// True only for [`Exit::Success`].
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }
}
