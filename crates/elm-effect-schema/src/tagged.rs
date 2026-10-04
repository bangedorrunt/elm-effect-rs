//! `TaggedError`: a schema-bearing error with a `_tag` discriminant.
//!
//! A tagged error decodes from an object whose `_tag` field names it;
//! [`catch_tag`] recovers one tag from the typed error channel, leaving
//! other tags and successes untouched.

use std::collections::BTreeMap;

use super::{Schema, SchemaError, SchemaKind, Unknown};

/// An error value carrying its own discriminant.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedError {
    /// The `_tag` discriminant.
    pub tag: String,
    /// The remaining fields.
    pub fields: BTreeMap<String, Unknown>,
}

impl TaggedError {
    /// Build a fieldless tagged error (tests and fallbacks).
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            fields: BTreeMap::new(),
        }
    }
}

/// Anything carrying a `_tag` discriminant.
pub trait Tagged {
    /// The discriminant value.
    fn tag(&self) -> &str;
}

impl Tagged for TaggedError {
    fn tag(&self) -> &str {
        &self.tag
    }
}

/// Recover one tag from the typed error channel (`catch_tag` shape):
/// a matching `Err` runs the handler; other tags and `Ok` pass through.
/// Defects (panics) never enter the typed channel, so they never match.
pub fn catch_tag<T, E, F>(result: Result<T, E>, tag: &str, handler: F) -> Result<T, E>
where
    E: Tagged,
    F: FnOnce(E) -> Result<T, E>,
{
    match result {
        Err(error) if error.tag() == tag => handler(error),
        other => other,
    }
}

impl Schema<TaggedError> {
    /// Decode the object whose `_tag` field equals `tag`; anything else
    /// (missing tag, other tag, non-object) is a typed mismatch.
    pub fn tagged(tag: &'static str) -> Self {
        Schema::new(
            move |unknown| match unknown {
                Unknown::Object(mut fields) => match fields.remove("_tag") {
                    Some(Unknown::Text(found)) if found == tag => {
                        Ok(TaggedError { tag: found, fields })
                    }
                    Some(Unknown::Text(_)) => Err(SchemaError::TypeMismatch {
                        path: "$._tag".to_string(),
                        expected: tag,
                        got: "other tag",
                    }),
                    _ => Err(SchemaError::TypeMismatch {
                        path: "$._tag".to_string(),
                        expected: "a matching _tag",
                        got: "missing or mistyped _tag",
                    }),
                },
                other => Err(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "object",
                    got: other.kind(),
                }),
            },
            |error| {
                let mut fields = error.fields.clone();
                fields.insert("_tag".to_string(), Unknown::Text(error.tag.clone()));
                Unknown::Object(fields)
            },
            SchemaKind::Object,
        )
    }
}
