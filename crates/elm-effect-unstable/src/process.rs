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
    /// The OS refused to run the program (no execute bit, protected file).
    /// Kept apart from [`ProcessError::Spawn`] so callers can tell "could not
    /// run it" from "it is not there".
    PermissionDenied(String),
    /// The child exited with a non-zero status.
    NonZeroExit(i32),
}

/// Classify a start failure: permission problems get their own variant.
fn spawn_error(error: std::io::Error) -> ProcessError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        ProcessError::PermissionDenied(error.to_string())
    } else {
        ProcessError::Spawn(error.to_string())
    }
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessError::MissingEnv(name) => write!(f, "missing env var: {name}"),
            ProcessError::Spawn(message) => write!(f, "spawn failed: {message}"),
            ProcessError::PermissionDenied(message) => write!(f, "permission denied: {message}"),
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

    /// Stderr as UTF-8 text, lossy on invalid sequences.
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A captured child run: status code, stdout, stderr, plus the combined
/// arrival-order interleaving of both streams (TS `.all`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombinedOutput {
    /// The exit status code (None when killed by signal).
    pub code: Option<i32>,
    /// Captured standard output.
    pub stdout: Vec<u8>,
    /// Captured standard error.
    pub stderr: Vec<u8>,
    /// Both streams merged in arrival order.
    pub all: Vec<u8>,
}

impl CombinedOutput {
    /// Success means exit code zero.
    pub fn is_success(&self) -> bool {
        self.code == Some(0)
    }

    /// Stdout as UTF-8 text, lossy on invalid sequences.
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Stderr as UTF-8 text, lossy on invalid sequences.
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Combined output as UTF-8 text, lossy on invalid sequences.
    pub fn all_text(&self) -> String {
        String::from_utf8_lossy(&self.all).into_owned()
    }
}

/// Drain one pipe on a thread, appending each chunk to its own buffer and
/// to the shared arrival-order buffer.
fn spawn_drain(
    pipe: Option<impl std::io::Read + Send + 'static>,
    own: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    all: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) -> std::thread::JoinHandle<()> {
    use std::io::Read;

    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    own.lock().unwrap().extend_from_slice(&chunk[..n]);
                    all.lock().unwrap().extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
    })
}

