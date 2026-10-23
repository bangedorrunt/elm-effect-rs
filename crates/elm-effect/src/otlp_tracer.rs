//! `observability/OtlpTracer.ts` at `effect@4.0.0`: a [`Tracer`] whose ended,
//! sampled spans are converted to OTLP span records and handed to a sink.
//!
//! The pin's `make` acquires the shared batch exporter and an
//! `OtlpSerialization` service; here the exporter is a [`OtlpSpanSink`] and
//! the JSON body is `OtlpSerialization`'s job, so this module stops at the
//! OTLP record shape. [`trace_data`] wraps recorded spans in the payload the
//! pin's exporter would send (`resourceSpans` → `scopeSpans` → `spans`).
//!
//! ## What is not carried
//!
//! - `context`: the pin's tracer hook into the interpreter has no seam here
//!   (see [`crate::tracer`]).
//! - Interrupt-only detection: a span records its outcome as an
//!   `Exit<(), String>` (the cause is already rendered), so an
//!   interrupt-only failure cannot be told from any other failure. Such a
//!   span exports as `Error`, not the pin's `Ok` + `status.interrupted`.
//! - Batch interval, batch size, shutdown timeout and HTTP headers are
//!   exporter configuration; the sink owns them.

use std::sync::{Arc, Mutex};

use crate::layer::Layer;
use crate::tracer::{
    AnySpan, AttributeValue, Span, SpanEvent, SpanKind, SpanLink, SpanOptions, SpanStatus, Tracer,
    TracerTag, native_tracer, random_hex,
};
use crate::Exit;

/// An OTLP attribute value. The pin stores `unknown` and lets serialization
/// decide; the port narrows to the four shapes a span can carry.
#[derive(Clone, Debug, PartialEq)]
pub enum OtlpValue {
    /// `stringValue`.
    String(String),
    /// `intValue`.
    Int(i64),
    /// `doubleValue`.
    Double(f64),
    /// `boolValue`.
    Bool(bool),
}

impl From<&AttributeValue> for OtlpValue {
    fn from(value: &AttributeValue) -> Self {
        match value {
            AttributeValue::Text(text) => OtlpValue::String(text.clone()),
            AttributeValue::Int(value) => OtlpValue::Int(*value),
            AttributeValue::Float(value) => OtlpValue::Double(*value),
            AttributeValue::Bool(value) => OtlpValue::Bool(*value),
        }
    }
}

/// An OTLP status code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum StatusCode {
    /// No explicit decision.
    #[default]
    Unset = 0,
    /// The wrapped operation succeeded.
    Ok = 1,
    /// The wrapped operation failed.
    Error = 2,
}

/// An OTLP span status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtlpStatus {
    /// The code.
    pub code: StatusCode,
    /// A human-readable message, when the code is `Ok` due to interruption
    /// or `Error`.
    pub message: Option<String>,
}

impl OtlpStatus {
    fn ok() -> Self {
        Self {
            code: StatusCode::Ok,
            message: None,
        }
    }

    fn error(message: Option<String>) -> Self {
        Self {
            code: StatusCode::Error,
            message,
        }
    }
}

/// One attribute in the OTLP payload.
pub type OtlpKeyValue = (String, OtlpValue);

/// One event on a span.
#[derive(Clone, Debug, PartialEq)]
pub struct OtlpEvent {
    /// The event name.
    pub name: String,
    /// Nanoseconds since the epoch.
    pub time_unix_nano: i64,
    /// Event attributes.
    pub attributes: Vec<OtlpKeyValue>,
}

/// One link from a span to another.
#[derive(Clone, Debug, PartialEq)]
pub struct OtlpLink {
    /// The linked span's trace id.
    pub trace_id: String,
    /// The linked span's id.
    pub span_id: String,
    /// Link attributes.
    pub attributes: Vec<OtlpKeyValue>,
}

/// One span in OTLP form, as `makeOtlpSpan` builds it.
///
/// The pin renders the 64-bit timestamps as strings because the OTLP/JSON
/// encoding does; this record keeps them as integers and leaves that
/// conversion to the serializer.
#[derive(Clone, Debug, PartialEq)]
pub struct OtlpSpan {
    /// The trace id (32 hex characters).
    pub trace_id: String,
    /// The span id (16 hex characters).
    pub span_id: String,
    /// The parent span id, when the span has a parent.
    pub parent_span_id: Option<String>,
    /// The span name.
    pub name: String,
    /// The OTLP kind number (1 internal … 5 consumer).
    pub kind: u8,
    /// Nanoseconds since the epoch.
    pub start_time_unix_nano: i64,
    /// Nanoseconds since the epoch.
    pub end_time_unix_nano: i64,
    /// Span attributes.
    pub attributes: Vec<OtlpKeyValue>,
    /// Recorded events, plus one `exception` per failure reason.
    pub events: Vec<OtlpEvent>,
    /// The outcome.
    pub status: OtlpStatus,
    /// Span links.
    pub links: Vec<OtlpLink>,
}

