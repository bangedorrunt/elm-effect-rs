//! Metric registry: named series keyed by (kind, id, description, attributes).
//!
//! The registry is an ordinary value (TS `MetricRegistry` map): updates
//! accumulate per registry, so separate registries stay isolated and `clear`
//! drops every series. Counter state mirrors TS `{ count, incremental }`;
//! plain counters never increment incrementally. A gauge's update replaces the
//! value and its modify adds to it, which is the opposite of a counter.
//!
//! A series is keyed by a tuple rather than a concatenation, so an id with a
//! colon in it is never confused with a description, and `{a: "b,c=d"}` is
//! never confused with `{a: "b", c: "d"}`. That is the TS `makeKey` trick,
//! carried structurally instead of as a JSON string. Series attributes merge
//! as the pin's `mergeAttributes` does: the metric's own attributes sit
//! underneath the contextual attributes, which sit underneath
//! `withAttributes` overlays, where the overlay applied first wins.

use std::collections::HashMap;

/// A metric attribute set.
///
/// Pairs keep insertion order because a snapshot reports the attribute order
/// the metric was described with, and the series key sorts them by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Attributes {
    names: Vec<String>,
    values: HashMap<String, String>,
}

impl Attributes {
    /// Pairs in insertion order; a later pair for a name keeps its position.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let mut attrs = Self::default();
        for (name, value) in pairs {
            attrs.set(name.into(), value.into());
        }
        attrs
    }

    /// The empty set, which shares the unattributed series.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Pairs in insertion order.
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.names
            .iter()
            .map(|name| (name.clone(), self.values[name].clone()))
            .collect()
    }

    fn set(&mut self, name: String, value: String) {
        if self.values.insert(name.clone(), value).is_none() {
            self.names.push(name);
        }
    }

    /// TS `mergeAttributes(self, other)`: `other`'s values win, `self`'s key
    /// order comes first.
    fn overlay(&self, other: &Attributes) -> Attributes {
        let mut merged = self.clone();
        for (name, value) in other.pairs() {
            merged.set(name, value);
        }
        merged
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn sorted_entries(&self) -> Vec<(String, String)> {
        let mut entries = self.pairs();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

/// The metric type a series belongs to (TS `metric.type`).
///
/// Two metrics of different types with the same id, description, and
/// attributes are still different series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MetricKind {
    Counter,
    Gauge,
}

/// What a metric contributes to its series key besides its type.
///
/// This is shared rather than duplicated per metric type because the attribute
/// precedence is the subtle part of the port, and a second copy of it is a
/// second place to get it wrong.
#[derive(Debug, Clone, PartialEq, Default)]
struct Identity {
    id: String,
    description: Option<String>,
    attributes: Attributes,
    overlays: Vec<Attributes>,
    constant_input: Option<f64>,
}

impl Identity {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: None,
            attributes: Attributes::default(),
            overlays: Vec::new(),
            constant_input: None,
        }
    }

    /// TS `mergeAttributes(mergeAttributes(own, contextual), overlays)` in
    /// insertion order. `overlays[0]` is the last `withAttributes` applied, so
    /// walking in order leaves the first-applied overlay last to win, which is
    /// the pin's precedence.
    fn merged_attributes(&self, contextual: &Attributes) -> Attributes {
        let mut merged = self.attributes.overlay(contextual);
        for overlay in self.overlays.iter() {
            merged = merged.overlay(overlay);
        }
        merged
    }

    /// The registry key. Attribute entries are sorted by name, so the record
    /// order a metric was described in is not part of its identity.
    fn key(&self, kind: MetricKind, contextual: &Attributes) -> SeriesKey {
        SeriesKey {
            kind,
            id: self.id.clone(),
            description: self.description.clone(),
            attributes: self.merged_attributes(contextual).sorted_entries(),
        }
    }

    /// TS `mapInput`: the constant replaces the tracked input everywhere,
    /// including a gauge's `modify`.
    fn mapped_input(&self, tracked: f64) -> f64 {
        self.constant_input.unwrap_or(tracked)
    }
}

/// The description/builder half shared by the metric types. `constant_input`
/// is TS `withConstantInput`, which applies to any metric.
trait Described {
    fn identity(&self) -> &Identity;
}

/// A named counter description (TS `Metric.counter(id, options)`).
#[derive(Debug, Clone, PartialEq)]
pub struct Counter {
    identity: Identity,
    incremental: bool,
}

