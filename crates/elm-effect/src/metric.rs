//! Metric registry: named series keyed by (type, id, description, attributes).
//!
//! The registry is an ordinary value (TS `MetricRegistry` map): updates
//! accumulate per registry, so separate registries stay isolated and `clear`
//! drops every series. Counter state mirrors TS `{ count, incremental }`;
//! plain counters never increment incrementally.
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
    pub fn from_pairs(pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
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

/// A named counter description (TS `Metric.counter(id, options)`).
///
/// `overlays` holds `withAttributes` applications in pipe order, first applied
/// first, because the pin applies the outermost pipe last and the first
/// overlay therefore wins.
#[derive(Debug, Clone, PartialEq)]
pub struct Counter {
    id: String,
    description: Option<String>,
    attributes: Attributes,
    overlays: Vec<Attributes>,
    incremental: bool,
    constant_input: Option<f64>,
}

impl Counter {
    /// Describe a counter with `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: None,
            attributes: Attributes::default(),
            overlays: Vec::new(),
            incremental: false,
            constant_input: None,
        }
    }

    /// Attach a description, which is distinct from attributes.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The metric's own attributes (TS `Metric.counter`'s option).
    pub fn with_attributes(mut self, attributes: Attributes) -> Self {
        self.attributes = self.attributes.overlay(&attributes);
        self
    }

    /// TS `Metric.withAttributes`: a contextual overlay. The first overlay
    /// applied wins over later ones, which both win over context.
    pub fn with_extra_attributes(mut self, attributes: Attributes) -> Self {
        self.overlays.insert(0, attributes);
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
        self.constant_input = Some(constant);
        self
    }

    /// The amount this input contributes, or `None` when the counter refuses
    /// it. TS `canUpdate` for an incremental counter is `value >= 0`, applied
    /// to the mapped input.
    pub fn input_amount(&self, tracked: f64) -> Option<f64> {
        let amount = self.constant_input.unwrap_or(tracked);
        if self.incremental && amount < 0.0 {
            None
        } else {
            Some(amount)
        }
    }

    /// The registry key. Attribute entries are sorted by name, so the record
    /// order a metric was described in is not part of its identity.
    // ponytail: counters only; add `kind` to SeriesKey the day a second type lands.
    fn key(&self, contextual: &Attributes) -> SeriesKey {
        SeriesKey {
            id: self.id.clone(),
            description: self.description.clone(),
            attributes: self.merged_attributes(contextual).sorted_entries(),
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
}

/// The registry key. A tuple encoding, so no field's text can impersonate
/// another field's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SeriesKey {
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

/// One registered series: its attributes (absent when unattributed) and state.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// `None` for the series an empty attribute set shares.
    pub attributes: Option<Attributes>,
    /// The counter's state.
    pub state: CounterState,
}

/// Registry of counter series by series key (TS `MetricRegistry`).
#[derive(Debug, Clone, Default)]
pub struct MetricRegistry {
    /// The state plus the merged attributes in insertion order, because a
    /// snapshot reports the order the metric was described with and the key
    /// stored a sorted copy.
    series: HashMap<SeriesKey, (CounterState, Attributes)>,
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
        let key = counter.key(contextual);
        let entry = self
            .series
            .entry(key)
            .or_insert_with(|| (CounterState::zero(), counter.merged_attributes(contextual)));
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
        self.series
            .get(&counter.key(contextual))
            .map(|(state, _)| *state)
            .unwrap_or_else(CounterState::zero)
    }

    /// Every series' attributes and state (TS `snapshot`).
    pub fn snapshot(&self) -> Vec<Series> {
        self.series
            .values()
            .map(|(state, attributes)| Series {
                attributes: if attributes.is_empty() {
                    None
                } else {
                    Some(attributes.clone())
                },
                state: *state,
            })
            .collect()
    }

    /// Drop every series.
    pub fn clear(&mut self) {
        self.series.clear();
    }
}