/// Where ended sampled spans go. The pin's batch exporter; a closure works
/// through the blanket impl, so tests capture without a socket.
pub trait OtlpSpanSink: Send + Sync {
    /// Receive one finished span.
    fn export(&self, span: OtlpSpan);
}

impl<F> OtlpSpanSink for F
where
    F: Fn(OtlpSpan) + Send + Sync,
{
    fn export(&self, span: OtlpSpan) {
        self(span)
    }
}

/// The resource the tracer reports its spans under (`OtlpResource`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OtlpResource {
    /// `service.name`.
    pub service_name: String,
    /// `service.version`.
    pub service_version: Option<String>,
    /// Additional resource attributes.
    pub attributes: Vec<OtlpKeyValue>,
}

/// The configuration `make` and `layer` take.
#[derive(Clone)]
pub struct OtlpTracerOptions {
    /// The collector's traces endpoint. Carried as metadata; the sink makes
    /// the request.
    pub url: String,
    /// The resource this tracer's spans belong to.
    pub resource: OtlpResource,
    /// Headers the sink should send (carried verbatim).
    pub headers: Vec<(String, String)>,
    /// Where ended sampled spans go.
    pub sink: Arc<dyn OtlpSpanSink>,
}

impl std::fmt::Debug for OtlpTracerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtlpTracerOptions")
            .field("url", &self.url)
            .field("resource", &self.resource)
            .field("headers", &self.headers.len())
            .finish_non_exhaustive()
    }
}

/// A [`Tracer`] that exports ended sampled spans as OTLP records.
#[derive(Clone)]
pub struct OtlpTracer {
    url: String,
    resource: OtlpResource,
    headers: Vec<(String, String)>,
    sink: Arc<dyn OtlpSpanSink>,
}

impl OtlpTracer {
    /// Build a tracer from explicit options.
    pub fn new(options: OtlpTracerOptions) -> Self {
        Self {
            url: options.url,
            resource: options.resource,
            headers: options.headers,
            sink: options.sink,
        }
    }

    /// The endpoint this tracer reports.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The resource this tracer reports.
    pub fn resource(&self) -> &OtlpResource {
        &self.resource
    }

    /// Headers for the sink.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
}

/// Build a tracer from explicit options (`OtlpTracer.make`).
pub fn make(options: OtlpTracerOptions) -> Arc<dyn Tracer> {
    Arc::new(OtlpTracer::new(options))
}

/// A layer providing the tracer for the effects it is installed into
/// (`OtlpTracer.layer`).
pub fn layer(options: OtlpTracerOptions) -> Layer<TracerTag, std::convert::Infallible, ()> {
    Layer::tracer(make(options))
}

/// A layer built from the OpenTelemetry environment (`layerFromConfig`).
///
/// Recognised variables: `OTEL_SDK_DISABLED`,
/// `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` (falling back to
/// `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/traces`), `OTEL_TRACES_EXPORTER`, and
/// `OTEL_EXPORTER_OTLP_TRACES_HEADERS` / `OTEL_EXPORTER_OTLP_HEADERS`
/// (`key=value` pairs, comma-separated). Disabled, no endpoint, or an
/// exporter list without `otlp` installs the native tracer, which is what an
/// absent tracer resolves to anyway.
pub fn layer_from_config(
    resource: OtlpResource,
    sink: Arc<dyn OtlpSpanSink>,
) -> Layer<TracerTag, std::convert::Infallible, ()> {
    let options = options_from_env(resource, sink);
    match options {
        Some(options) => layer(options),
        None => Layer::tracer(native_tracer()),
    }
}

