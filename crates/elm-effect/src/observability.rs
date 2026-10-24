//! Observability: log levels, redacted secrets,
//! loggers, metrics, tracers, and tap.
//!
//! Logging is an effect ([`log_info`] and friends build descriptions over
//! the [`LoggerTag`] service); secrets stay redacted at the type level
//! ([`Redacted`] hides bytes in `Debug` and in span fields). Spans wrap
//! effects ([`record_span`]) and record in-process: no socket, no exporter.
//! Fiber-local references (`CurrentLogLevel` and friends) are explicit
//! config values here, not implicit context.

use std::sync::{Arc, Mutex};

use crate::{Cause, Effect, Exit, Requirement, ServiceTag};

// LogLevel

pub use crate::log_level::LogLevel;

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

/// Default stringification never shows the bytes (mirrors `toString`).
impl<T> std::fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Equality compares protected values (mirrors `Equal`).
impl<T: PartialEq> PartialEq for Redacted<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Eq> Eq for Redacted<T> {}

/// Hash derives from the protected value (mirrors `Hash`).
impl<T: std::hash::Hash> std::hash::Hash for Redacted<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
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

impl Logger {
    /// `withConsoleLog(format)` in Effect 4: render one line per record and
    /// hand it to `sink`. Effect 4's formatter sees the whole log call; this
    /// crate's sink receives the joined message, so the formatter takes
    /// `(level, message, annotations)`. [`Console::logfmt_logger`] is the
    /// console-backed constructor built on this.
    ///
    /// [`Console::logfmt_logger`]: crate::console::Console::logfmt_logger
    pub fn with_line_sink<F, S>(sink: S, format: F) -> Self
    where
        S: Fn(&str) + Send + Sync + 'static,
        F: Fn(LogLevel, &str, &[(String, String)]) -> String + Send + Sync + 'static,
    {
        Self::new(move |level, message, annotations| {
            sink(&format(level, message, annotations));
        })
    }

    /// A logfmt line per record, through `sink` (Effect 4's
    /// `withConsoleLog(formatLogFmt)` shape).
    pub fn logfmt<S>(sink: S) -> Self
    where
        S: Fn(&str) + Send + Sync + 'static,
    {
        Self::with_line_sink(sink, format_logfmt)
    }
}

/// The `Logger` service tag.
pub struct LoggerTag;

impl ServiceTag for LoggerTag {
    type Service = Logger;
    const ID: &'static str = "elm-effect/logger";
}

/// Log one message at `Info` (`Effect.log` in Effect 4): it reaches the
/// [`LoggerTag`] sink and, when a span is current, is appended to that span
/// as an event with `effect.logLevel` (and `effect.fiberId` inside a fiber).
/// Logging itself never fails; building the description runs nothing.
pub fn log(message: impl Into<String>) -> Effect<(), std::convert::Infallible, LoggerTag> {
    log_at(LogLevel::Info, message)
}

/// Log at `level` (`Effect.logWithLevel` in Effect 4).
pub fn log_at(
    level: LogLevel,
    message: impl Into<String>,
) -> Effect<(), std::convert::Infallible, LoggerTag> {
    from_step(Box::new(LogEvent {
        level,
        message: message.into(),
    }))
}

/// Log an event at `Info` (Effect 4's `Effect.logInfo`).
pub fn log_info(message: String) -> Effect<(), std::convert::Infallible, LoggerTag> {
    log(message)
}

struct LogEvent {
    level: LogLevel,
    message: String,
}

