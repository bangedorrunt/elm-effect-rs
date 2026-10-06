//! Observability port at `effect@4.0.0`: log levels, redacted secrets,
//! loggers, metrics, tracers, and tap.
//!
//! Logging is an effect ([`log_info`] and friends build descriptions over
//! the [`LoggerTag`] service); secrets stay redacted at the type level
//! ([`Redacted`] hides bytes in `Debug` and in span fields). Spans wrap
//! effects ([`with_span`]) and record in-process: no socket, no exporter.
//! Fiber-local references (`CurrentLogLevel` and friends) are explicit
//! config values here, not implicit context.

use std::sync::{Arc, Mutex};

use elm_effect::{Cause, Effect, Exit, Requirement, ServiceTag};

// LogLevel

/// Log verbosity, ordered from most (`All`) to least (`None`) permissive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Everything passes.
    All,
    /// Fine-grained tracing.
    Trace,
    /// Debugging detail.
    Debug,
    /// Notable events.
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

/// True when a message at `level` passes a gate at `minimum`.
pub fn is_enabled(level: LogLevel, minimum: LogLevel) -> bool {
    level >= minimum
}

// Redacted

/// A secret value: [`Debug`] (and therefore span fields and logs) never
/// shows the bytes. Read with [`value`](Redacted::value); drop to wipe.
pub struct Redacted<T> {
    value: T,
}

impl<T> Redacted<T> {
    /// Wrap a secret. Cloning is explicit: [`value`](Redacted::value)
    /// borrows, so copies only happen on purpose.
    pub fn make(value: T) -> Self {
        Self { value }
    }

    /// Borrow the secret. The only way to read it.
    pub fn value(&self) -> &T {
        &self.value
    }
}

impl<T> std::fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T: Clone> Clone for Redacted<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

/// Values with a custom redacted representation.
pub trait Redactable {
    /// The redacted rendering (never the secret bytes).
    fn redact(&self) -> String;
}

impl<T: std::fmt::Debug> Redactable for Redacted<T> {
    fn redact(&self) -> String {
        format!("{:?}", self)
    }
}

// Logger

/// A log sink function.
type LogFn = Arc<dyn Fn(LogLevel, &str, &[(String, String)]) + Send + Sync>;

/// A log sink: level, message, and annotations.
pub struct Logger {
    log: LogFn,
}

impl Logger {
    /// Build from a sink function.
    pub fn new(log: impl Fn(LogLevel, &str, &[(String, String)]) + Send + Sync + 'static) -> Self {
        Self { log: Arc::new(log) }
    }

    /// Emit one record.
    pub fn log(&self, level: LogLevel, message: &str, annotations: &[(String, String)]) {
        (self.log)(level, message, annotations);
    }
}

impl Clone for Logger {
    fn clone(&self) -> Self {
        Self {
            log: Arc::clone(&self.log),
        }
    }
}

/// The `Logger` service tag.
pub struct LoggerTag;

impl ServiceTag for LoggerTag {
    type Service = Logger;
    const ID: &'static str = "elm-effect/logger";
}

/// Log an event as an effect: resolves the [`LoggerTag`] service, then emits.
/// Logging itself never fails; building the description runs nothing.
pub fn log_info(message: String) -> Effect<(), std::convert::Infallible, LoggerTag> {
    elm_effect::use_service::<LoggerTag>().flat_map(move |logger| {
        Effect::sync(move || {
            logger.log(LogLevel::Info, &message, &[]);
        })
    })
}

// Metric

/// A monotonically increasing counter.
#[derive(Debug)]
pub struct Counter {
    name: String,
    count: std::sync::atomic::AtomicU64,
}

