//! Log verbosity, shared by `Config` and the logger.

/// Log verbosity, ordered from most (`All`) to least (`None`) permissive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Everything passes.
    All,
    /// Fine-grained tracing.
    Trace,
    /// Debugging detail.
    Debug,
    /// Default minimum.
    #[default]
    Info,
    /// Suspicious states.
    Warn,
    /// Failures.
    Error,
    /// Unrecoverable failures.
    Fatal,
    /// Nothing passes.
    None,
}

impl LogLevel {
    /// Parse a level name. `Warning` is accepted as a spelling of `Warn`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "All" => Some(Self::All),
            "Trace" => Some(Self::Trace),
            "Debug" => Some(Self::Debug),
            "Info" => Some(Self::Info),
            "Warn" | "Warning" => Some(Self::Warn),
            "Error" => Some(Self::Error),
            "Fatal" => Some(Self::Fatal),
            "None" => Some(Self::None),
            _ => None,
        }
    }

    /// True when a message at this level passes a `minimum` threshold.
    pub fn passes(self, minimum: Self) -> bool {
        self >= minimum
    }
}
