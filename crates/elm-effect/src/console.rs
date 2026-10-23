//! `Console` port (`Console.ts` at the pin): leveled output + exit codes.
//!
//! A [`Console`] writes lines to two sinks (stdout lines, stderr lines).
//! Sinks are plain closures, so the module stays standalone: fakes observe
//! writes without a terminal by capturing into shared buffers. Stdout is
//! the success value; logs go to stderr.
//!
//! The `Logger.console*` constructors live here as [`Console`] methods so
//! `observability` stays includable on its own: a logger needs a line sink,
//! and the console is the one host sink this crate ships.
//!
//! Exit-code discipline, documented against `run_main` (`0` success, `1`
//! typed failure, `2` defect, `130` interruption).
//!
//! Residuals: `console` spinners/progress, styled output, and TTY width
//! detection.

use std::sync::{Arc, Mutex};

/// Success.
pub const EXIT_SUCCESS: i32 = 0;
/// Typed failure.
pub const EXIT_FAILURE: i32 = 1;
/// Defect (unexpected throw).
pub const EXIT_DEFECT: i32 = 2;
/// Interruption (128 + SIGINT).
pub const EXIT_INTERRUPTED: i32 = 130;

/// Line sink.
pub type Sink = Box<dyn Fn(&str) + Send + Sync>;

/// Leveled console over two sinks.
pub struct Console {
    out: Sink,
    err: Sink,
}

impl Console {
    /// Build from stdout/stderr sinks.
    pub fn new(out: Sink, err: Sink) -> Self {
        Self { out, err }
    }

    /// Write an stdout line.
    pub fn log(&self, line: &str) {
        (self.out)(line);
    }

    /// Write a stderr line.
    pub fn error(&self, line: &str) {
        (self.err)(line);
    }

    /// Debug line (stderr, like logs).
    pub fn debug(&self, line: &str) {
        (self.err)(line);
    }

    /// `Logger.consoleLogFmt`: a logger writing one logfmt line per record to
    /// this console's stdout.
    pub fn logfmt_logger(self) -> crate::observability::Logger {
        crate::observability::Logger::logfmt(move |line| self.log(line))
    }

    /// `Logger.withConsoleError(formatLogFmt)`: the stderr twin.
    pub fn logfmt_error_logger(self) -> crate::observability::Logger {
        crate::observability::Logger::logfmt(move |line| self.error(line))
    }
}

/// Capturing fake: stdout/stderr lines observed without a terminal.
#[derive(Clone, Debug, Default)]
pub struct FakeConsole {
    out: Arc<Mutex<Vec<String>>>,
    err: Arc<Mutex<Vec<String>>>,
}

impl FakeConsole {
    /// Empty capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// A [`Console`] writing into this fake.
    pub fn console(&self) -> Console {
        let out = self.out.clone();
        let err = self.err.clone();
        Console::new(
            Box::new(move |line| out.lock().expect("fake").push(line.to_owned())),
            Box::new(move |line| err.lock().expect("fake").push(line.to_owned())),
        )
    }

    /// Captured stdout lines.
    pub fn stdout_lines(&self) -> Vec<String> {
        self.out.lock().expect("fake").clone()
    }

    /// Captured stderr lines.
    pub fn stderr_lines(&self) -> Vec<String> {
        self.err.lock().expect("fake").clone()
    }
}
