//! `OtlpTracer` at `effect@4.0.0`: OTLP span identifiers, nested context, and
//! the status/event conversion the exporter consumes. The two identifier rows
//! are `Tracer.test.ts:50` and `Tracer.test.ts:142`; no pin test file links
//! `observability/OtlpTracer.ts` itself, so the conversion checks name the
//! behavior they witness.

use std::sync::{Arc, Mutex};

use elm_effect::otlp_tracer::{
    OtlpResource, OtlpSpan, OtlpSpanSink, OtlpTracerOptions, StatusCode, layer, make,
};
use elm_effect::tracer::{AnySpan, AttributeValue, SpanOptions, current_span, with_span};
use elm_effect::{Effect, Exit, Runtime};

fn is_hex_id(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn unwrap_success<A, E: std::fmt::Debug>(exit: Exit<A, E>) -> A {
    match exit {
        Exit::Success(value) => value,
        Exit::Failure(cause) => panic!("expected success, got failure: {cause:?}"),
    }
}

/// A sink that keeps every exported span.
fn recording_sink() -> (Arc<dyn OtlpSpanSink>, Arc<Mutex<Vec<OtlpSpan>>>) {
    let spans = Arc::new(Mutex::new(Vec::new()));
    let sink: Arc<dyn OtlpSpanSink> = {
        let spans = Arc::clone(&spans);
        Arc::new(move |span: OtlpSpan| spans.lock().expect("sink").push(span))
    };
    (sink, spans)
}

fn options(sink: Arc<dyn OtlpSpanSink>) -> OtlpTracerOptions {
    OtlpTracerOptions {
        url: "http://localhost:4318/v1/traces".to_owned(),
        resource: OtlpResource {
            service_name: "test-service".to_owned(),
            ..Default::default()
        },
        headers: Vec::new(),
        sink,
    }
}

#[test]
fn otlp_span_identifiers_are_hex_and_children_share_the_trace() {
    // Tracer.test.ts:50 — with OtlpTracer installed, a root mints a 32-char
    // trace id and a 16-char span id, and a child keeps the trace id.
    let (sink, exported) = recording_sink();
    let theta = make(options(sink));
    let effect = with_span(
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
    );

    let (root, child) = unwrap_success(Runtime::run(effect.provide_reference(
        &elm_effect::tracer::CURRENT_TRACER,
        Some(theta),
    )));

    assert!(is_hex_id(&root.trace_id(), 32), "trace id: {}", root.trace_id());
    assert!(is_hex_id(&root.span_id(), 16), "span id: {}", root.span_id());
    assert_eq!(child.trace_id(), root.trace_id());
    assert!(is_hex_id(&child.span_id(), 16));
    assert_ne!(child.span_id(), root.span_id());

    let exported = exported.lock().expect("sink");
    assert_eq!(exported.len(), 2, "both ended spans reach the sink");
    for span in exported.iter() {
        assert!(is_hex_id(&span.trace_id, 32));
        assert!(is_hex_id(&span.span_id, 16));
    }
}

#[test]
fn otlp_tracer_preserves_nested_span_context() {
    // Tracer.test.ts:142 — the tracer layer installs the backend and nested
    // spans stay parent and child.
    let (sink, exported) = recording_sink();
    let tracer_layer = layer(options(sink));
    let effect = with_span(
        "parent-span",
        SpanOptions {
            attributes: vec![("test".to_owned(), AttributeValue::Text("parent".to_owned()))],
            ..Default::default()
        },
        with_span(
            "child-span",
            SpanOptions {
                attributes: vec![("test".to_owned(), AttributeValue::Text("child".to_owned()))],
                ..Default::default()
            },
            Effect::<i32, String>::succeed(42),
        ),
    );
    let scope = Arc::new(elm_effect::Scope::new(11));
    let effect = tracer_layer.install(effect, &scope).expect("install");

    assert_eq!(unwrap_success(Runtime::run(effect)), 42);

    let exported = exported.lock().expect("sink");
    assert_eq!(exported.len(), 2);
    let parent = exported
        .iter()
        .find(|span| span.name == "parent-span")
        .expect("parent exported");
    let child = exported
        .iter()
        .find(|span| span.name == "child-span")
        .expect("child exported");
    assert_eq!(child.trace_id, parent.trace_id);
    assert_eq!(
        child.parent_span_id.as_deref(),
        Some(parent.span_id.as_str()),
        "the child names the parent span"
    );
    assert_eq!(
        parent.attributes,
        vec![(
            "test".to_owned(),
            elm_effect::otlp_tracer::OtlpValue::String("parent".to_owned())
        )]
    );
    assert_eq!(
        child.attributes,
        vec![(
            "test".to_owned(),
            elm_effect::otlp_tracer::OtlpValue::String("child".to_owned())
        )]
    );
}

#[test]
fn succeeded_spans_export_ok_status() {
    let (sink, exported) = recording_sink();
    let effect = with_span(
        "ok",
        SpanOptions::default(),
        Effect::<(), String>::succeed(()),
    )
        .provide_reference(&elm_effect::tracer::CURRENT_TRACER, Some(make(options(sink))));

    unwrap_success(Runtime::run(effect));
    let exported = exported.lock().expect("sink");
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].status.code, StatusCode::Ok);
    assert_eq!(exported[0].status.message, None);
    assert!(exported[0].events.is_empty());
}

