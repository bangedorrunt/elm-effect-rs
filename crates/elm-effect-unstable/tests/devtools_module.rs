//! DevToolsClient row port: `devtools/DevToolsClient.test.ts` reads the
//! wire and asserts a short-lived span arrives once in each state.
//!
//! The Rust port has no socket (see `devtools.rs`), so the observable
//! result is the recorded event list plus the one-line wire codec the
//! reader would parse.

use elm_effect_unstable::devtools::{DevEvent, DevToolsServer, decode, encode};

#[test]
fn short_lived_span_arrives_once_in_each_state() {
    // DevToolsClient.test.ts:7 — a span that opens and closes in the same
    // scope sends exactly one Started and one Ended, in that order.
    let server = DevToolsServer::new();
    let client = server.client();
    let span = client.span_start(None, "child");
    client.span_end(span);

    let events = server.drain();
    assert_eq!(events.len(), 2, "one event per state: {events:?}");
    assert!(
        matches!(&events[0], DevEvent::SpanStart { name, .. } if name == "child"),
        "first event opens the span: {events:?}"
    );
    assert!(
        matches!(&events[1], DevEvent::SpanEnd { id } if *id == span),
        "second event closes the same span: {events:?}"
    );
    // Each state survives the wire codec the reader parses.
    for event in &events {
        assert_eq!(decode(&encode(event)), Some(event.clone()), "{event:?}");
    }
}
