//! Wave-12 schema/observability fence: variants, compilers, OTLP,
//! Prometheus. Loads `schema_obs.rs` by path until the feature lands.

#[path = "../src/schema_obs.rs"]
mod schema_obs;

use schema_obs::{Aot, Attribute, Exporter, Jit, Metric, MetricPoint, Prometheus, Resource, VariantSchema, varint};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- variants ---

#[test]
fn variant_field_selection() {
    let mut v = VariantSchema::new();
    v.field_only("Circle", &["radius"]);
    let all = ["kind", "radius", "width"];
    assert_eq!(v.fields_for("Circle", &all), vec!["radius".to_string()]);
    assert_eq!(
        v.fields_for("Other", &all),
        vec!["kind".to_string(), "radius".to_string(), "width".to_string()]
    );
}

#[test]
fn variant_default_never_collides() {
    let v = VariantSchema::new();
    let mut fields = BTreeMap::new();
    fields.insert("a".to_string(), "1".to_string());
    // Unlisted `__default` keeps everything.
    assert_eq!(v.project("__default", &fields), fields);
}

#[test]
fn variant_projection_drops_rest() {
    let mut v = VariantSchema::new();
    v.field_only("Rect", &["width"]);
    let mut fields = BTreeMap::new();
    fields.insert("width".to_string(), "3".to_string());
    fields.insert("radius".to_string(), "9".to_string());
    let out = v.project("Rect", &fields);
    assert_eq!(out.len(), 1);
    assert_eq!(out.get("width"), Some(&"3".to_string()));
}

// ---------------------------------------------------------------- compilers ---

#[test]
fn aot_guard_and_diagnostic() {
    let c = Aot::compile(|v: &str| v == "ok", |v: &str| {
        if v == "ok" {
            None
        } else {
            Some(format!("expected ok, got {v:?}"))
        }
    });
    assert!(c.check("ok"));
    assert!(!c.check("nope"));
    assert_eq!(c.diagnose("nope"), Some("expected ok, got \"nope\"".to_string()));
    assert_eq!(c.diagnose("ok"), None);
}

#[test]
fn jit_compiles_lazily_with_same_semantics() {
    let jit = Jit::new();
    assert!(!jit.is_compiled());
    assert!(jit.check("ok", || Aot::compile(|v: &str| v == "ok", |_| None)));
    assert!(jit.is_compiled());
    assert!(!jit.check("nope", || Aot::compile(|_: &str| true, |_| None)));
}

#[test]
fn transforms_run_once() {
    let c = Aot::compile(|_: &str| true, |_| None).with_transforms(1);
    assert_eq!(c.transforms(), 1);
}

// -------------------------------------------------------------------- OTLP ---

#[test]
fn varint_negative_ten_bytes() {
    assert_eq!(varint(-1).len(), 10);
    assert_eq!(varint(0), vec![0]);
    assert_eq!(varint(1), vec![1]);
    assert_eq!(varint(300), vec![0xac, 0x02]);
}

#[test]
fn resource_percent_env_and_precedence() {
    let mut r = Resource::new();
    r.fill_from_env_text("service.name=env%20svc,other=1");
    assert_eq!(r.service_name(), "env svc");
    r.set("service.name", Attribute::Str("explicit".to_string()));
    assert_eq!(r.service_name(), "explicit");
    assert_eq!(r.service_version(), None);
    r.set("service.version", Attribute::Str("1.2".to_string()));
    assert_eq!(r.service_version(), Some("1.2".to_string()));
}

#[test]
fn resource_bigint_precision() {
    let mut r = Resource::new();
    r.set("big", Attribute::BigInt("9007199254740993".to_string()));
    assert_eq!(
        r.get("big"),
        Some(&Attribute::BigInt("9007199254740993".to_string()))
    );
    assert_eq!(r.service_name(), "unknown_service");
}

#[test]
fn exporter_batches_metrics() {
    let mut e = Exporter::new();
    e.export(vec![Metric {
        name: "x".to_string(),
        description: String::new(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Sum(1.0),
    }]);
    assert_eq!(e.batches().len(), 1);
}

// -------------------------------------------------------------- Prometheus ---

#[test]
fn prometheus_options_and_specials() {
    use schema_obs::{Metric, MetricPoint, Prometheus};
    use std::collections::BTreeMap;
    // Prefix + mapper compose.
    let mut p = Prometheus::new()
        .with_prefix("app")
        .with_mapper(|n| format!("{n}_total"));
    p.record(Metric {
        name: "hits".to_string(),
        description: String::new(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Sum(1.0),
    });
    let text = p.render();
    assert!(text.contains("app_hits_total 1"), "{text}");
    // Sanitization, escaping, NaN/Inf, empty registry, trailing newline.
    assert_eq!(Prometheus::new().render(), "");
    let mut p = Prometheus::new();
    let mut attrs = BTreeMap::new();
    attrs.insert("path".to_string(), "a\"b\\c".to_string());
    p.record(Metric {
        name: "9lives!".to_string(),
        description: "line1\nline2".to_string(),
        attrs,
        point: MetricPoint::Gauge(f64::NAN),
    });
    p.record(Metric {
        name: "up".to_string(),
        description: String::new(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Gauge(f64::INFINITY),
    });
    p.record(Metric {
        name: "down".to_string(),
        description: String::new(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Gauge(f64::NEG_INFINITY),
    });
    let text = p.render();
    assert!(text.contains("_9lives_"), "{text}");
    assert!(text.contains("path=\"a\\\"b\\\\c\""), "{text}");
    assert!(text.contains("line1\\nline2"), "{text}");
    assert!(text.contains("Nan") && text.contains("+Inf") && text.contains("-Inf"), "{text}");
    assert!(text.ends_with('\n'));
}

#[test]
fn prometheus_counter_shapes() {
    let mut p = Prometheus::new();
    p.record(Metric {
        name: "hits".to_string(),
        description: "total hits".to_string(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Sum(7.0),
    });
    let text = p.render();
    assert!(text.contains("# HELP hits total hits"));
    assert!(text.contains("# TYPE hits counter"));
    assert!(text.contains("hits 7"));
}

#[test]
fn prometheus_attributes_and_gauge() {
    let mut attrs = BTreeMap::new();
    attrs.insert("path".to_string(), "/".to_string());
    let mut p = Prometheus::new();
    p.record(Metric {
        name: "temp".to_string(),
        description: String::new(),
        attrs,
        point: MetricPoint::Gauge(-3.5),
    });
    let text = p.render();
    assert!(text.contains("temp{path=\"/\"} -3.5"));
}

#[test]
fn prometheus_histogram_and_summary() {
    let mut p = Prometheus::new();
    p.record(Metric {
        name: "lat".to_string(),
        description: String::new(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Histogram {
            bounds: vec![10.0, 20.0],
            counts: vec![2, 3],
        },
    });
    p.record(Metric {
        name: "q".to_string(),
        description: String::new(),
        attrs: BTreeMap::new(),
        point: MetricPoint::Summary { quantiles: vec![(0.5, 5.0)] },
    });
    let text = p.render();
    assert!(text.contains("lat_bucket{le=\"10\"} 2"));
    assert!(text.contains("lat_bucket{le=\"+Inf\"} 5"));
    assert!(text.contains("q{quantile=\"0.5\"} 5"));
}
