//! `ChannelSchema.ts` at `effect@4.0.0`: schema adapters at a channel boundary.
//!
//! The point of the module is what the INNER channel sees: typed values go in,
//! the encoded (wire) form crosses, typed values come back out. Each test here
//! therefore drives a real [`Pull`] and inspects the takes, rather than calling
//! the schema directly (which the schema crate's own fences already cover).

use std::sync::{Arc, Mutex};

use elm_effect::channel::Channel;
use elm_effect::channel_schema::{decode, duplex, encode};
use elm_effect::pull::Pull;
use elm_effect::take::Take;
use elm_effect::{Schema, SchemaError, Unknown};

/// Drain a pull to its `End`, collecting every take it produced.
fn take_all<A: Send + 'static, E: Send + 'static>(mut pull: Pull<A, E>) -> Vec<Take<A, E>> {
    let mut out = Vec::new();
    loop {
        match pull.pull() {
            Take::End => {
                out.push(Take::End);
                return out;
            }
            other => out.push(other),
        }
    }
}

/// The pull that fails is terminal: collect up to and including the first
/// `Fail`, so an assertion about the failure does not depend on what the
/// source would have produced next.
fn take_until_fail<A: Send + 'static, E: Send + 'static>(mut pull: Pull<A, E>) -> Vec<Take<A, E>> {
    let mut out = Vec::new();
    loop {
        match pull.pull() {
            Take::End => {
                out.push(Take::End);
                return out;
            }
            Take::Fail(e) => {
                out.push(Take::Fail(e));
                return out;
            }
            other => out.push(other),
        }
    }
}

fn wire(text: &str) -> Unknown {
    Unknown::Text(text.to_string())
}

#[test]
fn encode_emits_the_wire_spelling_and_passes_end_through() {
    // `NumberFromString`: the type is `f64`, the encoded form is the JS
    // `Number()` spelling, so the difference between in and out is visible.
    let takes = take_all(
        encode::<f64, SchemaError>(Schema::number_from_string())
            .run(Pull::of(vec![1.5, 2.0, -0.25])),
    );

    assert_eq!(
        takes,
        vec![
            Take::Items(vec![wire("1.5"), wire("2"), wire("-0.25")]),
            Take::End,
        ],
        "every item in the chunk must be encoded, and End must still end the stream"
    );
}

#[test]
fn an_empty_chunk_is_refused_because_a_chunk_is_non_empty() {
    // The pin encodes `Schema.NonEmptyArray(schema)`, so an empty chunk is a
    // schema failure, not a no-op. This crate's `filter_map` can produce one
    // (`Items(vec![])`), which is exactly why the rule needs a witness.
    let mut sent = false;
    let source = Pull::from_fn(move || {
        if sent {
            Take::End
        } else {
            sent = true;
            Take::Items(Vec::<f64>::new())
        }
    });

    let takes =
        take_until_fail(encode::<f64, SchemaError>(Schema::number_from_string()).run(source));

    assert!(
        matches!(takes.last(), Some(Take::Fail(SchemaError::TypeMismatch { path, .. })) if path == "$[0]"),
        "an empty chunk must fail as a missing element, got: {takes:?}"
    );
}

#[test]
fn decode_restores_the_typed_value() {
    let takes = take_all(
        decode::<f64, SchemaError>(Schema::number_from_string())
            .run(Pull::of(vec![wire("2.5"), wire("0")])),
    );

    assert_eq!(takes, vec![Take::Items(vec![2.5, 0.0]), Take::End]);
}

#[test]
fn decode_fails_the_pull_when_the_wire_value_does_not_conform() {
    // A boolean cannot be read as a number string. The pin surfaces this as
    // `Schema.SchemaError` on the channel's error channel, not as a dropped
    // item, so the failure must reach the pull as `Take::Fail`.
    let takes = take_until_fail(
        decode::<f64, SchemaError>(Schema::number_from_string())
            .run(Pull::of(vec![Unknown::Bool(true)])),
    );

    assert_eq!(takes.len(), 1, "one take, the failure: {takes:?}");
    assert!(
        matches!(takes[0], Take::Fail(SchemaError::TypeMismatch { .. })),
        "expected a schema type mismatch, got: {:?}",
        takes[0]
    );
}

