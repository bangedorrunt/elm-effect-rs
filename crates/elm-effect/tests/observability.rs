//! Observability: redaction absence, span records, tap passthrough.

// The module is part of the library. It used to be compiled into this test
// crate by `#[path]` include, which meant nothing outside a test could reach
// `record_span` at all.
use elm_effect::observability;

use std::sync::{Arc, Mutex};

use observability::{
    Counter, Gauge, LogLevel, Logger, Redacted, SpanRecorder, is_enabled, record_span, tap,
};

#[test]
fn secret_debug_is_redacted() {
    let secret = Redacted::make("hunter2".to_string());
    let shown = format!("{secret:?}");
    assert!(!shown.contains("hunter2"), "secret bytes leaked: {shown}");
    assert_eq!(secret.value(), "hunter2");

    // Span fields render through Debug too: redacted stays redacted.
    let tracer = SpanRecorder::new();
    let password = Redacted::make("s3cret".to_string());
    let effect = elm_effect::Effect::<u32, String>::succeed(1);
    let spanned = record_span(
        &tracer,
        "login",
        vec![("user".to_string(), format!("{:?}", "bob"))],
        effect,
    );
    let _ = spanned;
    let password_field = format!("password={:?}", password);
    assert!(!password_field.contains("s3cret"));
    let spans = tracer.spans();
    assert!(spans.is_empty());

    let traced = record_span(
        &tracer,
        "login",
        vec![("user".to_string(), "bob".to_string())],
        elm_effect::Effect::<u32, String>::succeed(1),
    );
    let exit = elm_effect::Runtime::run(traced);
    assert!(matches!(exit, elm_effect::Exit::Success(1)));
    let spans = tracer.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "login");
    assert_eq!(
        spans[0].attributes,
        vec![("user".to_string(), "bob".to_string())]
    );
}

#[test]
fn redacted_display_equal_hash_value() {
    // Redacted.test.ts:24 — default stringification is the placeholder.
    assert_eq!(format!("{}", Redacted::make("redacted")), "<redacted>");
    // :50 — equality compares protected values.
    assert_eq!(Redacted::make(1), Redacted::make(1));
    assert_ne!(Redacted::make(1), Redacted::make(2));
    // Chunk constructor values compare too (composites wrap like scalars).
    assert_eq!(Redacted::make(vec![1, 2]), Redacted::make(vec![1, 2]));
    // :55 — hash derives from the protected value.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let hash_of = |v: Redacted<i32>| {
        let mut h = DefaultHasher::new();
        v.hash(&mut h);
        h.finish()
    };
    assert_eq!(hash_of(Redacted::make(1)), hash_of(Redacted::make(1)));
    assert_ne!(hash_of(Redacted::make(1)), hash_of(Redacted::make(2)));
}

#[test]
fn span_name_and_attributes() {
    let tracer = SpanRecorder::new();
    let exit = elm_effect::Runtime::run(record_span(
        &tracer,
        "checkout",
        vec![
            ("cart".to_string(), "c9".to_string()),
            ("total".to_string(), "42".to_string()),
        ],
        elm_effect::Effect::<String, String>::succeed("ok".to_string()),
    ));
    assert!(matches!(exit, elm_effect::Exit::Success(_)));
    let spans = tracer.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "checkout");
    assert_eq!(spans[0].attributes.len(), 2);
}

