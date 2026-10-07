//! Observability: redaction absence, span records, tap passthrough.

#[path = "../src/observability.rs"]
mod observability;

use std::sync::{Arc, Mutex};

use observability::{
    Counter, Gauge, LogLevel, Logger, Redacted, Tracer, is_enabled, tap, with_span,
};

#[test]
fn secret_debug_is_redacted() {
    let secret = Redacted::make("hunter2".to_string());
    let shown = format!("{secret:?}");
    assert!(!shown.contains("hunter2"), "secret bytes leaked: {shown}");
    assert_eq!(secret.value(), "hunter2");

    // Span fields render through Debug too: redacted stays redacted.
    let tracer = Tracer::new();
    let password = Redacted::make("s3cret".to_string());
    let effect = elm_effect::Effect::<u32, String>::succeed(1);
    let spanned = with_span(
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

    let traced = with_span(
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
    assert_eq!(
        Redacted::make(vec![1, 2]),
        Redacted::make(vec![1, 2])
    );
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
    let tracer = Tracer::new();
    let exit = elm_effect::Runtime::run(with_span(
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
    let tracer = observability::Tracer::new();
    let use_tracer =
        elm_effect::use_service::<observability::TracerTag>().map(|tracer| tracer.spans().len());
    let trace_effect = use_tracer.provide::<observability::TracerTag, ()>(tracer);
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
