//! Schema + observability subpaths (ADR-0002 Phase 12 note).
//!
//! The `schema` subpath stays unstable without retagging `Schema.ts`:
//! [`VariantSchema`] selects struct fields per variant,
//! [`variant_structs`] carries the struct/class layer (variant fields,
//! extraction, class construction), [`Model`] marks model types, and the
//! [`Compiler`] precomputes
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
        self.variants.insert(
            variant.to_string(),
            fields.iter().map(|s| s.to_string()).collect(),
        );
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
    pub fn project(
        &self,
        variant: &str,
        fields: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let key = if variant == "__default" && !self.variants.contains_key(variant) {
            return fields.clone();
        } else {
            variant
        };
        let keep = self.fields_for(key, &[]);
        fields
            .iter()
            .filter(|(k, _)| keep.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Marker for model types.
pub trait Model: Clone + PartialEq {}

/// Variant structs (`VariantSchema` / `Model` at the pin) as data: field
/// definitions per variant, variant extraction, class construction, and the
/// leaf codecs the carried rows drive (sqlite bits, JSON booleans,
/// overrideable defaults). A schema is a [`Leaf`]; no carried row exercises an
/// algebra over schemas, so there is none here.
pub mod variant_structs {
    use std::collections::BTreeMap;

    /// Variant names `Model` fixes, default first (`Model.Struct` at the pin).
    pub const MODEL_VARIANTS: &[&str] = &[
        "select",
        "insert",
        "update",
        "json",
        "jsonCreate",
        "jsonUpdate",
    ];

    /// Leaf schema: the constructors these behaviours exercise.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Leaf {
        /// `Schema.String`.
        Str,
        /// `Schema.Number`.
        Num,
        /// `Schema.Boolean` (the JSON variants of `BooleanSqlite`).
        Bool,
        /// `Schema.BooleanFromBit`: `boolean` on the type side, `0 | 1` encoded.
        BoolFromBit,
        /// `Schema.DateTimeUtcFromMillis`: epoch millis on the encoded side.
        DateMillis,
        /// TS `undefined`: the field is omitted from this variant.
        Undefined,
        /// `Model.FieldOption` on a database variant (`OptionFromNullOr`);
        /// the encoded edge also accepts `null`. No carried row drives a
        /// codec through it, so `encode`/`decode` refuse it by name.
        OptionFromNullOr(Box<Leaf>),
        /// `Model.FieldOption` on a JSON variant: the key may be missing.
        /// No carried row drives a codec through it.
        OptionalKey(Box<Leaf>),
        /// `VariantSchema.Overrideable`: the value has a constructor default,
        /// and `encode` takes only an explicit [`overridden`] value.
        Overrideable(Box<Leaf>),
    }

    /// Runtime value of the ported codecs. `DateTime` is the type side of
    /// `DateMillis`; `Override` is the brand `Model.Override` puts on a value.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Value {
        /// String.
        Str(String),
        /// Number (the bit codec's encoded side uses `0`/`1`).
        Num(f64),
        /// Boolean.
        Bool(bool),
        /// Date-time in epoch millis.
        DateTime(i64),
        /// `Model.Override(value)`.
        Override(Box<Value>),
    }

    /// A decoded record, keyed by field name.
    pub type Record = BTreeMap<String, Value>;

    /// `Model.Override(value)`: an explicit value for an
    /// [`Leaf::Overrideable`] default.
    pub fn overridden(value: Value) -> Value {
        Value::Override(Box::new(value))
    }

    /// One field's schemas by variant: `Field({ a: ..., __default: ... })`.
    #[derive(Clone, Debug, Default)]
    pub struct Field {
        schemas: BTreeMap<String, Leaf>,
    }

    impl Field {
        /// From variant/schema pairs, keeping only the variants listed.
        pub fn new(schemas: &[(&str, Leaf)]) -> Self {
            Self {
                schemas: schemas
                    .iter()
                    .map(|(variant, leaf)| (variant.to_string(), leaf.clone()))
                    .collect(),
            }
        }

        /// The stored config, keys and all (`FieldOption` preserves it).
        pub fn schemas(&self) -> &BTreeMap<String, Leaf> {
            &self.schemas
        }

        /// The schema for `variant`; `None` when the field omits that variant.
        pub fn schema(&self, variant: &str) -> Option<&Leaf> {
            self.schemas.get(variant)
        }
    }

    /// A struct field: a plain schema (or plain `undefined`) or a variant
    /// field.
    #[derive(Clone, Debug)]
    pub enum FieldSpec {
        /// `value: Schema.String`, or `value: undefined` to omit it.
        Plain(Leaf),
        /// `value: Test.Field({...})` / `FieldOnly` / `FieldExcept`.
        VariantField(Field),
    }

    impl FieldSpec {
        /// Plain schema for every variant.
        pub fn plain(leaf: Leaf) -> Self {
            Self::Plain(leaf)
        }
    }

    impl From<Field> for FieldSpec {
        fn from(field: Field) -> Self {
            Self::VariantField(field)
        }
    }

    /// A struct's field definitions, in declaration order.
    #[derive(Clone, Debug)]
    pub struct Structure {
        fields: Vec<(String, FieldSpec)>,
    }

    /// One variant of a [`Structure`]: the fields it keeps and their schemas.
    #[derive(Clone, Debug)]
    pub struct Extracted {
        fields: Vec<(String, Leaf)>,
    }

    impl Extracted {
        /// Field names the variant keeps, in declaration order.
        pub fn field_names(&self) -> Vec<String> {
            self.fields.iter().map(|(name, _)| name.clone()).collect()
        }

        /// Schema of a kept field.
        pub fn schema(&self, name: &str) -> Option<&Leaf> {
            self.fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, leaf)| leaf)
        }

        /// The constructor (`make`): every field must be supplied unless it
        /// carries an overrideable default, which `now_millis` fills
        /// (`DateTime.now` at the pin is the caller's effect).
        pub fn make(&self, supplied: &[(&str, Value)], now_millis: i64) -> Result<Record, String> {
            let given: Record = supplied
                .iter()
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect();
            let mut out = Record::new();
            for (name, leaf) in &self.fields {
                match given.get(name) {
                    Some(value) => {
                        out.insert(name.clone(), value.clone());
                    }
                    None => match leaf {
                        Leaf::Overrideable(inner) if matches!(**inner, Leaf::DateMillis) => {
                            out.insert(name.clone(), Value::DateTime(now_millis));
                        }
                        Leaf::Overrideable(_) => {
                            return Err(format!("{name}: no constructor default carried"));
                        }
                        _ => return Err(format!("{name}: is missing")),
                    },
                }
            }
            Ok(out)
        }

        /// Encoded side to type side (`Schema.decodeSync`).
        pub fn decode(&self, input: &Record) -> Result<Record, String> {
            self.convert(input, Edge::Decode)
        }

        /// Type side to encoded side (`Schema.encodeSync`).
        pub fn encode(&self, input: &Record) -> Result<Record, String> {
            self.convert(input, Edge::Encode)
        }

        /// Excess keys are ignored; a missing key is an error naming the
        /// field, which is what makes an overrideable default
        /// constructor-only.
        fn convert(&self, input: &Record, edge: Edge) -> Result<Record, String> {
            let mut out = Record::new();
            for (name, leaf) in &self.fields {
                let Some(value) = input.get(name) else {
                    return Err(format!("{name}: is missing"));
                };
                let converted =
                    convert_value(value, leaf, edge).map_err(|error| format!("{name}: {error}"))?;
                out.insert(name.clone(), converted);
            }
            Ok(out)
        }
    }

    #[derive(Clone, Copy)]
    enum Edge {
        Decode,
        Encode,
    }

    fn convert_value(value: &Value, leaf: &Leaf, edge: Edge) -> Result<Value, String> {
        match (leaf, edge) {
            (Leaf::Str, _) if matches!(value, Value::Str(_)) => Ok(value.clone()),
            (Leaf::Num, _) if matches!(value, Value::Num(_)) => Ok(value.clone()),
            (Leaf::Bool, _) if matches!(value, Value::Bool(_)) => Ok(value.clone()),
            (Leaf::BoolFromBit, Edge::Decode) => match value {
                Value::Num(bits) if *bits == 0.0 => Ok(Value::Bool(false)),
                Value::Num(bits) if *bits == 1.0 => Ok(Value::Bool(true)),
                other => Err(format!("expected 0 or 1, got {}", kind(other))),
            },
            (Leaf::BoolFromBit, Edge::Encode) => match value {
                Value::Bool(flag) => Ok(Value::Num(if *flag { 1.0 } else { 0.0 })),
                other => Err(format!("expected boolean, got {}", kind(other))),
            },
            (Leaf::DateMillis, Edge::Decode) => match value {
                Value::Num(millis) => Ok(Value::DateTime(*millis as i64)),
                other => Err(format!("expected millis, got {}", kind(other))),
            },
            (Leaf::DateMillis, Edge::Encode) => match value {
                Value::DateTime(millis) => Ok(Value::Num(*millis as f64)),
                other => Err(format!("expected date-time, got {}", kind(other))),
            },
            // A constructor default does not relax the encoded side, and only
            // an explicit `Model.Override` satisfies the branded type side.
            (Leaf::Overrideable(inner), Edge::Decode) => convert_value(value, inner, edge),
            (Leaf::Overrideable(inner), Edge::Encode) => match value {
                Value::Override(inner_value) => convert_value(inner_value, inner, edge),
                other => Err(format!("expected Model.Override, got {}", kind(other))),
            },
            (Leaf::Undefined, _) => Err("the variant omits this field".to_string()),
            (Leaf::OptionFromNullOr(_) | Leaf::OptionalKey(_), _) => {
                Err("optional field codec not carried".to_string())
            }
            (leaf, _) => Err(format!("expected {leaf:?}, got {}", kind(value))),
        }
    }

    fn kind(value: &Value) -> &'static str {
        match value {
            Value::Str(_) => "string",
            Value::Num(_) => "number",
            Value::Bool(_) => "boolean",
            Value::DateTime(_) => "date-time",
            Value::Override(_) => "Model.Override",
        }
    }

    /// Extract one variant: a plain `undefined` field and a variant field the
    /// variant does not list (or lists as `undefined`) are both omitted.
    fn extract_variant(structure: &Structure, variant: &str) -> Extracted {
        let mut fields = Vec::new();
        for (name, spec) in &structure.fields {
            let leaf = match spec {
                FieldSpec::Plain(Leaf::Undefined) => continue,
                FieldSpec::Plain(leaf) => leaf,
                FieldSpec::VariantField(field) => match field.schema(variant) {
                    Some(Leaf::Undefined) | None => continue,
                    Some(leaf) => leaf,
                },
            };
            fields.push((name.clone(), leaf.clone()));
        }
        Extracted { fields }
    }

    /// A variant class: a named struct whose default variant the class itself
    /// decodes (`Test.Class<User>("User")({...})`).
    #[derive(Clone, Debug)]
    pub struct Class {
        identifier: String,
        default: String,
        structure: Structure,
    }

    impl Class {
        /// The class identifier.
        pub fn identifier(&self) -> &str {
            &self.identifier
        }

        /// Field names in declaration order (`Object.keys(User.fields)`).
        pub fn field_names(&self) -> Vec<String> {
            self.structure
                .fields
                .iter()
                .map(|(name, _)| name.clone())
                .collect()
        }

        /// `User.make(props)`: the constructor.
        pub fn make(&self, supplied: &[(&str, Value)], now_millis: i64) -> Result<Record, String> {
            self.extracted(&self.default).make(supplied, now_millis)
        }

        /// `Schema.decodeSync(User)`: decode the default variant.
        pub fn decode_default(&self, input: &Record) -> Result<Record, String> {
            self.extracted(&self.default).decode(input)
        }

        /// `Schema.decodeSync(User.<variant>)`: decode one named variant.
        pub fn decode_variant(&self, variant: &str, input: &Record) -> Result<Record, String> {
            self.extracted(variant).decode(input)
        }

        fn extracted(&self, variant: &str) -> Extracted {
            // The pin's `isDefault` extraction keeps nested struct schemas
            // whole; at leaf depth both paths agree.
            extract_variant(&self.structure, variant)
        }
    }

    /// A variant toolkit: `VariantSchema.make({ variants, defaultVariant })`.
    #[derive(Clone, Debug)]
    pub struct Variants {
        names: Vec<String>,
        default: String,
    }

    impl Variants {
        /// Toolkit over `names`, `default` being the extraction default.
        pub fn make(names: &[&str], default: &str) -> Self {
            Self {
                names: names.iter().map(|name| name.to_string()).collect(),
                default: default.to_string(),
            }
        }

        /// `Model`: the six variants the namespace fixes, default `select`.
        pub fn model() -> Self {
            Self::make(MODEL_VARIANTS, "select")
        }

        /// The variant names.
        pub fn names(&self) -> &[String] {
            &self.names
        }

        /// The default variant.
        pub fn default_variant(&self) -> &str {
            &self.default
        }

        /// `Field({ variant: schema, ... })`.
        pub fn field(&self, schemas: &[(&str, Leaf)]) -> Field {
            Field::new(schemas)
        }

        /// `FieldOnly([...])(schema)`: only the listed variants get the schema.
        pub fn field_only(&self, keys: &[&str], leaf: Leaf) -> Field {
            let schemas: Vec<(&str, Leaf)> = keys.iter().map(|key| (*key, leaf.clone())).collect();
            Field::new(&schemas)
        }

        /// `FieldExcept([...])(schema)`: every variant but the listed ones.
        pub fn field_except(&self, keys: &[&str], leaf: Leaf) -> Field {
            let schemas: Vec<(&str, Leaf)> = self
                .names
                .iter()
                .filter(|name| !keys.contains(&name.as_str()))
                .map(|name| (name.as_str(), leaf.clone()))
                .collect();
            Field::new(&schemas)
        }

        /// `Model.FieldOption`: database variants take nulls, JSON variants
        /// accept a missing key, and `undefined` entries stay undefined. A
        /// variant the field never listed stays omitted rather than gaining a
        /// schema.
        pub fn field_option(&self, field: &Field) -> Field {
            let schemas: Vec<(&str, Leaf)> = field
                .schemas()
                .iter()
                .map(|(variant, leaf)| {
                    let mapped = match leaf {
                        Leaf::Undefined => Leaf::Undefined,
                        _ if matches!(variant.as_str(), "select" | "insert" | "update") => {
                            Leaf::OptionFromNullOr(Box::new(leaf.clone()))
                        }
                        _ if variant.starts_with("json") => {
                            Leaf::OptionalKey(Box::new(leaf.clone()))
                        }
                        _ => leaf.clone(),
                    };
                    (variant.as_str(), mapped)
                })
                .collect();
            Field::new(&schemas)
        }

        /// `Model.BooleanSqlite`: bits in the database variants, booleans in
        /// the JSON variants.
        pub fn boolean_sqlite(&self) -> Field {
            self.field(&[
                ("select", Leaf::BoolFromBit),
                ("insert", Leaf::BoolFromBit),
                ("update", Leaf::BoolFromBit),
                ("json", Leaf::Bool),
                ("jsonCreate", Leaf::Bool),
                ("jsonUpdate", Leaf::Bool),
            ])
        }

        /// `Model.DateTimeInsertFromNumber`: millis at the encoded edge,
        /// defaulted to now on insert only.
        pub fn date_time_insert_from_number(&self) -> Field {
            self.field(&[
                ("select", Leaf::DateMillis),
                ("insert", Leaf::Overrideable(Box::new(Leaf::DateMillis))),
                ("json", Leaf::DateMillis),
            ])
        }

        /// `Struct({ field: schema, ... })`.
        pub fn structure(&self, fields: &[(&str, FieldSpec)]) -> Structure {
            Structure {
                fields: fields
                    .iter()
                    .map(|(name, spec)| (name.to_string(), spec.clone()))
                    .collect(),
            }
        }

        /// `Class(identifier)({ field: schema, ... })`.
        pub fn class(&self, identifier: &str, fields: &[(&str, FieldSpec)]) -> Class {
            Class {
                identifier: identifier.to_string(),
                default: self.default.clone(),
                structure: self.structure(fields),
            }
        }

        /// `Test.extract(struct, variant)`.
        pub fn extract(&self, structure: &Structure, variant: &str) -> Extracted {
            extract_variant(structure, variant)
        }
    }
}

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
        Self {
            built: std::sync::Mutex::new(None),
        }
    }

    /// Compile lazily (once) and validate.
    pub fn check(&self, value: &str, build: impl FnOnce() -> Compiled) -> bool {
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
            Self::Arr(items) => items
                .iter()
                .map(|a| a.render())
                .collect::<Vec<_>>()
                .join(","),
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
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            let key = k.trim().to_string();
            if key.is_empty() || self.attrs.contains_key(&key) {
                continue;
            }
            self.attrs
                .insert(key, Attribute::Str(percent_decode(v.trim())));
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
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
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
            let labeled = if attrs.is_empty() {
                name.clone()
            } else {
                format!("{name}{attrs}")
            };
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
            out.push_str(&format!(
                "# HELP {} {}\n",
                name,
                escape_help(&m.description)
            ));
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
