//! The `Class` family: `Schema.Class`, `Schema.TaggedClass`, `Schema.Error`
//! and `Schema.TaggedError` — a named, constructible schema over a field list.
//!
//! A JS class instance carries methods and a prototype, and a Rust value has
//! neither, so what an instance IS here is a [`ClassValue`]: the identifier it
//! was built under, the `_tag` a tagged class carries, and its decoded fields.
//! Everything the pin's class decides at runtime is kept: which identifier a
//! subclass reports, the `_tag` its wire carries, constructor defaults, the
//! excess-property policy, and `make` answering an existing instance unchanged.
//!
//! Deliberately not instance claims here: `immer` drafts, static members, a
//! memoized `ast`. Those are the JS object model; [`ClassValue::is_instance_of`]
//! is the identity question a decoded value can answer, and [`instance_of`] is
//! the schema the pin builds from an `instanceof` check.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    AdapterError, CONSTRUCTOR_DIE, ConstructError, Field, Schema, SchemaError, SchemaKind, Unknown,
};

/// How a constructor treats a key its field list does not declare.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExcessProperty {
    /// Keep the declared fields, drop the rest: the pin's default, and also
    /// what an explicit `"ignore"` asks for, because both leave the key off the
    /// instance.
    #[default]
    Ignore,
    /// The pin's explicit `"strip"`: the key is removed from the source object
    /// rather than merely left out. The instance is the same either way, so the
    /// two policies share one arm.
    Strip,
    /// `"error"`: an undeclared key fails the construction.
    Reject,
}

/// An instance of a class schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassValue {
    identifier: String,
    tag: Option<String>,
    fields: Vec<(String, Unknown)>,
}

impl ClassValue {
    /// The class this value was constructed under.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// The `_tag` a tagged class carries, `None` for a plain `Class`.
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// A field's decoded value.
    pub fn get(&self, name: &str) -> Option<&Unknown> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The declared field names, in declaration order.
    pub fn field_names(&self) -> Vec<&str> {
        self.fields.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// Whether this value was built by the class with this identifier: the
    /// question the pin asks with `instanceof`. A child instance does not
    /// answer for its parent, which is the one place the two differ.
    pub fn is_instance_of(&self, identifier: &str) -> bool {
        self.identifier == identifier
    }

    /// The wire object: the fields, with `_tag` first for a tagged class.
    pub fn to_unknown(&self) -> Unknown {
        let mut map = BTreeMap::new();
        if let Some(tag) = &self.tag {
            map.insert("_tag".to_string(), Unknown::Text(tag.clone()));
        }
        for (name, value) in &self.fields {
            map.insert(name.clone(), value.clone());
        }
        Unknown::Object(map)
    }

    /// The fields alone, without the discriminant: what a class's source struct
    /// decodes and encodes.
    fn fields_unknown(&self) -> Unknown {
        Unknown::Object(self.fields.iter().cloned().collect())
    }
}

impl std::fmt::Display for ClassValue {
    /// The pin's `Error` output format: `name: message` when the class has a
    /// `message` field, the identifier alone otherwise. A tagged class's tag is
    /// not part of it, because the pin prints the class name.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.get("message") {
            Some(Unknown::Text(message)) => write!(formatter, "{}: {message}", self.identifier),
            _ => write!(formatter, "{}", self.identifier),
        }
    }
}

/// The class a schema describes: what makes it a class rather than a struct.
#[derive(Clone)]
pub struct ClassMeta {
    identifier: String,
    tag: Option<String>,
    excess: ExcessProperty,
    /// The declared fields with their schemas, which is what a repr lists and
    /// what `mapFields` rebuilds from.
    fields: Vec<(String, Schema<Unknown>)>,
    /// The source schema the class was built around. Kept because a check lives
    /// on the source, not on the field list: `extend` composes over this so a
    /// parent's check survives into the child (`Schema.test.ts:7280`), which a
    /// rebuild from names could not do.
    source: Schema<Unknown>,
}