#[test]
fn tap_does_not_change_result() {
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let effect = tap(elm_effect::Effect::<u32, String>::succeed(7), {
        let seen = Arc::clone(&seen);
        move |v: &u32| {
            seen.lock().unwrap().push(*v);
        }
    });
    let exit = elm_effect::Runtime::run(effect);
    assert!(matches!(exit, elm_effect::Exit::Success(7)));
    assert_eq!(*seen.lock().unwrap(), vec![7]);

    // Failures pass through untouched (the observer never runs).
    let effect = tap(
        elm_effect::Effect::<u32, String>::fail("nope".to_string()),
        |_: &u32| panic!("must not run"),
    );
    let exit = elm_effect::Runtime::run(effect);
    assert!(matches!(exit, elm_effect::Exit::Failure(_)));

    // Levels order; metrics count and gauge.
    assert!(!is_enabled(LogLevel::Info, LogLevel::Warn));
    assert!(is_enabled(LogLevel::Error, LogLevel::Warn));
    let counter = Counter::new("hits");
    counter.inc();
    counter.inc_by(2);
    assert_eq!(counter.get(), 3);
    assert_eq!(counter.name(), "hits");
    let gauge = Gauge::new("temp");
    gauge.set(-4);
    assert_eq!(gauge.get(), -4);
    assert_eq!(gauge.name(), "temp");
    let logger_seen = Arc::new(Mutex::new(Vec::new()));
    let logger = Logger::new({
        let logger_seen = Arc::clone(&logger_seen);
        move |level: LogLevel, message: &str, _: &[(String, String)]| {
            logger_seen
                .lock()
                .unwrap()
                .push((level, message.to_string()));
        }
    });
    logger.log(LogLevel::Info, "hi", &[]);
    assert_eq!(logger_seen.lock().unwrap().len(), 1);
    let notes = Arc::new(Mutex::new(Vec::new()));
    let reporter = observability::ErrorReporter::new({
        let notes = Arc::clone(&notes);
        move |note: &str| notes.lock().unwrap().push(note.to_string())
    });
    reporter.report("boom");
    assert_eq!(notes.lock().unwrap().len(), 1);
    let reported = elm_effect::use_service::<observability::ErrorReporterTag>().map(|reporter| {
        reporter.report("x");
        1u32
    });
    let reported = reported.provide::<observability::ErrorReporterTag, ()>(reporter);
    assert!(matches!(
        elm_effect::Runtime::run(reported),
        elm_effect::Exit::Success(1)
    ));
    let annotations = observability::LogAnnotations(vec![("k".to_string(), "v".to_string())]);
    logger.log(LogLevel::Debug, "ctx", &annotations.0);
    assert_eq!(logger_seen.lock().unwrap().len(), 2);
}

#[test]
fn log_level_gates_and_info_effect() {
    use observability::{LogLevel as L, Redacted};
    use observability::{LoggerTag, MinimumLogLevel, Redactable, is_enabled};

    assert!(is_enabled(L::Fatal, L::All));
    assert!(!is_enabled(L::Trace, L::Debug));
    assert!(is_enabled(L::Fatal, L::Fatal));
    assert!(!is_enabled(L::Debug, L::None));
    assert_eq!(MinimumLogLevel(L::Info).0, L::Info);

    assert_eq!(Redacted::make(1u32).redact(), "<redacted>");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let logger = observability::Logger::new({
        let seen = Arc::clone(&seen);
        move |level: LogLevel, message: &str, _: &[(String, String)]| {
            seen.lock().unwrap().push((level, message.to_string()));
        }
    });
    let effect = observability::log_info("hello".to_string()).provide::<LoggerTag, ()>(logger);
    let exit = elm_effect::Runtime::run(effect);
    assert!(matches!(exit, elm_effect::Exit::Success(())));
    assert_eq!(seen.lock().unwrap().len(), 1);

    let registry = observability::MetricRegistry::new();
    registry.counter("hits").inc_by(2);
    assert_eq!(registry.counter("hits").get(), 2);
    let use_registry = elm_effect::use_service::<observability::MetricTag>()
        .map(|registry| registry.counter("hits").get());
    let metric_effect = use_registry.provide::<observability::MetricTag, ()>(registry);
    assert!(matches!(
        elm_effect::Runtime::run(metric_effect),
        elm_effect::Exit::Success(2)
    ));
    let tracer = observability::SpanRecorder::new();
    let use_tracer =
        elm_effect::use_service::<observability::SpanRecorderTag>().map(|tracer| tracer.spans().len());
    let trace_effect = use_tracer.provide::<observability::SpanRecorderTag, ()>(tracer);
    assert!(matches!(
        elm_effect::Runtime::run(trace_effect),
        elm_effect::Exit::Success(0)
    ));
}

