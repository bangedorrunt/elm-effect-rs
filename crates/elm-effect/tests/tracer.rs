//! `Tracer.test.ts` at `effect@4.0.0`: the span model, the `withSpan` family,
//! the logging bridge, and the layer tracing combinators. Each test names the
//! pin row it carries; OTLP identifiers and nesting live in
//! `tests/otlp_tracer.rs`, and the rows that stay `not-applicable` (the
//! `Tracer.context` hook, the interrupt sweep, JS stack sites) are listed in
//! `conformance/behavior-tests.tsv` with their reasons.

use std::sync::{Arc, Mutex};

use elm_effect::observability::{self, LogLevel, Logger, LoggerTag};
use elm_effect::tracer::{
    AnySpan, AttributeValue, CURRENT_SPAN, NativeSpan, NoopSpan, Span, SpanKind, SpanLink,
    SpanOptions, SpanStatus, Tracer, annotate_current_span, annotate_span, annotate_spans,
    current_span, external_span, link_span, make_span, use_span, with_minimum_trace_level,
    with_parent_span, with_span, with_trace_clock, with_tracer_enabled, with_tracer_timing,
};
use elm_effect::{
    ClockImpl, Duration, Effect, Exit, Layer, LayerSpanOptions, Runtime, Scope, ServiceTag,
    TestClock,
};

