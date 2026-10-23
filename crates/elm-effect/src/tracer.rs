//! `Tracer.ts` at `effect@4.0.0`: span identity and lifecycle, the tracer
//! contract, external parents, sampling references, and the native tracer.
//!
//! A span carries its name, trace and span identifiers, parent, attributes,
//! links, events, kind, sampling decision, and a [`SpanStatus`] that moves
//! from `Started` to `Ended` exactly once. The [`Span`] trait is the pin's
//! `Span` interface; [`NativeSpan`] is its in-memory implementation and
//! [`NoopSpan`] is what a disabled tracer or a `DisablePropagation` span
//! becomes.
//!
//! ## Fiber-locals become references
//!
//! The pin keeps the current span, tracer, and trace levels in fiber-local
//! context. This port keeps them in [`Reference`]s: an innermost-wins
//! override that inherits into forked children, which is the fiber-local
//! behaviour the rows observe. `withSpan` installs the new span as the
//! current span for its body and restores the outer one lexically when the
//! body ends, which is the pin's region rule.
//!
//! ## What is not ported
//!
//! The optional `Tracer.context` hook: the pin lets a tracer intercept the
//! `~effect/Effect/evaluate` primitive so it can observe continuations after
//! a region. Rust's interpreter has no such seam, so `withParentSpan`
//! restores the reference lexically and the `context`-hook row stays
//! `not-applicable`. `OtlpTracer` is a separate module in the pin and is
//! not part of this file.

use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::effect::{ErasedStep, Step};
use crate::observability::{LogLevel, STACK_TRACE, is_enabled};
use crate::service::Services;
use crate::{Cause, Effect, Exit, Reference, Requirement};

/// A span attribute value. The pin stores `unknown`; the port narrows to the
/// shapes its formatters and tests read, and renders everything else with
/// [`AttributeValue::render`] before it reaches a record.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    /// A string value.
    Text(String),
    /// An integer value.
    Int(i64),
    /// A floating-point value.
    Float(f64),
    /// A boolean value.
    Bool(bool),
}

impl AttributeValue {
    /// The attribute's text rendering (strings bare, numbers and booleans as
    /// their literal spelling).
    pub fn render(&self) -> String {
        match self {
            AttributeValue::Text(text) => text.clone(),
            AttributeValue::Int(value) => value.to_string(),
            AttributeValue::Float(value) => value.to_string(),
            AttributeValue::Bool(value) => value.to_string(),
        }
    }
}

impl From<&str> for AttributeValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for AttributeValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<i64> for AttributeValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for AttributeValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<bool> for AttributeValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// The OpenTelemetry-style role of the operation a span records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpanKind {
    /// Internal work.
    #[default]
    Internal,
    /// Server handling.
    Server,
    /// Client call.
    Client,
    /// Message production.
    Producer,
    /// Message consumption.
    Consumer,
}

impl SpanKind {
    /// The pin's lowercase spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            SpanKind::Internal => "internal",
            SpanKind::Server => "server",
            SpanKind::Client => "client",
            SpanKind::Producer => "producer",
            SpanKind::Consumer => "consumer",
        }
    }
}

/// A span's lifecycle: started at one timestamp, ended at another with the
/// outcome of the work it wrapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanStatus {
    /// Running, with the timestamp it started at.
    Started {
        /// Nanoseconds since the epoch (zero when timing is disabled).
        start_time: i64,
    },
    /// Finished, with its timestamps and the exit it saw.
    Ended {
        /// Nanoseconds since the epoch (zero when timing is disabled).
        start_time: i64,
        /// Nanoseconds since the epoch (zero when timing is disabled).
        end_time: i64,
        /// The outcome. The port renders a failure's cause because the span
        /// outlives the effect's `E`; see the module note on `Exit`.
        exit: Exit<(), String>,
    },
}

/// One event recorded on a span (the pin's `[name, startTime, attributes]`).
#[derive(Clone, Debug, PartialEq)]
pub struct SpanEvent {
    /// The event's name.
    pub name: String,
    /// Nanoseconds since the epoch.
    pub start_time: i64,
    /// Event attributes.
    pub attributes: Vec<(String, AttributeValue)>,
}

