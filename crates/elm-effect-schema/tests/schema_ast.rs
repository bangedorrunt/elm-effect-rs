use elm_effect_schema::{Schema, SchemaKind, Unknown};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn ast_visitor_walks_nested_kinds() {
    let schema = Schema::array(Schema::string());
    let mut count = 0usize;
    schema.representation(); // smoke
    schema.kind().visit(&mut |_| count += 1);
    assert_eq!(count, 2);
}

#[test]
fn to_formatter_on_before_runs_before_render() {
    let schema = Schema::string();
    let called = Arc::new(AtomicBool::new(false));
    let called2 = called.clone();
    let formatter = schema.to_formatter(Some(move |_value: &Unknown| {
        assert!(!called2.load(Ordering::SeqCst));
        called2.store(true, Ordering::SeqCst);
    }));
    let rendered = formatter(&Unknown::Text("hello".into()));
    assert!(called.load(Ordering::SeqCst));
    assert!(rendered.contains("hello"));
}

#[test]
fn representation_round_trips_kind() {
    let schema = Schema::string();
    let repr = schema.representation();
    assert!(repr.to_json_string().contains("String"));
}
