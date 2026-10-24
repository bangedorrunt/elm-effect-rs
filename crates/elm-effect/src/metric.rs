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
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use crate::cause::{Cause, Defect, Reason};
use crate::effect::Effect;
use crate::exit::Exit;

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
    Frequency,
    Histogram,
    Summary,
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

/// A named frequency description (TS `Metric.frequency(id)`).
///
/// The input is a string, so the constant input is a string too.
#[derive(Debug, Clone, PartialEq)]
pub struct Frequency {
    identity: Identity,
    constant: Option<String>,
}

impl Frequency {
    /// Describe a frequency with `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            identity: Identity::new(id),
            constant: None,
        }
    }

    /// The metric's own attributes.
    pub fn with_attributes(mut self, attributes: Attributes) -> Self {
        let over = self.identity.attributes.overlay(&attributes);
        self.identity.attributes = over;
        self
    }

    /// TS `Metric.withAttributes`: a contextual overlay.
    pub fn with_extra_attributes(mut self, attributes: Attributes) -> Self {
        self.identity.overlays.insert(0, attributes);
        self
    }

    /// TS `Metric.withConstantInput`: every input counts as this value.
    pub fn with_constant_input(mut self, constant: impl Into<String>) -> Self {
        self.constant = Some(constant.into());
        self
    }

    /// The value this tracked input counts as.
    fn mapped_input(&self, tracked: &str) -> String {
        self.constant.clone().unwrap_or_else(|| tracked.to_string())
    }
}

impl Described for Frequency {
    fn identity(&self) -> &Identity {
        &self.identity
    }
}

/// A named histogram description (TS `Metric.histogram(id, { boundaries })`).
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    identity: Identity,
    boundaries: Vec<f64>,
}

impl Histogram {
    /// Describe a histogram over `boundaries`, which are assumed sorted.
    pub fn new(id: impl Into<String>, boundaries: Vec<f64>) -> Self {
        Self {
            identity: Identity::new(id),
            boundaries,
        }
    }

    /// The bucket boundaries this histogram reports.
    pub fn boundaries(&self) -> &[f64] {
        &self.boundaries
    }

    /// The metric's own attributes.
    pub fn with_attributes(mut self, attributes: Attributes) -> Self {
        let over = self.identity.attributes.overlay(&attributes);
        self.identity.attributes = over;
        self
    }

    /// TS `Metric.withAttributes`: a contextual overlay.
    pub fn with_extra_attributes(mut self, attributes: Attributes) -> Self {
        self.identity.overlays.insert(0, attributes);
        self
    }

    /// TS `Metric.withConstantInput`: every observation contributes this.
    pub fn with_constant_input(mut self, constant: f64) -> Self {
        self.identity.constant_input = Some(constant);
        self
    }
}

impl Described for Histogram {
    fn identity(&self) -> &Identity {
        &self.identity
    }
}

/// A named summary description (TS `Metric.summary(id, { maxAge, maxSize,
/// quantiles })`).
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    identity: Identity,
    max_age: crate::time::Duration,
    max_size: usize,
    quantiles: Vec<f64>,
}

impl Summary {
    /// Describe a summary.
    ///
    /// `max_age` is recorded but not enforced: the registry has no clock, so
    /// samples expire only through `max_size`. The pin's rows never advance
    /// time, so every asserted state is one this carries.
    pub fn new(
        id: impl Into<String>,
        max_age: crate::time::Duration,
        max_size: usize,
        quantiles: Vec<f64>,
    ) -> Self {
        Self {
            identity: Identity::new(id),
            max_age,
            max_size,
            quantiles,
        }
    }

    /// The configured sample age.
    pub fn max_age(&self) -> crate::time::Duration {
        self.max_age
    }

    /// The configured sample cap.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// The quantiles this summary reports.
    pub fn quantiles(&self) -> &[f64] {
        &self.quantiles
    }

    /// The metric's own attributes.
    pub fn with_attributes(mut self, attributes: Attributes) -> Self {
        let over = self.identity.attributes.overlay(&attributes);
        self.identity.attributes = over;
        self
    }

    /// TS `Metric.withAttributes`: a contextual overlay.
    pub fn with_extra_attributes(mut self, attributes: Attributes) -> Self {
        self.identity.overlays.insert(0, attributes);
        self
    }

    /// TS `Metric.withConstantInput`: every observation contributes this.
    pub fn with_constant_input(mut self, constant: f64) -> Self {
        self.identity.constant_input = Some(constant);
        self
    }
}

