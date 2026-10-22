//! `Schema.File`: the schema of a [`File`].
//!
//! The pin's JSON codec writes `{ data, type, name, lastModified }`, where
//! `data` is the bytes Base64-encoded and `lastModified` is an integer of
//! milliseconds. Decoding refuses a `data` that is not standard Base64 with the
//! same message the pin's `isBase64()` check gives.
//!
//! The value side is this crate's [`File`], so the declaration id is what makes
//! a persisted schema read back as the constructor instead of as the object it
//! encodes through.

use std::collections::BTreeMap;

use elm_effect_schema::transform::base64_bytes_decode;
use elm_effect_schema::{Field, Schema, SchemaError, Unknown};

use crate::file::File;

/// `Schema.File`: a named blob, as the object its JSON codec writes.
pub fn file() -> Schema<File> {
    let wire = Schema::<BTreeMap<String, Unknown>>::struct_shape(vec![
        Field::required("data", Schema::as_unknown(Schema::string())),
        Field::required("type", Schema::as_unknown(Schema::string())),
        Field::required("name", Schema::as_unknown(Schema::string())),
        Field::required(
            "lastModified",
            Schema::as_unknown(Schema::<f64>::int()),
        ),
    ]);
    Schema::convert(
        wire,
        |mut fields: BTreeMap<String, Unknown>| {
            let data = match fields.remove("data") {
                Some(Unknown::Text(text)) => base64_bytes_decode(&text).ok_or_else(|| {
                    SchemaError::TypeMismatch {
                        path: "$.data".to_string(),
                        expected: "a valid Base64 string",
                        got: "malformed",
                    }
                })?,
                other => return Err(label("data", "a Base64 string", other.map(|v| v.kind()))),
            };
            let media_type = match fields.remove("type") {
                Some(Unknown::Text(text)) => text,
                other => return Err(label("type", "a media type", other.map(|v| v.kind()))),
            };
            let name = match fields.remove("name") {
                Some(Unknown::Text(text)) => text,
                other => return Err(label("name", "a name", other.map(|v| v.kind()))),
            };
            let last_modified = match fields.remove("lastModified") {
                Some(Unknown::Number(number)) => number as i64,
                other => return Err(label("lastModified", "a whole number", other.map(|v| v.kind()))),
            };
            Ok(File::new(data, name, media_type, last_modified))
        },
        |file: &File| {
            [
                (
                    "data".to_string(),
                    Unknown::Text(elm_effect_schema::transform::base64_bytes_encode(file.data())),
                ),
                (
                    "type".to_string(),
                    Unknown::Text(file.media_type().to_string()),
                ),
                ("name".to_string(), Unknown::Text(file.name().to_string())),
                (
                    "lastModified".to_string(),
                    Unknown::Number(file.last_modified() as f64),
                ),
            ]
            .into_iter()
            .collect()
        },
    )
    .with_declaration(
        "effect/schema/File",
        elm_effect_schema::serde_json::Value::Null,
        Vec::new(),
    )
    .with_code("Schema.File", "File")
}

/// A field-level refusal, which names the key rather than the whole document.
fn label(field: &str, expected: &'static str, got: Option<&'static str>) -> SchemaError {
    SchemaError::TypeMismatch {
        path: format!("$.{field}"),
        expected,
        got: got.unwrap_or("missing"),
    }
}