impl crate::effect::ErasedStep<(), std::convert::Infallible> for LogEvent {
    fn run_erased(
        self: Box<Self>,
        scope: &crate::service::Services,
    ) -> Result<(), Cause<std::convert::Infallible>> {
        let logger = scope
            .get_tag(LoggerTag::ID)
            .and_then(|value| value.downcast::<Logger>().ok());
        let Some(logger) = logger else {
            return Err(Cause::die(crate::Defect::new(
                "logging requires the LoggerTag service",
            )));
        };
        logger.log(self.level, &self.message, &[]);

        let span = scope
            .get_ref(crate::tracer::CURRENT_SPAN.id)
            .and_then(|value| {
                value
                    .downcast_ref::<Option<crate::tracer::AnySpan>>()
                    .cloned()
            })
            .flatten();
        if let Some(span) = span {
            let mut attributes = Vec::with_capacity(2);
            let fiber = scope
                .get_tag(<crate::fiber::FiberTag as ServiceTag>::ID)
                .and_then(|value| {
                    value
                        .downcast_ref::<crate::fiber::Fiber>()
                        .map(|fiber| fiber.id())
                });
            if let Some(fiber_id) = fiber {
                attributes.push((
                    "effect.fiberId".to_owned(),
                    crate::tracer::AttributeValue::Int(fiber_id.0 as i64),
                ));
            }
            attributes.push((
                "effect.logLevel".to_owned(),
                crate::tracer::AttributeValue::Text(level_name(self.level).to_owned()),
            ));
            span.event(&self.message, crate::tracer::now_nanos(scope), attributes);
        }
        Ok(())
    }
}

/// Build a description from a private step; the fields are crate-private, so
/// the constructor lives here rather than at the call site.
fn from_step<A, E, R>(step: Box<dyn crate::effect::ErasedStep<A, E> + Send>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    use std::marker::PhantomData;
    Effect {
        step: crate::effect::Step::Suspend(step),
        services: crate::service::empty_services(),
        marker: PhantomData,
    }
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
/// for tests and debugging. This is the audit sink the crate added, not the
/// Effect 4's `Tracer` backend; that one lives in [`crate::tracer`].
#[derive(Clone, Debug, Default)]
pub struct SpanRecorder {
    spans: Arc<Mutex<Vec<SpanRecord>>>,
}

impl SpanRecorder {
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

/// The `SpanRecorder` service tag.
pub struct SpanRecorderTag;

impl ServiceTag for SpanRecorderTag {
    type Service = SpanRecorder;
    const ID: &'static str = "elm-effect/span-recorder";
}

/// Effect 4's `Cause.StackTrace` annotation key, verbatim.
///
/// Effect 4 stores a JS `StackFrame` object here. A Rust frame has no `stack`
/// thunk to capture and no parent chain, so the frame's name is what a cause
/// keeps: that is the part the rows assert on (`trace.name`).
pub const STACK_TRACE: &str = "effect/Cause/StackTrace";

/// Wrap an effect in a span: records name and attributes, then runs the
/// effect, passing its outcome through.
///
/// A failure raised inside the span gains the span's name as its stack frame.
/// Innermost wins: an inner span annotates first, so an outer one leaves an
/// existing frame alone (Effect 4 records the frame at the raise site, not at
/// every span the cause travels through).
pub fn record_span<A, E, R>(
    tracer: &SpanRecorder,
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
    let frame = record.name.clone();
    Effect::sync(move || {
        tracer.record(&record.name, record.attributes.clone());
    })
    .flat_map(|_| effect)
    .catch_cause(move |cause| {
        let cause = if cause.annotation(STACK_TRACE).is_some() {
            cause
        } else {
            cause.annotate(STACK_TRACE, frame.clone())
        };
        Effect::<A, E, R>::fail_cause(cause)
    })
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

// Log records: one log call's items, and the four formatters over them

/// One item of a log call. Effect 4's `Effect.log` takes any number of
/// values, so a message is a sequence of items rather than one string.
#[derive(Clone, Debug, PartialEq)]
pub enum LogItem {
    /// A string message.
    Text(String),
    /// An integer. Effect 4 logs a BigInt as `123n`, which is what the
    /// text formatters render; JSON carries it as the string `"123n"`.
    BigInt(i128),
    /// A structured value.
    Object(Vec<(String, LogItem)>),
    /// A plain error: JSON carries its name and message, not a bare string.
    Error { name: String, message: String },
    /// A cause: its rows in order, as text.
    Cause(Vec<String>),
}

impl LogItem {
    /// The text rendering: `123n` for integers, `{"value":123n}` for
    /// objects (nested integers keep the same suffix).
    pub fn text(&self) -> String {
        match self {
            LogItem::Text(text) => text.clone(),
            LogItem::BigInt(value) => format!("{value}n"),
            LogItem::Object(fields) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|(key, value)| format!("\"{key}\":{}", value.text()))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            LogItem::Error { name, message } => format!("{name}: {message}"),
            LogItem::Cause(rows) => rows.join(", "),
        }
    }

    /// The JSON rendering: integers become the string `"123n"`, and an error
    /// becomes its `{name, message}` pair.
    pub fn json(&self) -> String {
        match self {
            LogItem::Text(text) => json_string(text),
            LogItem::BigInt(value) => json_string(&format!("{value}n")),
            LogItem::Object(fields) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|(key, value)| format!("{}:{}", json_string(key), value.json()))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            LogItem::Error { name, message } => format!(
                "{{\"name\":{},\"message\":{}}}",
                json_string(name),
                json_string(message)
            ),
            LogItem::Cause(rows) => {
                let inner: Vec<String> = rows.iter().map(|row| json_string(row)).collect();
                format!("[{}]", inner.join(","))
            }
        }
    }
}

fn json_string(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The level's uppercase name, which is what every formatter prints and what
/// Effect 4's JSON rows compare against.
pub fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "TRACE",
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
        LogLevel::Fatal => "FATAL",
        LogLevel::All => "ALL",
        LogLevel::None => "NONE",
    }
}