impl ClassMeta {
    /// The discriminant a tagged class demands. The union dispatcher and the
    /// tagged-union index read it from here, because a class strips `_tag`
    /// before its source struct sees it and so has no field carrying it.
    pub(crate) fn discriminant(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// The names the pin's `Object.keys(E.fields)` lists: `_tag` first for a
    /// tagged class, because that is the discriminant's place in the field map.
    fn declared(&self) -> Vec<String> {
        let names = self.fields.iter().map(|(name, _)| name.clone());
        match self.tag {
            Some(_) => {
                let mut all = vec!["_tag".to_string()];
                all.extend(names);
                all
            }
            None => names.collect(),
        }
    }
}

/// `Schema.Class<E>("E")(fields)`: a named constructible schema with no
/// discriminant.
pub fn class(identifier: &str, fields: Vec<(&str, Schema<Unknown>)>) -> Schema<ClassValue> {
    build(meta_from_fields(
        identifier,
        None,
        ExcessProperty::Ignore,
        own(fields),
    ))
}

/// `Schema.TaggedClass<E>()("E", fields)`: the identifier is the tag, and the
/// wire carries it as `_tag`.
pub fn tagged_class(identifier: &str, fields: Vec<(&str, Schema<Unknown>)>) -> Schema<ClassValue> {
    build(meta_from_fields(
        identifier,
        Some(identifier.to_string()),
        ExcessProperty::Ignore,
        own(fields),
    ))
}

/// `Schema.Error<E>()("E")(fields)`: a tagged error whose identifier defaults to
/// its tag.
pub fn error(tag: &str, fields: Vec<(&str, Schema<Unknown>)>) -> Schema<ClassValue> {
    error_named(tag, tag, fields)
}

/// `Schema.Error<E>("A")("B")(fields)`: a distinct identifier and tag.
pub fn error_named(
    identifier: &str,
    tag: &str,
    fields: Vec<(&str, Schema<Unknown>)>,
) -> Schema<ClassValue> {
    tagged_error(identifier, tag, fields)
}

/// `Schema.TaggedError<E>()(tag, fields)`, and its two-argument
/// `Schema.TaggedError<E>(identifier)(tag, fields)` spelling.
pub fn tagged_error(
    identifier: &str,
    tag: &str,
    fields: Vec<(&str, Schema<Unknown>)>,
) -> Schema<ClassValue> {
    build(meta_from_fields(
        identifier,
        Some(tag.to_string()),
        ExcessProperty::Ignore,
        own(fields),
    ))
}

/// `Schema.instanceOf(A)`: a value that must already be an instance of the
/// named class. Decoding and encoding pass it through unchanged, which is the
/// pin's `declare`, and the formatter prints it the way `format` prints a class
/// instance: `A({"a":"a"})`, the constructor name around the instance's own
/// keys.
///
/// A class with its own `toString` prints through it instead, which is
/// [`instance_of_with_to_string`]: a Rust value cannot be asked whether it
/// defines one, so the caller says so.
pub fn instance_of(
    class_name: &str,
    predicate: impl Fn(&Unknown) -> bool + Send + Sync + 'static,
) -> Schema<Unknown> {
    build_instance(class_name, predicate, None)
}

/// `Schema.instanceOf(A)` for a class that defines a `toString`: the formatter
/// calls it rather than printing the instance's keys, which is the pin's
/// `format` rule (`v.toString !== Object.prototype.toString`).
pub fn instance_of_with_to_string(
    class_name: &str,
    predicate: impl Fn(&Unknown) -> bool + Send + Sync + 'static,
    to_string: impl Fn(&Unknown) -> String + Send + Sync + 'static,
) -> Schema<Unknown> {
    build_instance(class_name, predicate, Some(Arc::new(to_string)))
}

fn build_instance(
    class_name: &str,
    predicate: impl Fn(&Unknown) -> bool + Send + Sync + 'static,
    to_string: Option<Arc<dyn Fn(&Unknown) -> String + Send + Sync>>,
) -> Schema<Unknown> {
    let mut out = Schema::declare(predicate, &format!("an instance of {class_name}"));
    let name = class_name.to_string();
    out.formatter = Some(Arc::new(move |value: &Unknown| match &to_string {
        Some(render) => render(value),
        None => format!("{name}({})", super::format_value(value)),
    }));
    out
}

fn own(fields: Vec<(&str, Schema<Unknown>)>) -> Vec<(String, Schema<Unknown>)> {
    fields
        .into_iter()
        .map(|(name, schema)| (name.to_string(), schema))
        .collect()
}

/// `Schema.Class<E>("E")(source)`: a class whose source is an existing schema,
/// so the source's checks and annotations are the class's own. The declared
/// fields are the source's, which is what its excess policy and its field list
/// read; a source with no declared field list contributes none.
///
/// This is the spelling the pin uses when a class wraps a refined struct, and
/// the reason it exists: `Class("A")(Struct({...}).check(f))` must run `f` once
/// on decode, which a field list rebuilt from scratch could not do.
pub fn class_from(identifier: &str, source: Schema<Unknown>) -> Schema<ClassValue> {
    let fields = source
        .fields()
        .map(|fields| {
            fields
                .iter()
                .map(|field| (field.name().to_string(), field.inner().clone()))
                .collect()
        })
        .unwrap_or_default();
    build_with(
        ClassMeta {
            identifier: identifier.to_string(),
            tag: None,
            excess: ExcessProperty::Ignore,
            fields,
            source: source.clone(),
        },
        source,
    )
}

/// The metadata for a class declared from a bare field list. The source is a
/// declared field list (`struct_shape`), not a `struct_of` map: a class field is
/// an occurrence with a name and an optionality, and that is what carries a
/// constructor default and names a failing path.
fn meta_from_fields(
    identifier: &str,
    tag: Option<String>,
    excess: ExcessProperty,
    fields: Vec<(String, Schema<Unknown>)>,
) -> ClassMeta {
    let source = Schema::as_unknown(Schema::struct_shape(
        fields
            .iter()
            .map(|(name, schema)| Field::required(name, schema.clone()))
            .collect(),
    ));
    ClassMeta {
        identifier: identifier.to_string(),
        tag,
        excess,
        fields,
        source,
    }
}

/// The one place a class schema is assembled: the source struct is the codec,
/// and the class adds its identity, its discriminant and its excess policy.
fn build(meta: ClassMeta) -> Schema<ClassValue> {
    let source = meta.source.clone();
    build_with(meta, source)
}

/// The assembly both constructors share: a class around an already-built
/// source schema.
fn build_with(meta: ClassMeta, source: Schema<Unknown>) -> Schema<ClassValue> {
    let declared = meta.declared();
    let identifier = meta.identifier.clone();
    // Kept beside the moved copies for the annotation below.
    let annotated_identifier = identifier.clone();
    let tag = meta.tag.clone();
    let excess = meta.excess;

    let decode_source = source.clone();
    let decode_declared = declared.clone();
    let decode_identifier = identifier.clone();
    let decode_tag = tag.clone();
    let encode_source = source.clone();
    let encode_identifier = identifier.clone();
    let encode_tag = tag.clone();
    let construct_source = source;
    let construct_declared = declared;
    let construct_identifier = identifier;
    let construct_tag = tag;

    let decode = move |unknown: Unknown| {
        let input = match &decode_tag {
            Some(tag) => strip_tag(unknown, tag)?,
            None => unknown,
        };
        let input = apply_excess(input, &decode_declared, excess)?;
        class_value(
            &decode_identifier,
            decode_tag.clone(),
            decode_source.decode(input)?,
        )
    };
    let encode = move |value: &ClassValue| {
        // Encoding is instance-directed: a value built by another class is not
        // this schema's to encode, which is the pin's `Expected A`.
        if value.identifier != encode_identifier {
            return Err(SchemaError::TypeMismatchDynamic {
                path: "$".to_string(),
                expected: format!("an instance of {encode_identifier}"),
                got: "another class's instance",
            });
        }
        let wire = encode_source.try_encode(&value.fields_unknown())?;
        Ok(match &encode_tag {
            // `_tag` travels first, which is the order the pin writes it.
            Some(tag) => {
                let mut map = BTreeMap::new();
                map.insert("_tag".to_string(), Unknown::Text(tag.clone()));
                if let Unknown::Object(fields) = wire {
                    map.extend(fields);
                }
                Unknown::Object(map)
            }
            None => wire,
        })
    };
    let mut out = Schema::new(decode, encode, SchemaKind::Object).with_class_meta(meta);
    // The pin's class sets an `identifier` annotation, which is what a repr and
    // `resolveAnnotations` read; the class metadata is the runtime half of the
    // same name.
    out = Schema::annotated_with(out, vec![("identifier", annotated_identifier.as_str())]);
    out = out.with_construct(Arc::new(move |unknown: Unknown| {
        let input = match &construct_tag {
            Some(tag) => strip_tag_constructing(unknown, tag)?,
            None => unknown,
        };
        let input = apply_excess(input, &construct_declared, excess)?;
        class_value(
            &construct_identifier,
            construct_tag.clone(),
            construct_source.construct(input)?,
        )
        .map_err(ConstructError::from)
    }));
    out
}

/// Build a class value from `input` through `source`, applying the class's
/// discriminant rule and excess policy. Shared by the constructor on the schema
/// and by `make_unchecked`, so the two cannot disagree about what a class
/// instance is.
fn construct_value(
    meta: &ClassMeta,
    source: &Schema<Unknown>,
    unknown: Unknown,
) -> Result<ClassValue, ConstructError> {
    let input = match &meta.tag {
        Some(tag) => strip_tag_constructing(unknown, tag)?,
        None => unknown,
    };
    let input = apply_excess(input, &meta.declared(), meta.excess)?;
    class_value(&meta.identifier, meta.tag.clone(), source.construct(input)?)
        .map_err(ConstructError::from)
}

/// The discriminant is a required key on the way in, and only the tag this class
/// declares is its value; a missing or foreign `_tag` is refused by name.
fn strip_tag(unknown: Unknown, tag: &str) -> Result<Unknown, SchemaError> {
    let Unknown::Object(fields) = unknown else {
        return Err(SchemaError::TypeMismatch {
            path: "$".to_string(),
            expected: "object",
            got: unknown.kind(),
        });
    };
    match fields.get("_tag") {
        Some(Unknown::Text(found)) if found == tag => Ok(Unknown::Object(
            fields
                .into_iter()
                .filter(|(name, _)| name != "_tag")
                .collect(),
        )),
        Some(Unknown::Text(_)) => Err(SchemaError::TypeMismatchDynamic {
            path: "$._tag".to_string(),
            expected: format!("\"{tag}\""),
            got: "another tag",
        }),
        _ => Err(SchemaError::TypeMismatchDynamic {
            path: "$._tag".to_string(),
            expected: format!("\"{tag}\""),
            got: "missing",
        }),
    }
}

/// The construction path's discriminant rule. A constructor SUPPLIES the tag
/// (`new E({ id: 1 })` carries none in its props), so an absent `_tag` is
/// normal; a `_tag` that names another class is still refused, and one that
/// matches is dropped so it is not treated as a field.
fn strip_tag_constructing(unknown: Unknown, tag: &str) -> Result<Unknown, SchemaError> {
    let Unknown::Object(fields) = unknown else {
        return Err(SchemaError::TypeMismatch {
            path: "$".to_string(),
            expected: "object",
            got: unknown.kind(),
        });
    };
    match fields.get("_tag") {
        Some(Unknown::Text(found)) if found != tag => Err(SchemaError::TypeMismatchDynamic {
            path: "$._tag".to_string(),
            expected: format!("\"{tag}\" or no tag"),
            got: "another tag",
        }),
        _ => Ok(Unknown::Object(
            fields
                .into_iter()
                .filter(|(name, _)| name != "_tag")
                .collect(),
        )),
    }
}

/// The excess-property policy: `Ignore`/`Strip` keep the declared fields only,
/// `Reject` fails naming the first key it cannot account for.
fn apply_excess(
    unknown: Unknown,
    declared: &[String],
    policy: ExcessProperty,
) -> Result<Unknown, SchemaError> {
    let Unknown::Object(fields) = unknown else {
        return Ok(unknown);
    };
    if policy == ExcessProperty::Reject {
        if let Some(extra) = fields.keys().find(|name| !declared.contains(name)) {
            return Err(SchemaError::TypeMismatchDynamic {
                path: format!("$.{extra}"),
                expected: "a declared field".to_string(),
                got: "an excess property",
            });
        }
    }
    Ok(Unknown::Object(
        fields
            .into_iter()
            .filter(|(name, _)| declared.contains(name))
            .collect(),
    ))
}

/// A decoded object plus the class identity. The object is the source struct's
/// `Unknown`, so its fields are already decoded values.
fn class_value(
    identifier: &str,
    tag: Option<String>,
    decoded: Unknown,
) -> Result<ClassValue, SchemaError> {
    let Unknown::Object(fields) = decoded else {
        return Err(SchemaError::TypeMismatch {
            path: "$".to_string(),
            expected: "object",
            got: decoded.kind(),
        });
    };
    Ok(ClassValue {
        identifier: identifier.to_string(),
        tag,
        fields: fields.into_iter().collect(),
    })
}

impl Schema<ClassValue> {
    /// The class this schema describes, or `None` when it is no longer one.
    pub fn class_meta(&self) -> Option<&ClassMeta> {
        self.class_meta.as_deref()
    }

