//! Declaration revivers for the schemas this crate adapts.
//!
//! `SchemaRepresentation` revives a persisted declaration by id: the document
//! names `effect/schema/Cause` and a reviver rebuilds the constructor from its
//! type parameters. Those constructors live beside this crate's own types, so
//! their revivers do too; [`declaration_revivers`] is composed after the schema
//! crate's builtins by whoever revives a document that may hold one.
//!
//! Every declaration this crate writes is below, and each one rebuilds the
//! constructor beside the value type it speaks for.
//!
//! Each reviver answers `Schema<Unknown>`, because a revived document speaks
//! the wire language: the adapter is re-typed through [`Schema::as_unknown`],
//! which keeps the declaration identity the round trip compares.

use elm_effect_schema::representation_doc::{DeclarationInput, DeclarationReviver, ReprError};
use elm_effect_schema::{Schema, Unknown};

use crate::graph::Kind;
use crate::schema_cause::{cause_reason_schema, cause_schema};
use crate::schema_chunk::chunk;
use crate::schema_data::byte_size;
use crate::schema_datetime::date;
use crate::schema_exit::exit_schema;
use crate::schema_file::file;
use crate::schema_graph::graph;
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
        DeclarationReviver::new("effect/schema/Date", |input| {
            parameters("effect/schema/Date", input, 0)?;
            Ok(Schema::as_unknown(date()))
        }),
        DeclarationReviver::new("effect/schema/Chunk", |input| {
            let mut parameters = parameters("effect/schema/Chunk", input, 1)?;
            Ok(Schema::as_unknown(chunk(parameters.remove(0))))
        }),
        DeclarationReviver::new("effect/schema/RegExp", |input| {
            parameters("effect/schema/RegExp", input, 0)?;
            Ok(Schema::as_unknown(regexp()))
        }),
        DeclarationReviver::new("effect/schema/Graph", |input| {
            // The kind rides in the payload as text, not in the parameter list,
            // so a document whose payload names neither kind is refused here.
            let kind = match &input.payload {
                elm_effect_schema::serde_json::Value::String(name) => Kind::parse(name)
                    .map_err(|error| ReprError {
                        message: error.0,
                        path: "[\"representation\"][\"payload\"]".to_string(),
                    })?,
                _ => {
                    return Err(ReprError {
                        message: "`effect/schema/Graph` takes a directed/undirected payload"
                            .to_string(),
                        path: "[\"representation\"][\"payload\"]".to_string(),
                    });
                }
            };
            let mut parameters = parameters("effect/schema/Graph", input, 2)?;
            let edge = parameters.remove(1);
            let node = parameters.remove(0);
            Ok(Schema::as_unknown(graph(kind, node, edge)))
        }),
        DeclarationReviver::new("effect/schema/File", |input| {
            parameters("effect/schema/File", input, 0)?;
            Ok(Schema::as_unknown(file()))
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