/// A span created outside this tracer, carrying the identifiers a remote
/// tracing system assigned it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSpan {
    /// The remote span id.
    pub span_id: String,
    /// The remote trace id.
    pub trace_id: String,
    /// The remote sampling decision (defaults to sampled).
    pub sampled: bool,
    /// Context annotations the remote span carried.
    pub annotations: Vec<(String, String)>,
}

/// Create an [`ExternalSpan`] with the pin's defaults: sampled, no
/// annotations.
pub fn external_span(span_id: impl Into<String>, trace_id: impl Into<String>) -> ExternalSpan {
    ExternalSpan {
        span_id: span_id.into(),
        trace_id: trace_id.into(),
        sampled: true,
        annotations: Vec::new(),
    }
}

/// Either an Effect-managed span or one propagated from another system.
#[derive(Clone)]
pub enum AnySpan {
    /// A span this port created.
    Span(Arc<dyn Span>),
    /// A span another tracing system created.
    External(ExternalSpan),
}

impl AnySpan {
    /// The span id (`"noop"` for a disabled span).
    pub fn span_id(&self) -> String {
        match self {
            AnySpan::Span(span) => span.span_id(),
            AnySpan::External(span) => span.span_id.clone(),
        }
    }

    /// The trace id.
    pub fn trace_id(&self) -> String {
        match self {
            AnySpan::Span(span) => span.trace_id(),
            AnySpan::External(span) => span.trace_id.clone(),
        }
    }

    /// The span name (`""` for an external span: it has no name here).
    pub fn name(&self) -> String {
        match self {
            AnySpan::Span(span) => span.name().to_owned(),
            AnySpan::External(_) => String::new(),
        }
    }

    /// The parent this span was created under, when it has one.
    pub fn parent(&self) -> Option<AnySpan> {
        match self {
            AnySpan::Span(span) => span.parent(),
            AnySpan::External(_) => None,
        }
    }

    /// The sampling decision that applies to descendants.
    pub fn sampled(&self) -> bool {
        match self {
            AnySpan::Span(span) => span.sampled(),
            AnySpan::External(span) => span.sampled,
        }
    }

    /// True for the span a disabled tracer installs: it never propagates.
    pub fn is_noop(&self) -> bool {
        self.span_id() == "noop"
    }

    /// The live status of a managed span (`Started` for an external one).
    pub fn status(&self) -> SpanStatus {
        match self {
            AnySpan::Span(span) => span.status(),
            AnySpan::External(_) => SpanStatus::Started { start_time: 0 },
        }
    }

    /// The attributes of a managed span.
    pub fn attributes(&self) -> Vec<(String, AttributeValue)> {
        match self {
            AnySpan::Span(span) => span.attributes(),
            AnySpan::External(_) => Vec::new(),
        }
    }