    /// The identifier a repr names (`A.identifier`), which is also what a
    /// refusal reports.
    pub fn class_identifier(&self) -> Option<&str> {
        self.class_meta().map(|meta| meta.identifier.as_str())
    }

    /// The `_tag` a tagged class carries, `None` for a plain `Class`.
    pub fn class_tag(&self) -> Option<&str> {
        self.class_meta().and_then(|meta| meta.tag.as_deref())
    }

    /// The declared field names, `_tag` first for a tagged class: the list the
    /// pin's `Object.keys(E.fields)` gives.
    pub fn class_fields(&self) -> Vec<String> {
        self.class_meta()
            .map(ClassMeta::declared)
            .unwrap_or_default()
    }

    /// The pin's constructor options: `new A(props, { parseOptions })` selects
    /// the excess-property policy the construction runs with.
    pub fn with_excess(self, policy: ExcessProperty) -> Self {
        match self.class_meta.as_deref().cloned() {
            Some(mut meta) => {
                meta.excess = policy;
                // The source is untouched: the pin's parseOptions decide one
                // construction call, they do not rebuild the schema, so a check
                // on the source still runs.
                let source = meta.source.clone();
                build_with(meta, source)
            }
            None => self,
        }
    }

    /// `mapFields` on a class: the transformer receives the declared fields and
    /// answers the list the rebuilt class should have. The identifier and the
    /// tag stay, and the source is rebuilt from the mapped list, so a check on
    /// the original source does not survive (the pin rebuilds the struct too).
    pub fn map_fields(self, transform: impl Fn(Vec<Field>) -> Vec<Field>) -> Schema<ClassValue> {
        let Some(mut meta) = self.class_meta.as_deref().cloned() else {
            return self;
        };
        let declared = transform(
            meta.fields
                .iter()
                .map(|(name, schema)| Field::required(name, schema.clone()))
                .collect(),
        );
        let fields: Vec<(String, Schema<Unknown>)> = declared
            .into_iter()
            .map(|field| (field.name().to_string(), field.inner().clone()))
            .collect();
        // The pin rebuilds the struct too, so a check on the original source
        // does not survive this one. `extend` is the derivation that keeps it.
        build(meta_from_fields(
            &meta.identifier,
            meta.tag,
            meta.excess,
            fields,
        ))
    }