/// Drain both pipes on threads, appending each chunk to its own buffer and
/// to the shared arrival-order buffer. Threads (not `wait_with_output`) so
/// the combined stream keeps cross-stream arrival order.
fn drain_combined(mut child: std::process::Child) -> Result<CombinedOutput, ProcessError> {
    let stdout_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let all_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let stdout_thread = spawn_drain(
        child.stdout.take(),
        std::sync::Arc::clone(&stdout_buf),
        std::sync::Arc::clone(&all_buf),
    );
    let stderr_thread = spawn_drain(
        child.stderr.take(),
        std::sync::Arc::clone(&stderr_buf),
        std::sync::Arc::clone(&all_buf),
    );
    let status = child
        .wait()
        .map_err(|e| ProcessError::Spawn(e.to_string()))?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    Ok(CombinedOutput {
        code: status.code(),
        stdout: stdout_buf.lock().unwrap().clone(),
        stderr: stderr_buf.lock().unwrap().clone(),
        all: all_buf.lock().unwrap().clone(),
    })
}
/// Run `program` with `args`, capturing both streams.
pub fn run(program: &str, args: &[&str]) -> Result<CommandOutput, ProcessError> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(spawn_error)?;
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
    let program = if words.is_empty() {
        String::new()
    } else {
        words.remove(0)
    };
    CommandSpec {
        program,
        args: words,
        ..CommandSpec::default()
    }
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

    /// Build the OS command: program, args, directory, environment.
    /// Stdio stays at its defaults; callers pick their pipes.
    fn std_command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args);
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
    }

    /// Spawn the child, returning a handle.
    pub fn spawn(&self) -> Result<ChildHandle, ProcessError> {
        let mut command = self.std_command();
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
            .spawn()
            .map(|child| ChildHandle { child: Some(child) })
            .map_err(spawn_error)
    }

    /// Run with `input` as the child's stdin, collecting both streams.
    /// A thread feeds stdin so a large input never deadlocks the capture.
    pub fn run_with_input(&self, input: &[u8]) -> Result<CommandOutput, ProcessError> {
        let mut command = self.std_command();
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().map_err(spawn_error)?;
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = input.to_vec();
            std::thread::spawn(move || {
                let _ = std::io::Write::write_all(&mut stdin, &bytes);
            });
        }
        let output = child
            .wait_with_output()
            .map_err(|e| ProcessError::Spawn(e.to_string()))?;
        Ok(CommandOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
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
        let child = self
            .child
            .take()
            .ok_or_else(|| ProcessError::Spawn("already waited".to_string()))?;
        let output = child
            .wait_with_output()
            .map_err(|e| ProcessError::Spawn(e.to_string()))?;
        Ok(CommandOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Wait for the exit, collecting stdout, stderr, and their arrival-order
    /// interleaving (TS `.all`). Both pipes drain on threads, so neither
    /// stream stalls the other.
    pub fn wait_combined(&mut self) -> Result<CombinedOutput, ProcessError> {
        let child = self
            .child
            .take()
            .ok_or_else(|| ProcessError::Spawn("already waited".to_string()))?;
        drain_combined(child)
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

/// Which stream of the previous stage feeds the next stdin.
/// Mirrors TS `pipeTo(..., { from })`: stdout by default, stderr, or the
/// combined arrival-order interleaving (`all`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PipeFrom {
    /// The previous stdout (the default).
    #[default]
    Stdout,
    /// The previous stderr.
    Stderr,
    /// Both streams merged in arrival order.
    All,
}

/// One pipeline stage plus the stream it reads from its predecessor.
/// The first stage's source is ignored: it inherits stdin.
#[derive(Clone, Debug)]
struct PipeStage {
    spec: CommandSpec,
    from: PipeFrom,
}

/// A chain of commands with the previous output piped into the next stdin.
#[derive(Clone, Debug)]
pub struct Pipeline {
    stages: Vec<PipeStage>,
}

impl Pipeline {
    /// Build from stages (at least one): every edge reads stdout.
    pub fn new(stages: Vec<CommandSpec>) -> Self {
        Self::new_with_sources(
            stages
                .into_iter()
                .map(|spec| (spec, PipeFrom::Stdout))
                .collect(),
        )
    }

    /// Build from stages with an explicit source per edge. The first stage's
    /// source is ignored (it inherits stdin like [`Pipeline::new`]).
    pub fn new_with_sources(stages: Vec<(CommandSpec, PipeFrom)>) -> Self {
        Self {
            stages: stages
                .into_iter()
                .map(|(spec, from)| PipeStage { spec, from })
                .collect(),
        }
    }

    /// True for multi-stage (piped) commands.
    pub fn is_piped(&self) -> bool {
        self.stages.len() > 1
    }

    /// Run the chain: each stage reads the selected stream of the previous
    /// one. Stdout edges stream live pipe-to-pipe; stderr/all edges drain
    /// the previous child first (both pipes, on threads, so neither stalls).
    /// Returns the last stage output (its code and streams).
    pub fn run(&self) -> Result<CommandOutput, ProcessError> {
        if self.stages.is_empty() {
            return Err(ProcessError::Spawn("empty pipeline".to_string()));
        }
        let mut children: Vec<std::process::Child> = Vec::new();
        let mut feed: Option<Vec<u8>> = None;
        for (index, stage) in self.stages.iter().enumerate() {
            let last = index + 1 == self.stages.len();
            let mut command = stage.spec.std_command();
            command
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if index == 0 {
                command.stdin(std::process::Stdio::inherit());
            } else {
                match stage.from {
                    PipeFrom::Stdout => {
                        let stdout = children.last_mut().and_then(|child| child.stdout.take());
                        command.stdin(match stdout {
                            Some(stdout) => stdout.into(),
                            None => std::process::Stdio::inherit(),
                        });
                    }
                    PipeFrom::Stderr | PipeFrom::All => {
                        let previous = children
                            .pop()
                            .ok_or_else(|| ProcessError::Spawn("empty pipeline".to_string()))?;
                        let drained = drain_combined(previous)?;
                        command.stdin(std::process::Stdio::piped());
                        feed = Some(match stage.from {
                            PipeFrom::Stderr => drained.stderr,
                            _ => drained.all,
                        });
                    }
                }
            }
            let mut child = command.spawn().map_err(spawn_error)?;
            if let Some(bytes) = feed.take() {
                if let Some(mut stdin) = child.stdin.take() {
                    std::thread::spawn(move || {
                        let _ = std::io::Write::write_all(&mut stdin, &bytes);
                    });
                }
            }
            if last {
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

// Command guards

/// Either runnable form: a single standard command or a multi-stage
/// pipeline. Mirrors TS `Command = StandardCommand | PipedCommand`.
#[derive(Clone, Debug)]
pub enum Command {
    Standard(CommandSpec),
    Piped(Pipeline),
}

impl Command {
    /// Any command counts: both constructors are commands, and
    /// non-commands are unrepresentable here (the type system rejects
    /// TS's `null` / unknown-tag cases at compile time).
    pub fn is_command(&self) -> bool {
        matches!(self, Command::Standard(_) | Command::Piped(_))
    }

    /// Only single commands (array or template form).
    pub fn is_standard_command(&self) -> bool {
        matches!(self, Command::Standard(_))
    }

    /// Only multi-stage pipelines.
    pub fn is_piped_command(&self) -> bool {
        matches!(self, Command::Piped(_))
    }
}