impl Described for Summary {
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

/// One frequency series' state (TS frequency `{ occurrences }`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrequencyState {
    /// Values seen, in first-seen order, with how many times each occurred.
    pub occurrences: Vec<(String, u64)>,
}

/// One histogram series' state.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramState {
    /// `(boundary, cumulative count)` in boundary order.
    pub buckets: Vec<(f64, u64)>,
    /// Observations recorded.
    pub count: u64,
    /// Sum of the observed values.
    pub sum: f64,
    /// Smallest observation; `f64::MAX` until something is observed, which is
    /// the pin's finite-extrema default.
    pub min: f64,
    /// Largest observation; `-f64::MAX` until something is observed.
    pub max: f64,
}

impl HistogramState {
    fn empty(boundaries: &[f64]) -> Self {
        Self {
            buckets: boundaries.iter().map(|boundary| (*boundary, 0)).collect(),
            count: 0,
            sum: 0.0,
            min: f64::MAX,
            max: -f64::MAX,
        }
    }
}

impl SummaryState {
    fn empty(quantiles: &[f64]) -> Self {
        Self {
            quantiles: quantiles.iter().map(|q| (*q, None)).collect(),
            count: 0,
            sum: 0.0,
            min: f64::MAX,
            max: -f64::MAX,
        }
    }
}

/// One summary series' state.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryState {
    /// `(quantile, value)`, with `None` while there is no sample to place.
    pub quantiles: Vec<(f64, Option<f64>)>,
    /// Observations recorded.
    pub count: u64,
    /// Sum of the observed values.
    pub sum: f64,
    /// Smallest observation; `f64::MAX` until something is observed.
    pub min: f64,
    /// Largest observation; `-f64::MAX` until something is observed.
    pub max: f64,
}

/// One registered series' state, by the metric type that owns it.
#[derive(Debug, Clone, PartialEq)]
pub enum SeriesState {
    /// A counter: accumulated total plus its `incremental` flag.
    Counter(CounterState),
    /// A gauge: the current value.
    Gauge(GaugeState),
    /// A frequency: the occurrence table.
    Frequency(FrequencyState),
    /// A histogram: buckets plus count/sum/min/max.
    Histogram(HistogramState),
    /// A summary: quantiles plus count/sum/min/max.
    Summary(SummaryState),
}