/// Read the OTLP tracer configuration from the environment.
pub fn options_from_env(
    resource: OtlpResource,
    sink: Arc<dyn OtlpSpanSink>,
) -> Option<OtlpTracerOptions> {
    if env_flag("OTEL_SDK_DISABLED") {
        return None;
    }
    let exporters = env_string("OTEL_TRACES_EXPORTER").unwrap_or_else(|| "otlp".to_owned());
    if !exporters
        .split(',')
        .map(str::trim)
        .any(|name| name == "otlp" || name == "otlp-http")
    {
        return None;
    }
    let url = env_string("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").or_else(|| {
        env_string("OTEL_EXPORTER_OTLP_ENDPOINT")
            .map(|base| format!("{}/v1/traces", base.trim_end_matches('/')))
    })?;
    let headers = env_string("OTEL_EXPORTER_OTLP_TRACES_HEADERS")
        .or_else(|| env_string("OTEL_EXPORTER_OTLP_HEADERS"))
        .map(|raw| {
            raw.split(',')
                .filter_map(|pair| pair.split_once('='))
                .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    Some(OtlpTracerOptions {
        url,
        resource,
        headers,
        sink,
    })
}

fn env_string(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    (!value.is_empty()).then_some(value)
}

fn env_flag(name: &str) -> bool {
    matches!(
        env_string(name).as_deref(),
        Some("1" | "true" | "TRUE" | "True")
    )
}

/// Group recorded spans into the payload the pin's exporter sends
/// (`TraceData`).
pub fn trace_data(resource: OtlpResource, scope_name: &str, spans: Vec<OtlpSpan>) -> TraceData {
    TraceData {
        resource_spans: vec![ResourceSpan {
            resource,
            scope_spans: vec![ScopeSpan {
                scope_name: scope_name.to_owned(),
                spans,
                schema_url: None,
            }],
            schema_url: None,
        }],
    }
}

/// The root OTLP traces payload.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceData {
    /// Spans grouped by resource.
    pub resource_spans: Vec<ResourceSpan>,
}

/// One resource's spans.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceSpan {
    /// The resource.
    pub resource: OtlpResource,
    /// The spans, grouped by instrumentation scope.
    pub scope_spans: Vec<ScopeSpan>,
    /// Optional schema url.
    pub schema_url: Option<String>,
}

/// One instrumentation scope's spans.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeSpan {
    /// The scope name (the pin uses the resource's service name).
    pub scope_name: String,
    /// The spans.
    pub spans: Vec<OtlpSpan>,
    /// Optional schema url.
    pub schema_url: Option<String>,
}

impl Tracer for OtlpTracer {
    fn span(&self, name: &str, options: SpanOptions) -> Arc<dyn Span> {
        Arc::new(OtlpSpanHandle {
            name: name.to_owned(),
            parent: options.parent,
            kind: options.kind,
            sampled: options.sampled.unwrap_or(true),
            status: Mutex::new(SpanStatus::Started {
                start_time: options.start_time,
            }),
            attributes: Mutex::new(options.attributes),
            events: Mutex::new(Vec::new()),
            links: Mutex::new(options.links),
            trace_id: Mutex::new(None),
            span_id: Mutex::new(None),
            sink: Arc::clone(&self.sink),
        })
    }
}

/// The port's `SpanImpl`: identifiers are minted on first read, and the span
/// exports itself when it ends.
struct OtlpSpanHandle {
    name: String,
    parent: Option<AnySpan>,
    kind: SpanKind,
    sampled: bool,
    status: Mutex<SpanStatus>,
    attributes: Mutex<Vec<(String, AttributeValue)>>,
    events: Mutex<Vec<SpanEvent>>,
    links: Mutex<Vec<SpanLink>>,
    trace_id: Mutex<Option<String>>,
    span_id: Mutex<Option<String>>,
    sink: Arc<dyn OtlpSpanSink>,
}

impl std::fmt::Debug for OtlpSpanHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OtlpSpan({})", self.name)
    }
}

impl OtlpSpanHandle {
    fn guard<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn minted_trace_id(&self) -> String {
        let mut slot = Self::guard(&self.trace_id);
        slot.get_or_insert_with(|| {
            self.parent
                .as_ref()
                .map(AnySpan::trace_id)
                .unwrap_or_else(|| random_hex(32))
        })
        .clone()
    }

    fn minted_span_id(&self) -> String {
        let mut slot = Self::guard(&self.span_id);
        slot.get_or_insert_with(|| random_hex(16)).clone()
    }

