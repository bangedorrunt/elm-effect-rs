//! Metric registry scoping: isolation across registries, clear/re-register,
//! and the attribute series key (TS `Metric.test.ts` registry rows).

use elm_effect::{Attributes, Counter, Gauge, MetricRegistry, SeriesState};

/// TS `series`: `(attributes ?? null, count)`, ordered the way the pin's own
/// `JSON.stringify` sort orders them (`null` sorts before any object).
fn series(registry: &MetricRegistry) -> Vec<(Option<Vec<(String, String)>>, f64)> {
    let mut rows: Vec<(Option<Vec<(String, String)>>, f64)> = registry
        .snapshot()
        .into_iter()
        .filter_map(|s| match s.state {
            SeriesState::Counter(state) => Some((s.attributes.map(|a| a.pairs()), state.count)),
            SeriesState::Gauge(_) => None,
        })
        .collect();
    rows.sort_by(|a, b| sort_key(&a.0).cmp(&sort_key(&b.0)));
    rows
}

fn sort_key(attrs: &Option<Vec<(String, String)>>) -> String {
    match attrs {
        None => "null".to_string(),
        Some(pairs) => {
            let body = pairs
                .iter()
                .map(|(k, v)| format!("{k:?}:{v:?}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
    }
}

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

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
    match snapshots[0].state {
        SeriesState::Counter(state) => {
            assert_eq!(state.count, 10.0);
            assert!(!state.incremental);
        }
        SeriesState::Gauge(_) => panic!("expected a counter series"),
    }
    assert_eq!(registry.value(&counter).count, 10.0);
}

#[test]
fn attributed_metrics_stay_scoped_to_active_registry() {
    let counter = Counter::new("requests").with_attributes(Attributes::from_pairs([
        ("x", "a"),
        ("y", "b"),
    ]));
    let mut registry_a = MetricRegistry::new();
    let mut registry_b = MetricRegistry::new();
    registry_a.update(&counter, 1.0);
    registry_b.update(&counter, 10.0);
    assert_eq!(registry_a.value(&counter).count, 1.0);
    assert_eq!(registry_b.value(&counter).count, 10.0);
}

#[test]
fn no_attributes_and_empty_attributes_share_one_series() {
    let plain = Counter::new("shares");
    let empty_record = Counter::new("shares").with_attributes(Attributes::empty());
    let empty_array = Counter::new("shares").with_attributes(Attributes::empty());
    let mut registry = MetricRegistry::new();
    registry.update(&plain, 1.0);
    registry.update(&empty_record, 10.0);
    registry.update(&empty_array, 100.0);
    assert_eq!(registry.snapshot().len(), 1);
    assert_eq!(registry.value(&plain).count, 111.0);
}

#[test]
fn distinct_attribute_sets_stay_in_separate_series() {
    let mut registry = MetricRegistry::new();
    let joined =
        Counter::new("distinct").with_attributes(Attributes::from_pairs([("a", "b,c=d")]));
    let split = Counter::new("distinct")
        .with_attributes(Attributes::from_pairs([("a", "b"), ("c", "d")]));
    registry.update(&joined, 1.0);
    registry.update(&split, 10.0);
    assert_eq!(registry.value(&joined).count, 1.0);
    assert_eq!(registry.value(&split).count, 10.0);
    assert_eq!(registry.snapshot().len(), 2);
}

#[test]
fn a_colon_in_the_id_is_not_a_description() {
    let mut registry = MetricRegistry::new();
    let first = Counter::new("colon:requests");
    let second = Counter::new("colon").with_description("requests");
    registry.update(&first, 1.0);
    registry.update(&second, 10.0);
    assert_eq!(registry.value(&first).count, 1.0);
    assert_eq!(registry.value(&second).count, 10.0);
    assert_eq!(registry.snapshot().len(), 2);
}

#[test]
fn a_description_is_not_attributes() {
    let mut registry = MetricRegistry::new();
    let first = Counter::new("descr").with_description(r#"[["route","/users"]]"#);
    let second = Counter::new("descr").with_attributes(Attributes::from_pairs([
        ("route", "/users"),
    ]));
    registry.update(&first, 1.0);
    registry.update(&second, 10.0);
    assert_eq!(registry.value(&first).count, 1.0);
    assert_eq!(registry.value(&second).count, 10.0);
    assert_eq!(registry.snapshot().len(), 2);
}

#[test]
fn equal_names_with_different_values_stay_separate() {
    let mut registry = MetricRegistry::new();
    let first = Counter::new("names").with_attributes(Attributes::from_pairs([
        ("route", "/users"),
        ("method", "GET"),
    ]));
    let second = Counter::new("names").with_attributes(Attributes::from_pairs([
        ("route", "/users"),
        ("method", "POST"),
    ]));
    registry.update(&first, 1.0);
    registry.update(&second, 10.0);
    assert_eq!(registry.value(&first).count, 1.0);
    assert_eq!(registry.value(&second).count, 10.0);
}

#[test]
fn equal_attributes_in_different_record_orders_share_a_series() {
    let mut registry = MetricRegistry::new();
    let first = Counter::new("orders").with_attributes(Attributes::from_pairs([
        ("route", "/users"),
        ("method", "GET"),
    ]));
    let second = Counter::new("orders").with_attributes(Attributes::from_pairs([
        ("method", "GET"),
        ("route", "/users"),
    ]));
    registry.update(&first, 1.0);
    registry.update(&second, 10.0);
    assert_eq!(registry.value(&first).count, 11.0);
    assert_eq!(registry.snapshot().len(), 1);
}

#[test]
fn contextual_attributes_that_change_split_a_plain_metric() {
    let counter = Counter::new("ctx");
    let tenant_a = Attributes::from_pairs([("tenant", "a")]);
    let tenant_b = Attributes::from_pairs([("tenant", "b")]);
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, 1.0, &tenant_a);
    registry.update_with(&counter, 10.0, &tenant_b);
    registry.update_with(&counter, 100.0, &tenant_a);
    registry.update(&counter, 1000.0);
    assert_eq!(
        series(&registry),
        vec![
            (None, 1000.0),
            (Some(vec![pair("tenant", "a")]), 101.0),
            (Some(vec![pair("tenant", "b")]), 10.0),
        ]
    );
}

#[test]
fn merging_metric_and_contextual_attributes_shares_a_series() {
    let first = Counter::new("merge").with_attributes(Attributes::from_pairs([
        ("route", "/users"),
    ]));
    let second =
        Counter::new("merge").with_attributes(Attributes::from_pairs([("method", "GET")]));
    let first_context = Attributes::from_pairs([("method", "GET")]);
    let second_context = Attributes::from_pairs([("route", "/users")]);
    let mut registry = MetricRegistry::new();
    registry.update_with(&first, 1.0, &first_context);
    registry.update_with(&second, 10.0, &second_context);
    assert_eq!(
        registry.value_with(&first, &first_context).count,
        11.0
    );
}

#[test]
fn snapshots_preserve_attribute_order() {
    let metric = Counter::new("order").with_attributes(Attributes::from_pairs([
        ("route", "/users"),
        ("method", "GET"),
    ]));
    let mut registry = MetricRegistry::new();
    registry.update(&metric, 1.0);
    let snapshots = registry.snapshot();
    let attributes = snapshots[0]
        .attributes
        .as_ref()
        .expect("the attributed series reports its attributes");
    let keys: Vec<String> = attributes.pairs().into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, vec!["route", "method"]);
}

#[test]
fn with_attributes_follows_the_contextual_attributes() {
    let counter = Counter::new("wa").with_extra_attributes(Attributes::from_pairs([
        ("route", "/users"),
    ]));
    let tenant_a = Attributes::from_pairs([("tenant", "a")]);
    let tenant_b = Attributes::from_pairs([("tenant", "b")]);
    let mut registry = MetricRegistry::new();
    registry.update(&counter, 1.0);
    registry.update_with(&counter, 10.0, &tenant_a);
    registry.update_with(&counter, 100.0, &tenant_b);
    registry.update(&counter, 1000.0);
    assert_eq!(
        series(&registry),
        vec![
            (Some(vec![pair("route", "/users")]), 1001.0),
            (
                Some(vec![pair("tenant", "a"), pair("route", "/users")]),
                10.0
            ),
            (
                Some(vec![pair("tenant", "b"), pair("route", "/users")]),
                100.0
            ),
        ]
    );
}

#[test]
fn inner_attributes_win_over_outer_contextual_and_metric() {
    let counter = Counter::new("precedence")
        .with_attributes(Attributes::from_pairs([
            ("a", "own"),
            ("b", "own"),
            ("c", "own"),
            ("d", "own"),
        ]))
        .with_extra_attributes(Attributes::from_pairs([("a", "inner")]))
        .with_extra_attributes(Attributes::from_pairs([("a", "outer"), ("b", "outer")]));
    let context = Attributes::from_pairs([("a", "context"), ("b", "context"), ("c", "context")]);
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, 1.0, &context);
    registry.update(&counter, 10.0);
    assert_eq!(
        series(&registry),
        vec![
            (
                Some(vec![
                    pair("a", "inner"),
                    pair("b", "outer"),
                    pair("c", "context"),
                    pair("d", "own"),
                ]),
                1.0
            ),
            (
                Some(vec![
                    pair("a", "inner"),
                    pair("b", "outer"),
                    pair("c", "own"),
                    pair("d", "own"),
                ]),
                10.0
            ),
        ]
    );
}

