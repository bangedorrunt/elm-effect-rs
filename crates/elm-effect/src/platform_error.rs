//! `PlatformError.ts` port: normalized platform failure shapes.
//!
//! Host failures arrive in one vocabulary: [`PlatformError::BadArgument`]
//! for invalid arguments (module, method, description) and
//! [`PlatformError::System`] for OS failures (module, method, path,
//! syscall, errno, message). [`is_platform_error`] is the structural
//! guard; [`bad_argument`] and [`system_error`] build each case.

/// Normalized platform failure.
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformError {
    /// Invalid argument passed to a platform API.
    BadArgument {
        /// Calling module (`FileSystem`).
        module: String,
        /// Calling method (`readFile`).
        method: String,
        /// What was wrong.
        description: String,
    },
    /// Operating-system failure.
    System {
        /// Calling module.
        module: String,
        /// Calling method.
        method: String,
        /// Path involved, if any.
        path: Option<String>,
        /// Syscall name, if known.
        syscall: Option<String>,
        /// Errno name/number, if known.
        errno: Option<String>,
        /// Human message.
        message: String,
    },
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadArgument { module, method, description } => {
                write!(f, "{module}.{method}: bad argument: {description}")
            }
            Self::System { module, method, path, message, .. } => {
                match path {
                    Some(p) => write!(f, "{module}.{method}({p}): {message}"),
                    None => write!(f, "{module}.{method}: {message}"),
                }
            }
        }
    }
}

impl std::error::Error for PlatformError {}

/// Structural guard (`isPlatformError`): everything built here qualifies.
pub fn is_platform_error(_: &PlatformError) -> bool {
    true
}

/// Build a bad-argument error.
pub fn bad_argument(module: &str, method: &str, description: &str) -> PlatformError {
    PlatformError::BadArgument {
        module: module.to_string(),
        method: method.to_string(),
        description: description.to_string(),
    }
}

/// Build a system error from a [`std::io::Error`].
pub fn system_error(module: &str, method: &str, path: Option<&str>, error: &std::io::Error) -> PlatformError {
    PlatformError::System {
        module: module.to_string(),
        method: method.to_string(),
        path: path.map(str::to_string),
        syscall: None,
        errno: error.raw_os_error().map(|n| n.to_string()),
        message: error.to_string(),
    }
}
