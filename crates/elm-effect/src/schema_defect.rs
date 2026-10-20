//! `Schema.Defect`: the schema of a [`Defect`].
//!
//! The pin's defect wire is an Error-like object, and only `message` is
//! required: `{ message }` alone decodes, `name` is advisory, and `stack`
//! becomes the runtime defect's backtrace. This port's [`Defect`] keeps a
//! message and a backtrace, so the name is not part of the value and encoding
//! writes the base `Error` name back rather than the one that was read. The
//! pin's other half, decoding an arbitrary non-object value as the defect
//! itself, has no carrier: a [`Defect`] here is always this struct.

use std::collections::BTreeMap;

use elm_effect_schema::{Field, Schema, SchemaError, Unknown};

use crate::Defect;

/// `Schema.Defect()`: decode `{ name, message }` into a [`Defect`], and encode
/// a defect as the same object.
pub fn defect_schema() -> Schema<Defect> {
    Schema::convert(
        Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
            Field::optional_key("name", Schema::as_unknown(Schema::string())),
            Field::required("message", Schema::as_unknown(Schema::string())),
            // The pin maps `stack` onto the error it builds; here that is the
            // captured backtrace. Encoding leaves it out again, which is the
            // pin's `includeStack: false` default, so a decoded backtrace does
            // not survive an encode.
            Field::optional_key("stack", Schema::as_unknown(Schema::string())),
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
            let backtrace = match fields.get("stack") {
                Some(Unknown::Text(stack)) => stack.clone(),
                _ => String::new(),
            };
            Ok(Defect { message, backtrace })
        },
        |defect: &Defect| {
            BTreeMap::from([
                ("name".to_string(), Unknown::Text("Error".to_string())),
                ("message".to_string(), Unknown::Text(defect.message.clone())),
            ])
        },
    )
}