    /// One attribute by key.
    pub fn attribute_value(&self, key: &str) -> Option<AttributeValue> {
        self.attributes()
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// The links of a managed span.
    pub fn links(&self) -> Vec<SpanLink> {
        match self {
            AnySpan::Span(span) => span.links(),
            AnySpan::External(_) => Vec::new(),
        }
    }

    /// The events of a managed span.
    pub fn events(&self) -> Vec<SpanEvent> {
        match self {
            AnySpan::Span(span) => span.events(),
            AnySpan::External(_) => Vec::new(),
        }
    }

    /// Set one attribute on a managed span (a noop or external span drops it).
    pub fn attribute(&self, key: &str, value: AttributeValue) {
        if let AnySpan::Span(span) = self {
            span.attribute(key, value);
        }
    }

    /// Append links to a managed span (a noop or external span drops them).
    pub fn add_links(&self, links: Vec<SpanLink>) {
        if let AnySpan::Span(span) = self {
            span.add_links(links);
        }
    }

    /// Record an event on a managed span (a noop or external span drops it).
    pub fn event(
        &self,
        name: &str,
        start_time: i64,
        attributes: Vec<(String, AttributeValue)>,
    ) {
        if let AnySpan::Span(span) = self {
            span.event(name, start_time, attributes);
        }
    }

    /// The kind of a managed span.
    pub fn kind(&self) -> SpanKind {
        match self {
            AnySpan::Span(span) => span.kind(),
            AnySpan::External(_) => SpanKind::Internal,
        }
    }
}

impl From<Arc<dyn Span>> for AnySpan {
    fn from(span: Arc<dyn Span>) -> Self {
        AnySpan::Span(span)
    }
}

impl PartialEq for AnySpan {
    /// Identity is the span id, which is how the pin's tests compare a link's
    /// span to the span value they created.
    fn eq(&self, other: &Self) -> bool {
        self.span_id() == other.span_id()
    }
}

impl fmt::Debug for AnySpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnySpan::Span(span) => write!(
                f,
                "Span({} span_id={} trace_id={})",
                span.name(),
                span.span_id(),
                span.trace_id()
            ),
            AnySpan::External(span) => write!(
                f,
                "ExternalSpan(span_id={} trace_id={})",
                span.span_id, span.trace_id
            ),
        }
    }
}

/// A relationship from one span to another (`Tracer.SpanLink`).
#[derive(Clone, Debug)]
pub struct SpanLink {
    /// The linked span.
    pub span: AnySpan,
    /// Attributes describing the relationship.
    pub attributes: Vec<(String, AttributeValue)>,
}

impl PartialEq for SpanLink {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span && self.attributes == other.attributes
    }
}

/// What a [`Span`] exposes. Mutation is through `&self`: a span is shared
/// between the tracer, the interpreter, and whatever the body hands it to.
pub trait Span: Send + Sync + fmt::Debug {
    /// The span name.
    fn name(&self) -> &str;
    /// The span id.
    fn span_id(&self) -> String;
    /// The trace id.
    fn trace_id(&self) -> String;
    /// The parent this span was created under.
    fn parent(&self) -> Option<AnySpan>;
    /// The current lifecycle status.
    fn status(&self) -> SpanStatus;
    /// The current attributes, in insertion order.
    fn attributes(&self) -> Vec<(String, AttributeValue)>;
    /// Set one attribute.
    fn attribute(&self, key: &str, value: AttributeValue);
    /// The events recorded so far.
    fn events(&self) -> Vec<SpanEvent>;
    /// Record one event.
    fn event(
        &self,
        name: &str,
        start_time: i64,
        attributes: Vec<(String, AttributeValue)>,
    );
    /// The links recorded so far.
    fn links(&self) -> Vec<SpanLink>;
    /// Append links.
    fn add_links(&self, links: Vec<SpanLink>);
    /// Whether this span and its descendants are sampled.
    fn sampled(&self) -> bool;
    /// The span kind.
    fn kind(&self) -> SpanKind;
    /// Move the status to `Ended`. The first call wins; later ones are
    /// ignored, because a span has exactly one end at the pin.
    fn end(&self, end_time: i64, exit: Exit<(), String>);

    /// One attribute by key.
    fn attribute_value(&self, key: &str) -> Option<AttributeValue> {
        self.attributes()
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }
}

/// The default in-memory span: eager identifiers, stored attributes, events,
/// and links, and one `Started` to `Ended` transition.
#[derive(Debug)]
pub struct NativeSpan {
    name: String,
    span_id: String,
    trace_id: String,
    parent: Option<AnySpan>,
    #[allow(dead_code)] // the pin's `annotations` Context, kept for parity
    annotations: Vec<(String, String)>,
    kind: SpanKind,
    sampled: bool,
    status: Mutex<SpanStatus>,
    attributes: Mutex<Vec<(String, AttributeValue)>>,
    events: Mutex<Vec<SpanEvent>>,
    links: Mutex<Vec<SpanLink>>,
}