    /// `make`: construction. An instance built by this class is answered
    /// unchanged (the pin's `A.make(instance) === instance`); every other input
    /// constructs through the source struct, so a constructor default fills and
    /// a source check runs once.
    pub fn make(&self, input: Unknown) -> Result<ClassValue, AdapterError> {
        self.construct(input)
            .map_err(|error| AdapterError::report(error, CONSTRUCTOR_DIE))
    }

    /// `make` with no props argument: an empty object, which is what a
    /// zero-field class expects (and what an all-optional class accepts).
    pub fn make_void(&self) -> Result<ClassValue, AdapterError> {
        self.construct(Unknown::Object(BTreeMap::new()))
            .map_err(|error| AdapterError::report(error, CONSTRUCTOR_DIE))
    }

    /// `new A(props, { disableChecks: true })`: the constructor with the
    /// source's checks skipped. Constructor defaults still fill, because they
    /// belong to the field, not to the check (`Schema.test.ts:3802`).
    pub fn make_unchecked(&self, input: Unknown) -> Result<ClassValue, AdapterError> {
        let Some(meta) = self.class_meta.as_deref() else {
            return Err(AdapterError::Issue(SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "a class schema",
                got: "no class metadata",
            }));
        };
        let source = meta.source.clone().without_checks();
        construct_value(meta, &source, input)
            .map_err(|error| AdapterError::report(error, CONSTRUCTOR_DIE))
    }