impl Counter {
    /// A counter with this name, starting at zero.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Add one.
    pub fn inc(&self) {
        self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Add `n`.
    pub fn inc_by(&self, n: u64) {
        self.count.fetch_add(n, std::sync::atomic::Ordering::SeqCst);
    }

    /// The current count.
    pub fn get(&self) -> u64 {
        self.count.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A settable gauge.
#[derive(Debug)]
pub struct Gauge {
    name: String,
    value: Mutex<i64>,
}

impl Gauge {
    /// A gauge with this name, starting at zero.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: Mutex::new(0),
        }
    }

    /// Set the value.
    pub fn set(&self, value: i64) {
        *self.value.lock().unwrap() = value;
    }

    /// Read the value.
    pub fn get(&self) -> i64 {
        *self.value.lock().unwrap()
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A named metric registry backing the [`MetricTag`] service.
#[derive(Clone, Debug, Default)]
pub struct MetricRegistry {
    counters: Arc<Mutex<std::collections::HashMap<String, Arc<Counter>>>>,
}

impl MetricRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The named counter, creating it on first use.
    pub fn counter(&self, name: &str) -> Arc<Counter> {
        let mut guard = self.counters.lock().unwrap();
        guard
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Counter::new(name)))
            .clone()
    }
}

/// The `Metric` service tag.
pub struct MetricTag;

impl ServiceTag for MetricTag {
    type Service = MetricRegistry;
    const ID: &'static str = "elm-effect/metric";
}

// Tracer

/// One recorded span: name plus pre-rendered attribute pairs. Secrets must
/// be rendered with `{:?}` (never `{}` on raw bytes) before they get here:
/// [`Redacted`] then shows `<redacted>` and the bytes stay out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanRecord {
    /// The span name.
    pub name: String,
    /// Attribute pairs.
    pub attributes: Vec<(String, String)>,
}

/// An in-process span recorder. No socket, no exporter: spans accumulate
/// for tests and debugging.
#[derive(Clone, Debug, Default)]
pub struct Tracer {
    spans: Arc<Mutex<Vec<SpanRecord>>>,
}

impl Tracer {
    /// An empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spans recorded so far.
    pub fn spans(&self) -> Vec<SpanRecord> {
        self.spans.lock().unwrap().clone()
    }

    /// Record one span directly.
    pub fn record(&self, name: &str, attributes: Vec<(String, String)>) {
        self.spans.lock().unwrap().push(SpanRecord {
            name: name.to_string(),
            attributes,
        });
    }
}

/// The `Tracer` service tag.
pub struct TracerTag;

impl ServiceTag for TracerTag {
    type Service = Tracer;
    const ID: &'static str = "elm-effect/tracer";
}

/// Wrap an effect in a span: records name and attributes, then runs the
/// effect unchanged (success and failure both pass through).
pub fn with_span<A, E, R>(
    tracer: &Tracer,
    name: &str,
    attributes: Vec<(String, String)>,
    effect: Effect<A, E, R>,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    let tracer = tracer.clone();
    let record = SpanRecord {
        name: name.to_string(),
        attributes,
    };
    Effect::sync(move || {
        tracer.record(&record.name, record.attributes.clone());
    })
    .flat_map(|_| effect)
}

/// Report an error where operators cannot fail: defects and notes for the
/// telemetry sink.
pub struct ErrorReporter {
    report: Arc<dyn Fn(&str) + Send + Sync>,
}

impl ErrorReporter {
    /// Build from a sink function.
    pub fn new(report: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            report: Arc::new(report),
        }
    }

    /// Report one note.
    pub fn report(&self, note: &str) {
        (self.report)(note);
    }
}

impl Clone for ErrorReporter {
    fn clone(&self) -> Self {
        Self {
            report: Arc::clone(&self.report),
        }
    }
}

/// The `ErrorReporter` service tag.
pub struct ErrorReporterTag;

impl ServiceTag for ErrorReporterTag {
    type Service = ErrorReporter;
    const ID: &'static str = "elm-effect/error-reporter";
}

// References (explicit config, not fiber-locals)

/// Current annotations attached to every log record.
#[derive(Clone, Debug, Default)]
pub struct LogAnnotations(pub Vec<(String, String)>);

/// Minimum level that passes.
#[derive(Clone, Copy, Debug)]
pub struct MinimumLogLevel(pub LogLevel);

// tap

/// Observe a success without changing it: runs `f` on the value and passes
/// the original result and any failure through untouched.
pub fn tap<A, E, R, F>(effect: Effect<A, E, R>, f: F) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
    F: FnOnce(&A) + Send + 'static,
{
    effect.map(|a| {
        f(&a);
        a
    })
}

