//! `Schema.Exit`: the schema of an [`Exit`].
//!
//! The wire form is the pin's two tagged objects: a success carries its value,
//! a failure carries the cause's wire array. Both value schemas are the
//! caller's, so an `Exit` over `FiniteFromString` decodes text and encodes
//! numbers, and a defect travels whatever schema the caller gave it.

use std::collections::BTreeMap;

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::exit::Exit;
use crate::schema_cause::cause_schema;

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
    let cause = cause_schema(error, defect);
    let decode_success = success.clone();
    let decode_cause = cause.clone();
    Schema::try_convert(
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
                        decode_success
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
            Exit::Success(value) => Ok(Unknown::Object(BTreeMap::from([
                ("_tag".to_string(), Unknown::Text("Success".to_string())),
                ("value".to_string(), success.try_encode(value)?),
            ]))),
            Exit::Failure(failure) => Ok(Unknown::Object(BTreeMap::from([
                ("_tag".to_string(), Unknown::Text("Failure".to_string())),
                ("cause".to_string(), cause.try_encode(failure)?),
            ]))),
        },
    )
}