    /// `make` given a value this class already built: the same value back, with
    /// no source check run again.
    pub fn make_instance(&self, value: &ClassValue) -> Result<ClassValue, AdapterError> {
        match self.class_identifier() {
            Some(identifier) if value.identifier() == identifier => Ok(value.clone()),
            Some(identifier) => Err(AdapterError::Issue(SchemaError::TypeMismatchDynamic {
                path: "$".to_string(),
                expected: format!("an instance of {identifier}"),
                got: "another class's instance",
            })),
            None => Err(AdapterError::Issue(SchemaError::TypeMismatch {
                path: "$".to_string(),
                expected: "a class schema",
                got: "no class metadata",
            })),
        }
    }

    /// The wire form of a value this class built, or the refusal naming the
    /// class when the value came from another one.
    pub fn encode_instance(&self, value: &ClassValue) -> Result<Unknown, SchemaError> {
        self.try_encode(value)
    }

    /// `A.extend("B")(fields)`: the parent's fields and its tag, with the
    /// child's identifier and additional fields. The tag deliberately stays the
    /// parent's, which is what the pin's `_tag` inheritance asserts.
    pub fn extend(
        self,
        identifier: &str,
        fields: Vec<(&str, Schema<Unknown>)>,
    ) -> Schema<ClassValue> {
        let child = Schema::as_unknown(Schema::struct_shape(
            own(fields)
                .into_iter()
                .map(|(name, schema)| Field::required(&name, schema))
                .collect(),
        ));
        self.extend_from(identifier, child)
    }