// Ignore logging

/// How a swallowed cause is logged by [`ignore`] and [`ignore_cause`].
/// Mirrors the `log` option of TS `Effect.ignore` / `Effect.ignoreCause`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IgnoreLog {
    /// Never log (`log: false` / omitted).
    #[default]
    No,
    /// Log at the ambient current level (`log: true`).
    Current,
    /// Log at the provided level, overriding current (`log: "Error"`).
    Level(LogLevel),
}

/// Log options for [`ignore`] / [`ignore_cause`]: level selection plus an
/// optional prepend message (`message` replaces the rendered cause).
#[derive(Clone, Debug, Default)]
pub struct IgnoreOptions {
    pub log: IgnoreLog,
    pub message: Option<String>,
}

fn log_ignored<E: std::fmt::Debug>(
    cause: &Cause<E>,
    options: &IgnoreOptions,
    current: LogLevel,
    minimum: LogLevel,
    logger: &Logger,
) {
    let level = match options.log {
        IgnoreLog::No => return,
        IgnoreLog::Current => current,
        IgnoreLog::Level(level) => level,
    };
    if !is_enabled(level, minimum) {
        return;
    }
    let body = match &options.message {
        Some(message) => message.clone(),
        None => format!("{cause:?}"),
    };
    logger.log(level, &body, &[]);
}

/// Swallow every cause of `effect`, succeeding with `()`: typed failures,
/// defects, and interrupts all become success. When `options.log` selects a
/// level, the swallowed cause is logged (gated by `minimum`); an explicit
/// `message` replaces the rendered cause. Levels are explicit config values
/// here, not ambient context.
pub fn ignore<A, E, R>(
    effect: Effect<A, E, R>,
    options: IgnoreOptions,
    current: LogLevel,
    minimum: LogLevel,
    logger: &Logger,
) -> Effect<(), std::convert::Infallible, R>
where
    A: Send + 'static,
    E: std::fmt::Debug + Send + 'static,
    R: Requirement,
{
    let logger = logger.clone();
    effect.exit().map(move |exit| {
        if let Exit::Failure(cause) = exit {
            log_ignored(&cause, &options, current, minimum, &logger);
        }
    })
}

/// Cause-level twin of [`ignore`]: same swallow-everything plus log
/// semantics, for pipelines that already think in causes.
pub fn ignore_cause<A, E, R>(
    effect: Effect<A, E, R>,
    options: IgnoreOptions,
    current: LogLevel,
    minimum: LogLevel,
    logger: &Logger,
) -> Effect<(), std::convert::Infallible, R>
where
    A: Send + 'static,
    E: std::fmt::Debug + Send + 'static,
    R: Requirement,
{
    ignore(effect, options, current, minimum, logger)
}

// Log line formatters

/// Quote a logfmt value only when it needs it: values with whitespace
/// or quotes (or empty) render `"..."` with inner quotes escaped;
/// anything else renders bare, so strings are never double-quoted.
pub fn quote_logfmt_value(value: &str) -> String {
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// One `key=value` logfmt field.
pub fn format_logfmt_field(key: &str, value: &str) -> String {
    format!("{}={}", key, quote_logfmt_value(value))
}

/// A logfmt line: level, message, then annotations (TS `formatLogFmt`;
/// only the quoting contract is pinned, layouts stay minimal).
pub fn format_logfmt(level: LogLevel, message: &str, annotations: &[(String, String)]) -> String {
    let mut parts = vec![
        format!("level={:?}", level),
        format!("message={}", quote_logfmt_value(message)),
    ];
    parts.extend(
        annotations
            .iter()
            .map(|(key, value)| format_logfmt_field(key, value)),
    );
    parts.join(" ")
}

/// A simple human line: message then annotations (TS `formatSimple`;
/// same quoting contract as [`format_logfmt`]).
pub fn format_simple(message: &str, annotations: &[(String, String)]) -> String {
    let mut parts = vec![format!("message={}", quote_logfmt_value(message))];
    parts.extend(
        annotations
            .iter()
            .map(|(key, value)| format_logfmt_field(key, value)),
    );
    parts.join(" ")
}