impl NativeSpan {
    /// Build a span from options. The identifiers are generated here: a
    /// child takes its trace id from the parent, a root mints a new one.
    pub fn new(name: impl Into<String>, options: SpanOptions) -> Self {
        let name = name.into();
        let trace_id = options
            .parent
            .as_ref()
            .map(AnySpan::trace_id)
            .unwrap_or_else(|| random_hex(32));
        let start_time = options.start_time;
        Self {
            name,
            span_id: random_hex(16),
            trace_id,
            parent: options.parent,
            annotations: options.annotations,
            kind: options.kind,
            sampled: options.sampled.unwrap_or(true),
            status: Mutex::new(SpanStatus::Started { start_time }),
            attributes: Mutex::new(options.attributes),
            events: Mutex::new(Vec::new()),
            links: Mutex::new(options.links),
        }
    }

    fn guard<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

impl Span for NativeSpan {
    fn name(&self) -> &str {
        &self.name
    }

    fn span_id(&self) -> String {
        self.span_id.clone()
    }

    fn trace_id(&self) -> String {
        self.trace_id.clone()
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
        let mut status = Self::guard(&self.status);
        if let SpanStatus::Started { start_time } = *status {
            *status = SpanStatus::Ended {
                start_time,
                end_time,
                exit,
            };
        }
    }
}

/// The span a disabled tracer installs. It keeps its name so the body can
/// still read it, has the id `"noop"`, and is skipped as a parent.
#[derive(Debug)]
pub struct NoopSpan {
    name: String,
    parent: Option<AnySpan>,
    sampled: bool,
    status: Mutex<SpanStatus>,
}

impl NoopSpan {
    /// A noop span with this name and parent.
    pub fn new(name: impl Into<String>, parent: Option<AnySpan>, sampled: bool) -> Self {
        Self {
            name: name.into(),
            parent,
            sampled,
            status: Mutex::new(SpanStatus::Started { start_time: 0 }),
        }
    }

    fn guard<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

impl Span for NoopSpan {
    fn name(&self) -> &str {
        &self.name
    }

    fn span_id(&self) -> String {
        "noop".to_owned()
    }

    fn trace_id(&self) -> String {
        self.parent
            .as_ref()
            .map(AnySpan::trace_id)
            .unwrap_or_else(|| "noop".to_owned())
    }

    fn parent(&self) -> Option<AnySpan> {
        self.parent.clone()
    }

    fn status(&self) -> SpanStatus {
        Self::guard(&self.status).clone()
    }

    fn attributes(&self) -> Vec<(String, AttributeValue)> {
        Vec::new()
    }

    fn attribute(&self, _key: &str, _value: AttributeValue) {}

    fn events(&self) -> Vec<SpanEvent> {
        Vec::new()
    }

    fn event(&self, _name: &str, _start_time: i64, _attributes: Vec<(String, AttributeValue)>) {}

    fn links(&self) -> Vec<SpanLink> {
        Vec::new()
    }

    fn add_links(&self, _links: Vec<SpanLink>) {}

    fn sampled(&self) -> bool {
        self.sampled
    }

    fn kind(&self) -> SpanKind {
        SpanKind::Internal
    }