/// A lowercase hex string of exactly `len` characters.
fn is_hex_id(text: &str, len: usize) -> bool {
    text.len() == len && text.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn unwrap_success<A, E: std::fmt::Debug>(exit: Exit<A, E>) -> A {
    match exit {
        Exit::Success(value) => value,
        Exit::Failure(cause) => panic!("expected success, got failure: {cause:?}"),
    }
}

#[test]
fn native_span_identifiers_are_hex_and_share_the_trace() {
    // Tracer.test.ts:44 — a root mints a 32-char trace id and a 16-char span
    // id, and a child keeps the parent's trace id with a fresh span id.
    let exit = Runtime::run(with_span(
        "root",
        SpanOptions::default(),
        current_span().flat_map(|root| {
            let root = root.expect("root span");
            with_span(
                "child",
                SpanOptions {
                    parent: Some(root.clone()),
                    ..Default::default()
                },
                current_span(),
            )
            .map(move |child| (root, child.expect("child span")))
        }),
    ));
    let (root, child) = unwrap_success(exit);

    assert!(
        is_hex_id(&root.trace_id(), 32),
        "trace id: {}",
        root.trace_id()
    );
    assert!(
        is_hex_id(&root.span_id(), 16),
        "span id: {}",
        root.span_id()
    );
    assert_eq!(child.trace_id(), root.trace_id(), "a child joins the trace");
    assert!(
        is_hex_id(&child.span_id(), 16),
        "child span id: {}",
        child.span_id()
    );
    assert_ne!(
        child.span_id(),
        root.span_id(),
        "siblings never share an id"
    );
}

#[test]
fn with_span_sets_the_parent_span() {
    // Tracer.test.ts:76 — the ambient span becomes the parent.
    let exit = Runtime::run(with_span(
        "B",
        SpanOptions::default(),
        with_span("A", SpanOptions::default(), current_span()),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.name(), "A");
    assert_eq!(span.parent().expect("parent").name(), "B");
}

#[test]
fn with_span_root_ignores_the_ambient_parent() {
    // Tracer.test.ts:87 — `root: true` starts a new trace regardless.
    let exit = Runtime::run(with_span(
        "B",
        SpanOptions::default(),
        with_span(
            "A",
            SpanOptions {
                root: true,
                ..Default::default()
            },
            current_span(),
        ),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.name(), "A");
    assert_eq!(span.parent(), None);
}

#[test]
fn with_span_accepts_an_external_parent() {
    // Tracer.test.ts:98 — the parent may come from another tracing system.
    let external = AnySpan::External(external_span("000", "111"));
    let exit = Runtime::run(with_span(
        "A",
        SpanOptions {
            parent: Some(external),
            ..Default::default()
        },
        current_span(),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.name(), "A");
    assert_eq!(span.parent().expect("parent").span_id(), "000");
}

#[test]
fn minimum_trace_level_forces_an_unsampled_child() {
    // Tracer.test.ts:116 — a sampled parent does not rescue a span below the
    // minimum trace level.
    let parent = AnySpan::External(external_span("000", "111"));
    let exit = Runtime::run(with_minimum_trace_level(
        LogLevel::Error,
        with_span(
            "A",
            SpanOptions {
                parent: Some(parent),
                level: Some(LogLevel::Info),
                ..Default::default()
            },
            current_span(),
        ),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert!(!span.sampled(), "Info is below the Error threshold");
}

#[test]
fn a_top_level_span_has_no_parent() {
    // Tracer.test.ts:133 — no ambient parent means no parent field, and no
    // synthetic stack trace attribute.
    let exit = Runtime::run(with_span("A", SpanOptions::default(), current_span()));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.name(), "A");
    assert_eq!(span.parent(), None);
    assert_eq!(span.attribute_value("code.stacktrace"), None);
}

#[test]
fn with_span_records_attributes_and_kind() {
    // Tracer.test.ts:202 — attributes and the OpenTelemetry kind ride on the
    // span without changing the body's value.
    let exit = Runtime::run(with_span(
        "test-span",
        SpanOptions {
            kind: SpanKind::Server,
            attributes: vec![
                ("code.filepath".to_owned(), "test.ts".into()),
                ("code.lineno".to_owned(), 1i64.into()),
            ],
            ..Default::default()
        },
        Effect::<u32, String>::succeed(42),
    ));
    assert_eq!(unwrap_success(exit), 42);

    let span = Runtime::run(with_span(
        "test-span",
        SpanOptions {
            kind: SpanKind::Server,
            attributes: vec![("code.filepath".to_owned(), "test.ts".into())],
            ..Default::default()
        },
        current_span(),
    ));
    let span = unwrap_success(span).expect("span");
    assert_eq!(
        span.attribute_value("code.filepath"),
        Some(AttributeValue::Text("test.ts".to_owned()))
    );
    assert_eq!(span.kind(), SpanKind::Server);
}

#[test]
fn annotate_spans_sets_key_value_pairs() {
    // Tracer.test.ts:218 — `annotateSpans("key", "value")`.
    let exit = Runtime::run(with_span(
        "A",
        SpanOptions::default(),
        annotate_span("key", "value", current_span()),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.name(), "A");
    assert_eq!(span.parent(), None);
    assert_eq!(
        span.attribute_value("key"),
        Some(AttributeValue::Text("value".to_owned()))
    );
}

#[test]
fn annotate_spans_takes_a_record() {
    // Tracer.test.ts:230 — the record form sets every pair.
    let exit = Runtime::run(with_span(
        "A",
        SpanOptions::default(),
        annotate_spans(
            vec![
                ("key".to_owned(), "value".into()),
                ("key2".to_owned(), "value2".into()),
            ],
            current_span(),
        ),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(
        span.attribute_value("key"),
        Some(AttributeValue::Text("value".to_owned()))
    );
    assert_eq!(
        span.attribute_value("key2"),
        Some(AttributeValue::Text("value2".to_owned()))
    );
}

#[test]
fn use_span_ends_the_span_when_the_body_dies() {
    // Tracer.test.ts:246 — the span is ended with the body's exit even when
    // the body defects, and the defect still escapes.
    use elm_effect::Defect;

    let seen: Arc<Mutex<Option<Arc<dyn Span>>>> = Arc::new(Mutex::new(None));
    let capture = Arc::clone(&seen);
    let exit = Runtime::run(use_span("A", SpanOptions::default(), move |span| {
        *capture.lock().expect("capture") = Some(span);
        Effect::<(), String>::die(Defect::new("boom"))
    }));
    assert!(
        matches!(exit, Exit::Failure(_)),
        "the defect escapes the span"
    );

    let span = seen.lock().expect("capture").clone().expect("span A");
    match span.status() {
        SpanStatus::Ended { exit, .. } => {
            let rendered = format!("{exit:?}");
            assert!(
                rendered.contains("boom"),
                "the ended span keeps the body's defect: {rendered}"
            );
        }
        other => panic!("the span must be ended, got: {other:?}"),
    }
}

#[test]
fn annotate_current_span_attaches_to_the_running_span() {
    // Tracer.test.ts:337 — `annotateCurrentSpan` reaches the ambient span.
    let exit = Runtime::run(with_span(
        "A",
        SpanOptions::default(),
        annotate_current_span(vec![("key".to_owned(), "value".into())])
            .flat_map(|_| current_span()),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(
        span.attribute_value("key"),
        Some(AttributeValue::Text("value".to_owned()))
    );
}

#[test]
fn with_parent_span_sets_the_parent() {
    // Tracer.test.ts:376 — an explicit parent overrides the ambient one.
    let external = AnySpan::External(external_span("456", "123"));
    let exit = Runtime::run(with_parent_span(
        external,
        with_span("A", SpanOptions::default(), current_span()),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.parent().expect("parent").span_id(), "456");
}

#[test]
fn with_tracer_enabled_false_installs_a_noop_span() {
    // Tracer.test.ts:393 — a disabled tracer still names the span, but it is
    // the noop span; enabling it again gives a normal span.
    let disabled = Runtime::run(with_tracer_enabled(
        false,
        with_span("A", SpanOptions::default(), current_span()),
    ));
    let disabled = unwrap_success(disabled).expect("span A");
    assert_eq!(disabled.name(), "A");
    assert_eq!(disabled.span_id(), "noop");

    let enabled = Runtime::run(with_tracer_enabled(
        true,
        with_span("B", SpanOptions::default(), current_span()),
    ));
    let enabled = unwrap_success(enabled).expect("span B");
    assert_eq!(enabled.name(), "B");
    assert_ne!(enabled.span_id(), "noop");
}

#[test]
fn timings_are_recorded_when_enabled() {
    // Tracer.test.ts:411 — with timing on, the start time comes from the
    // ambient clock (nanoseconds).
    use elm_effect::{ClockImpl, Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    clock.adjust(Duration::millis(1.0));
    let handle: Arc<dyn ClockImpl> = clock.clone();
    let exit = Runtime::run(with_trace_clock(
        handle,
        with_span("A", SpanOptions::default(), current_span()),
    ));
    let span = unwrap_success(exit).expect("span A");

    match span.status() {
        SpanStatus::Ended { start_time, .. } => {
            assert_eq!(start_time, 1_000_000, "one millisecond, in nanoseconds");
        }
        other => panic!("expected an ended span, got: {other:?}"),
    }
}

#[test]
fn timings_are_zero_when_disabled() {
    // Tracer.test.ts:423 — with timing off, the start time is zero.
    use elm_effect::{ClockImpl, Duration, TestClock};

    let clock = Arc::new(TestClock::new());
    clock.adjust(Duration::millis(1.0));
    let handle: Arc<dyn ClockImpl> = clock.clone();
    let exit = Runtime::run(with_trace_clock(
        handle,
        with_tracer_timing(
            false,
            with_span("A", SpanOptions::default(), current_span()),
        ),
    ));
    let span = unwrap_success(exit).expect("span A");

    match span.status() {
        SpanStatus::Ended {
            start_time,
            end_time,
            ..
        } => {
            assert_eq!((start_time, end_time), (0, 0));
        }
        other => panic!("expected an ended span, got: {other:?}"),
    }
}

#[test]
fn timing_disabled_zeroes_start_and_end_for_both_constructors() {
    // Tracer.test.ts:435 — `useSpan` and `withSpan` agree when timing is off.
    let exit = Runtime::run(with_tracer_timing(
        false,
        use_span("useSpan", SpanOptions::default(), |span| {
            Effect::<Arc<dyn Span>, std::convert::Infallible>::succeed(span)
        })
        .flat_map(|used| {
            with_span("withSpan", SpanOptions::default(), current_span())
                .map(move |spanned| (used, spanned.expect("with span")))
        }),
    ));
    let (used, spanned) = unwrap_success(exit);

    let zeroes = |span: AnySpan| match span.status() {
        SpanStatus::Ended {
            start_time,
            end_time,
            ..
        } => (start_time, end_time),
        other => panic!("expected an ended span, got: {other:?}"),
    };
    assert_eq!(zeroes(used.into()), (0, 0));
    assert_eq!(zeroes(spanned), (0, 0));
}

#[test]
fn link_spans_links_two_spans() {
    // Tracer.test.ts:454 — a link from creation and one added by `linkSpans`
    // both end up on the span.
    let child_a = unwrap_success(Runtime::run(make_span("childA", SpanOptions::default())));
    let child_b = unwrap_success(Runtime::run(make_span("childB", SpanOptions::default())));

    let exit = Runtime::run(with_span(
        "A",
        SpanOptions {
            links: vec![SpanLink {
                span: child_b.clone().into(),
                attributes: Vec::new(),
            }],
            ..Default::default()
        },
        link_span(child_a.clone().into(), current_span()),
    ));
    let span = unwrap_success(exit).expect("span A");

    let ids: Vec<String> = span
        .links()
        .iter()
        .map(|link| link.span.span_id())
        .collect();
    assert_eq!(
        ids,
        vec![child_b.span_id(), child_a.span_id()],
        "creation links come first, then the added one"
    );
}

#[test]
fn disable_propagation_installs_a_noop_span() {
    // Tracer.test.ts:540 — a disabled span is named but has the noop id, and
    // propagation turns back on after its region.
    let disabled = Runtime::run(with_span(
        "A",
        SpanOptions {
            disable_propagation: true,
            ..Default::default()
        },
        current_span(),
    ));
    let disabled = unwrap_success(disabled).expect("span A");
    assert_eq!(disabled.name(), "A");
    assert_eq!(disabled.span_id(), "noop");

    let enabled = Runtime::run(with_span("B", SpanOptions::default(), current_span()));
    let enabled = unwrap_success(enabled).expect("span B");
    assert_eq!(enabled.name(), "B");
    assert_ne!(enabled.span_id(), "noop");
}

#[test]
fn a_disabled_span_is_skipped_as_a_parent() {
    // Tracer.test.ts:556 — the child of a disabled span points at the enabled
    // ancestor, not at the noop.
    let exit = Runtime::run(with_span(
        "parent",
        SpanOptions::default(),
        with_span(
            "disabled",
            SpanOptions {
                disable_propagation: true,
                ..Default::default()
            },
            with_span("child", SpanOptions::default(), current_span()),
        ),
    ));
    let child = unwrap_success(exit).expect("child span");

    assert_eq!(child.name(), "child");
    assert_eq!(child.parent().expect("parent").name(), "parent");
}

#[test]
fn a_custom_tracer_receives_the_creation_options() {
    // The `Tracer` contract itself: a backend sees the name and the resolved
    // options, and whatever span it returns becomes the current span.
    struct RecordingTracer {
        seen: Arc<Mutex<Vec<(String, SpanOptions)>>>,
    }

    impl Tracer for RecordingTracer {
        fn span(&self, name: &str, options: SpanOptions) -> Arc<dyn Span> {
            self.seen
                .lock()
                .expect("recorder")
                .push((name.to_owned(), options.clone()));
            Arc::new(NativeSpan::new(name, options))
        }
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let tracer = Arc::new(RecordingTracer {
        seen: Arc::clone(&seen),
    });
    let exit = Runtime::run(
        with_span(
            "custom",
            SpanOptions {
                kind: SpanKind::Client,
                ..Default::default()
            },
            current_span(),
        )
        .provide_reference(&elm_effect::tracer::CURRENT_TRACER, Some(tracer)),
    );
    let span = unwrap_success(exit).expect("span");

    assert_eq!(span.name(), "custom");
    let seen = seen.lock().expect("recorder");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "custom");
    assert_eq!(seen[0].1.kind, SpanKind::Client);
    assert_eq!(seen[0].1.sampled, Some(true));
}

/// The noop span is a real span value: it keeps its name and never samples
/// anything but what it was told. `NoopSpan` is public so a fake tracer can
/// reuse it.
#[test]
fn noop_span_keeps_its_name_and_sampling() {
    let span = NoopSpan::new("A", None, false);
    assert_eq!(span.name(), "A");
    assert_eq!(span.span_id(), "noop");
    assert!(!span.sampled());
}

// `CURRENT_SPAN` is part of the public surface: a caller can read the
// reference's key, which is the pin's `ParentSpanKey`.
#[test]
fn current_span_reference_uses_the_pin_key() {
    assert_eq!(CURRENT_SPAN.id, "effect/Tracer/ParentSpan");
}

#[test]
fn log_appends_a_span_event() {
    // Tracer.test.ts:183 — `Effect.log("event")` reaches the logger and, when
    // a span is current, lands on it as an event carrying the level.
    // The port's clock reports milliseconds, so the pin's 0.01 ms becomes
    // 10 ms (10_000_000 ns) to keep a non-zero timestamp.
    let clock = Arc::new(TestClock::new());
    clock.adjust(Duration::millis(10.0));
    let handle: Arc<dyn ClockImpl> = clock.clone();
    let logger = Logger::new(|_, _, _| {});

    let exit = Runtime::run(with_trace_clock(
        handle,
        with_span(
            "A",
            SpanOptions::default(),
            observability::log("event")
                .provide::<LoggerTag, ()>(logger)
                .flat_map(|()| current_span()),
        ),
    ));
    let span = unwrap_success(exit).expect("span A");

    assert_eq!(span.name(), "A");
    assert!(span.parent().is_none());
    let events = span.events();
    assert_eq!(events.len(), 1, "one log call, one event");
    assert_eq!(events[0].name, "event");
    assert_eq!(
        events[0].start_time, 10_000_000,
        "the trace clock timestamp"
    );
    assert_eq!(
        events[0].attributes,
        vec![(
            "effect.logLevel".to_owned(),
            AttributeValue::Text("INFO".to_owned())
        )],
        "outside a fiber there is no fiber id to record"
    );
}

#[test]
fn log_records_the_running_fiber_id_on_the_span_event() {
    // Tracer.test.ts:183 — inside a fiber the event also carries
    // `effect.fiberId`, the fiber that ran the log call.
    let runtime = Runtime::new();
    let logger = Logger::new(|_, _, _| {});
    let handle = runtime.spawn_effect(with_span(
        "A",
        SpanOptions::default(),
        observability::log("event")
            .provide::<LoggerTag, ()>(logger)
            .flat_map(|()| current_span()),
    ));
    let fiber_id = handle.id();
    let span = unwrap_success(handle.join()).expect("span A");

    let events = span.events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].attributes,
        vec![
            (
                "effect.fiberId".to_owned(),
                AttributeValue::Int(fiber_id.0 as i64)
            ),
            (
                "effect.logLevel".to_owned(),
                AttributeValue::Text("INFO".to_owned())
            ),
        ]
    );
}

#[test]
fn layer_parent_span_sets_the_parent_for_a_span_created_inside() {
    // Tracer.test.ts:472 — `Layer.parentSpan` makes an existing span ambient
    // for the layer it is installed into.
    let scope = Arc::new(Scope::new(1));
    let parent: Arc<dyn Span> = Arc::new(NativeSpan::new("parent", SpanOptions::default()));
    let layer = Layer::parent_span(AnySpan::from(Arc::clone(&parent)));
    let effect = make_span("child", SpanOptions::default()).map(|child| child.parent());

    let provided = layer.install(effect, &scope).expect("install");
    let seen = unwrap_success(Runtime::run(provided)).expect("parent");

    assert_eq!(seen.name(), "parent");
    assert_eq!(seen.span_id(), parent.span_id());
}

#[test]
fn layer_span_creates_the_parent_and_ends_it_when_the_scope_closes() {
    // Tracer.test.ts:488 — `Layer.span` creates a span for the layer's
    // constructors; the span is the parent of theirs, and `onEnd` runs when
    // the build scope closes.
    // The onEnd calls, in order: span name and the exit it saw.
    type Ended = Arc<Mutex<Vec<(String, Exit<(), String>)>>>;

    let scope = Arc::new(Scope::new(2));
    let ended: Ended = Arc::new(Mutex::new(Vec::new()));
    let on_end = {
        let ended = Arc::clone(&ended);
        Arc::new(move |span: Arc<dyn Span>, exit: Exit<(), String>| {
            ended
                .lock()
                .expect("onEnd")
                .push((span.name().to_owned(), exit));
        })
    };
    let layer = Layer::span(
        "parent",
        LayerSpanOptions {
            span: SpanOptions::default(),
            on_end: Some(on_end),
        },
    );
    let effect = make_span("child", SpanOptions::default()).map(|child| child.parent());

    let provided = layer.install(effect, &scope).expect("install");
    let parent = unwrap_success(Runtime::run(provided)).expect("parent");
    assert_eq!(parent.name(), "parent");
    assert!(matches!(parent.status(), SpanStatus::Started { .. }));
    assert!(ended.lock().expect("onEnd").is_empty(), "not ended yet");

    scope.close();
    assert!(
        matches!(parent.status(), SpanStatus::Ended { .. }),
        "the layer scope close ends the span"
    );
    assert_eq!(
        *ended.lock().expect("onEnd"),
        vec![("parent".to_owned(), Exit::Success(()))]
    );
}

#[test]
fn layer_with_span_wraps_construction_and_ends_the_span() {
    // Tracer.test.ts:514 — constructors of the wrapped layer run inside the
    // span, and the span ends when the build scope does.
    struct Probe;

    impl ServiceTag for Probe {
        type Service = String;
        const ID: &'static str = "test/tracer/probe";
    }

    let scope = Arc::new(Scope::new(3));
    let ended = Arc::new(Mutex::new(false));
    let on_end = {
        let ended = Arc::clone(&ended);
        Arc::new(move |_span: Arc<dyn Span>, _exit: Exit<(), String>| {
            *ended.lock().expect("onEnd") = true;
        })
    };
    let inner: Layer<Probe, std::convert::Infallible, ()> = Layer::from_factory(|| {
        current_span().map(|span| span.map(|span| span.name().to_owned()).unwrap_or_default())
    })
    .expect("no cycle");
    let layer = inner.with_span(
        "outer",
        LayerSpanOptions {
            span: SpanOptions::default(),
            on_end: Some(on_end),
        },
    );

    let built = layer.build(&scope).expect("build");
    assert_eq!(
        built.get::<Probe>().as_deref(),
        Some("outer"),
        "the constructor ran under the layer span"
    );
    assert!(!*ended.lock().expect("onEnd"));
    scope.close();
    assert!(*ended.lock().expect("onEnd"));
}
