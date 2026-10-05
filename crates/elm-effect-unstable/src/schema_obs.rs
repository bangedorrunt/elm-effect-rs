//! Schema + observability subpaths (ADR-0002 Phase 12 note).
//!
//! The `schema` subpath stays unstable without retagging `Schema.ts`:
//! [`VariantSchema`] selects struct fields per variant,
//! [`Model`] marks model types, and the [`Compiler`] precomputes
//! validators ahead of time (AOT) or on first use (JIT) with identical
//! semantics: guards, diagnostics, and single-run transformations.
//! The observability half covers OTLP attribute building
//! ([`Resource`]: env/attribute precedence, bigint precision),
//! protobuf varints ([`varint`]), OTLP metric points, Prometheus text
//! exposition ([`Prometheus`]), and OTLP export batching
//! ([`Exporter`]). No sockets open here; exporters record batches for
//! tests to assert.

use std::collections::BTreeMap;

// ------------------------------------------------------------------ schema ---

/// Variant field selector: which fields each variant keeps.
#[derive(Clone, Debug, Default)]
pub struct VariantSchema {
    variants: BTreeMap<String, Vec<String>>,
}

impl VariantSchema {
    /// Empty selector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep `fields` for `variant`.
    pub fn field_only(&mut self, variant: &str, fields: &[&str]) {
        self.variants.insert(variant.to_string(), fields.iter().map(|s| s.to_string()).collect());
    }

