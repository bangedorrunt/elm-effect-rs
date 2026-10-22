//! `ChannelSchema.ts` at `effect@4.0.0`: schema adapters at a channel boundary.
//!
//! A `Take::Items` is this crate's chunk, so encoding a channel means running
//! the pin's `Schema.NonEmptyArray(schema)` over each chunk: the wire form
//! stays a chunk, and an EMPTY chunk is refused the same way the pin refuses
//! it (its array schema needs one element). [`duplex`] is the bidirectional
//! case: encode the input side, run the user's channel on the encoded chunks,
//! then decode the output side.
//!
//! Two pin exports are deliberately absent. `encodeUnknown` and `decodeUnknown`
//! are `encode` and `decode` with a type parameter erased, and in this crate
//! the encoded form is always [`Unknown`], so the erased twin would be the same
//! function under a second name.

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::channel::Channel;
use crate::take::Take;

/// Encode typed chunks into the schema's wire form (`ChannelSchema.encode`).
///
/// The channel's error channel carries the schema's failure, so `E` has to be
/// able to absorb a [`SchemaError`].
pub fn encode<A, E>(schema: Schema<A>) -> Channel<A, E, Unknown>
where
    A: Send + Sync + 'static,
    E: From<SchemaError> + Send + 'static,
{
    // The pin encodes `Schema.NonEmptyArray(schema)`, not the bare schema: a
    // chunk is a non-empty array, and one schema failure refuses the chunk.
    let chunk_schema = Schema::non_empty_array(schema);
    Channel::from_fn(move |take| match take {
        Take::Items(items) => match chunk_schema.try_encode(&items) {
            Ok(Unknown::Array(encoded)) => vec![Take::Items(encoded)],
            // An array schema cannot encode to anything else; keeping the value
            // whole beats silently flattening it.
            Ok(other) => vec![Take::Items(vec![other])],
            Err(error) => vec![Take::Fail(E::from(error))],
        },
        Take::End => vec![Take::End],
        Take::Fail(error) => vec![Take::Fail(error)],
    })
}

/// Decode wire chunks into typed values (`ChannelSchema.decode`).
pub fn decode<A, E>(schema: Schema<A>) -> Channel<Unknown, E, A>
where
    A: Send + Sync + 'static,
    E: From<SchemaError> + Send + 'static,
{
    let chunk_schema = Schema::non_empty_array(schema);
    Channel::from_fn(move |take| match take {
        Take::Items(items) => match chunk_schema.decode(Unknown::Array(items)) {
            Ok(values) => vec![Take::Items(values)],
            Err(error) => vec![Take::Fail(E::from(error))],
        },
        Take::End => vec![Take::End],
        Take::Fail(error) => vec![Take::Fail(error)],
    })
}

/// Wrap a channel that speaks the wire form so callers speak typed values
/// (`ChannelSchema.duplex`): the input side is encoded, the output side
/// decoded, and the wrapped channel in between never sees a typed value.
pub fn duplex<In, Out, E>(
    channel: Channel<Unknown, E, Unknown>,
    input_schema: Schema<In>,
    output_schema: Schema<Out>,
) -> Channel<In, E, Out>
where
    In: Send + Sync + 'static,
    Out: Send + Sync + 'static,
    E: From<SchemaError> + Send + 'static,
{
    encode(input_schema)
        .pipe(channel)
        .pipe(decode(output_schema))
}