fn run_ignore(
    options: observability::IgnoreOptions,
    current: observability::LogLevel,
) -> Vec<(observability::LogLevel, String)> {
    use observability::LogLevel;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let logger = observability::Logger::new({
        let seen = Arc::clone(&seen);
        move |level: LogLevel, message: &str, _: &[(String, String)]| {
            seen.lock().unwrap().push((level, message.to_string()));
        }
    });
    let effect = observability::ignore(
        elm_effect::Effect::<(), String>::fail("boom".to_string()),
        options,
        current,
        LogLevel::Trace,
        &logger,
    );
    let exit = elm_effect::Runtime::run(effect);
    assert!(
        matches!(exit, elm_effect::Exit::Success(())),
        "ignore must swallow the failure"
    );
    seen.lock().unwrap().clone()
}

fn run_ignore_cause(
    options: observability::IgnoreOptions,
    current: observability::LogLevel,
) -> Vec<(observability::LogLevel, String)> {
    use observability::LogLevel;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let logger = observability::Logger::new({
        let seen = Arc::clone(&seen);
        move |level: LogLevel, message: &str, _: &[(String, String)]| {
            seen.lock().unwrap().push((level, message.to_string()));
        }
    });
    let effect = observability::ignore_cause(
        elm_effect::Effect::<(), String>::fail("boom".to_string()),
        options,
        current,
        LogLevel::Trace,
        &logger,
    );
    let exit = elm_effect::Runtime::run(effect);
    assert!(
        matches!(exit, elm_effect::Exit::Success(())),
        "ignore_cause must swallow the failure"
    );
    seen.lock().unwrap().clone()
}

#[test]
fn ignore_omitted_logs_nothing() {
    let logs = run_ignore(
        observability::IgnoreOptions::default(),
        observability::LogLevel::Info,
    );
    assert!(logs.is_empty(), "omitted log option must stay silent");
}

#[test]
fn ignore_false_logs_nothing() {
    let logs = run_ignore(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::No,
            message: None,
        },
        observability::LogLevel::Info,
    );
    assert!(logs.is_empty(), "log: false must stay silent");
}

#[test]
fn ignore_true_logs_at_current_level() {
    use observability::LogLevel;
    let logs = run_ignore(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::Current,
            message: None,
        },
        LogLevel::Warn,
    );
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].0, LogLevel::Warn);
    assert!(logs[0].1.contains("boom"), "log carries the cause");
}

#[test]
fn ignore_level_overrides_current() {
    use observability::LogLevel;
    let logs = run_ignore(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::Level(LogLevel::Error),
            message: None,
        },
        LogLevel::Warn,
    );
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].0, LogLevel::Error);
    assert!(logs[0].1.contains("boom"), "log carries the cause");
}

#[test]
fn ignore_message_replaces_cause() {
    use observability::LogLevel;
    let logs = run_ignore(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::Current,
            message: Some("Ignoring failure".to_string()),
        },
        LogLevel::Info,
    );
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].1, "Ignoring failure");
}

#[test]
fn ignore_cause_omitted_logs_nothing() {
    let logs = run_ignore_cause(
        observability::IgnoreOptions::default(),
        observability::LogLevel::Info,
    );
    assert!(logs.is_empty(), "omitted log option must stay silent");
}

#[test]
fn ignore_cause_false_logs_nothing() {
    let logs = run_ignore_cause(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::No,
            message: None,
        },
        observability::LogLevel::Info,
    );
    assert!(logs.is_empty(), "log: false must stay silent");
}

#[test]
fn ignore_cause_true_logs_at_current_level() {
    use observability::LogLevel;
    let logs = run_ignore_cause(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::Current,
            message: None,
        },
        LogLevel::Warn,
    );
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].0, LogLevel::Warn);
    assert!(logs[0].1.contains("boom"), "log carries the cause");
}

#[test]
fn ignore_cause_level_overrides_current() {
    use observability::LogLevel;
    let logs = run_ignore_cause(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::Level(LogLevel::Error),
            message: None,
        },
        LogLevel::Warn,
    );
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].0, LogLevel::Error);
    assert!(logs[0].1.contains("boom"), "log carries the cause");
}

#[test]
fn ignore_cause_message_replaces_cause() {
    use observability::LogLevel;
    let logs = run_ignore_cause(
        observability::IgnoreOptions {
            log: observability::IgnoreLog::Current,
            message: Some("Ignoring cause".to_string()),
        },
        LogLevel::Info,
    );
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].1, "Ignoring cause");
}