    fn end(&self, end_time: i64, exit: Exit<(), String>) {
        let mut status = Self::guard(&self.status);
        if let SpanStatus::Started { start_time } = *status {
            *status = SpanStatus::Ended {
                start_time,
                end_time,
                exit,
            };
        }
    }
}

/// Span creation options (`Tracer.SpanOptions`, with the trace options the
/// pin splits out folded in).
#[derive(Clone, Debug, Default)]
pub struct SpanOptions {
    /// An explicit parent. `root` wins over this when set.
    pub parent: Option<AnySpan>,
    /// Attributes set at creation.
    pub attributes: Vec<(String, AttributeValue)>,
    /// Links set at creation.
    pub links: Vec<SpanLink>,
    /// The pin's context annotations, carried verbatim.
    pub annotations: Vec<(String, String)>,
    /// The span kind.
    pub kind: SpanKind,
    /// Ignore the ambient parent and start a new trace.
    pub root: bool,
    /// Force the sampling decision; otherwise it is derived.
    pub sampled: Option<bool>,
    /// The span's trace level, defaulting to the ambient current level.
    pub level: Option<LogLevel>,
    /// Nanoseconds since the epoch, filled by the combinator that creates it.
    pub start_time: i64,
    /// Create a noop span that is skipped as a parent (`DisablePropagation`).
    pub disable_propagation: bool,
}

/// A tracing backend: it allocates spans. The pin's optional `context` hook
/// has no counterpart here; see the module note.
pub trait Tracer: Send + Sync {
    /// Create a span from `name` and `options`.
    fn span(&self, name: &str, options: SpanOptions) -> Arc<dyn Span>;
}

/// The `Tracer.ParentSpan` service a layer provides (`Layer.parentSpan`,
/// `Layer.span`). Building it also installs [`CURRENT_SPAN`], so the
/// reference and the tag always name the same span.
pub struct ParentSpanTag;

impl crate::ServiceTag for ParentSpanTag {
    type Service = AnySpan;
    const ID: &'static str = "effect/Tracer/ParentSpan";
}

/// The `Tracer.Tracer` service a backend layer provides. Building it also
/// installs [`CURRENT_TRACER`], the same tag-to-reference mirror as
/// [`ParentSpanTag`].
pub struct TracerTag;

impl crate::ServiceTag for TracerTag {
    type Service = Arc<dyn Tracer>;
    const ID: &'static str = "effect/Tracer";
}

/// The default tracer: in-memory [`NativeSpan`]s, exported nowhere.
pub fn native_tracer() -> Arc<dyn Tracer> {
    Arc::new(NativeTracer)
}

struct NativeTracer;

impl Tracer for NativeTracer {
    fn span(&self, name: &str, options: SpanOptions) -> Arc<dyn Span> {
        Arc::new(NativeSpan::new(name, options))
    }
}

// Ambient references (the pin's fiber-cached services)

/// The span children derive their parent from (`ParentSpan` at the pin).
pub static CURRENT_SPAN: Reference<Option<AnySpan>> = Reference {
    id: "effect/Tracer/ParentSpan",
    default: None,
};

/// The active tracer; `None` means [`native_tracer`].
pub static CURRENT_TRACER: Reference<Option<Arc<dyn Tracer>>> = Reference {
    id: "effect/Tracer",
    default: None,
};

/// Whether span creation is enabled (`withTracerEnabled`).
pub static TRACER_ENABLED: Reference<bool> = Reference {
    id: "effect/Tracer/Enabled",
    default: true,
};

/// Whether spans carry start and end timestamps (`withTracerTiming`).
pub static TRACER_TIMING: Reference<bool> = Reference {
    id: "effect/Tracer/Timing",
    default: true,
};

/// The level a span takes when its options do not name one (default `Info`).
pub static CURRENT_TRACE_LEVEL: Reference<LogLevel> = Reference {
    id: "effect/Tracer/CurrentTraceLevel",
    default: LogLevel::Info,
};

/// The threshold below which spans are not sampled (default `All`).
pub static MINIMUM_TRACE_LEVEL: Reference<LogLevel> = Reference {
    id: "effect/Tracer/MinimumTraceLevel",
    default: LogLevel::All,
};

/// A span-timestamp source, overriding the ambient clock tag. The pin reads
/// the fiber's `TestClock` here; a reference keeps that explicit without
/// adding `Clock` to every `withSpan` call's requirement.
pub static TRACE_CLOCK: Reference<Option<Arc<dyn crate::ClockImpl>>> = Reference {
    id: "effect/Tracer/Clock",
    default: None,
};

// Combinators (the pin's Effect.withSpan family)

/// Wrap `effect` in a span (`Effect.withSpan`): create the span, run the body
/// with it as the current span, then end it with the body's outcome.
///
/// A failure inside the span gains the span name as its stack frame, like
/// the pin's raise-site capture; an existing frame is kept.
pub fn with_span<A, E, R>(
    name: impl Into<String>,
    options: SpanOptions,
    effect: Effect<A, E, R>,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    // A span outlives the effect's `E`, so the ended status stores the
    // rendered cause; that is the one place `Debug` is owed.
    E: fmt::Debug + Send + 'static,
    R: Requirement,
{
    from_step(Box::new(WithSpan {
        name: name.into(),
        options,
        effect,
    }))
}

/// The current span, if one is installed (`Effect.currentSpan`).
pub fn current_span() -> Effect<Option<AnySpan>, std::convert::Infallible, ()> {
    crate::read_reference(&CURRENT_SPAN)
}

/// The current parent span (`Effect.currentParentSpan`): the current span
/// with disabled (noop) spans skipped.
pub fn current_parent_span() -> Effect<Option<AnySpan>, std::convert::Infallible, ()> {
    current_span().map(propagated_parent)
}

/// Run `effect` under an explicit parent span (`Effect.withParentSpan`).
pub fn with_parent_span<A, E, R>(span: AnySpan, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&CURRENT_SPAN, Some(span))
}

/// Run `effect` with this tracer (`Effect.withTracer`).
pub fn with_tracer<A, E, R>(tracer: Arc<dyn Tracer>, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&CURRENT_TRACER, Some(tracer))
}

/// Enable or disable span creation for `effect` (`Effect.withTracerEnabled`).
pub fn with_tracer_enabled<A, E, R>(enabled: bool, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&TRACER_ENABLED, enabled)
}

