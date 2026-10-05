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

// Command builders and handles

/// A child command: program plus arguments, directory, and environment.
#[derive(Clone, Debug, Default)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    cwd: Option<std::path::PathBuf>,
    envs: Vec<(String, String)>,
    clear_env: bool,
}

/// Split a template into words: whitespace separates, single/double quotes
/// group, backslash escapes the next character (astral escapes survive as
/// ordinary characters).
pub fn split_template(template: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = template.chars().peekable();
    let mut in_word = false;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            in_word = true;
        } else if c == '\\' {
            if let Some(next) = chars.next() {
                current.push(next);
            }
            in_word = true;
        } else if c.is_whitespace() {
            if in_word {
                words.push(std::mem::take(&mut current));
                in_word = false;
            }
        } else {
            current.push(c);
            in_word = true;
        }
    }
    if in_word {
        words.push(current);
    }
    words
}

/// Build from a template: the first word is the program, the rest are args.
pub fn command_template(template: &str) -> CommandSpec {
    let mut words = split_template(template);
    let program = if words.is_empty() { String::new() } else { words.remove(0) };
    CommandSpec { program, args: words, ..CommandSpec::default() }
}

/// Build from a program plus an argument array.
pub fn command_array(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
        ..CommandSpec::default()
    }
}

impl CommandSpec {
    /// The program.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Set the working directory.
    pub fn cwd(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Add an environment variable.
    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.envs.push((name.to_string(), value.to_string()));
        self
    }

    /// Start with an empty environment.
    pub fn clear_env(mut self) -> Self {
        self.clear_env = true;
        self
    }

    /// Spawn the child, returning a handle.
    pub fn spawn(&self) -> Result<ChildHandle, ProcessError> {
        let mut command = std::process::Command::new(&self.program);
        command
            .args(&self.args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(dir) = &self.cwd {
            command.current_dir(dir);
        }
        if self.clear_env {
            command.env_clear();
        }
        for (name, value) in &self.envs {
            command.env(name, value);
        }
        command
            .spawn()
            .map(|child| ChildHandle { child: Some(child) })
            .map_err(|e| ProcessError::Spawn(e.to_string()))
    }
}

/// A running child: wait for the exit, read the streams, or kill it.
pub struct ChildHandle {
    child: Option<std::process::Child>,
}

impl ChildHandle {
    /// The OS process id, if still running.
    pub fn id(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// Wait for the exit, collecting stdout and stderr.
    pub fn wait(&mut self) -> Result<CommandOutput, ProcessError> {
        let child = self.child.take().ok_or_else(|| ProcessError::Spawn("already waited".to_string()))?;
        let output = child
            .wait_with_output()
            .map_err(|e| ProcessError::Spawn(e.to_string()))?;
        Ok(CommandOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Kill the child.
    pub fn kill(&mut self) -> Result<(), ProcessError> {
        match self.child.as_mut() {
            Some(child) => child.kill().map_err(|e| ProcessError::Spawn(e.to_string())),
            None => Ok(()),
        }
    }
}

// Pipelines

/// A chain of commands with stdout piped into the next stdin.
#[derive(Clone, Debug)]
pub struct Pipeline {
    stages: Vec<CommandSpec>,
}

impl Pipeline {
    /// Build from stages (at least one).
    pub fn new(stages: Vec<CommandSpec>) -> Self {
        Self { stages }
    }

    /// True for multi-stage (piped) commands.
    pub fn is_piped(&self) -> bool {
        self.stages.len() > 1
    }

    /// Run the chain: each stage reads the previous stdout. Returns the
    /// last stage output (its code and streams).
    pub fn run(&self) -> Result<CommandOutput, ProcessError> {
        if self.stages.is_empty() {
            return Err(ProcessError::Spawn("empty pipeline".to_string()));
        }
        let mut previous: Option<std::process::ChildStdout> = None;
        let mut children: Vec<std::process::Child> = Vec::new();
        for (index, stage) in self.stages.iter().enumerate() {
            let last = index + 1 == self.stages.len();
            let mut command = std::process::Command::new(stage.program());
            command
                .args(stage.args())
                .stdin(match previous.take() {
                    Some(stdout) => stdout.into(),
                    None => std::process::Stdio::inherit(),
                })
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if let Some(dir) = &stage.cwd {
                command.current_dir(dir);
            }
            if stage.clear_env {
                command.env_clear();
            }
            for (name, value) in &stage.envs {
                command.env(name, value);
            }
            let mut child = command.spawn().map_err(|e| ProcessError::Spawn(e.to_string()))?;
            if !last {
                previous = child.stdout.take();
            } else {
                let output = child
                    .wait_with_output()
                    .map_err(|e| ProcessError::Spawn(e.to_string()))?;
                for mut earlier in children.drain(..) {
                    let _: Result<_, _> = earlier.wait();
                }
                return Ok(CommandOutput {
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                });
            }
            children.push(child);
        }
        Err(ProcessError::Spawn("empty pipeline".to_string()))
    }
}

// File-descriptor names

/// Parse an extra fd name (`"fd3"` and up) to its number. The standard
/// streams (`stdin`/`stdout`/`stderr`, `fd0`–`fd2`) and anything else
/// parse to `None`: only *additional* fds have names.
pub fn parse_fd_name(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("fd")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u32 = digits.parse().ok()?;
    (number >= 3).then_some(number)
}

/// Render an fd number back to its name.
pub fn fd_name(number: u32) -> String {
    format!("fd{number}")
}
