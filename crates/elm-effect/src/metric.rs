//! Metric registry: named counter series with explicit provisioning.
//!
//! The registry is an ordinary value (TS `MetricRegistry` map): updates
//! accumulate per registry, so separate registries stay isolated and
//! `clear` drops every series. Counter state mirrors TS
//! `{ count, incremental }`; plain counters never increment incrementally.
//! Attribute series, histograms, and frequencies arrive as later rows.

use std::collections::HashMap;

/// A named counter description (TS `Metric.counter(id)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Counter {
    id: String,
}

impl Counter {
    /// Describe a counter with `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// The registry key.
    pub fn id(&self) -> &str {
        &self.id
    }
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

/// Registry of counter series by metric id (TS `MetricRegistry`).
#[derive(Debug, Clone, Default)]
pub struct MetricRegistry {
    series: HashMap<String, CounterState>,
}

impl MetricRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `amount` to the counter's total.
    pub fn update(&mut self, counter: &Counter, amount: f64) {
        let state = self.series.entry(counter.id.clone()).or_insert_with(CounterState::zero);
        state.count += amount;
    }

    /// The counter's total; zero when never updated.
    pub fn value(&self, counter: &Counter) -> CounterState {
        self.series.get(&counter.id).copied().unwrap_or_else(CounterState::zero)
    }

    /// Every series state (TS `snapshot`, states only).
    pub fn snapshot(&self) -> Vec<CounterState> {
        self.series.values().copied().collect()
    }

    /// Drop every series.
    pub fn clear(&mut self) {
        self.series.clear();
    }
}