#[test]
fn ignore_cause_swallows_defects() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let logger = observability::Logger::new({
        let seen = Arc::clone(&seen);
        move |level: observability::LogLevel, message: &str, _: &[(String, String)]| {
            seen.lock().unwrap().push((level, message.to_string()));
        }
    });
    let effect = observability::ignore_cause(
        elm_effect::Effect::<(), String>::die(elm_effect::Defect::new("boom".to_string())),
        observability::IgnoreOptions::default(),
        observability::LogLevel::Info,
        observability::LogLevel::Trace,
        &logger,
    );
    let exit = elm_effect::Runtime::run(effect);
    assert!(
        matches!(exit, elm_effect::Exit::Success(())),
        "ignore_cause must swallow defects"
    );
}

#[test]
fn ignore_cause_swallows_interrupts() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let logger = observability::Logger::new({
        let seen = Arc::clone(&seen);
        move |level: observability::LogLevel, message: &str, _: &[(String, String)]| {
            seen.lock().unwrap().push((level, message.to_string()));
        }
    });
    let effect = observability::ignore_cause(
        elm_effect::Effect::<(), String>::interrupt(),
        observability::IgnoreOptions::default(),
        observability::LogLevel::Info,
        observability::LogLevel::Trace,
        &logger,
    );
    let exit = elm_effect::Runtime::run(effect);
    assert!(
        matches!(exit, elm_effect::Exit::Success(())),
        "ignore_cause must swallow interrupts"
    );
}

#[test]
fn format_logfmt_quotes_only_when_needed() {
    use observability::{LogLevel, format_logfmt};
    let output = format_logfmt(
        LogLevel::Info,
        "Welcome to the Effect Playground!",
        &[
            ("annotation1".to_string(), "first".to_string()),
            ("annotation2".to_string(), "second".to_string()),
            ("annotation3".to_string(), "\"omg\"".to_string()),
        ],
    );
    assert!(output.contains("message=\"Welcome to the Effect Playground!\""));
    assert!(output.contains("annotation1=first"));
    assert!(output.contains("annotation2=second"));
    assert!(output.contains("annotation3=\"\\\"omg\\\"\""));
    assert!(!output.contains("message=\"\\\"Welcome"));
    assert!(!output.contains("annotation1=\"\\\"first\""));
}

#[test]
fn format_simple_quotes_only_when_needed() {
    use observability::format_simple;
    let output = format_simple(
        "hello world",
        &[("annotation".to_string(), "value with spaces".to_string())],
    );
    assert!(output.contains("message=\"hello world\""));
    assert!(output.contains("annotation=\"value with spaces\""));
    assert!(!output.contains("message=\"\\\"hello world\""));
}

// --- Logger records (Logger.test.ts rows) ---

fn logger_items() -> Vec<observability::LogItem> {
    use observability::LogItem;
    vec![
        LogItem::BigInt(123),
        LogItem::Object(vec![("value".to_owned(), LogItem::BigInt(123))]),
    ]
}

// Logger.test.ts:11 — one call with a span and an annotation prints a header,
// then the message items as their own lines, then the annotations.
#[test]
fn logger_should_output_logs() {
    use observability::{LogAnnotations, LogItem, LogLevel, default_log_lines};
    let annotations = LogAnnotations::default().annotate("key", "value");
    let lines = default_log_lines(
        LogLevel::Info,
        1,
        Some(("span", 0)),
        &[
            LogItem::Text("info".to_owned()),
            LogItem::Text("message".to_owned()),
        ],
        &annotations,
    );

    assert!(lines[0].contains("INFO"), "header level: {}", lines[0]);
    assert!(lines[0].contains("(#1)"), "header fiber: {}", lines[0]);
    assert!(lines[0].contains("span=0ms:"), "header span: {}", lines[0]);
    assert_eq!(lines[1], "info");
    assert_eq!(lines[2], "message");
    assert_eq!(lines[3], "{key: \"value\"}");
}