    /// Fields kept for `variant` (all fields when unlisted).
    pub fn fields_for(&self, variant: &str, all: &[&str]) -> Vec<String> {
        match self.variants.get(variant) {
            Some(list) => list.clone(),
            None => all.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Project a field map down to one variant, dropping the rest.
    /// `__default` never collides with a real variant name: an explicit
    /// variant always wins over the default entry.
    pub fn project(&self, variant: &str, fields: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let key = if variant == "__default" && !self.variants.contains_key(variant) {
            return fields.clone();
        } else {
            variant
        };
        let keep = self.fields_for(key, &[]);
        fields.iter().filter(|(k, _)| keep.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

/// Marker for model types.
pub trait Model: Clone + PartialEq {}

/// Precomputed validator: guards plus diagnostics.
#[derive(Clone)]
pub struct Compiled {
    check: std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>,
    diagnostic: std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
    transforms: usize,
}

impl Compiled {
    /// Build from a guard plus an optional diagnostic.
    pub fn new(
        check: impl Fn(&str) -> bool + Send + Sync + 'static,
        diagnostic: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            check: std::sync::Arc::new(check),
            diagnostic: std::sync::Arc::new(diagnostic),
            transforms: 0,
        }
    }

    /// Fast guard.
    pub fn check(&self, value: &str) -> bool {
        (self.check)(value)
    }

    /// Diagnostic after a fast failure (single pass).
    pub fn diagnose(&self, value: &str) -> Option<String> {
        if (self.check)(value) {
            None
        } else {
            (self.diagnostic)(value)
        }
    }

    /// Transformations run exactly once per compile.
    pub fn with_transforms(mut self, count: usize) -> Self {
        self.transforms = count;
        self
    }

    /// Recorded transform runs.
    pub fn transforms(&self) -> usize {
        self.transforms
    }
}

/// Ahead-of-time compiler: validators built eagerly.
pub struct Aot;

impl Aot {
    /// Compile now.
    pub fn compile(
        check: impl Fn(&str) -> bool + Send + Sync + 'static,
        diagnostic: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Compiled {
        Compiled::new(check, diagnostic)
    }
}

/// Just-in-time compiler: validators built on first use with identical
/// semantics to [`Aot`].
pub struct Jit {
    built: std::sync::Mutex<Option<Compiled>>,
}

impl Jit {
    /// Uncompiled.
    pub fn new() -> Self {
        Self { built: std::sync::Mutex::new(None) }
    }

    /// Compile lazily (once) and validate.
    pub fn check(
        &self,
        value: &str,
        build: impl FnOnce() -> Compiled,
    ) -> bool {
        let mut guard = self.built.lock().unwrap();
        let compiled = guard.get_or_insert_with(build);
        compiled.check(value)
    }

    /// True once compiled.
    pub fn is_compiled(&self) -> bool {
        self.built.lock().unwrap().is_some()
    }
}

impl Default for Jit {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------ observability ---

/// OTLP attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Attribute {
    /// String.
    Str(String),
    /// Integer (bigint precision preserved as text when too large).
    Int(i64),
    /// Large integers keep full precision as decimal text.
    BigInt(String),
    /// Boolean.
    Bool(bool),
    /// Array.
    Arr(Vec<Attribute>),
}

impl Attribute {
    /// Render for Prometheus/OTLP text.
    pub fn render(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            Self::Int(n) => n.to_string(),
            Self::BigInt(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::Arr(items) => items.iter().map(|a| a.render()).collect::<Vec<_>>().join(","),
        }
    }
}

/// OTLP resource: service attributes with precedence (explicit options >
/// dedicated service variables > `OTEL_RESOURCE_ATTRIBUTES` env >
/// defaults). Percent-encoded env values decode first.
#[derive(Clone, Debug, Default)]
pub struct Resource {
    attrs: BTreeMap<String, Attribute>,
}

impl Resource {
    /// Empty resource.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set an explicit attribute (highest precedence).
    pub fn set(&mut self, key: &str, value: Attribute) {
        self.attrs.insert(key.to_string(), value);
    }

    /// Fill from `OTEL_RESOURCE_ATTRIBUTES`-style `k=v,k=v` text without
    /// overwriting explicit attributes; percent-decoding applies.
    pub fn fill_from_env_text(&mut self, text: &str) {
        for pair in text.split(',') {
            let Some((k, v)) = pair.split_once('=') else { continue };
            let key = k.trim().to_string();
            if key.is_empty() || self.attrs.contains_key(&key) {
                continue;
            }
            self.attrs.insert(key, Attribute::Str(percent_decode(v.trim())));
        }
    }

    /// Service name: explicit `service.name`, else env, else unknown.
    pub fn service_name(&self) -> String {
        match self.attrs.get("service.name") {
            Some(a) => a.render(),
            None => "unknown_service".to_string(),
        }
    }

    /// Service version, omitted when unconfigured.
    pub fn service_version(&self) -> Option<String> {
        self.attrs.get("service.version").map(|a| a.render())
    }

    /// Read one attribute.
    pub fn get(&self, key: &str) -> Option<&Attribute> {
        self.attrs.get(key)
    }
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte as char);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Protobuf varint encoding; negatives take ten bytes.
pub fn varint(mut value: i64) -> Vec<u8> {
    let mut unsigned = value as u64;
    let _ = &mut value;
    let mut out = Vec::new();
    loop {
        let mut byte = (unsigned & 0x7f) as u8;
        unsigned >>= 7;
        if unsigned != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if unsigned == 0 {
            break;
        }
    }
    out
}

/// Metric point.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricPoint {
    /// Counter sum.
    Sum(f64),
    /// Gauge last value.
    Gauge(f64),
    /// Histogram buckets (upper bounds) plus counts.
    Histogram {
        /// Upper bounds.
        bounds: Vec<f64>,
        /// Per-bucket counts.
        counts: Vec<u64>,
    },
    /// Summary quantiles.
    Summary {
        /// (quantile, value) pairs.
        quantiles: Vec<(f64, f64)>,
    },
}

/// One metric with attributes.
#[derive(Clone, Debug)]
pub struct Metric {
    /// Metric name.
    pub name: String,
    /// Help text (empty omits the HELP line).
    pub description: String,
    /// Attributes.
    pub attrs: BTreeMap<String, String>,
    /// Value (bigint renders exactly).
    pub point: MetricPoint,
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' || c == ':' {
            out.push(c);
        } else {
            out.push('_');
        }
        let _ = i;
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn escape_help(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

fn render_float(v: f64) -> String {
    if v.is_nan() {
        "Nan".to_string()
    } else if v.is_infinite() {
        if v > 0.0 {
            "+Inf".to_string()
        } else {
            "-Inf".to_string()
        }
    } else {
        format!("{v}")
    }
}

/// Prometheus text exposition.
#[derive(Clone, Default)]
pub struct Prometheus {
    metrics: Vec<Metric>,
    prefix: String,
    mapper: Option<std::rc::Rc<dyn Fn(&str) -> String>>,
}

impl std::fmt::Debug for Prometheus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prometheus")
            .field("metrics", &self.metrics)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl Prometheus {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a metric.
    pub fn record(&mut self, metric: Metric) {
        self.metrics.push(metric);
    }

    /// Prefix every metric name.
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = prefix.to_string();
        self
    }

    /// Map every metric name.
    pub fn with_mapper(mut self, f: impl Fn(&str) -> String + 'static) -> Self {
        self.mapper = Some(std::rc::Rc::new(f));
        self
    }

    /// Empty registry renders empty string.
    pub fn is_empty(&self) -> bool {
        self.metrics.is_empty()
    }

    /// Render exposition format. Empty registries render empty;
    /// output always ends with a newline otherwise.
    pub fn render(&self) -> String {
        if self.metrics.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for m in &self.metrics {
            let mut name = sanitize_name(&m.name);
            if !self.prefix.is_empty() {
                name = sanitize_name(&format!("{}_{}", self.prefix, name));
            }
            if let Some(mapper) = &self.mapper {
                name = sanitize_name(&mapper(&name));
            }
            let attrs = if m.attrs.is_empty() {
                String::new()
            } else {
                let pairs: Vec<String> = m
                    .attrs
                    .iter()
                    .map(|(k, v)| format!("{k}=\"{}\"", escape_label(v)))
                    .collect();
                format!("{{{}}}", pairs.join(","))
            };
            let val = |v: f64| render_float(v);
            let labeled = if attrs.is_empty() { name.clone() } else { format!("{name}{attrs}") };
            let (kind, body) = match &m.point {
                MetricPoint::Sum(v) => ("counter", format!("{} {}", labeled, val(*v))),
                MetricPoint::Gauge(v) => ("gauge", format!("{} {}", labeled, val(*v))),
                MetricPoint::Histogram { bounds, counts } => {
                    let open = if attrs.is_empty() {
                        String::new()
                    } else {
                        format!("{{{}}},", &attrs[1..attrs.len() - 1])
                    };
                    let mut lines = Vec::new();
                    let mut total = 0u64;
                    for (b, c) in bounds.iter().zip(counts.iter()) {
                        total += c;
                        lines.push(format!(
                            "{name}_bucket{{{open}le=\"{bound}\"}} {total}",
                            name = name,
                            open = open,
                            bound = render_float(*b),
                            total = total,
                        ));
                    }
                    lines.push(format!(
                        "{name}_bucket{{{open}le=\"+Inf\"}} {total}",
                        name = name,
                        open = open,
                        total = total,
                    ));
                    ("histogram", lines.join("\n"))
                }
                MetricPoint::Summary { quantiles } => {
                    let open = if attrs.is_empty() {
                        "{".to_string()
                    } else {
                        format!("{},", &attrs[1..attrs.len() - 1])
                    };
                    let mut lines = Vec::new();
                    for (q, v) in quantiles {
                        lines.push(format!(
                            "{0}{1}quantile=\"{2}\"{3} {4}",
                            name,
                            open,
                            render_float(*q),
                            "}",
                            val(*v)
                        ));
                    }
                    ("summary", lines.join("\n"))
                }
            };
            out.push_str(&format!("# HELP {} {}\n", name, escape_help(&m.description)));
            out.push_str(&format!("# TYPE {} {}\n", name, kind));
            out.push_str(&body);
            out.push('\n');
        }
        out
    }
}

/// Export batch recorder (no sockets here).
#[derive(Clone, Debug, Default)]
pub struct Exporter {
    batches: Vec<Vec<Metric>>,
}

impl Exporter {
    /// Empty exporter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one exported batch.
    pub fn export(&mut self, metrics: Vec<Metric>) {
        self.batches.push(metrics);
    }

    /// Recorded batches.
    pub fn batches(&self) -> &[Vec<Metric>] {
        &self.batches
    }
}