/// `formatStructured`: the items exactly as they were logged, for a sink that
/// wants values rather than text.
pub fn format_structured(items: &[LogItem]) -> Vec<LogItem> {
    items.to_vec()
}

/// `formatJson`: `{"level":..,"message":[..]}`. The message is the item
/// list, so its order is the caller's.
pub fn format_json_record(level: LogLevel, items: &[LogItem]) -> String {
    let inner: Vec<String> = items.iter().map(LogItem::json).collect();
    format!(
        "{{\"level\":{},\"message\":[{}]}}",
        json_string(level_name(level)),
        inner.join(",")
    )
}

// The annotation methods the log-record rows need, on the type the module
// already had: `annotateLogs` sets one key, `annotateLogsScoped` returns a
// guard that carries the scoped set and restores the previous one.
impl LogAnnotations {
    /// The value behind `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, value)| value.as_str())
    }

    /// The annotation pairs, in insertion order.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// `annotateLogs`: set one key.
    pub fn annotate(&self, key: &str, value: &str) -> Self {
        let mut next = self.0.clone();
        match next.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value.to_owned(),
            None => next.push((key.to_owned(), value.to_owned())),
        }
        Self(next)
    }

    /// `annotateLogsScoped`: the returned guard carries the scoped set, and
    /// the set it replaced comes back when the guard drops. A `None` value is
    /// Effect 4's `NaN`: it applies while scoped and is gone afterwards.
    pub fn annotate_scoped(&self, key: &str, value: Option<&str>) -> ScopedAnnotations {
        let applied = match value {
            Some(value) => self.annotate(key, value),
            None => self.annotate(key, "NaN"),
        };
        ScopedAnnotations {
            current: applied,
            previous: self.clone(),
        }
    }

    /// One line's rendering: `{key: "value"}` for the default logger, and
    /// `key=value` pairs for the logfmt formatters.
    pub fn object_line(&self) -> String {
        let inner: Vec<String> = self
            .0
            .iter()
            .map(|(key, value)| format!("{key}: {value:?}"))
            .collect();
        format!("{{{}}}", inner.join(", "))
    }
}

/// A scoped annotation set: [`now`](Self::now) while it is live, the previous
/// set once it drops.
pub struct ScopedAnnotations {
    current: LogAnnotations,
    previous: LogAnnotations,
}

