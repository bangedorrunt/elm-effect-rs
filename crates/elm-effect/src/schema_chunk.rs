//! `Schema.Chunk`.
//!
//! The pin links a chunk to the array of its elements (`chunkLink`): decoding
//! collects the array and encoding hands the elements back as one. The
//! declaration id is what makes a persisted schema read back as the constructor
//! rather than as the array it encodes through.

use elm_effect_schema::Schema;

use crate::chunk::Chunk;

/// `Schema.Chunk(inner)`: a chunk whose elements conform to `inner`.
pub fn chunk<T>(inner: Schema<T>) -> Schema<Chunk<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let parameter = Schema::as_unknown(inner.clone());
    let code = format!("Schema.Chunk({})", parameter.to_code());
    let ty = format!("Chunk.Chunk<{}>", parameter.to_type());
    Schema::convert(
        Schema::array(inner),
        |values: Vec<T>| Ok(Chunk::from_iter(values)),
        |chunk: &Chunk<T>| chunk.as_slice().to_vec(),
    )
    .with_declaration(
        "effect/schema/Chunk",
        elm_effect_schema::serde_json::Value::Null,
        vec![parameter],
    )
    .with_code(&code, &ty)
}
