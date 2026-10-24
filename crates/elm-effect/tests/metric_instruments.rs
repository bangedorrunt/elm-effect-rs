//! Metric instruments: frequency, histogram, summary, and their boundaries.

use elm_effect::{
    Attributes, Duration, Frequency, Histogram, MetricRegistry, Summary, linear_boundaries,
};

#[test]
fn metric_frequency_records_occurrences() {
    // Metric.test.ts:658 — each observed value is counted, in first-seen order.
    let frequency = Frequency::new("status").with_attributes(Attributes::from_pairs([("a", "b")]));
    let mut registry = MetricRegistry::new();
    registry.record(&frequency, "foo");
    registry.record(&frequency, "bar");
    assert_eq!(
        registry.frequency_value(&frequency).occurrences,
        vec![("foo".to_string(), 1), ("bar".to_string(), 1)]
    );

    // :672 — a constant input replaces every value, so one key counts twice.
    let constant = Frequency::new("status-constant").with_constant_input("constant");
    registry.record(&constant, "foo");
    registry.record(&constant, "bar");
    assert_eq!(
        registry.frequency_value(&constant).occurrences,
        vec![("constant".to_string(), 2)]
    );
}

#[test]
fn metric_empty_instruments_use_finite_extrema() {
    // :688 — the empty states carry Effect 4's finite sentinels.
    let registry = MetricRegistry::new();
    let histogram = Histogram::new("empty-histogram", Vec::new());
    let summary = Summary::new("empty-summary", Duration::from_secs(60), 10, Vec::new());
    let histogram_state = registry.histogram_value(&histogram);
    let summary_state = registry.summary_value(&summary);
    assert_eq!(histogram_state.min, f64::MAX);
    assert_eq!(histogram_state.max, -f64::MAX);
    assert_eq!(summary_state.min, f64::MAX);
    assert_eq!(summary_state.max, -f64::MAX);
}

#[test]
fn metric_linear_boundaries() {
    // :708 — count - 1 finite boundaries, then infinity.
    assert_eq!(
        linear_boundaries(10.0, 20.0, 5),
        vec![10.0, 30.0, 50.0, 70.0, f64::INFINITY]
    );
}

#[test]
fn metric_histogram_observations() {
    // :716 — negative-only observations still report their maximum.
    let mut registry = MetricRegistry::new();
    let negative = Histogram::new("negative", vec![-10.0, -5.0, 0.0]);
    registry.observe_histogram(&negative, -10.0);
    registry.observe_histogram(&negative, -5.0);
    let state = registry.histogram_value(&negative);
    assert_eq!(state.min, -10.0);
    assert_eq!(state.max, -5.0);

    // :726 — buckets are cumulative and carry count/sum/min/max.
    let boundaries = linear_boundaries(0.0, 1.0, 10);
    let histogram = Histogram::new("latency", boundaries.clone());
    registry.observe_histogram(&histogram, 1.0);
    registry.observe_histogram(&histogram, 3.0);
    let state = registry.histogram_value(&histogram);
    assert_eq!(
        state.buckets,
        vec![
            (0.0, 0),
            (1.0, 1),
            (2.0, 1),
            (3.0, 2),
            (4.0, 2),
            (5.0, 2),
            (6.0, 2),
            (7.0, 2),
            (8.0, 2),
            (f64::INFINITY, 2),
        ]
    );
    assert_eq!(
        (state.count, state.sum, state.min, state.max),
        (2u64, 4.0, 1.0, 3.0)
    );

    // :745 — a constant input is what every bucket sees.
    let constant = Histogram::new("constant", boundaries.clone()).with_constant_input(1.0);
    registry.observe_histogram(&constant, 1.0);
    registry.observe_histogram(&constant, 3.0);
    let state = registry.histogram_value(&constant);
    assert_eq!(
        (state.count, state.sum, state.min, state.max),
        (2u64, 2.0, 1.0, 1.0)
    );
    assert_eq!(state.buckets[1], (1.0, 2));

    // :765 — the reported boundaries keep their exact values.
    let precise = Histogram::new("precision", vec![0.005, 0.01, 0.025, 0.05, 0.075, 0.1]);
    let state = registry.histogram_value(&precise);
    let reported: Vec<f64> = state
        .buckets
        .iter()
        .map(|(boundary, _)| *boundary)
        .collect();
    assert_eq!(reported, vec![0.005, 0.01, 0.025, 0.05, 0.075, 0.1]);
}

#[test]
fn metric_summary_observations() {
    // :782 — negative-only observations report their maximum.
    let mut registry = MetricRegistry::new();
    let negative = Summary::new("summary-negative", Duration::from_secs(60), 10, vec![0.5]);
    registry.observe_summary(&negative, -10.0);
    registry.observe_summary(&negative, -5.0);
    let state = registry.summary_value(&negative);
    assert_eq!(state.min, -10.0);
    assert_eq!(state.max, -5.0);

    // :796 — quantiles, count, sum, min, and max over two samples.
    let summary = Summary::new("summary", Duration::from_secs(60), 10, vec![0.0, 0.1, 0.9])
        .with_attributes(Attributes::from_pairs([("a", "b")]));
    registry.observe_summary(&summary, 1.0);
    registry.observe_summary(&summary, 3.0);
    let state = registry.summary_value(&summary);
    assert_eq!(
        state.quantiles,
        vec![(0.0, Some(1.0)), (0.1, Some(1.0)), (0.9, Some(3.0))]
    );
    assert_eq!(
        (state.count, state.sum, state.min, state.max),
        (2u64, 4.0, 1.0, 3.0)
    );

    // :817 — a constant input places every sample at 1.
    let constant = Summary::new(
        "summary-constant",
        Duration::from_secs(60),
        10,
        vec![0.0, 0.1, 0.9],
    )
    .with_constant_input(1.0);
    registry.observe_summary(&constant, 1.0);
    registry.observe_summary(&constant, 3.0);
    let state = registry.summary_value(&constant);
    assert_eq!(
        state.quantiles,
        vec![(0.0, Some(1.0)), (0.1, Some(1.0)), (0.9, Some(1.0))]
    );
    assert_eq!(
        (state.count, state.sum, state.min, state.max),
        (2u64, 2.0, 1.0, 1.0)
    );

    // :841 — the median over a chunk that overshoots lands on the sample below
    // the half-way point.
    let overshoot = Summary::new("summary-overshoot", Duration::from_secs(60), 15, vec![0.5]);
    for sample in [10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 20.0, 30.0, 40.0, 50.0] {
        registry.observe_summary(&overshoot, sample);
    }
    let state = registry.summary_value(&overshoot);
    assert_eq!(state.quantiles, vec![(0.5, Some(10.0))]);
    assert_eq!(
        (state.count, state.sum, state.min, state.max),
        (10u64, 200.0, 10.0, 50.0)
    );
}
