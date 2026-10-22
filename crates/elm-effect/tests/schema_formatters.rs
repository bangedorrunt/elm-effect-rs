//! The `toFormatter` rows whose schema adapters live in this crate.
//!
//! The port's formatter takes the schema's wire value (`Unknown`), so each test
//! builds the wire value through the same schema's `try_encode` and then
//! formats it. That keeps the claim end to end: what the schema writes out is
//! what the formatter renders, and the rendering is the pin's.
//!
//! The schema crate cannot host these: `DateTime` and `RegExp` are this crate's
//! types, and the dependency runs the other way.

use elm_effect::{DateTime, Kind, date, graph, regexp};
use elm_effect_schema::{Schema, Unknown};

fn wire<T: Send + Sync + 'static>(schema: &Schema<T>, value: &T) -> Unknown {
    schema.try_encode(value).expect("the value encodes")
}

/// toFormatter.test.ts:541 `Date`: the instant's ISO text, with no quotes.
#[test]
fn formatter_renders_a_date_as_its_iso_text() {
    let schema = date();
    let format = schema.formatter();
    assert_eq!(
        format(&wire(&schema, &DateTime::from_epoch_millis(0))),
        "1970-01-01T00:00:00.000Z"
    );
}

/// toFormatter.test.ts:551 `RegExp`: the pattern literal, not the wire object.
#[test]
fn formatter_renders_a_regexp_as_a_literal() {
    let schema = regexp();
    let format = schema.formatter();
    assert_eq!(
        format(&wire(&schema, &elm_effect::RegExp::new("a", ""))),
        "/a/"
    );
    assert_eq!(
        format(&wire(&schema, &elm_effect::RegExp::new("a", "i"))),
        "/a/i"
    );
}

/// `Schema.Graph`'s formatter is the kind walk, so a graph renders its shape
/// through the node and edge schemas it was declared with.
#[test]
fn formatter_renders_a_graph_through_its_declared_schemas() {
    let schema = graph(Kind::Directed, Schema::string(), Schema::number());
    let value = schema
        .decode(Unknown::from_json(elm_effect_schema::serde_json::json!({
            "type": "directed",
            "nodes": [{ "index": 0, "data": "A" }],
            "edges": []
        })))
        .expect("decodes");
    let rendered = schema.formatter()(&wire(&schema, &value));
    assert!(
        rendered.contains("\"A\""),
        "the node payload renders through its own schema: {rendered}"
    );
}