impl ScopedAnnotations {
    /// The annotations in force inside the scope.
    pub fn now(&self) -> &LogAnnotations {
        &self.current
    }

    /// The annotations that come back when the scope closes.
    pub fn previous(&self) -> &LogAnnotations {
        &self.previous
    }
}

/// The default logger's lines: a header, then one line per message item with
/// text items in the order they were logged, then the annotations line. A
/// cause is rendered after the text items, which is the ordering Effect 4
/// uses.
pub fn default_log_lines(
    level: LogLevel,
    fiber_id: u64,
    span: Option<(&str, u128)>,
    items: &[LogItem],
    annotations: &LogAnnotations,
) -> Vec<String> {
    let span_part = match span {
        Some((name, millis)) => format!(" {name}={millis}ms:"),
        None => ":".to_owned(),
    };
    let mut lines = vec![format!(
        "[00:00:00.000] {} (#{fiber_id}){span_part}",
        level_name(level)
    )];
    lines.extend(
        items
            .iter()
            .filter(|item| matches!(item, LogItem::Text(_)))
            .map(LogItem::text),
    );
    lines.extend(
        items
            .iter()
            .filter(|item| !matches!(item, LogItem::Text(_)))
            .map(LogItem::text),
    );
    if !annotations.pairs().is_empty() {
        lines.push(annotations.object_line());
    }
    lines
}

/// One `message=` field per item: Effect 4's logfmt and simple formatters
/// print each item of a call as its own field, quoted only when the logfmt
/// rule says so.
pub fn message_fields(items: &[LogItem]) -> String {
    items
        .iter()
        .map(|item| format!("message={}", quote_logfmt_value(&item.text())))
        .collect::<Vec<String>>()
        .join(" ")
}

/// `formatLogFmt` over a whole record: level, fiber, one `message=` field per
/// item, then the annotations.
pub fn format_logfmt_record(
    level: LogLevel,
    fiber_id: u64,
    items: &[LogItem],
    annotations: &LogAnnotations,
) -> String {
    let mut parts = vec![
        format!("level={}", level_name(level)),
        format!("fiber=#{fiber_id}"),
        message_fields(items),
    ];
    parts.extend(
        annotations
            .pairs()
            .iter()
            .map(|(key, value)| format_logfmt_field(key, value)),
    );
    parts.join(" ")
}

/// `formatSimple` over a whole record: the same fields as
/// [`format_logfmt_record`], which is why the two agree on the message part.
pub fn format_simple_record(
    level: LogLevel,
    fiber_id: u64,
    items: &[LogItem],
    annotations: &LogAnnotations,
) -> String {
    let mut parts = vec![
        format!("level={}", level_name(level)),
        format!("fiber=#{fiber_id}"),
        message_fields(items),
    ];
    parts.extend(
        annotations
            .pairs()
            .iter()
            .map(|(key, value)| format_logfmt_field(key, value)),
    );
    parts.join(" ")
}

/// A record formatter: level, items, and annotations in, one line out.
type RecordFormat = Arc<dyn Fn(LogLevel, &[LogItem], &LogAnnotations) -> String + Send + Sync>;
/// Where a formatted line goes.
type LineSink = Arc<dyn Fn(String) + Send + Sync>;

/// A logger whose formatter and sink are both replaceable: the
/// `Logger.formatJson.pipe(Logger.map(..))` shape, where the formatter writes
/// the line and the sink decides what becomes of it.
pub struct FormattedLogger {
    format: RecordFormat,
    sink: LineSink,
}

impl FormattedLogger {
    /// Emit one record through the formatter and then the sink.
    pub fn log(&self, level: LogLevel, items: &[LogItem], annotations: &LogAnnotations) {
        (self.sink)((self.format)(level, items, annotations));
    }

    /// The `formatJson` + map shape: a JSON line into a collecting sink.
    pub fn json_sink(sink: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            format: Arc::new(|level, items, _annotations| format_json_record(level, items)),
            sink: Arc::new(sink),
        }
    }
}
