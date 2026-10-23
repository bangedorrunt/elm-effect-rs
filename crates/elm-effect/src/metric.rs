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
        match entry.0.occurrences.iter_mut().find(|(key, _)| key == &mapped) {
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
    pub fn observe_summary_with(
        &mut self,
        summary: &Summary,
        value: f64,
        contextual: &Attributes,
    ) {
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
    pub fn summary_value_with(
        &self,
        summary: &Summary,
        contextual: &Attributes,
    ) -> SummaryState {
        self.summaries
            .get(&summary.identity().key(MetricKind::Summary, contextual))
            .map(|(state, _, _)| state.clone())
            .unwrap_or_else(|| SummaryState::empty(summary.quantiles()))
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
            .chain(
                self.frequencies
                    .values()
                    .map(|(state, attributes)| Series {
                        attributes: reported(attributes),
                        state: SeriesState::Frequency(state.clone()),
                    }),
            )
            .chain(
                self.histograms
                    .values()
                    .map(|(state, attributes)| Series {
                        attributes: reported(attributes),
                        state: SeriesState::Histogram(state.clone()),
                    }),
            )
            .chain(self.summaries.values().map(|(state, attributes, _)| Series {
                attributes: reported(attributes),
                state: SeriesState::Summary(state.clone()),
            }))
            .collect()
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

/// A series reports no attributes rather than an empty set.
fn reported(attributes: &Attributes) -> Option<Attributes> {
    if attributes.is_empty() {
        None
    } else {
        Some(attributes.clone())
    }
}
