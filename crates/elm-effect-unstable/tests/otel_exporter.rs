//! OTel exporter fence (feature `otel`): span name/attributes land in the
//! SDK with no collector. The recorder is the facade's own
//! `elm_effect::observability::SpanRecorder`; only name/attribute strings
//! cross the adapter boundary.

#![cfg(feature = "otel")]

use elm_effect::observability::SpanRecorder;
use elm_effect_unstable::otel::{ExportSpan, OtelExporter, attribute};

#[test]
fn exporter_records_span_name_and_attributes() {
    let tracer = SpanRecorder::new();
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

#[test]
fn otlp_env_headers_decode() {
    use elm_effect_unstable::otel::parse_otlp_headers;
    // OtlpEnvHeaders.test.ts:5 - percent-encoded header values decode.
    assert_eq!(
        parse_otlp_headers("authorization=Bearer%20token,x-comma=comma%2Cvalue"),
        [
            ("authorization".to_owned(), "Bearer token".to_owned()),
            ("x-comma".to_owned(), "comma,value".to_owned()),
        ]
    );
}

#[test]
fn otlp_resource_precedence() {
    use elm_effect_unstable::otel::{OtlpResourceOptions, otlp_resource_attributes};
    use std::collections::BTreeMap;
    let record =
        |pairs: Vec<(String, String)>| pairs.into_iter().collect::<BTreeMap<String, String>>();
    let empty = OtlpResourceOptions::default();
    // OtlpResource.test.ts:90 - dedicated service vars beat attributes text.
    assert_eq!(
        record(otlp_resource_attributes(
            Some("env-service"),
            Some("env-version"),
            Some("service.name=env-attribute-service,service.version=env-attribute-version"),
            &empty,
        )),
        BTreeMap::from([
            ("service.name".to_owned(), "env-service".to_owned()),
            ("service.version".to_owned(), "env-version".to_owned()),
        ])
    );
    // Explicit options beat everything per key.
    let options = OtlpResourceOptions {
        service_name: Some("explicit-service"),
        service_version: Some("explicit-version"),
        attributes: &[
            ("custom.attribute", "explicit"),
            ("service.name", "explicit-attribute-service"),
            ("service.version", "explicit-attribute-version"),
        ],
    };
    assert_eq!(
        record(otlp_resource_attributes(
            Some("env-service"),
            Some("env-version"),
            Some("service.name=env-attribute-service"),
            &options,
        )),
        BTreeMap::from([
            ("custom.attribute".to_owned(), "explicit".to_owned()),
            ("service.name".to_owned(), "explicit-service".to_owned()),
            ("service.version".to_owned(), "explicit-version".to_owned()),
        ])
    );
    // Unset service keys are omitted, never empty.
    assert_eq!(
        record(otlp_resource_attributes(None, None, None, &empty)),
        BTreeMap::new()
    );
}