    /// Convert to the OTLP record (`makeOtlpSpan`). Only meaningful once the
    /// status is `Ended`.
    fn to_otlp(&self) -> OtlpSpan {
        let status = Self::guard(&self.status).clone();
        let SpanStatus::Ended {
            start_time,
            end_time,
            exit,
        } = status
        else {
            panic!("an OTLP span exports only after it ended");
        };

        let attributes: Vec<OtlpKeyValue> = Self::guard(&self.attributes)
            .iter()
            .map(|(key, value)| (key.clone(), OtlpValue::from(value)))
            .collect();
        let mut events: Vec<OtlpEvent> = Self::guard(&self.events)
            .iter()
            .map(|event| OtlpEvent {
                name: event.name.clone(),
                time_unix_nano: event.start_time,
                attributes: event
                    .attributes
                    .iter()
                    .map(|(key, value)| (key.clone(), OtlpValue::from(value)))
                    .collect(),
            })
            .collect();

        let otlp_status = match &exit {
            Exit::Success(()) => OtlpStatus::ok(),
            Exit::Failure(cause) => {
                let errors = cause.pretty_errors();
                let message = errors.first().map(|error| error.to_string());
                for error in errors {
                    events.push(OtlpEvent {
                        name: "exception".to_owned(),
                        time_unix_nano: end_time,
                        attributes: vec![
                            (
                                "exception.type".to_owned(),
                                OtlpValue::String("Error".to_owned()),
                            ),
                            (
                                "exception.message".to_owned(),
                                OtlpValue::String(error.to_string()),
                            ),
                            (
                                "exception.stacktrace".to_owned(),
                                OtlpValue::String("No stack trace available".to_owned()),
                            ),
                        ],
                    });
                }
                OtlpStatus::error(message)
            }
        };

        let links = Self::guard(&self.links)
            .iter()
            .map(|link| OtlpLink {
                trace_id: link.span.trace_id(),
                span_id: link.span.span_id(),
                attributes: link
                    .attributes
                    .iter()
                    .map(|(key, value)| (key.clone(), OtlpValue::from(value)))
                    .collect(),
            })
            .collect();

        OtlpSpan {
            trace_id: self.minted_trace_id(),
            span_id: self.minted_span_id(),
            parent_span_id: self.parent.as_ref().map(AnySpan::span_id),
            name: self.name.clone(),
            kind: otlp_kind(self.kind),
            start_time_unix_nano: start_time,
            end_time_unix_nano: end_time,
            attributes,
            events,
            status: otlp_status,
            links,
        }
    }
}

fn otlp_kind(kind: SpanKind) -> u8 {
    match kind {
        SpanKind::Internal => 1,
        SpanKind::Server => 2,
        SpanKind::Client => 3,
        SpanKind::Producer => 4,
        SpanKind::Consumer => 5,
    }
}

impl Span for OtlpSpanHandle {
    fn name(&self) -> &str {
        &self.name
    }

    fn span_id(&self) -> String {
        self.minted_span_id()
    }

    fn trace_id(&self) -> String {
        self.minted_trace_id()
    }

    fn parent(&self) -> Option<AnySpan> {
        self.parent.clone()
    }

    fn status(&self) -> SpanStatus {
        Self::guard(&self.status).clone()
    }

    fn attributes(&self) -> Vec<(String, AttributeValue)> {
        Self::guard(&self.attributes).clone()
    }

    fn attribute(&self, key: &str, value: AttributeValue) {
        let mut attributes = Self::guard(&self.attributes);
        match attributes.iter_mut().find(|(name, _)| name == key) {
            Some(slot) => slot.1 = value,
            None => attributes.push((key.to_owned(), value)),
        }
    }

    fn events(&self) -> Vec<SpanEvent> {
        Self::guard(&self.events).clone()
    }

    fn event(&self, name: &str, start_time: i64, attributes: Vec<(String, AttributeValue)>) {
        Self::guard(&self.events).push(SpanEvent {
            name: name.to_owned(),
            start_time,
            attributes,
        });
    }

    fn links(&self) -> Vec<SpanLink> {
        Self::guard(&self.links).clone()
    }

    fn add_links(&self, links: Vec<SpanLink>) {
        Self::guard(&self.links).extend(links);
    }

    fn sampled(&self) -> bool {
        self.sampled
    }

    fn kind(&self) -> SpanKind {
        self.kind
    }

    fn end(&self, end_time: i64, exit: Exit<(), String>) {
        let ended = {
            let mut status = Self::guard(&self.status);
            match *status {
                SpanStatus::Started { start_time } => {
                    *status = SpanStatus::Ended {
                        start_time,
                        end_time,
                        exit,
                    };
                    true
                }
                SpanStatus::Ended { .. } => false,
            }
        };
        if ended && self.sampled {
            self.sink.export(self.to_otlp());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_numbers_follow_the_otlp_enum() {
        assert_eq!(otlp_kind(SpanKind::Internal), 1);
        assert_eq!(otlp_kind(SpanKind::Consumer), 5);
    }
}
