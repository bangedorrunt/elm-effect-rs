//! OpenTelemetry exporter adapter (feature `otel`).
//!
//! The adapter boundary is deliberate: it takes plain [`ExportSpan`] values
//! (name plus string attributes, the shape the stable `Tracer` records)
//! rather than depending on any particular recorder module, several of
//! which load test-only via `#[path]` includes. The SDK provider runs
//! in-process with its in-memory exporter, so tests assert span
//! name/attributes with no collector, no socket, no export pipeline.
//!
//! Residuals: OTLP transport, sampler/resource configuration, and metric
//! export (tracing only in this pass).

use opentelemetry::trace::{Span, Tracer as OtelTracer, TracerProvider as _};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

/// Plain span shape at the adapter boundary: name plus string attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSpan {
    /// Span name.
    pub name: String,
    /// Attribute pairs.
    pub attributes: Vec<(String, String)>,
}

impl ExportSpan {
    /// Build from a name and attribute pairs.
    pub fn new(name: &str, attributes: Vec<(String, String)>) -> Self {
        Self {
            name: name.to_owned(),
            attributes,
        }
    }
}

/// In-process exporter: SDK provider plus collected span data.
#[derive(Clone, Debug)]
pub struct OtelExporter {
    provider: SdkTracerProvider,
    exporter: InMemorySpanExporter,
}

impl OtelExporter {
    /// Provider with a synchronous processor (spans land on `end`).
    pub fn new() -> Self {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        Self { provider, exporter }
    }

    /// Export name/attributes as finished SDK spans.
    pub fn export_spans(&self, spans: &[ExportSpan]) {
        let scope = InstrumentationScope::builder("elm-effect-unstable").build();
        let tracer = self.provider.tracer_with_scope(scope);
        for span in spans {
            let mut active = tracer.start(span.name.clone());
            for (key, value) in &span.attributes {
                active.set_attribute(KeyValue::new(key.clone(), value.clone()));
            }
            active.end();
        }
    }

    /// Collected span data so far.
    pub fn exported(&self) -> Vec<SpanData> {
        self.exporter
            .get_finished_spans()
            .expect("in-memory export")
    }
}

impl Default for OtelExporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Find a string attribute by key.
pub fn attribute(data: &SpanData, key: &str) -> Option<String> {
    data.attributes.iter().find_map(|kv| {
        if kv.key.as_str() == key {
            Some(kv.value.to_string())
        } else {
            None
        }
    })
}

/// Decode `%XX` sequences, leaving `+` alone (OTLP header values
/// use URL-encoding without form semantics; distinct from the query
/// decoder on purpose).
fn decode_header_value(text: &str) -> String {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 1 {
            let hex = |b: u8| (b as char).to_digit(16);
            match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `k=v,k=v` OTLP header lists with percent-decoded values
/// (`OtlpEnv.headers` core; `=`-less members are skipped).
pub fn parse_otlp_headers(text: &str) -> Vec<(String, String)> {
    text.split(',')
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            Some((name.trim().to_owned(), decode_header_value(value)))
        })
        .collect()
}
