//! `Schema.RegExp`.
//!
//! The pin's declaration is an `instanceOf` whose JSON codec is a struct of
//! `source` and `flags`; the port's [`RegExp`] is already exactly that pair, so
//! the codec reads and writes those two keys.

use std::collections::BTreeMap;

use elm_effect_schema::{Field, Schema, SchemaError, Unknown};

use crate::regexp::RegExp;

/// `Schema.RegExp`: a regex pattern and its flags.
pub fn regexp() -> Schema<RegExp> {
    let wire = Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
        Field::required("source", Schema::as_unknown(Schema::string())),
        Field::required("flags", Schema::as_unknown(Schema::string())),
    ]);
    Schema::convert(
        wire,
        |fields: BTreeMap<String, Unknown>| match (fields.get("source"), fields.get("flags")) {
            (Some(Unknown::Text(source)), Some(Unknown::Text(flags))) => {
                Ok(RegExp::new(source.clone(), flags.clone()))
            }
            _ => Err(refused("a source and flags string")),
        },
        |regexp: &RegExp| {
            [
                ("source".to_string(), Unknown::Text(regexp.source.clone())),
                ("flags".to_string(), Unknown::Text(regexp.flags.clone())),
            ]
            .into_iter()
            .collect()
        },
    )
    .with_declaration(
        "effect/schema/RegExp",
        elm_effect_schema::serde_json::Value::Null,
        Vec::new(),
    )
    .with_code("Schema.RegExp", "globalThis.RegExp")
}

fn refused(expected: &'static str) -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected,
        got: "object",
    }
}