// Logger.test.ts:29 — replacing the logger means the replaced sink is what
// receives the formatted line, exactly once.
#[test]
fn logger_replace_logger() {
    use observability::{FormattedLogger, LogAnnotations, LogItem, LogLevel};
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let collected = seen.clone();
    let logger = FormattedLogger::json_sink(move |line| collected.lock().unwrap().push(line));

    logger.log(
        LogLevel::Info,
        &[
            LogItem::Text("info".to_owned()),
            LogItem::Text("message".to_owned()),
        ],
        &LogAnnotations::default(),
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the replacement sink ran once"
    );
    assert!(seen.lock().unwrap()[0].starts_with("{\"level\":\"INFO\""));
}

// Logger.test.ts:93 — a BigInt message renders `123n` in text and as the
// string "123n" in JSON, and structured keeps the value itself.
#[test]
fn logger_formats_bigint_messages_consistently() {
    use observability::{
        LogAnnotations, LogLevel, format_json_record, format_logfmt_record, format_simple_record,
        format_structured,
    };
    let items = logger_items();
    let annotations = LogAnnotations::default();

    // The pin's own line carries a timestamp prefix, so its assertion spells
    // the level and fiber with a leading space; here the line starts there.
    let simple = format_simple_record(LogLevel::Info, 1, &items, &annotations);
    assert!(simple.starts_with("level=INFO fiber=#1 "), "got {simple}");
    assert!(
        simple.contains("message=123n message=\"{\\\"value\\\":123n}\""),
        "got {simple}"
    );

    let logfmt = format_logfmt_record(LogLevel::Info, 1, &items, &annotations);
    assert!(logfmt.starts_with("level=INFO fiber=#1 "), "got {logfmt}");
    assert!(
        logfmt.contains("message=123n message=\"{\\\"value\\\":123n}\""),
        "got {logfmt}"
    );

    assert_eq!(
        format_structured(&items),
        items,
        "structured keeps the values"
    );
    assert_eq!(
        format_json_record(LogLevel::Info, &items),
        "{\"level\":\"INFO\",\"message\":[\"123n\",{\"value\":\"123n\"}]}"
    );
}

// Logger.test.ts:118 — the JSON formatter carries a plain error's name and
// message rather than a bare string.
#[test]
fn logger_format_json_includes_error_messages() {
    use observability::{LogItem, LogLevel, format_json_record};
    let json = format_json_record(
        LogLevel::Error,
        &[LogItem::Error {
            name: "Error".to_owned(),
            message: "boom".to_owned(),
        }],
    );
    assert_eq!(
        json,
        "{\"level\":\"ERROR\",\"message\":[{\"name\":\"Error\",\"message\":\"boom\"}]}"
    );
}

// Logger.test.ts:134 — a scoped annotation applies inside the scope and is
// gone outside it.
#[test]
fn logger_annotate_logs_scoped_applies_only_while_scoped() {
    use observability::LogAnnotations;
    let base = LogAnnotations::default();
    assert_eq!(base.get("requestId"), None);
    {
        let scoped = base.annotate_scoped("requestId", Some("req-123"));
        assert_eq!(scoped.now().get("requestId"), Some("req-123"));
        assert_eq!(scoped.previous().get("requestId"), None);
    }
    assert_eq!(base.get("requestId"), None, "outside the scope it is gone");
}

// Logger.test.ts:155 — the scope restores the annotations it replaced.
#[test]
fn logger_annotate_logs_scoped_restores_previous() {
    use observability::LogAnnotations;
    let outer = LogAnnotations::default().annotate("outer", "program");
    assert_eq!(outer.get("outer"), Some("program"));
    {
        let scoped = outer.annotate_scoped("inner", Some("scope"));
        assert_eq!(scoped.now().get("outer"), Some("program"));
        assert_eq!(scoped.now().get("inner"), Some("scope"));
        // The previous set is the one that comes back, unchanged.
        assert_eq!(scoped.previous(), &outer);
        assert_eq!(scoped.previous().get("inner"), None);
    }
    assert_eq!(
        outer.get("inner"),
        None,
        "the restored set has no inner key"
    );
}

