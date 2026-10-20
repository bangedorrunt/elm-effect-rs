//! Construction failures, and the adapter contract that reports them.
//!
//! A constructor default (`withConstructorDefault`) is an Effect in the pin, so
//! it can fail with schema issues or die with anything at all, and the two are
//! not interchangeable: every construction adapter may only report an issue. A
//! die is therefore kept as its own variant all the way out, where [`AdapterError`]
//! re-throws it with the adapter's own wording.

use super::{SchemaError, Unknown};

/// Why a constructor default produced no value, and why construction failed.
///
/// The pin's default fails with a `Cause`: a combined cause can hold issues and
/// a defect at once, and the adapter still refuses it (a die is not an issue),
/// which is why a die keeps the issues beside it.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstructError {
    /// A schema issue, and the only failure a construction adapter may report.
    Issue(SchemaError),
    /// A defect (`Effect.die`). No adapter may turn this into an issue or into
    /// `None`; `issues` is whatever rode beside the defect in the same cause.
    Die {
        /// What the default died with.
        defect: Unknown,
        /// Issues from the same combined cause. The adapter does not report
        /// them, but construction still hands them back for inspection.
        issues: Vec<SchemaError>,
    },
}

impl ConstructError {
    /// A plain die with nothing beside it.
    pub fn die(defect: Unknown) -> Self {
        Self::Die {
            defect,
            issues: Vec::new(),
        }
    }

    /// True when a default died rather than failing with a schema issue.
    pub fn is_die(&self) -> bool {
        matches!(self, Self::Die { .. })
    }

    /// The issue, when this failure is one. A die never answers an issue: that
    /// is the whole point of the split.
    pub fn issue(&self) -> Option<&SchemaError> {
        match self {
            Self::Issue(error) => Some(error),
            Self::Die { .. } => None,
        }
    }

    /// Re-root at `prefix`, the way [`SchemaError::at`] does for an issue. A
    /// defect has no path of its own, so only the issues move.
    pub fn at(self, prefix: &str) -> Self {
        match self {
            Self::Issue(error) => Self::Issue(error.at(prefix)),
            Self::Die { defect, issues } => Self::Die {
                defect,
                issues: issues.into_iter().map(|issue| issue.at(prefix)).collect(),
            },
        }
    }

    /// The decode-shaped form of this failure.
    ///
    /// Decoding never runs a constructor default, so it has no die channel and
    /// never sees one; reaching this with a die is a bug in this crate, and it
    /// is reported rather than dropped.
    pub fn expect_issue(self) -> SchemaError {
        match self {
            Self::Issue(error) => error,
            Self::Die { .. } => SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "a schema issue",
                got: "a constructor default's defect on the decode path",
            },
        }
    }
}

impl From<SchemaError> for ConstructError {
    fn from(error: SchemaError) -> Self {
        Self::Issue(error)
    }
}

impl std::fmt::Display for ConstructError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Issue(error) => error.fmt(f),
            Self::Die { defect, .. } => write!(f, "constructor default died with {defect:?}"),
        }
    }
}

impl std::error::Error for ConstructError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.issue()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

/// The struct constructor's message when a default died (`Schema.test.ts:3640`).
pub const CONSTRUCTOR_DIE: &str = "Constructor adapter can only throw schema issues";

/// The option adapter's message when a default died (`Schema.test.ts:3670`).
pub const OPTION_DIE: &str = "Option adapter can only return none for schema issues";

/// What a construction adapter reports.
///
/// `make` and `makeOption` may only report a schema issue, so a die arrives
/// here as the adapter's own contract message with the defect attached.
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterError {
    /// The input failed as a schema issue.
    Issue(SchemaError),
    /// A constructor default died. `message` is the adapter's wording,
    /// `defect` is what the default died with, and `issues` is what rode beside
    /// the defect in the same cause: the adapter cannot report those, but it
    /// hands them back so a caller inspecting the failure still sees them.
    Die {
        /// The adapter's contract message.
        message: &'static str,
        /// What the default died with.
        defect: Unknown,
        /// Issues from the same combined cause, kept as they were.
        issues: Vec<SchemaError>,
    },
}

impl AdapterError {
    /// The issue a refusal carried beside its defect, if the whole cause had
    /// one: the pin's `Cause.findError(error.cause)` on the thrown adapter
    /// error, which is how a caller recovers the schema issue.
    pub fn issue(&self) -> Option<&SchemaError> {
        match self {
            Self::Issue(error) => Some(error),
            Self::Die { issues, .. } => issues.first(),
        }
    }

    /// Report a construction failure the way an adapter must.
    pub fn report(error: ConstructError, die_message: &'static str) -> Self {
        match error {
            ConstructError::Issue(issue) => Self::Issue(issue),
            ConstructError::Die { defect, issues } => Self::Die {
                message: die_message,
                defect,
                issues,
            },
        }
    }
}

impl From<SchemaError> for AdapterError {
    fn from(error: SchemaError) -> Self {
        Self::Issue(error)
    }
}

/// A constructor default that calls another schema's `make` re-throws what that
/// adapter reported: a check failure stays an issue, a die stays a die.
impl From<AdapterError> for ConstructError {
    fn from(error: AdapterError) -> Self {
        match error {
            AdapterError::Issue(issue) => Self::Issue(issue),
            AdapterError::Die { defect, issues, .. } => Self::Die { defect, issues },
        }
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Issue(error) => error.fmt(f),
            Self::Die { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for AdapterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Issue(error) => Some(error as &(dyn std::error::Error + 'static)),
            Self::Die { issues, .. } => issues
                .first()
                .map(|issue| issue as &(dyn std::error::Error + 'static)),
        }
    }
}