impl Counter {
    /// Describe a counter with `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            identity: Identity::new(id),
            incremental: false,
        }
    }

    /// Attach a description, which is distinct from attributes.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.identity.description = Some(description.into());
        self
    }

    /// The metric's own attributes (TS `Metric.counter`'s option).
    pub fn with_attributes(mut self, attributes: Attributes) -> Self {
        let over = self.identity.attributes.overlay(&attributes);
        self.identity.attributes = over;
        self
    }

    /// TS `Metric.withAttributes`: a contextual overlay. The first overlay
    /// applied wins over later ones, which both win over context.
    pub fn with_extra_attributes(mut self, attributes: Attributes) -> Self {
        self.identity.overlays.insert(0, attributes);
        self
    }

    /// TS `Metric.counter(id, { incremental: true })`: only non-negative
    /// inputs are applied.
    pub fn incremental(mut self) -> Self {
        self.incremental = true;
        self
    }

    /// TS `Metric.withConstantInput`: every input contributes this instead of
    /// the tracked value.
    pub fn with_constant_input(mut self, constant: f64) -> Self {
        self.identity.constant_input = Some(constant);
        self
    }

    /// The amount this input contributes, or `None` when the counter refuses
    /// it. TS `canUpdate` for an incremental counter is `value >= 0`, applied
    /// to the mapped input.
    fn input_amount(&self, tracked: f64) -> Option<f64> {
        let amount = self.identity.mapped_input(tracked);
        if self.incremental && amount < 0.0 {
            None
        } else {
            Some(amount)
        }
    }
}

impl Described for Counter {
    fn identity(&self) -> &Identity {
        &self.identity
    }
}

/// A named gauge description (TS `Metric.gauge(id, options)`).
///
/// A gauge has no `incremental` refusal rule: every input is applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Gauge {
    identity: Identity,
}

impl Gauge {
    /// Describe a gauge with `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            identity: Identity::new(id),
        }
    }

    /// TS `Metric.withAttributes`: a contextual overlay. The first overlay
    /// applied wins over later ones, which both win over context.
    pub fn with_extra_attributes(mut self, attributes: Attributes) -> Self {
        self.identity.overlays.insert(0, attributes);
        self
    }

    /// TS `Metric.withConstantInput`: every input contributes this instead of
    /// the tracked value.
    pub fn with_constant_input(mut self, constant: f64) -> Self {
        self.identity.constant_input = Some(constant);
        self
    }
}

impl Described for Gauge {
    fn identity(&self) -> &Identity {
        &self.identity
    }
}

/// The registry key. A tuple encoding, so no field's text can impersonate
/// another field's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SeriesKey {
    kind: MetricKind,
    id: String,
    description: Option<String>,
    attributes: Vec<(String, String)>,
}

/// One counter series state (TS counter `{ count, incremental }`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CounterState {
    /// Accumulated total.
    pub count: f64,
    /// Plain counters report non-incremental snapshots.
    pub incremental: bool,
}

impl CounterState {
    fn zero() -> Self {
        Self {
            count: 0.0,
            incremental: false,
        }
    }
}

/// One gauge series state (TS gauge `{ value }`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaugeState {
    /// The current value, set by an update and moved by a modify.
    pub value: f64,
}

impl GaugeState {
    fn zero() -> Self {
        Self { value: 0.0 }
    }
}

/// One registered series' state, by the metric type that owns it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeriesState {
    /// A counter: accumulated total plus its `incremental` flag.
    Counter(CounterState),
    /// A gauge: the current value.
    Gauge(GaugeState),
}

/// One registered series: its attributes (absent when unattributed) and state.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// `None` for the series an empty attribute set shares.
    pub attributes: Option<Attributes>,
    /// The state, by metric type.
    pub state: SeriesState,
}

/// Registry of counter and gauge series by series key (TS `MetricRegistry`).
#[derive(Debug, Clone, Default)]
pub struct MetricRegistry {
    /// The state plus the merged attributes in insertion order, because a
    /// snapshot reports the order the metric was described with and the key
    /// stored a sorted copy. Counter and gauge series are kept apart so a
    /// registry entry is always the kind its key names.
    counters: HashMap<SeriesKey, (CounterState, Attributes)>,
    gauges: HashMap<SeriesKey, (GaugeState, Attributes)>,
}