#[test]
fn decode_names_the_bad_element_mid_chunk_and_emits_no_partial_chunk() {
    // The middle element is the wrong shape, so the pull must fail naming
    // `$[1]`: any other path would point at the wrong item, and an `Items`
    // take before the `Fail` would leak a half-decoded chunk.
    let takes = take_until_fail(
        decode::<f64, SchemaError>(Schema::number_from_string())
            .run(Pull::of(vec![wire("1"), Unknown::Bool(true), wire("3")])),
    );

    assert_eq!(
        takes.len(),
        1,
        "one take, the failure: nothing of the chunk may escape: {takes:?}"
    );
    assert!(
        matches!(
            &takes[0],
            Take::Fail(SchemaError::TypeMismatch { path, .. }) if path == "$[1]"
        ),
        "expected the failure to name the middle element, got: {:?}",
        takes[0]
    );
}

#[test]
fn encode_refuses_a_chunk_whose_element_fails_its_refinement() {
    // Encoding runs the refinement too, so a chunk holding one negative value
    // has no wire form at all. The failure names `$[1]`, and the earlier
    // encodable elements must not be handed downstream on their own.
    let positive = Schema::number().refine("a positive number", |value| *value > 0.0);
    let takes = take_until_fail(
        encode::<f64, SchemaError>(positive).run(Pull::of(vec![1.0, -2.0, 3.0])),
    );

    assert_eq!(
        takes.len(),
        1,
        "one take, the failure: the encodable head must not be emitted alone: {takes:?}"
    );
    assert!(
        matches!(
            &takes[0],
            Take::Fail(SchemaError::TypeMismatch { path, .. }) if path == "$[1]"
        ),
        "expected the refusal to name the negative element, got: {:?}",
        takes[0]
    );
}

#[test]
fn duplex_hands_the_inner_channel_the_encoded_chunk_and_decodes_its_output() {
    // The inner channel stands in for the encoded side of a real boundary. It
    // records what it was handed, which is the whole assertion: it must see the
    // wire spelling, never the `f64`.
    let seen: Arc<Mutex<Vec<Take<Unknown, SchemaError>>>> = Arc::new(Mutex::new(Vec::new()));
    let inner_seen = Arc::clone(&seen);
    let inner: Channel<Unknown, SchemaError, Unknown> = Channel::from_fn(move |take| {
        inner_seen.lock().expect("recorder").push(take.clone());
        vec![take]
    });

    // The two schemas differ on purpose: input `f64` -> wire text, output wire
    // text -> `String`. A swapped or skipped side cannot pass both assertions.
    let takes = take_all(
        duplex(inner, Schema::number_from_string(), Schema::string()).run(Pull::of(vec![3.5])),
    );

    assert_eq!(
        seen.lock().expect("recorder").as_slice(),
        &[Take::Items(vec![wire("3.5")]), Take::End],
        "the inner channel must see the encoded chunk"
    );
    assert_eq!(
        takes,
        vec![Take::Items(vec!["3.5".to_string()]), Take::End],
        "the output side must be decoded with the output schema"
    );
}

#[test]
fn duplex_refuses_an_empty_chunk_from_the_inner_channel() {
    // A chunk is the pin's NonEmptyArray: zero elements has no wire form on
    // either side. An inner channel that answers with an empty chunk must
    // fail the pull at the decode edge, naming the missing first element,
    // rather than ending the stream early or handing an empty chunk upstream.
    let inner: Channel<Unknown, SchemaError, Unknown> =
        Channel::from_fn(|_take| vec![Take::Items(Vec::new())]);

    let takes = take_until_fail(
        duplex(
            inner,
            Schema::number_from_string(),
            Schema::number_from_string(),
        )
        .run(Pull::of(vec![1.0])),
    );

    assert!(
        matches!(
            takes.last(),
            Some(Take::Fail(SchemaError::TypeMismatch { path, got, .. }))
                if path == "$[0]" && *got == "missing"
        ),
        "expected the emptiness rule (missing `$[0]`), got: {takes:?}"
    );
}

#[test]
fn duplex_fails_when_the_inner_channel_emits_a_non_conforming_chunk() {
    // Encoded in, then the inner channel answers something the output schema
    // cannot read. That must fail the pull, not pass the wire value upstream.
    let inner: Channel<Unknown, SchemaError, Unknown> =
        Channel::from_fn(|_take| vec![Take::Items(vec![wire("not a number")])]);

    let takes = take_until_fail(
        duplex(
            inner,
            Schema::number_from_string(),
            Schema::number_from_string(),
        )
        .run(Pull::of(vec![1.0])),
    );

    assert!(
        matches!(
            takes.last(),
            Some(Take::Fail(SchemaError::TypeMismatch { .. }))
        ),
        "expected the output schema to refuse the chunk, got: {takes:?}"
    );
}