/// Enable or disable span timestamps for `effect`
/// (`Effect.withTracerTiming`).
pub fn with_tracer_timing<A, E, R>(enabled: bool, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&TRACER_TIMING, enabled)
}

/// Override the sampling threshold for `effect`.
pub fn with_minimum_trace_level<A, E, R>(level: LogLevel, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&MINIMUM_TRACE_LEVEL, level)
}

/// Override the default span level for `effect`.
pub fn with_trace_level<A, E, R>(level: LogLevel, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&CURRENT_TRACE_LEVEL, level)
}

/// Give span timestamps a clock (a `TestClock`, typically) for `effect`.
pub fn with_trace_clock<A, E, R>(
    clock: Arc<dyn crate::ClockImpl>,
    effect: Effect<A, E, R>,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    effect.provide_reference(&TRACE_CLOCK, Some(clock))
}

/// Add attributes to the current span for the duration of `effect`
/// (`Effect.annotateSpans`).
pub fn annotate_spans<A, E, R>(
    attributes: Vec<(String, AttributeValue)>,
    effect: Effect<A, E, R>,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    from_step(Box::new(AnnotateSpans { attributes, effect }))
}

/// Add one attribute to the current span for the duration of `effect`.
pub fn annotate_span<A, E, R>(
    key: impl Into<String>,
    value: impl Into<AttributeValue>,
    effect: Effect<A, E, R>,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    annotate_spans(vec![(key.into(), value.into())], effect)
}

/// Add attributes to the span that is current right now
/// (`Effect.annotateCurrentSpan`).
pub fn annotate_current_span(
    attributes: Vec<(String, AttributeValue)>,
) -> Effect<(), std::convert::Infallible, ()> {
    annotate_spans(attributes, Effect::succeed(()))
}

/// Create a span and run `f` with it, ending it when the body ends
/// (`Effect.useSpan`).
pub fn use_span<A, E, R, F>(name: impl Into<String>, options: SpanOptions, f: F) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: fmt::Debug + Send + 'static,
    R: Requirement,
    F: FnOnce(Arc<dyn Span>) -> Effect<A, E, R> + Send + 'static,
{
    from_step(Box::new(UseSpan {
        name: name.into(),
        options,
        f,
        marker: PhantomData,
    }))
}

/// Create a span without ending it (`Effect.makeSpan`). The caller owns its
/// lifetime and ends it with [`Span::end`].
pub fn make_span(
    name: impl Into<String>,
    options: SpanOptions,
) -> Effect<Arc<dyn Span>, std::convert::Infallible, ()> {
    from_step(Box::new(MakeSpan {
        name: name.into(),
        options,
    }))
}

