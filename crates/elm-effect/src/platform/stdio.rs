//! `Stdio` port (`Stdio.ts` at the pin): standard I/O as a service.
//!
//! I/O devices are services: [`Stdio`] is the trait, [`LiveStdio`] talks to
//! the real descriptors, [`FakeStdio`] records writes and replays canned
//! input lines so tests observe writes without a terminal.
//!
//! Residuals: streaming stdin/stdout/stderr sinks (Stream/Sink machinery),
//! file-descriptor redirection, and `layerTest`-style DI layers.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Standard I/O device contract.
pub trait Stdio: Send + Sync {
    /// Process arguments.
    fn args(&self) -> Vec<String>;
    /// Whether stdin is a terminal.
    fn stdin_is_terminal(&self) -> bool;
    /// Whether stdout is a terminal.
    fn stdout_is_terminal(&self) -> bool;
    /// Write a stdout chunk.
    fn write_stdout(&self, chunk: &str);
    /// Write a stderr chunk.
    fn write_stderr(&self, chunk: &str);
    /// Next stdin line (`None` at EOF).
    fn read_stdin_line(&self) -> Option<String>;
}

/// Live descriptors. Tests use [`FakeStdio`] by design (no terminal in
/// CI); the live device exists for binaries, hence `allow(dead_code)`.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct LiveStdio;

impl Stdio for LiveStdio {
    fn args(&self) -> Vec<String> {
        std::env::args().collect()
    }

    fn stdin_is_terminal(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }

    fn stdout_is_terminal(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdout())
    }

    fn write_stdout(&self, chunk: &str) {
        use std::io::Write as _;
        let _ = write!(std::io::stdout(), "{chunk}");
    }

    fn write_stderr(&self, chunk: &str) {
        use std::io::Write as _;
        let _ = write!(std::io::stderr(), "{chunk}");
    }

    fn read_stdin_line(&self) -> Option<String> {
        use std::io::BufRead as _;
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Some(line)
            }
        }
    }
}

#[derive(Debug, Default)]
struct FakeState {
    stdout: Vec<String>,
    stderr: Vec<String>,
    stdin: VecDeque<String>,
}

/// Fake device: canned input, recorded output.
#[derive(Clone, Debug, Default)]
pub struct FakeStdio {
    state: Arc<Mutex<FakeState>>,
    args: Vec<String>,
    stdin_terminal: bool,
    stdout_terminal: bool,
}

impl FakeStdio {
    /// Fake with canned stdin lines and process args.
    pub fn new(stdin_lines: Vec<String>, args: Vec<String>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                stdout: Vec::new(),
                stderr: Vec::new(),
                stdin: stdin_lines.into(),
            })),
            args,
            stdin_terminal: false,
            stdout_terminal: false,
        }
    }

    /// Override the terminal flags (`layerTest` overrides).
    pub fn with_terminals(mut self, stdin_terminal: bool, stdout_terminal: bool) -> Self {
        self.stdin_terminal = stdin_terminal;
        self.stdout_terminal = stdout_terminal;
        self
    }

    /// Recorded stdout chunks.
    pub fn stdout_chunks(&self) -> Vec<String> {
        self.state.lock().expect("fake").stdout.clone()
    }

    /// Recorded stderr chunks.
    pub fn stderr_chunks(&self) -> Vec<String> {
        self.state.lock().expect("fake").stderr.clone()
    }
}

impl Stdio for FakeStdio {
    fn args(&self) -> Vec<String> {
        self.args.clone()
    }

    fn stdin_is_terminal(&self) -> bool {
        self.stdin_terminal
    }

    fn stdout_is_terminal(&self) -> bool {
        self.stdout_terminal
    }

    fn write_stdout(&self, chunk: &str) {
        self.state
            .lock()
            .expect("fake")
            .stdout
            .push(chunk.to_owned());
    }

    fn write_stderr(&self, chunk: &str) {
        self.state
            .lock()
            .expect("fake")
            .stderr
            .push(chunk.to_owned());
    }

    fn read_stdin_line(&self) -> Option<String> {
        self.state.lock().expect("fake").stdin.pop_front()
    }
}