#[test]
fn failed_spans_export_error_status_with_an_exception_event() {
    let (sink, exported) = recording_sink();
    let effect = with_span(
        "boom",
        SpanOptions::default(),
        Effect::<(), String>::fail("bad".to_owned()),
    )
    .provide_reference(&elm_effect::tracer::CURRENT_TRACER, Some(make(options(sink))));

    let exit = Runtime::run(effect);
    assert!(matches!(exit, Exit::Failure(_)), "the body still fails");

    let exported = exported.lock().expect("sink");
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].status.code, StatusCode::Error);
    // The port's span status stores the rendered cause, so the message is the
    // rendering rather than the raw failure value.
    assert!(
        exported[0]
            .status
            .message
            .as_deref()
            .is_some_and(|message| message.contains("bad")),
        "status message: {:?}",
        exported[0].status.message
    );
    let exception = exported[0]
        .events
        .iter()
        .find(|event| event.name == "exception")
        .expect("an exception event");
    assert!(
        exception.attributes.iter().any(|(key, value)| {
            key == "exception.message"
                && matches!(value, elm_effect::otlp_tracer::OtlpValue::String(text) if text.contains("bad"))
        }),
        "the exception names the failure"
    );
}

#[test]
fn unsampled_spans_are_not_exported() {
    let (sink, exported) = recording_sink();
    let effect = with_span(
        "quiet",
        SpanOptions {
            sampled: Some(false),
            ..Default::default()
        },
        Effect::<(), String>::succeed(()),
    )
    .provide_reference(&elm_effect::tracer::CURRENT_TRACER, Some(make(options(sink))));

    unwrap_success(Runtime::run(effect));
    assert!(exported.lock().expect("sink").is_empty());
}

#[test]
fn external_parents_keep_their_span_id() {
    // `externalSpan` is the port's remote-context parent; the OTLP record
    // names it rather than minting a parent.
    let (sink, exported) = recording_sink();
    let effect = with_span(
        "child",
        SpanOptions {
            parent: Some(AnySpan::External(elm_effect::tracer::external_span(
                "0000000000000000",
                "11111111111111111111111111111111",
            ))),
            ..Default::default()
        },
        Effect::<(), String>::succeed(()),
    )
    .provide_reference(&elm_effect::tracer::CURRENT_TRACER, Some(make(options(sink))));

    unwrap_success(Runtime::run(effect));
    let exported = exported.lock().expect("sink");
    assert_eq!(exported.len(), 1);
    assert_eq!(
        exported[0].parent_span_id.as_deref(),
        Some("0000000000000000")
    );
    assert_eq!(
        exported[0].trace_id, "11111111111111111111111111111111",
        "a child joins the remote trace"
    );
}

#[test]
fn kind_maps_to_the_otlp_number() {
    let (sink, exported) = recording_sink();
    let effect = with_span(
        "client",
        SpanOptions {
            kind: elm_effect::tracer::SpanKind::Client,
            ..Default::default()
        },
        Effect::<(), String>::succeed(()),
    )
    .provide_reference(&elm_effect::tracer::CURRENT_TRACER, Some(make(options(sink))));

    unwrap_success(Runtime::run(effect));
    assert_eq!(exported.lock().expect("sink")[0].kind, 3);
}