/// Add links to the current span for the duration of `effect`
/// (`Effect.linkSpans`).
pub fn link_spans<A, E, R>(links: Vec<SpanLink>, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    from_step(Box::new(LinkSpans { links, effect }))
}

/// Link the current span to `span` with no attributes.
pub fn link_span<A, E, R>(span: AnySpan, effect: Effect<A, E, R>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    link_spans(
        vec![SpanLink {
            span,
            attributes: Vec::new(),
        }],
        effect,
    )
}

// Interpreter plumbing

/// Build a description from a private step. `pub(crate)` fields are the only
/// way in, which is why this lives here rather than in a caller.
fn from_step<A, E, R>(step: Box<dyn ErasedStep<A, E> + Send>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    Effect {
        step: Step::Suspend(step),
        services: crate::service::empty_services(),
        marker: PhantomData,
    }
}

/// Read an ambient reference from the resolved services.
fn ambient<T>(scope: &Services, reference: &'static Reference<T>) -> T
where
    T: Clone + Send + Sync + 'static,
{
    scope
        .get_ref(reference.id)
        .and_then(|value| value.downcast_ref::<T>().cloned())
        .unwrap_or_else(|| reference.default.clone())
}

/// A disabled span is skipped and its own parent becomes the candidate, so a
/// child of a disabled span still points at the enabled ancestor.
fn propagated_parent(mut span: Option<AnySpan>) -> Option<AnySpan> {
    while let Some(current) = span {
        if current.is_noop() {
            span = current.parent();
        } else {
            return Some(current);
        }
    }
    None
}

/// Nanoseconds since the epoch, from the trace clock reference, else the
/// ambient clock tag, else the wall clock.
pub(crate) fn now_nanos(scope: &Services) -> i64 {
    let clock = ambient(scope, &TRACE_CLOCK).unwrap_or_else(|| crate::effect::ambient_clock(scope));
    (clock.now_millis() as i64).saturating_mul(1_000_000)
}

/// Create a span under the ambient tracer and parent, returning it with
/// whether it carries timestamps.
pub(crate) fn create_span(
    scope: &Services,
    name: &str,
    mut options: SpanOptions,
) -> (Arc<dyn Span>, bool) {
    let tracer = ambient(scope, &CURRENT_TRACER).unwrap_or_else(native_tracer);
    let enabled = ambient(scope, &TRACER_ENABLED);
    let timing = ambient(scope, &TRACER_TIMING);
    let minimum = ambient(scope, &MINIMUM_TRACE_LEVEL);
    let current_level = ambient(scope, &CURRENT_TRACE_LEVEL);

    let ambient_parent = propagated_parent(ambient(scope, &CURRENT_SPAN));
    let parent = if options.root {
        None
    } else {
        options.parent.clone().or(ambient_parent)
    };
    let level = options.level.unwrap_or(current_level);
    let sampled = options.sampled.unwrap_or_else(|| {
        is_enabled(level, minimum) && parent.as_ref().is_none_or(AnySpan::sampled)
    });
    let start_time = if timing { now_nanos(scope) } else { 0 };

    options.parent = parent.clone();
    options.start_time = start_time;
    options.sampled = Some(sampled);

    let span: Arc<dyn Span> = if !enabled || options.disable_propagation {
        Arc::new(NoopSpan::new(name.to_owned(), parent, sampled))
    } else {
        tracer.span(name, options)
    };
    (span, timing)
}

/// End a span with the body's outcome (`None` is success).
pub(crate) fn finish_span(
    scope: &Services,
    span: &Arc<dyn Span>,
    timing: bool,
    failure: Option<String>,
) {
    let end_time = if timing { now_nanos(scope) } else { 0 };
    let exit = match failure {
        None => Exit::Success(()),
        Some(rendered) => Exit::fail(rendered),
    };
    span.end(end_time, exit);
}

struct WithSpan<A: Send + 'static, E: Send + 'static, R: Requirement> {
    name: String,
    options: SpanOptions,
    effect: Effect<A, E, R>,
}

impl<A, E, R> ErasedStep<A, E> for WithSpan<A, E, R>
where
    A: Send + 'static,
    E: fmt::Debug + Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let Self {
            name,
            options,
            effect,
        } = *self;
        let (span, timing) = create_span(scope, &name, options);
        let body = effect.provide_reference(&CURRENT_SPAN, Some(AnySpan::from(Arc::clone(&span))));
        match body.run_sync_with(scope) {
            Ok(value) => {
                finish_span(scope, &span, timing, None);
                Ok(value)
            }
            Err(cause) => {
                finish_span(scope, &span, timing, Some(format!("{cause:?}")));
                let cause = if cause.annotation(STACK_TRACE).is_some() {
                    cause
                } else {
                    cause.annotate(STACK_TRACE, name)
                };
                Err(cause)
            }
        }
    }
}

