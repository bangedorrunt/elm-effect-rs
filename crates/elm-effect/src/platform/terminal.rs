//! `Terminal` port (`Terminal.ts` at the pin): interactive reads.
//!
//! [`Terminal`] reads lines with an optional prompt; [`QuitError`] marks an
//! explicit user quit (Ctrl-D at an empty script, EOF on a live terminal).
//! [`FakeTerminal`] replays scripted lines so tests never need a TTY.
//!
//! Residuals: raw-mode key events, cursor addressing, and quit-confirm
//! flows (all need a real terminal).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Explicit user quit (EOF / Ctrl-D).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuitError;

impl std::fmt::Display for QuitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "quit by user")
    }
}

/// Interactive line reader.
pub trait Terminal: Send + Sync {
    /// Read one line after showing `prompt`. `Err(QuitError)` on quit.
    fn read_line(&self, prompt: &str) -> Result<String, QuitError>;
    /// Whether input is a terminal.
    fn is_terminal(&self) -> bool;
}

/// Live terminal: prompt to stdout, line from stdin. Same test-only
/// rationale as live stdio: binaries use it, tests use the fake.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct LiveTerminal;

impl Terminal for LiveTerminal {
    fn read_line(&self, prompt: &str) -> Result<String, QuitError> {
        use std::io::{BufRead as _, Write as _};
        let _ = write!(std::io::stdout(), "{prompt}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => Err(QuitError),
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Ok(line)
            }
        }
    }

    fn is_terminal(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }
}

/// Scripted terminal for tests.
#[derive(Clone, Debug, Default)]
pub struct FakeTerminal {
    lines: Arc<Mutex<VecDeque<Result<String, QuitError>>>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl FakeTerminal {
    /// Replay these line outcomes in order.
    pub fn new(lines: Vec<Result<String, QuitError>>) -> Self {
        Self {
            lines: Arc::new(Mutex::new(lines.into())),
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Prompts shown so far.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("fake").clone()
    }
}

impl Terminal for FakeTerminal {
    fn read_line(&self, prompt: &str) -> Result<String, QuitError> {
        self.prompts.lock().expect("fake").push(prompt.to_owned());
        self.lines
            .lock()
            .expect("fake")
            .pop_front()
            .unwrap_or(Err(QuitError))
    }

    fn is_terminal(&self) -> bool {
        false
    }
}