#[test]
fn a_cleared_registry_registers_the_attributed_series_again() {
    let counter = Counter::new("clr").with_extra_attributes(Attributes::from_pairs([
        ("route", "/users"),
    ]));
    let mut registry = MetricRegistry::new();
    registry.update(&counter, 1.0);
    registry.clear();
    registry.update(&counter, 10.0);
    assert_eq!(
        series(&registry),
        vec![(Some(vec![pair("route", "/users")]), 10.0)]
    );
}

#[test]
fn empty_attributes_use_the_unattributed_series() {
    let counter = Counter::new("empty");
    let empty_record = Counter::new("empty").with_extra_attributes(Attributes::empty());
    let empty_array = Counter::new("empty").with_extra_attributes(Attributes::empty());
    let mut registry = MetricRegistry::new();
    registry.update(&empty_record, 10.0);
    registry.update(&empty_array, 100.0);
    registry.update(&counter, 1.0);
    assert_eq!(series(&registry), vec![(None, 111.0)]);
}

// The counter custom-input rows (Metric.test.ts 428-541). Each uses the same
// contextual attribute set, so every row also exercises the series key.

fn custom_attributes() -> Attributes {
    Attributes::from_pairs([("x", "a"), ("y", "b")])
}

#[test]
fn custom_increment_with_value() {
    let counter = Counter::new("counter");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, 1.0, &contextual);
    registry.update_with(&counter, 2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, 3.0);
    assert!(!state.incremental);
}