// Logger.test.ts:182 (the NaN it.each) — a NaN annotation applies while
// scoped and the initial annotations come back afterwards, whether they were
// empty or already carried the key.
#[test]
fn logger_annotate_logs_scoped_restores_nan_annotations() {
    use observability::LogAnnotations;
    for initial in [
        LogAnnotations::default(),
        LogAnnotations::default().annotate("measurement", "previous"),
    ] {
        let scoped = initial.annotate_scoped("measurement", None);
        assert_eq!(
            scoped.now().get("measurement"),
            Some("NaN"),
            "NaN applies inside the scope"
        );
        assert_eq!(scoped.previous(), &initial, "the initial set comes back");
    }
}

// Logger.test.ts:206 — the default logger keeps the message items in order
// and renders a cause after them.
#[test]
fn logger_default_keeps_message_item_order_with_a_cause() {
    use observability::{LogAnnotations, LogItem, LogLevel, default_log_lines};
    let lines = default_log_lines(
        LogLevel::Info,
        7,
        None,
        &[
            LogItem::Text("first".to_owned()),
            LogItem::Cause(vec!["boom".to_owned()]),
            LogItem::Text("second".to_owned()),
        ],
        &LogAnnotations::default(),
    );

    assert!(
        lines[0].contains("INFO") && lines[0].contains("(#7):"),
        "header: {}",
        lines[0]
    );
    assert_eq!(lines[1], "first");
    assert_eq!(lines[2], "second", "the text items keep their order");
    assert!(
        lines[3].contains("boom"),
        "the cause follows them: {}",
        lines[3]
    );
}

#[test]
fn pipe_supports_extracting_the_protected_value() {
    // Redacted.test.ts:17 — the tag's data-last pipe form; `value` is the
    // extracting function, and it is the only way to read the bytes.
    let inner = vec![1, 2, 3];
    let redacted = Redacted::make(inner.clone());
    // The port has no pipe operator; `Redacted::value(redacted)` is the same
    // application the tag's `pipe` performs.
    assert_eq!(Redacted::value(&redacted), &inner);
    // And the Debug side still refuses to show the bytes.
    assert!(!format!("{redacted:?}").contains('2'));
}

/// Effect.test.ts:270 `caught sandboxed failure captures stack frame`.
/// A die raised inside a span keeps that span as its stack frame, and the
/// frame survives `sandbox` + `flip` turning the cause back into a value.
#[test]
fn a_sandboxed_failure_captures_the_span_as_its_stack_frame() {
    use elm_effect::{Defect, Effect, Exit, Runtime};
    use observability::STACK_TRACE;

    let tracer = SpanRecorder::new();
    let captured = record_span(
        &tracer,
        "test span",
        Vec::new(),
        Effect::<(), String>::die(Defect::new("boom")),
    )
    .sandbox()
    .flip();

    let cause = match Runtime::run(captured) {
        Exit::Success(cause) => cause,
        Exit::Failure(cause) => panic!("flip turned the sandboxed cause back into a failure: {cause:?}"),
    };
    assert!(
        matches!(cause.reasons.first(), Some(elm_effect::Reason::Die(_))),
        "the die is the reason: {cause:?}"
    );
    assert_eq!(
        cause.annotation(STACK_TRACE),
        Some("test span"),
        "the raise site's span is the frame the cause carries"
    );
    // The span itself was still recorded, not just its frame.
    assert_eq!(tracer.spans().len(), 1);
}

/// Effect.test.ts:282 `failure without a current frame has no stack annotation
/// and reuses the original exit`.
/// Outside any span a failure is passed through untouched: no frame is
/// invented, and the exit a run produces is the one the failure describes.
#[test]
fn a_failure_outside_a_span_carries_no_stack_frame() {
    use elm_effect::{Effect, Exit, Runtime};
    use observability::STACK_TRACE;

    let exit = Runtime::run(Effect::<(), String>::fail("boom".to_owned()));
    assert_eq!(
        exit,
        Exit::<(), String>::fail("boom".to_owned()),
        "the run reproduces the failure's own exit"
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(
            cause.annotation(STACK_TRACE),
            None,
            "no current frame means no stack annotation"
        ),
        Exit::Success(()) => panic!("a failing effect must not succeed"),
    }
}
