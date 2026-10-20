//! An [`Exit`] is the outcome of running an effect: ordinary data holding a
//! success value or the [`Cause`] of failure. It runs nothing.

use crate::Cause;
use crate::Defect;

/// The outcome of an effect that already ran: success with a value, or failure
/// with a cause. Matching on it performs no I/O and needs no runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit<A, E, D = Defect> {
    /// The effect succeeded with this value.
    Success(A),
    /// The effect failed for this reason.
    Failure(Cause<E, D>),
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

/// Tag-compatible rendering (`Success(1)`, `Failure(Cause([Fail("e")]))`).
/// Values render with `Debug`, matching the tag's inspect for the covered
/// shapes (strings quoted, numbers bare).
impl<A: std::fmt::Debug, E: std::fmt::Debug, D: std::fmt::Display> std::fmt::Display
    for Exit<A, E, D>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exit::Success(value) => write!(f, "Success({value:?})"),
            Exit::Failure(cause) => write!(f, "Failure({cause})"),
        }
    }
}
