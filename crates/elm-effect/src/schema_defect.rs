//! `Schema.Defect`: the schema of a [`Defect`].
//!
//! The pin's defect wire is an Error-like object, `{ name, message }`, and
//! decoding it produces the runtime's defect. This port's [`Defect`] keeps a
//! message and a captured backtrace, so the name is not part of the value:
//! encoding writes the base `Error` name back rather than the one that was
//! read, which is the one place this schema is lossy.

use std::collections::BTreeMap;

use elm_effect_schema::{Field, Schema, SchemaError, Unknown};

use crate::Defect;

/// `Schema.Defect()`: decode `{ name, message }` into a [`Defect`], and encode
/// a defect as the same object.
pub fn defect_schema() -> Schema<Defect> {
    Schema::convert(
        Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
            Field::required("name", Schema::as_unknown(Schema::string())),
            Field::required("message", Schema::as_unknown(Schema::string())),
        ]),
        |fields: BTreeMap<String, Unknown>| {
            let message = match fields.get("message") {
                Some(Unknown::Text(message)) => message.clone(),
                _ => {
                    return Err(SchemaError::TypeMismatch {
                        path: "$.message".to_string(),
                        expected: "the defect's message",
                        got: "missing",
                    });
                }
            };
            Ok(Defect {
                message,
                backtrace: String::new(),
            })
        },
        |defect: &Defect| {
            BTreeMap::from([
                ("name".to_string(), Unknown::Text("Error".to_string())),
                ("message".to_string(), Unknown::Text(defect.message.clone())),
            ])
        },
    )
}