#[test]
fn custom_increment_with_constant() {
    let counter = Counter::new("counter").with_constant_input(1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, 1.0, &contextual);
    registry.update_with(&counter, 2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, 2.0);
    assert!(!state.incremental);
}

#[test]
fn custom_decrement_with_value() {
    let counter = Counter::new("counter");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, -1.0, &contextual);
    registry.update_with(&counter, -2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, -3.0);
    assert!(!state.incremental);
}

#[test]
fn custom_decrement_with_constant() {
    let counter = Counter::new("counter").with_constant_input(-1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, -1.0, &contextual);
    registry.update_with(&counter, -2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, -2.0);
    assert!(!state.incremental);
}

// The four bigint rows carry the same arithmetic as their f64 neighbours. The
// module's established representation is an f64 count, which is exact for these
// magnitudes.
// ponytail: f64 count; switch the series state to i128 if a row needs > 2^53.
#[test]
fn custom_increment_with_bigint_value() {
    let counter = Counter::new("counter");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, 1.0, &contextual);
    registry.update_with(&counter, 2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, 3.0);
    assert!(!state.incremental);
}

#[test]
fn custom_increment_with_bigint_constant() {
    let counter = Counter::new("counter").with_constant_input(1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, 1.0, &contextual);
    registry.update_with(&counter, 2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, 2.0);
    assert!(!state.incremental);
}

