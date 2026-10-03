//! Law L2: causes classify every failure as [`Reason::Fail`], [`Reason::Die`],
//! or [`Reason::Interrupt`]. A cause is a vector of reasons: no `Empty`,
//! `Sequential`, or `Parallel` variants, and [`Cause::empty`] is an empty
//! vector, not a special case.

use std::backtrace::Backtrace;

/// Identity of the fiber that was interrupted, when known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FiberId(pub u64);

/// An unexpected, untyped failure: a panic, a broken invariant, an I/O error
/// nobody modeled. Reportable via [`std::error::Error`], but never a typed
/// error value `E`: the types cannot collapse.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Defect {
    /// What went wrong, in one line.
    pub message: String,
    /// Captured [`Backtrace`] rendered to text at construction. Stored as a
    /// string (not `Backtrace`) so causes keep structural equality.
    pub backtrace: String,
}

impl Defect {
    /// Capture the current backtrace alongside `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            backtrace: Backtrace::capture().to_string(),
        }
    }
}

/// One failure reason. A typed error, a defect, or an interruption: the three
/// never share a constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason<E> {
    /// An expected, typed error value.
    Fail(E),
    /// An unexpected defect. Never an `E`.
    Die(Defect),
    /// The fiber was interrupted. Carries the interrupting fiber when known.
    Interrupt { fiber_id: Option<FiberId> },
}

/// Why an effect failed: a vector of [`Reason`]. Ordinary data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cause<E> {
    /// Every reason, in the order recorded. Empty means no failure recorded.
    pub reasons: Vec<Reason<E>>,
}

impl<E> Cause<E> {
    /// A cause with no reasons. There is no `Empty` variant to confuse with
    /// "failed with nothing".
    pub fn empty() -> Self {
        Self {
            reasons: Vec::new(),
        }
    }

    /// A single typed failure.
    pub fn fail(error: E) -> Self {
        Self {
            reasons: vec![Reason::Fail(error)],
        }
    }

    /// A single defect. This never counts as a typed error value.
    pub fn die(defect: Defect) -> Self {
        Self {
            reasons: vec![Reason::Die(defect)],
        }
    }

    /// A single interruption, naming the interrupter when known.
    pub fn interrupt(fiber_id: Option<FiberId>) -> Self {
        Self {
            reasons: vec![Reason::Interrupt { fiber_id }],
        }
    }

    /// The first typed error, if any reason is a `Fail`.
    pub fn failure(&self) -> Option<&E> {
        self.reasons.iter().find_map(|r| match r {
            Reason::Fail(e) => Some(e),
            _ => None,
        })
    }

    /// Every defect carried by this cause. Empty for pure typed failures.
    pub fn defects(&self) -> Vec<&Defect> {
        self.reasons
            .iter()
            .filter_map(|r| match r {
                Reason::Die(d) => Some(d),
                _ => None,
            })
            .collect()
    }
}

impl<E> Default for Cause<E> {
    fn default() -> Self {
        Self::empty()
    }
}
