//! Declaration revivers for the schemas this crate adapts.
//!
//! `SchemaRepresentation` revives a persisted declaration by id: the document
//! names `effect/schema/Cause` and a reviver rebuilds the constructor from its
//! type parameters. Those constructors live beside this crate's own types, so
//! their revivers do too; [`declaration_revivers`] is composed after the schema
//! crate's builtins by whoever revives a document that may hold one.
//!
//! Only the adapters whose types this crate exports appear here. `Schema.Date`
//! and `Schema.Graph` speak for the parked modules `datetime.rs` and
//! `graph.rs`, which the crate root does not export, so their revivers have no
//! shipped constructor to rebuild; `Schema.Chunk` and `Schema.RegExp` do, and
//! their revivers are below.
//!
//! Each reviver answers `Schema<Unknown>`, because a revived document speaks
//! the wire language: the adapter is re-typed through [`Schema::as_unknown`],
//! which keeps the declaration identity the round trip compares.

use elm_effect_schema::representation_doc::{DeclarationInput, DeclarationReviver, ReprError};
use elm_effect_schema::{Schema, Unknown};

use crate::schema_cause::{cause_reason_schema, cause_schema};
use crate::schema_chunk::chunk;
use crate::schema_data::byte_size;
use crate::schema_exit::exit_schema;
use crate::schema_regexp::regexp;

/// The revivers for every declaration this crate's adapters write.
///
/// Compose with the schema crate's own: a document holding a `Schema.Cause`
/// needs this list, a document holding a `Schema.Option` needs
/// `elm_effect_schema::builtin_declaration_revivers()`.
pub fn declaration_revivers() -> Vec<DeclarationReviver> {
    vec![
        DeclarationReviver::new("effect/schema/ByteSize", |input| {
            parameters("effect/schema/ByteSize", input, 0)?;
            Ok(Schema::as_unknown(byte_size()))
        }),
        DeclarationReviver::new("effect/schema/Chunk", |input| {
            let mut parameters = parameters("effect/schema/Chunk", input, 1)?;
            Ok(Schema::as_unknown(chunk(parameters.remove(0))))
        }),
        DeclarationReviver::new("effect/schema/RegExp", |input| {
            parameters("effect/schema/RegExp", input, 0)?;
            Ok(Schema::as_unknown(regexp()))
        }),
        DeclarationReviver::new("effect/schema/CauseReason", |input| {
            let mut parameters = parameters("effect/schema/CauseReason", input, 2)?;
            let defect = parameters.remove(1);
            let error = parameters.remove(0);
            Ok(Schema::as_unknown(cause_reason_schema(error, defect)))
        }),
        DeclarationReviver::new("effect/schema/Cause", |input| {
            let mut parameters = parameters("effect/schema/Cause", input, 2)?;
            let defect = parameters.remove(1);
            let error = parameters.remove(0);
            Ok(Schema::as_unknown(cause_schema(error, defect)))
        }),
        DeclarationReviver::new("effect/schema/Exit", |input| {
            let mut parameters = parameters("effect/schema/Exit", input, 3)?;
            let defect = parameters.remove(2);
            let error = parameters.remove(1);
            let value = parameters.remove(0);
            Ok(Schema::as_unknown(exit_schema(value, error, defect)))
        }),
    ]
}

/// The pin's arity check: a document whose type-parameter count does not match
/// the constructor is refused here, before a shifted list builds a wrong value.
fn parameters(
    id: &str,
    input: DeclarationInput,
    expected: usize,
) -> Result<Vec<Schema<Unknown>>, ReprError> {
    if input.parameters.len() != expected {
        return Err(ReprError {
            message: format!(
                "`{id}` takes {expected} type parameter(s), got {}",
                input.parameters.len()
            ),
            path: "[\"typeParameters\"]".to_string(),
        });
    }
    Ok(input.parameters)
}