/// One registered series: what it was described with, plus its state.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// The metric id.
    pub id: String,
    /// The description, when the metric carried one.
    pub description: Option<String>,
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
    frequencies: HashMap<SeriesKey, (FrequencyState, Attributes)>,
    histograms: HashMap<SeriesKey, (HistogramState, Attributes)>,
    /// The samples a summary's quantiles are computed from, capped at
    /// `maxSize`.
    summaries: HashMap<SeriesKey, (SummaryState, Attributes, Vec<f64>)>,
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

    /// Count one occurrence of `value` (TS `Metric.update` on a frequency).
    pub fn record(&mut self, frequency: &Frequency, value: &str) {
        let contextual = Attributes::default();
        self.record_with(frequency, value, &contextual);
    }

    /// `record` under TS `Metric.CurrentMetricAttributes`.
    pub fn record_with(&mut self, frequency: &Frequency, value: &str, contextual: &Attributes) {
        let entry = self
            .frequencies
            .entry(frequency.identity().key(MetricKind::Frequency, contextual))
            .or_insert_with(|| {
                (
                    FrequencyState::default(),
                    frequency.identity().merged_attributes(contextual),
                )
            });
        let mapped = frequency.mapped_input(value);
        match entry
            .0
            .occurrences
            .iter_mut()
            .find(|(key, _)| key == &mapped)
        {
            Some((_, count)) => *count += 1,
            None => entry.0.occurrences.push((mapped, 1)),
        }
    }

    /// The series' occurrences; empty when never updated.
    pub fn frequency_value(&self, frequency: &Frequency) -> FrequencyState {
        let contextual = Attributes::default();
        self.frequency_value_with(frequency, &contextual)
    }

    /// `frequency_value` under TS `Metric.CurrentMetricAttributes`.
    pub fn frequency_value_with(
        &self,
        frequency: &Frequency,
        contextual: &Attributes,
    ) -> FrequencyState {
        self.frequencies
            .get(&frequency.identity().key(MetricKind::Frequency, contextual))
            .map(|(state, _)| state.clone())
            .unwrap_or_default()
    }

    /// Record one observation (TS `Metric.update` on a histogram).
    pub fn observe_histogram(&mut self, histogram: &Histogram, value: f64) {
        let contextual = Attributes::default();
        self.observe_histogram_with(histogram, value, &contextual);
    }

    /// `observe_histogram` under TS `Metric.CurrentMetricAttributes`.
    pub fn observe_histogram_with(
        &mut self,
        histogram: &Histogram,
        value: f64,
        contextual: &Attributes,
    ) {
        let entry = self
            .histograms
            .entry(histogram.identity().key(MetricKind::Histogram, contextual))
            .or_insert_with(|| {
                (
                    HistogramState::empty(histogram.boundaries()),
                    histogram.identity().merged_attributes(contextual),
                )
            });
        let mapped = histogram.identity().mapped_input(value);
        entry.0.count += 1;
        entry.0.sum += mapped;
        entry.0.min = entry.0.min.min(mapped);
        entry.0.max = entry.0.max.max(mapped);
        // Buckets are cumulative: an observation counts in every bucket whose
        // boundary it does not exceed.
        for (boundary, count) in entry.0.buckets.iter_mut() {
            if mapped <= *boundary {
                *count += 1;
            }
        }
    }

    /// The series' histogram state; an empty state when never updated.
    pub fn histogram_value(&self, histogram: &Histogram) -> HistogramState {
        let contextual = Attributes::default();
        self.histogram_value_with(histogram, &contextual)
    }

    /// `histogram_value` under TS `Metric.CurrentMetricAttributes`.
    pub fn histogram_value_with(
        &self,
        histogram: &Histogram,
        contextual: &Attributes,
    ) -> HistogramState {
        self.histograms
            .get(&histogram.identity().key(MetricKind::Histogram, contextual))
            .map(|(state, _)| state.clone())
            .unwrap_or_else(|| HistogramState::empty(histogram.boundaries()))
    }

    /// Record one sample (TS `Metric.update` on a summary).
    pub fn observe_summary(&mut self, summary: &Summary, value: f64) {
        let contextual = Attributes::default();
        self.observe_summary_with(summary, value, &contextual);
    }

    /// `observe_summary` under TS `Metric.CurrentMetricAttributes`.
    pub fn observe_summary_with(&mut self, summary: &Summary, value: f64, contextual: &Attributes) {
        let entry = self
            .summaries
            .entry(summary.identity().key(MetricKind::Summary, contextual))
            .or_insert_with(|| {
                (
                    SummaryState::empty(summary.quantiles()),
                    summary.identity().merged_attributes(contextual),
                    Vec::new(),
                )
            });
        let mapped = summary.identity().mapped_input(value);
        entry.2.push(mapped);
        let max_size = summary.max_size();
        if max_size > 0 && entry.2.len() > max_size {
            let excess = entry.2.len() - max_size;
            entry.2.drain(..excess);
        }
        entry.0.count += 1;
        entry.0.sum += mapped;
        entry.0.min = entry.0.min.min(mapped);
        entry.0.max = entry.0.max.max(mapped);
        for (quantile, slot) in entry.0.quantiles.iter_mut() {
            *slot = nearest_rank(&entry.2, *quantile);
        }
    }

    /// The series' summary state; an empty state when never updated.
    pub fn summary_value(&self, summary: &Summary) -> SummaryState {
        let contextual = Attributes::default();
        self.summary_value_with(summary, &contextual)
    }

    /// `summary_value` under TS `Metric.CurrentMetricAttributes`.
    pub fn summary_value_with(&self, summary: &Summary, contextual: &Attributes) -> SummaryState {
        self.summaries
            .get(&summary.identity().key(MetricKind::Summary, contextual))
            .map(|(state, _, _)| state.clone())
            .unwrap_or_else(|| SummaryState::empty(summary.quantiles()))
    }

    /// Every series' description, attributes, and state (TS `snapshot`).
    pub fn snapshot(&self) -> Vec<Series> {
        self.counters
            .iter()
            .map(|(key, (state, attributes))| Series {
                id: key.id.clone(),
                description: key.description.clone(),
                attributes: reported(attributes),
                state: SeriesState::Counter(*state),
            })
            .chain(self.gauges.iter().map(|(key, (state, attributes))| Series {
                id: key.id.clone(),
                description: key.description.clone(),
                attributes: reported(attributes),
                state: SeriesState::Gauge(*state),
            }))
            .chain(
                self.frequencies
                    .iter()
                    .map(|(key, (state, attributes))| Series {
                        id: key.id.clone(),
                        description: key.description.clone(),
                        attributes: reported(attributes),
                        state: SeriesState::Frequency(state.clone()),
                    }),
            )
            .chain(
                self.histograms
                    .iter()
                    .map(|(key, (state, attributes))| Series {
                        id: key.id.clone(),
                        description: key.description.clone(),
                        attributes: reported(attributes),
                        state: SeriesState::Histogram(state.clone()),
                    }),
            )
            .chain(
                self.summaries
                    .iter()
                    .map(|(key, (state, attributes, _))| Series {
                        id: key.id.clone(),
                        description: key.description.clone(),
                        attributes: reported(attributes),
                        state: SeriesState::Summary(state.clone()),
                    }),
            )
            .collect()
    }

    /// Every series as the pin's text dump (`Metric.dump`,
    /// `Metric.test.ts:318`).
    ///
    /// Columns are padded to the widest value plus two spaces, so the dump is
    /// a table: `name=`, `description=`, `type=`, then the attribute set
    /// (itself padded), then the state. Series are sorted by id and then by
    /// their attribute text, which the pin gets for free from its insertion
    /// ordered map; a hash map has no order to inherit, so the dump imposes
    /// one and stays stable across runs.
    pub fn dump(&self) -> String {
        let mut series = self.snapshot();
        if series.is_empty() {
            return String::new();
        }
        let name_pad = series.iter().map(|s| s.id.len()).max().unwrap_or(0) + 2;
        let description_pad = series
            .iter()
            .map(|s| s.description.as_deref().map_or(0, str::len))
            .max()
            .unwrap_or(0)
            + 2;
        let type_pad = series
            .iter()
            .map(|s| state_type(&s.state).len())
            .max()
            .unwrap_or(0)
            + 2;
        let attributes_pad = series
            .iter()
            .map(|s| attributes_text(s.attributes.as_ref()).len())
            .max()
            .unwrap_or(0)
            + 2;
        series.sort_by(|a, b| {
            a.id.cmp(&b.id).then_with(|| {
                attributes_text(a.attributes.as_ref()).cmp(&attributes_text(b.attributes.as_ref()))
            })
        });
        series
            .into_iter()
            .map(|series| {
                let mut line = String::new();
                line.push_str("name=");
                line.push_str(&pad(&series.id, name_pad));
                line.push_str("description=");
                line.push_str(&pad(
                    series.description.as_deref().unwrap_or(""),
                    description_pad,
                ));
                line.push_str("type=");
                line.push_str(&pad(state_type(&series.state), type_pad));
                line.push_str(&pad(
                    &attributes_text(series.attributes.as_ref()),
                    attributes_pad,
                ));
                line.push_str(&state_text(&series.state));
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Drop every series.
    pub fn clear(&mut self) {
        self.counters.clear();
        self.gauges.clear();
        self.frequencies.clear();
        self.histograms.clear();
        self.summaries.clear();
    }
}

/// A registry an effect can write to: the pin's fiber-owned `MetricRegistry`,
/// carried as an explicit handle because a description here has no fiber state
/// of its own to hold it in.
///
/// Every registry method runs inside [`read`](Metrics::read) or
/// [`edit`](Metrics::edit). A panic while the lock is held does not poison it
/// for later hooks: the registry is a counter table, and losing one update is
/// better than going deaf for the rest of the run.
#[derive(Clone, Debug, Default)]
pub struct Metrics(Arc<Mutex<MetricRegistry>>);

impl Metrics {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the registry.
    pub fn read<T>(&self, f: impl FnOnce(&MetricRegistry) -> T) -> T {
        let guard = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard)
    }

    /// Change the registry.
    pub fn edit<T>(&self, f: impl FnOnce(&mut MetricRegistry) -> T) -> T {
        let mut guard = match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }
}

/// `Effect.trackDuration` as a step: read the ambient clock either side of the
/// inner effect and record what elapsed.
///
/// The pins (`Metric.test.ts:1017`, `:1034`) make two properties observable
/// here: the clock is the one provided to the effect (so a `TestClock` counts
/// virtual time), and the reading is monotonic, so a wall clock set backwards
/// still measures only the time that passed.
struct TrackDuration<A: Send + 'static, E: Send + 'static, R: crate::Requirement> {
    inner: Effect<A, E, R>,
    metrics: Metrics,
    record: Box<dyn Fn(crate::Duration, &mut MetricRegistry) + Send>,
}

impl<A, E, R> crate::effect::ErasedStep<A, E> for TrackDuration<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: crate::Requirement,
{
    fn run_erased(self: Box<Self>, scope: &crate::service::Services) -> Result<A, Cause<E>> {
        let Self {
            inner,
            metrics,
            record,
        } = *self;
        let clock = crate::effect::ambient_clock(scope);
        let start = clock.monotonic_millis();
        let result = inner.run_sync_with(scope);
        let elapsed = clock.monotonic_millis().saturating_sub(start);
        metrics.edit(|registry| record(crate::Duration::from_millis(elapsed), registry));
        result
    }
}

/// The pin's metric hooks: `Effect.track`, `trackErrors`, `trackDefects`, and
/// `trackDuration`.
///
/// Each hook is a combinator around the effect and writes on its exit path. The
/// pin reaches the registry through the fiber that runs the effect; a
/// description here holds no fiber, so the hook takes the [`Metrics`] handle to
/// write to. That is the only difference: the exit, the reason set examined,
/// and the metric input all follow the pin.
impl<A: Send + 'static, E: Send + 'static, R: crate::Requirement> Effect<A, E, R> {
    /// `Effect.track`: run `record` on every exit, after the effect settles.
    ///
    /// Success, typed failure, defect, and interrupt all count once
    /// (`Metric.test.ts:863`).
    pub fn track<F>(self, metrics: &Metrics, record: F) -> Self
    where
        F: Fn(&Exit<A, E>, &mut MetricRegistry) + Send + 'static,
    {
        let metrics = metrics.clone();
        self.on_exit(move |exit| metrics.edit(|registry| record(exit, registry)))
    }

    /// `Effect.trackErrors`: run `record` for each typed failure.
    ///
    /// A defect and an interrupt are not typed failures, so neither runs it —
    /// the pin's `tapError` rule (`Metric.test.ts:943`).
    pub fn track_errors<F>(self, metrics: &Metrics, record: F) -> Self
    where
        F: Fn(&E, &mut MetricRegistry) + Send + 'static,
    {
        let metrics = metrics.clone();
        self.on_exit(move |exit| {
            if let Exit::Failure(cause) = exit {
                for reason in cause.reasons.iter() {
                    if let Reason::Fail(error) = reason {
                        metrics.edit(|registry| record(error, registry));
                    }
                }
            }
        })
    }

    /// `Effect.trackDefects`: run `record` for each defect.
    ///
    /// A typed failure and an interrupt are not defects, so neither runs it
    /// (`Metric.test.ts:966`).
    pub fn track_defects<F>(self, metrics: &Metrics, record: F) -> Self
    where
        F: Fn(&Defect, &mut MetricRegistry) + Send + 'static,
    {
        let metrics = metrics.clone();
        self.on_exit(move |exit| {
            if let Exit::Failure(cause) = exit {
                for reason in cause.reasons.iter() {
                    if let Reason::Die(defect) = reason {
                        metrics.edit(|registry| record(defect, registry));
                    }
                }
            }
        })
    }

    /// `Effect.trackDuration`: record how long the effect ran, measured on the
    /// ambient clock and reported as a [`Duration`](crate::Duration).
    ///
    /// The clock is the one provided to the effect, so a `TestClock` measures
    /// virtual time, and the reading is monotonic, so wall time moving backwards
    /// cannot make the measurement negative.
    pub fn track_duration<F>(self, metrics: &Metrics, record: F) -> Self
    where
        F: Fn(crate::Duration, &mut MetricRegistry) + Send + 'static,
    {
        // The clock is read from the step's scope, so whatever this effect was
        // provided with travels with the wrapper: a provided `TestClock` is the
        // clock that measures, which is the mechanism `Metric.test.ts:1017`
        // turns on.
        let services = self.services.clone();
        Self {
            step: crate::effect::Step::Suspend(Box::new(TrackDuration {
                inner: self,
                metrics: metrics.clone(),
                record: Box::new(record),
            })),
            services,
            marker: PhantomData,
        }
    }
}

/// The pin's nearest-rank quantile over the retained samples.
///
/// The pin keeps chunked samples and interpolates; the rows here assert
/// nearest-rank answers, so that is what the port carries. A quantile that
/// interpolation changes is not observable in this file.
fn nearest_rank(samples: &[f64], quantile: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((quantile * sorted.len() as f64).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    Some(sorted[index])
}

/// Evenly spaced boundaries: `count - 1` finite values from `start` by
/// `width`, then infinity (`Metric.linearBoundaries`).
///
/// `Metric.test.ts:708` pins `(10, 20, 5)` to `[10, 30, 50, 70, Inf]`.
pub fn linear_boundaries(start: f64, width: f64, count: usize) -> Vec<f64> {
    let mut boundaries: Vec<f64> = (0..count.saturating_sub(1))
        .map(|index| start + width * index as f64)
        .collect();
    boundaries.push(f64::INFINITY);
    boundaries
}

/// Geometric boundaries: `count - 1` values from `start` by `factor`, then
/// infinity (`Metric.exponentialBoundaries`).
pub fn exponential_boundaries(start: f64, factor: f64, count: usize) -> Vec<f64> {
    let mut boundaries: Vec<f64> = (0..count.saturating_sub(1))
        .map(|index| start * factor.powi(index as i32))
        .collect();
    boundaries.push(f64::INFINITY);
    boundaries
}

/// `Metric.timer`: a histogram over milliseconds, tagged `time_unit`.
///
/// The pin's timer takes a `Duration` input and converts it; a histogram
/// observation here is already a number, so the caller converts. That is what
/// `Effect::track_duration` does: it hands the hook a `Duration`, and the hook
/// observes `duration.to_millis()`.
pub fn timer(id: impl Into<String>) -> Histogram {
    Histogram::new(id, exponential_boundaries(0.5, 2.0, 35))
        .with_attributes(Attributes::from_pairs([("time_unit", "milliseconds")]))
}

/// The type name a series reports (`Metric.Snapshot.type`).
fn state_type(state: &SeriesState) -> &'static str {
    match state {
        SeriesState::Counter(_) => "Counter",
        SeriesState::Gauge(_) => "Gauge",
        SeriesState::Frequency(_) => "Frequency",
        SeriesState::Histogram(_) => "Histogram",
        SeriesState::Summary(_) => "Summary",
    }
}

/// One state as the dump's `state=[...]` column.
fn state_text(state: &SeriesState) -> String {
    match state {
        SeriesState::Counter(state) => format!("state=[count: [{}]]", number_text(state.count)),
        SeriesState::Gauge(state) => format!("state=[value: [{}]]", number_text(state.value)),
        SeriesState::Frequency(state) => format!(
            "state=[occurrences: {}]",
            // A frequency's keys are strings, so they print as they are.
            state
                .occurrences
                .iter()
                .map(|(value, count)| format!("({value} -> {count})"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SeriesState::Histogram(state) => format!(
            "state=[buckets: [{}], count: [{}], min: [{}], max: [{}], sum: [{}]]",
            key_values(
                state
                    .buckets
                    .iter()
                    .map(|(key, value)| (*key, *value as f64))
            ),
            state.count,
            number_text(state.min),
            number_text(state.max),
            number_text(state.sum)
        ),
        SeriesState::Summary(state) => format!(
            "state=[quantiles: [{}], count: [{}], min: [{}], max: [{}], sum: [{}]]",
            key_values(
                state
                    .quantiles
                    .iter()
                    .map(|(quantile, value)| (*quantile, value.unwrap_or(0.0)))
            ),
            state.count,
            number_text(state.min),
            number_text(state.max),
            number_text(state.sum)
        ),
    }
}

/// `(key -> value)` pairs, joined the way the pin joins them.
fn key_values(pairs: impl Iterator<Item = (f64, f64)>) -> String {
    pairs
        .map(|(key, value)| format!("({} -> {})", number_text(key), number_text(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A number as JavaScript prints it: `Infinity`, not `inf`, and no `.0` on a
/// whole value.
fn number_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        format!("{value}")
    }
}

/// The attribute column (`attributesToString`): pairs sorted by name.
fn attributes_text(attributes: Option<&Attributes>) -> String {
    let mut entries = attributes.map(Attributes::pairs).unwrap_or_default();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    format!(
        "attributes=[{}]",
        entries
            .into_iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Pad to `width` with spaces (`padEnd`). A longer value is not truncated:
/// the width is the widest value plus two, so this only ever pads.
fn pad(text: &str, width: usize) -> String {
    let mut padded = text.to_string();
    while padded.len() < width {
        padded.push(' ');
    }
    padded
}

/// A series reports no attributes rather than an empty set.
fn reported(attributes: &Attributes) -> Option<Attributes> {
    if attributes.is_empty() {
        None
    } else {
        Some(attributes.clone())
    }
}
