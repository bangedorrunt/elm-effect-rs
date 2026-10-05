//! Process access: arguments, environment, and child commands.
//!
//! Thin over [`std::env`] and [`std::process`]: no new semantics,
//! just the ported surface with typed errors.

use std::collections::HashMap;

/// What can go wrong running or reading the process.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessError {
    /// A required environment variable is missing.
    MissingEnv(String),
    /// Spawning the child failed.
    Spawn(String),
    /// The child exited with a non-zero status.
    NonZeroExit(i32),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessError::MissingEnv(name) => write!(f, "missing env var: {name}"),
            ProcessError::Spawn(message) => write!(f, "spawn failed: {message}"),
            ProcessError::NonZeroExit(code) => write!(f, "exit code: {code}"),
        }
    }
}

impl std::error::Error for ProcessError {}

/// The command-line arguments, including the executable name.
pub fn args() -> Vec<String> {
    std::env::args().collect()
}

/// The value of an environment variable, if present.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The value of an environment variable, or a typed missing error.
pub fn require_env(name: &str) -> Result<String, ProcessError> {
    std::env::var(name).map_err(|_| ProcessError::MissingEnv(name.to_string()))
}

/// The whole environment as a map.
pub fn env_all() -> HashMap<String, String> {
    std::env::vars().collect()
}

/// A captured child run: status code, stdout, stderr.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOutput {
    /// The exit status code (None when killed by signal).
    pub code: Option<i32>,
    /// Captured standard output.
    pub stdout: Vec<u8>,
    /// Captured standard error.
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    /// Success means exit code zero.
    pub fn is_success(&self) -> bool {
        self.code == Some(0)
    }

    /// Stdout as UTF-8 text, lossy on invalid sequences.
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Run `program` with `args`, capturing both streams.
pub fn run(program: &str, args: &[&str]) -> Result<CommandOutput, ProcessError> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| ProcessError::Spawn(error.to_string()))?;
    Ok(CommandOutput {
        code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}
