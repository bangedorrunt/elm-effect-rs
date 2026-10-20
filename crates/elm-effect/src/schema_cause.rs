//! `Schema.Cause` and `Schema.CauseReason`: the schemas of a [`Cause`] and of
//! one [`Reason`].
//!
//! The wire forms are the pin's: one reason is a tagged object, and a cause is
//! the array of them. Both value schemas are the caller's, so a cause over
//! `FiniteFromString` decodes text and encodes numbers, and the defect side is a
//! value the defect schema owns rather than a message the runtime rendered
//! earlier. That is why [`Cause`] is generic over its defect: the schema needs
//! the value, not a rendering of it.
//!
//! This lives beside the type it speaks for because the schema crate cannot
//! name `Cause` (the dependency runs the other way).

use std::collections::BTreeMap;

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::cause::{Cause, FiberId, Reason};

/// `Schema.CauseReason(error, defect)` as a value: the two schemas it was built
/// from stay reachable, and an annotation over it keeps them.
///
/// The pin's rows read `schema.error` and `schema.defect` and assert the same
/// answers after `annotate({})`, so the pair rides the value rather than being
/// thrown away once the codec is built.
#[derive(Clone)]
pub struct CauseReasonSchema<E, D> {
    error: Schema<E>,
    defect: Schema<D>,
    schema: Schema<Reason<E, D>>,
}

impl<E, D> CauseReasonSchema<E, D>
where
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    /// Build the reason codec from the two value schemas.
    pub fn new(error: Schema<E>, defect: Schema<D>) -> Self {
        // The reason object is checked here rather than by a union of three
        // member schemas: the tag decides which field schema runs, and a member
        // union would answer "no member matched" instead of naming the one
        // field that failed, which is the path the pin's issues carry.
        let wire = Schema::<Unknown>::any();
        let decode_error = error.clone();
        let decode_defect = defect.clone();
        let encode_error = error.clone();
        let encode_defect = defect.clone();
        let schema = Schema::<Reason<E, D>>::try_convert(
            wire,
            move |item: Unknown| {
                let Unknown::Object(mut fields) = item else {
                    return Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: "a reason object",
                        got: item.kind(),
                    });
                };
                let tag = match fields.remove("_tag") {
                    Some(Unknown::Text(tag)) => tag,
                    _ => {
                        return Err(SchemaError::TypeMismatch {
                            path: "$._tag".to_string(),
                            expected: "a reason tag",
                            got: "missing or mistyped",
                        });
                    }
                };
                let mut field = |name: &str| {
                    fields.remove(name).ok_or_else(|| SchemaError::TypeMismatch {
                        path: format!("$.{name}"),
                        expected: "the reason's value",
                        got: "missing",
                    })
                };
                match tag.as_str() {
                    "Fail" => Ok(Reason::Fail(
                        decode_error
                            .decode(field("error")?)
                            .map_err(|error| error.at("$.error"))?,
                    )),
                    "Die" => Ok(Reason::Die(
                        decode_defect
                            .decode(field("defect")?)
                            .map_err(|error| error.at("$.defect"))?,
                    )),
                    "Interrupt" => {
                        let raw = field("fiberId")?;
                        let fiber_id = match raw {
                            Unknown::Null => None,
                            Unknown::Number(number) if number >= 0.0 && number.fract() == 0.0 => {
                                Some(FiberId(number as u64))
                            }
                            other => {
                                return Err(SchemaError::TypeMismatch {
                                    path: "$.fiberId".to_string(),
                                    expected: "a fiber id or null",
                                    got: other.kind(),
                                });
                            }
                        };
                        Ok(Reason::Interrupt { fiber_id })
                    }
                    other => Err(SchemaError::TypeMismatchDynamic {
                        path: "$._tag".to_string(),
                        expected: format!("a reason tag, got {other}"),
                        got: "other tag",
                    }),
                }
            },
            move |reason: &Reason<E, D>| {
                let mut fields = BTreeMap::new();
                match reason {
                    Reason::Fail(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Fail".to_string()));
                        fields.insert("error".to_string(), encode_error.try_encode(value)?);
                    }
                    Reason::Die(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Die".to_string()));
                        fields.insert("defect".to_string(), encode_defect.try_encode(value)?);
                    }
                    Reason::Interrupt { fiber_id } => {
                        fields.insert("_tag".to_string(), Unknown::Text("Interrupt".to_string()));
                        fields.insert(
                            "fiberId".to_string(),
                            match fiber_id {
                                Some(id) => Unknown::Number(id.0 as f64),
                                None => Unknown::Null,
                            },
                        );
                    }
                }
                Ok(Unknown::Object(fields))
            },
        );
        Self {
            error,
            defect,
            schema,
        }
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
    pub fn schema(&self) -> &Schema<Reason<E, D>> {
        &self.schema
    }

    /// An annotation over the reason schema, which keeps both accessors.
    pub fn annotate<S: Into<String>>(&self, pairs: Vec<(S, S)>) -> Self {
        Self {
            schema: Schema::annotated_with(self.schema.clone(), pairs),
            ..self.clone()
        }
    }
}

/// `Schema.Cause(error, defect)` as a value: the same two accessors, over the
/// cause codec the pin's rows read.
#[derive(Clone)]
pub struct CauseSchema<E, D> {
    error: Schema<E>,
    defect: Schema<D>,
    schema: Schema<Cause<E, D>>,
}

impl<E, D> CauseSchema<E, D>
where
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    /// Build the cause codec from the two value schemas.
    pub fn new(error: Schema<E>, defect: Schema<D>) -> Self {
        let reason = CauseReasonSchema::new(error.clone(), defect.clone());
        let reason_schema = reason.schema().clone();
        let decode_reason = reason_schema.clone();
        let encode_reason = reason_schema;
        let wire = Schema::<Vec<Unknown>>::array(Schema::<Unknown>::any());
        let schema = Schema::<Cause<E, D>>::try_convert(
            wire,
            move |items: Vec<Unknown>| {
                let mut reasons = Vec::with_capacity(items.len());
                for (index, item) in items.into_iter().enumerate() {
                    reasons.push(
                        decode_reason
                            .decode(item)
                            .map_err(|error| error.at(&format!("$[{index}]")))?,
                    );
                }
                Ok(Cause::from_reasons(reasons))
            },
            move |cause: &Cause<E, D>| {
                cause
                    .reasons
                    .iter()
                    .map(|reason| encode_reason.try_encode(reason))
                    .collect()
            },
        );
        Self {
            error,
            defect,
            schema,
        }
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
    pub fn schema(&self) -> &Schema<Cause<E, D>> {
        &self.schema
    }

    /// An annotation over the cause schema, which keeps both accessors.
    pub fn annotate<S: Into<String>>(&self, pairs: Vec<(S, S)>) -> Self {
        Self {
            schema: Schema::annotated_with(self.schema.clone(), pairs),
            ..self.clone()
        }
    }
}

/// `Schema.Cause(error, defect)`: decode the wire array into a cause, and encode
/// a cause back through the same two schemas.
pub fn cause_schema<E, D>(error: Schema<E>, defect: Schema<D>) -> Schema<Cause<E, D>>
where
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    CauseSchema::new(error, defect).schema().clone()
}

/// `Schema.CauseReason(error, defect)`: one reason's codec, for a caller that
/// holds a single reason rather than a whole cause.
pub fn cause_reason_schema<E, D>(error: Schema<E>, defect: Schema<D>) -> Schema<Reason<E, D>>
where
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    CauseReasonSchema::new(error, defect).schema().clone()
}