    /// `A.extend("B")(source)`: the spelling whose child fields are a schema,
    /// which is how the pin passes a refined struct. The parent's source and the
    /// child's are composed through `mapFields` with `unsafePreserveChecks`, so
    /// each side's check runs once on the child rather than being dropped by a
    /// rebuild.
    pub fn extend_from(
        self,
        identifier: &str,
        source: Schema<Unknown>,
    ) -> Schema<ClassValue> {
        let Some(meta) = self.class_meta.as_deref().cloned() else {
            return self;
        };
        let extra: Vec<Field> = source
            .fields()
            .map(|fields| fields.to_vec())
            .unwrap_or_default();
        let child_fields: Vec<(String, Schema<Unknown>)> = extra
            .iter()
            .map(|field| (field.name().to_string(), field.inner().clone()))
            .collect();
        let parent_source = meta.source.clone();
        let merged = parent_source.map_fields_preserving_checks(|mut declared| {
            declared.extend(extra.iter().cloned());
            declared
        });
        // The child's own chain wraps the merged shape, which is the pin's order:
        // its filter sees the same object both sides' fields live on.
        let child_filters = source.applied_filters.clone();
        let composed = if child_filters.is_empty() {
            merged
        } else {
            Schema::filtered(merged, child_filters, false)
        };
        let mut all = meta.fields;
        all.extend(child_fields);
        build_with(
            ClassMeta {
                identifier: identifier.to_string(),
                tag: meta.tag,
                excess: meta.excess,
                fields: all,
                source: composed.clone(),
            },
            composed,
        )
    }
}
