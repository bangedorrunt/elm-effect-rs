//! OTel exporter fence (feature `otel`): span name/attributes land in the
//! SDK with no collector. The stable recorder loads via `#[path]` (it is
//! test-wired, not facade-wired); only name/attribute strings cross the
//! adapter boundary.

#![cfg(feature = "otel")]

#[path = "../../elm-effect/src/observability.rs"]
mod observability;

use elm_effect_unstable::otel::{ExportSpan, OtelExporter, attribute};
use observability::Tracer;

#[test]
fn exporter_records_span_name_and_attributes() {
    let tracer = Tracer::new();
    tracer.record(
        "db.query",
        vec![("db.system".to_owned(), "postgres".to_owned())],
    );
    tracer.record("http.server", vec![]);
    let spans: Vec<ExportSpan> = tracer
        .spans()
        .iter()
        .map(|s| ExportSpan::new(&s.name, s.attributes.clone()))
        .collect();
    let exporter = OtelExporter::new();
    exporter.export_spans(&spans);
    let exported = exporter.exported();
    assert_eq!(exported.len(), 2);
    assert_eq!(exported[0].name, "db.query");
    assert_eq!(
        attribute(&exported[0], "db.system"),
        Some("postgres".to_owned())
    );
    assert_eq!(exported[1].name, "http.server");
    assert_eq!(attribute(&exported[1], "db.system"), None);
}
