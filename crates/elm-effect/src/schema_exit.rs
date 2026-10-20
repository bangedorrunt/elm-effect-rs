//! `Schema.Exit`: the schema of an [`Exit`].
//!
//! The wire form is the pin's two tagged objects: a success carries its value,
//! a failure carries the cause's wire array. All three value schemas are the
//! caller's, so an `Exit` over `FiniteFromString` decodes text and encodes
//! numbers, and a defect travels whatever schema the caller gave it.

use std::collections::BTreeMap;

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::exit::Exit;
use crate::schema_cause::cause_schema;

/// `Schema.Exit(value, error, defect)` as a value: the three schemas it was
/// built from stay reachable, and an annotation over it keeps them
/// (`Schema.test.ts:3501`).
#[derive(Clone)]
pub struct ExitSchema<A, E, D> {
    value: Schema<A>,
    error: Schema<E>,
    defect: Schema<D>,
    schema: Schema<Exit<A, E, D>>,
}

impl<A, E, D> ExitSchema<A, E, D>
where
    A: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    /// Build the exit codec from the three value schemas.
    pub fn new(value: Schema<A>, error: Schema<E>, defect: Schema<D>) -> Self {
        let cause = cause_schema(error.clone(), defect.clone());
        let decode_value = value.clone();
        let decode_cause = cause.clone();
        let encode_value = value.clone();
        let schema = Schema::<Exit<A, E, D>>::try_convert(
            Schema::<Unknown>::any(),
            move |wire: Unknown| {
                let Unknown::Object(mut fields) = wire else {
                    return Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: "an Exit",
                        got: wire.kind(),
                    });
                };
                let tag = match fields.remove("_tag") {
                    Some(Unknown::Text(tag)) => tag,
                    _ => {
                        return Err(SchemaError::TypeMismatch {
                            path: "$._tag".to_string(),
                            expected: "an Exit tag",
                            got: "missing or mistyped",
                        });
                    }
                };
                match tag.as_str() {
                    "Success" => {
                        let value =
                            fields
                                .remove("value")
                                .ok_or_else(|| SchemaError::TypeMismatch {
                                    path: "$.value".to_string(),
                                    expected: "the success value",
                                    got: "missing",
                                })?;
                        Ok(Exit::Success(
                            decode_value
                                .decode(value)
                                .map_err(|error| error.at("$.value"))?,
                        ))
                    }
                    "Failure" => {
                        let cause_wire =
                            fields
                                .remove("cause")
                                .ok_or_else(|| SchemaError::TypeMismatch {
                                    path: "$.cause".to_string(),
                                    expected: "the failure's cause",
                                    got: "missing",
                                })?;
                        Ok(Exit::Failure(
                            decode_cause
                                .decode(cause_wire)
                                .map_err(|error| error.at("$.cause"))?,
                        ))
                    }
                    other => Err(SchemaError::TypeMismatchDynamic {
                        path: "$._tag".to_string(),
                        expected: format!("an Exit tag, got {other}"),
                        got: "other tag",
                    }),
                }
            },
            move |exit: &Exit<A, E, D>| match exit {
                Exit::Success(success) => Ok(Unknown::Object(BTreeMap::from([
                    ("_tag".to_string(), Unknown::Text("Success".to_string())),
                    ("value".to_string(), encode_value.try_encode(success)?),
                ]))),
                Exit::Failure(failure) => Ok(Unknown::Object(BTreeMap::from([
                    ("_tag".to_string(), Unknown::Text("Failure".to_string())),
                    ("cause".to_string(), cause.try_encode(failure)?),
                ]))),
            },
        );
        Self {
            value,
            error,
            defect,
            schema,
        }
    }

    /// The success value's schema (`schema.value`).
    pub fn value(&self) -> &Schema<A> {
        &self.value
    }

    /// The error side's schema (`schema.error`).
    pub fn error(&self) -> &Schema<E> {
        &self.error
    }

    /// The defect side's schema (`schema.defect`).
    pub fn defect(&self) -> &Schema<D> {
        &self.defect
    }

    /// The codec itself.
    pub fn schema(&self) -> &Schema<Exit<A, E, D>> {
        &self.schema
    }

    /// An annotation over the exit schema, which keeps the three accessors.
    pub fn annotate<S: Into<String>>(&self, pairs: Vec<(S, S)>) -> Self {
        Self {
            schema: Schema::annotated_with(self.schema.clone(), pairs),
            ..self.clone()
        }
    }
}

/// `Schema.Exit(success, error, defect)`: decode the two wire shapes into an
/// exit, and encode an exit back through the same three schemas.
pub fn exit_schema<A, E, D>(
    success: Schema<A>,
    error: Schema<E>,
    defect: Schema<D>,
) -> Schema<Exit<A, E, D>>
where
    A: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    ExitSchema::new(success, error, defect).schema().clone()
}
