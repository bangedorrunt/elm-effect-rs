//! Metric registry scoping: isolation across registries, clear/re-register.

use elm_effect::{Counter, MetricRegistry};

#[test]
fn shared_metrics_stay_scoped_to_active_registry() {
    let counter = Counter::new("requests");
    let mut registry_a = MetricRegistry::new();
    let mut registry_b = MetricRegistry::new();
    registry_a.update(&counter, 1.0);
    registry_b.update(&counter, 10.0);
    registry_a.update(&counter, 2.0);
    let value_a = registry_a.value(&counter);
    let value_b = registry_b.value(&counter);
    assert_eq!(value_a.count, 3.0);
    assert!(!value_a.incremental);
    assert_eq!(value_b.count, 10.0);
    assert!(!value_b.incremental);
}

#[test]
fn cleared_registry_reregisters_unattributed_metric() {
    let counter = Counter::new("events");
    let mut registry = MetricRegistry::new();
    registry.update(&counter, 1.0);
    registry.clear();
    registry.update(&counter, 10.0);
    let snapshots = registry.snapshot();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].count, 10.0);
    assert!(!snapshots[0].incremental);
    assert_eq!(registry.value(&counter).count, 10.0);
}
