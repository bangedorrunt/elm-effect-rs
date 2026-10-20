//! `Schema.Cause`: the schema of a [`Cause`].
//!
//! The wire form is the pin's array of tagged reasons. Both value schemas are
//! the caller's, so a cause over `FiniteFromString` decodes text and encodes
//! numbers, and the defect side is a value the defect schema owns rather than
//! a message the runtime rendered earlier. That is why [`Cause`] is generic
//! over its defect: the schema needs the value, not a rendering of it.
//!
//! This lives beside the type it speaks for because the schema crate cannot
//! name `Cause` (the dependency runs the other way).

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::cause::{Cause, FiberId, Reason};

/// `Schema.Cause(error, defect)`: decode the wire array into a cause, and
/// encode a cause back through the same two schemas.
pub fn cause_schema<E, D>(error: Schema<E>, defect: Schema<D>) -> Schema<Cause<E, D>>
where
    E: Clone + Send + Sync + 'static,
    D: Clone + Send + Sync + 'static,
{
    // The reason objects are checked here rather than by a union of three
    // member schemas: the tag decides which field schema runs, and a member
    // union would answer "no member matched" instead of naming the one field
    // that failed, which is the path the pin's issues carry.
    let wire = Schema::<Vec<Unknown>>::array(Schema::<Unknown>::any());
    let decode_error = error.clone();
    let decode_defect = defect.clone();
    Schema::try_convert(
        wire,
        move |items: Vec<Unknown>| {
            let mut reasons = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                let path = format!("$[{index}]");
                let Unknown::Object(mut fields) = item else {
                    return Err(SchemaError::TypeMismatch {
                        path,
                        expected: "a reason object",
                        got: item.kind(),
                    });
                };
                let tag = match fields.remove("_tag") {
                    Some(Unknown::Text(tag)) => tag,
                    _ => {
                        return Err(SchemaError::TypeMismatch {
                            path: format!("{path}._tag"),
                            expected: "a reason tag",
                            got: "missing or mistyped",
                        });
                    }
                };
                let mut field = |name: &str| {
                    fields
                        .remove(name)
                        .ok_or_else(|| SchemaError::TypeMismatch {
                            path: format!("{path}.{name}"),
                            expected: "the reason's value",
                            got: "missing",
                        })
                };
                let reason = match tag.as_str() {
                    "Fail" => Reason::Fail(
                        decode_error
                            .decode(field("error")?)
                            .map_err(|error| error.at(&format!("{path}.error")))?,
                    ),
                    "Die" => Reason::Die(
                        decode_defect
                            .decode(field("defect")?)
                            .map_err(|error| error.at(&format!("{path}.defect")))?,
                    ),
                    "Interrupt" => {
                        let raw = field("fiberId")?;
                        let fiber_id = match raw {
                            Unknown::Null => None,
                            Unknown::Number(number) if number >= 0.0 && number.fract() == 0.0 => {
                                Some(FiberId(number as u64))
                            }
                            other => {
                                return Err(SchemaError::TypeMismatch {
                                    path: format!("{path}.fiberId"),
                                    expected: "a fiber id or null",
                                    got: other.kind(),
                                });
                            }
                        };
                        Reason::Interrupt { fiber_id }
                    }
                    other => {
                        return Err(SchemaError::TypeMismatchDynamic {
                            path: format!("{path}._tag"),
                            expected: format!("a reason tag, got {other}"),
                            got: "other tag",
                        });
                    }
                };
                reasons.push(reason);
            }
            Ok(Cause::from_reasons(reasons))
        },
        move |cause: &Cause<E, D>| {
            let mut out = Vec::with_capacity(cause.reasons.len());
            for reason in &cause.reasons {
                let mut fields = std::collections::BTreeMap::new();
                match reason {
                    Reason::Fail(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Fail".to_string()));
                        fields.insert("error".to_string(), error.try_encode(value)?);
                    }
                    Reason::Die(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Die".to_string()));
                        fields.insert("defect".to_string(), defect.try_encode(value)?);
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
                out.push(Unknown::Object(fields));
            }
            Ok(out)
        },
    )
}