struct UseSpan<A: Send + 'static, E: Send + 'static, R: Requirement, F> {
    name: String,
    options: SpanOptions,
    f: F,
    marker: PhantomData<fn() -> (A, E, R)>,
}

impl<A, E, R, F> ErasedStep<A, E> for UseSpan<A, E, R, F>
where
    A: Send + 'static,
    E: fmt::Debug + Send + 'static,
    R: Requirement,
    F: FnOnce(Arc<dyn Span>) -> Effect<A, E, R> + Send + 'static,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let Self {
            name, options, f, ..
        } = *self;
        let (span, timing) = create_span(scope, &name, options);
        let body = (f)(Arc::clone(&span))
            .provide_reference(&CURRENT_SPAN, Some(AnySpan::from(Arc::clone(&span))));
        match body.run_sync_with(scope) {
            Ok(value) => {
                finish_span(scope, &span, timing, None);
                Ok(value)
            }
            Err(cause) => {
                finish_span(scope, &span, timing, Some(format!("{cause:?}")));
                Err(cause)
            }
        }
    }
}

struct MakeSpan {
    name: String,
    options: SpanOptions,
}

impl ErasedStep<Arc<dyn Span>, std::convert::Infallible> for MakeSpan {
    fn run_erased(
        self: Box<Self>,
        scope: &Services,
    ) -> Result<Arc<dyn Span>, Cause<std::convert::Infallible>> {
        let (span, _) = create_span(scope, &self.name, self.options);
        Ok(span)
    }
}

struct AnnotateSpans<A: Send + 'static, E: Send + 'static, R: Requirement> {
    attributes: Vec<(String, AttributeValue)>,
    effect: Effect<A, E, R>,
}

impl<A, E, R> ErasedStep<A, E> for AnnotateSpans<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let Self { attributes, effect } = *self;
        if let Some(span) = ambient(scope, &CURRENT_SPAN) {
            for (key, value) in attributes {
                span.attribute(&key, value);
            }
        }
        effect.run_sync_with(scope)
    }
}

struct LinkSpans<A: Send + 'static, E: Send + 'static, R: Requirement> {
    links: Vec<SpanLink>,
    effect: Effect<A, E, R>,
}

impl<A, E, R> ErasedStep<A, E> for LinkSpans<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let Self { links, effect } = *self;
        if let Some(span) = ambient(scope, &CURRENT_SPAN) {
            span.add_links(links);
        }
        effect.run_sync_with(scope)
    }
}

// Identifiers

static ID_SEED: AtomicU64 = AtomicU64::new(0);

/// A lowercase hex id of `chars` characters. The pin uses a cryptographic
/// source; spans only need uniqueness, so a xorshift seeded from the wall
/// clock and a process counter is enough here.
pub(crate) fn random_hex(chars: usize) -> String {
    let counter = ID_SEED.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let mut state = nanos ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = String::with_capacity(chars);
    while out.len() < chars {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let value = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        for shift in (0..64).step_by(4) {
            if out.len() == chars {
                break;
            }
            let nibble = (value >> shift) & 0xF;
            out.push(char::from_digit(nibble as u32, 16).expect("nibble is a hex digit"));
        }
    }
    out
}