#[test]
fn custom_decrement_with_bigint_value() {
    let counter = Counter::new("counter");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, -1.0, &contextual);
    registry.update_with(&counter, -2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, -3.0);
    assert!(!state.incremental);
}

#[test]
fn custom_decrement_with_bigint_constant() {
    let counter = Counter::new("counter").with_constant_input(-1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, -1.0, &contextual);
    registry.update_with(&counter, -2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, -2.0);
    assert!(!state.incremental);
}

#[test]
fn incremental_counter_refuses_a_decrement() {
    let counter = Counter::new("counter")
        .incremental()
        .with_constant_input(-1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, -1.0, &contextual);
    registry.update_with(&counter, -2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, 0.0);
    assert!(state.incremental);
}

#[test]
fn incremental_bigint_counter_refuses_a_decrement() {
    let counter = Counter::new("counter")
        .incremental()
        .with_constant_input(-1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.update_with(&counter, -1.0, &contextual);
    registry.update_with(&counter, -2.0, &contextual);
    let state = registry.value_with(&counter, &contextual);
    assert_eq!(state.count, 0.0);
    assert!(state.incremental);
}

// The gauge rows (Metric.test.ts 247, 556-654). A gauge's update replaces the
// value and its modify adds to it, which is the opposite of a counter.

#[test]
fn modifies_and_reads_the_attributed_series() {
    let gauge = Gauge::new("gauge");
    let tagged = Gauge::new("gauge").with_extra_attributes(Attributes::from_pairs([
        ("route", "/users"),
    ]));
    let contextual = Attributes::default();
    let mut registry = MetricRegistry::new();
    registry.set_with(&tagged, 5.0, &contextual);
    registry.add_with(&tagged, 2.0, &contextual);
    registry.set_with(&gauge, 1.0, &contextual);
    assert_eq!(registry.gauge_value_with(&tagged, &contextual).value, 7.0);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, 1.0);
}

#[test]
fn gauge_set_with_value() {
    let gauge = Gauge::new("gauge");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, 1.0, &contextual);
    registry.set_with(&gauge, 2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, 2.0);
}

#[test]
fn gauge_set_with_constant() {
    let gauge = Gauge::new("gauge").with_constant_input(1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, 1.0, &contextual);
    registry.set_with(&gauge, 2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, 1.0);
}

#[test]
fn gauge_set_with_negative_value() {
    let gauge = Gauge::new("gauge");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, -1.0, &contextual);
    registry.set_with(&gauge, -2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, -2.0);
}

#[test]
fn gauge_set_with_negative_constant() {
    let gauge = Gauge::new("gauge").with_constant_input(-1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, -1.0, &contextual);
    registry.set_with(&gauge, -2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, -1.0);
}

// The four bigint gauge rows carry the same arithmetic as their f64 neighbours
// under the module's established f64 value representation.
// ponytail: f64 value; switch the series state to i128 if a row needs > 2^53.
#[test]
fn gauge_set_with_bigint_value() {
    let gauge = Gauge::new("gauge");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, 1.0, &contextual);
    registry.set_with(&gauge, 2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, 2.0);
}

#[test]
fn gauge_set_with_bigint_constant() {
    let gauge = Gauge::new("gauge").with_constant_input(1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, 1.0, &contextual);
    registry.set_with(&gauge, 2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, 1.0);
}

#[test]
fn gauge_set_with_negative_bigint_value() {
    let gauge = Gauge::new("gauge");
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, -1.0, &contextual);
    registry.set_with(&gauge, -2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, -2.0);
}

#[test]
fn gauge_set_with_negative_bigint_constant() {
    let gauge = Gauge::new("gauge").with_constant_input(-1.0);
    let contextual = custom_attributes();
    let mut registry = MetricRegistry::new();
    registry.set_with(&gauge, -1.0, &contextual);
    registry.set_with(&gauge, -2.0, &contextual);
    assert_eq!(registry.gauge_value_with(&gauge, &contextual).value, -1.0);
}