impl MetricRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// TS `Metric.update(counter, amount)`: add `amount` to the series.
    pub fn update(&mut self, counter: &Counter, amount: f64) {
        let contextual = Attributes::default();
        self.update_with(counter, amount, &contextual);
    }

    /// `update` under TS `Metric.CurrentMetricAttributes`.
    ///
    /// The series is registered by the update call itself, so a refused input
    /// still leaves `{ count: 0, incremental: true }` in place, exactly as the
    /// pin's hook does: the hook is registered first and `canUpdate` gates only
    /// the accumulation.
    pub fn update_with(&mut self, counter: &Counter, amount: f64, contextual: &Attributes) {
        let entry = self
            .counters
            .entry(counter.identity().key(MetricKind::Counter, contextual))
            .or_insert_with(|| {
                (
                    CounterState::zero(),
                    counter.identity().merged_attributes(contextual),
                )
            });
        entry.0.incremental = counter.incremental;
        if let Some(amount) = counter.input_amount(amount) {
            entry.0.count += amount;
        }
    }

    /// The series' count; zero when never updated.
    pub fn value(&self, counter: &Counter) -> CounterState {
        let contextual = Attributes::default();
        self.value_with(counter, &contextual)
    }

    /// `value` under TS `Metric.CurrentMetricAttributes`.
    pub fn value_with(&self, counter: &Counter, contextual: &Attributes) -> CounterState {
        self.counters
            .get(&counter.identity().key(MetricKind::Counter, contextual))
            .map(|(state, _)| *state)
            .unwrap_or_else(CounterState::zero)
    }

    /// TS `Metric.update(gauge, value)`: set the series to `value`.
    pub fn set(&mut self, gauge: &Gauge, value: f64) {
        let contextual = Attributes::default();
        self.set_with(gauge, value, &contextual);
    }

    /// `set` under TS `Metric.CurrentMetricAttributes`.
    pub fn set_with(&mut self, gauge: &Gauge, value: f64, contextual: &Attributes) {
        let entry = self
            .gauges
            .entry(gauge.identity().key(MetricKind::Gauge, contextual))
            .or_insert_with(|| {
                (
                    GaugeState::zero(),
                    gauge.identity().merged_attributes(contextual),
                )
            });
        entry.0.value = gauge.identity().mapped_input(value);
    }

    /// TS `Metric.modify(gauge, delta)`: add `delta` to the series.
    pub fn add(&mut self, gauge: &Gauge, delta: f64) {
        let contextual = Attributes::default();
        self.add_with(gauge, delta, &contextual);
    }

    /// `add` under TS `Metric.CurrentMetricAttributes`.
    ///
    /// The delta goes through `mapInput` too, so a constant input applies to a
    /// modify exactly as it does to an update.
    pub fn add_with(&mut self, gauge: &Gauge, delta: f64, contextual: &Attributes) {
        let entry = self
            .gauges
            .entry(gauge.identity().key(MetricKind::Gauge, contextual))
            .or_insert_with(|| {
                (
                    GaugeState::zero(),
                    gauge.identity().merged_attributes(contextual),
                )
            });
        entry.0.value += gauge.identity().mapped_input(delta);
    }

    /// The series' value; zero when never updated.
    pub fn gauge_value(&self, gauge: &Gauge) -> GaugeState {
        let contextual = Attributes::default();
        self.gauge_value_with(gauge, &contextual)
    }

    /// `gauge_value` under TS `Metric.CurrentMetricAttributes`.
    pub fn gauge_value_with(&self, gauge: &Gauge, contextual: &Attributes) -> GaugeState {
        self.gauges
            .get(&gauge.identity().key(MetricKind::Gauge, contextual))
            .map(|(state, _)| *state)
            .unwrap_or_else(GaugeState::zero)
    }

    /// Every series' attributes and state (TS `snapshot`).
    pub fn snapshot(&self) -> Vec<Series> {
        self.counters
            .values()
            .map(|(state, attributes)| Series {
                attributes: reported(attributes),
                state: SeriesState::Counter(*state),
            })
            .chain(self.gauges.values().map(|(state, attributes)| Series {
                attributes: reported(attributes),
                state: SeriesState::Gauge(*state),
            }))
            .collect()
    }

    /// Drop every series.
    pub fn clear(&mut self) {
        self.counters.clear();
        self.gauges.clear();
    }
}

/// A series reports no attributes rather than an empty set.
fn reported(attributes: &Attributes) -> Option<Attributes> {
    if attributes.is_empty() {
        None
    } else {
        Some(attributes.clone())
    }
}
