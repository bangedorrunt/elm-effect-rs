//! Representation of a schema as its kind tree. The host layer for a schema AST
//! walk; rendering is deliberately a debug rendering today.

use crate::Schema;
use crate::SchemaKind;
use crate::Unknown;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaRepresentation {
    Node {
        kind: String,
        children: Vec<SchemaRepresentation>,
    },
}

impl SchemaRepresentation {
    pub fn from_kind(kind: &SchemaKind) -> Self {
        Self::build(kind)
    }

    fn build(kind: &SchemaKind) -> Self {
        match kind {
            SchemaKind::String
            | SchemaKind::Number
            | SchemaKind::Boolean
            | SchemaKind::Object
            | SchemaKind::Json => Self::Node {
                kind: kind.label(),
                children: vec![],
            },
            SchemaKind::Array(inner)
            | SchemaKind::Optional(inner)
            | SchemaKind::Rest(inner)
            | SchemaKind::OptionalKey(inner)
            | SchemaKind::Annotated { inner, .. } => Self::Node {
                kind: kind.label(),
                children: vec![Self::build(inner)],
            },
            SchemaKind::Struct(fields) => Self::Node {
                kind: "Struct".to_string(),
                children: fields.iter().map(|(_, k)| Self::build(k)).collect(),
            },
            SchemaKind::StructWithRest(fields, rest) => Self::Node {
                kind: "StructWithRest".to_string(),
                children: fields
                    .iter()
                    .map(|(_, k)| Self::build(k))
                    .chain(std::iter::once(Self::build(rest)))
                    .collect(),
            },
            SchemaKind::Record(k, v) => Self::Node {
                kind: "Record".to_string(),
                children: vec![Self::build(k), Self::build(v)],
            },
            SchemaKind::Tuple(items) => Self::Node {
                kind: "Tuple".to_string(),
                children: items.iter().map(Self::build).collect(),
            },
            SchemaKind::TupleWithRest(items, rest) => {
                let mut children: Vec<_> = items.iter().map(Self::build).collect();
                children.extend(rest.iter().map(Self::build));
                Self::Node {
                    kind: "TupleWithRest".to_string(),
                    children,
                }
            }
            SchemaKind::Check { base, .. } => Self::Node {
                kind: "Check".to_string(),
                children: vec![Self::build(base)],
            },
            SchemaKind::Literals(_) => Self::Node {
                kind: "Literals".to_string(),
                children: vec![],
            },
            SchemaKind::Literal(_) => Self::Node {
                kind: "Literal".to_string(),
                children: vec![],
            },
            SchemaKind::Reference(name) => Self::Node {
                kind: format!("Reference({name})"),
                children: vec![],
            },
            SchemaKind::Opaque(name) => Self::Node {
                kind: format!("Opaque({name})"),
                children: vec![],
            },
            SchemaKind::Union { members, .. } => Self::Node {
                kind: "Union".to_string(),
                children: members.iter().map(Self::build).collect(),
            },
            SchemaKind::TemplateLiteral(parts) => Self::Node {
                kind: "TemplateLiteral".to_string(),
                children: parts.iter().map(Self::build).collect(),
            },
        }
    }

    pub fn to_json_string(&self) -> String {
        match self {
            Self::Node { kind, children } => {
                if children.is_empty() {
                    kind.clone()
                } else {
                    format!(
                        "{}({})",
                        kind,
                        children
                            .iter()
                            .map(Self::to_json_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
        }
    }
}

impl SchemaKind {
    /// Walk every kind node in the schema tree, including nested children.
    pub fn visit<F>(&self, f: &mut F)
    where
        F: FnMut(&SchemaKind),
    {
        f(self);
        match self {
            SchemaKind::Array(inner)
            | SchemaKind::Optional(inner)
            | SchemaKind::Rest(inner)
            | SchemaKind::OptionalKey(inner)
            | SchemaKind::Annotated { inner, .. } => inner.visit(f),
            SchemaKind::Struct(fields) => {
                for (_, child) in fields {
                    child.visit(f);
                }
            }
            SchemaKind::StructWithRest(fields, rest) => {
                for (_, child) in fields {
                    child.visit(f);
                }
                rest.visit(f);
            }
            SchemaKind::Record(k, v) => {
                k.visit(f);
                v.visit(f);
            }
            SchemaKind::Tuple(items) => {
                for child in items {
                    child.visit(f);
                }
            }
            SchemaKind::TupleWithRest(items, rest) => {
                for child in items.iter().chain(rest) {
                    child.visit(f);
                }
            }
            SchemaKind::Check { base, .. } => base.visit(f),
            SchemaKind::Union { members, .. } => {
                for child in members {
                    child.visit(f);
                }
            }
            SchemaKind::TemplateLiteral(parts) => {
                for child in parts {
                    child.visit(f);
                }
            }
            SchemaKind::Literals(_)
            | SchemaKind::Literal(_)
            | SchemaKind::Opaque(_)
            | SchemaKind::Reference(_)
            | SchemaKind::String
            | SchemaKind::Number
            | SchemaKind::Boolean
            | SchemaKind::Object
            | SchemaKind::Json => {}
        }
    }
}

impl SchemaKind {
    /// Convenience label for a kind.
    fn label(&self) -> String {
        match self {
            SchemaKind::String => "String".to_string(),
            SchemaKind::Number => "Number".to_string(),
            SchemaKind::Boolean => "Boolean".to_string(),
            SchemaKind::Object => "Object".to_string(),
            SchemaKind::Array(_) => "Array".to_string(),
            SchemaKind::Optional(_) => "Optional".to_string(),
            SchemaKind::OptionalKey(_) => "OptionalKey".to_string(),
            SchemaKind::Rest(_) => "Rest".to_string(),
            SchemaKind::Annotated { .. } => "Annotated".to_string(),
            SchemaKind::Literal(_) => "Literal".to_string(),
            SchemaKind::Literals(_) => "Literals".to_string(),
            SchemaKind::Opaque(name) => format!("Opaque({name})"),
            SchemaKind::Reference(name) => format!("Reference({name})"),
            SchemaKind::Json => "Json".to_string(),
            SchemaKind::Tuple(_) => "Tuple".to_string(),
            SchemaKind::TupleWithRest(_, _) => "TupleWithRest".to_string(),
            SchemaKind::Struct(_) => "Struct".to_string(),
            SchemaKind::StructWithRest(_, _) => "StructWithRest".to_string(),
            SchemaKind::Record(_, _) => "Record".to_string(),
            SchemaKind::Check { .. } => "Check".to_string(),
            SchemaKind::Union { .. } => "Union".to_string(),
            SchemaKind::TemplateLiteral(_) => "TemplateLiteral".to_string(),
        }
    }
}

impl<T> Schema<T> {
    pub fn kind(&self) -> crate::SchemaKind {
        self.repr_kind()
    }

    pub fn representation(&self) -> SchemaRepresentation {
        SchemaRepresentation::from_kind(&self.repr_kind())
    }
}

pub fn default_on_before(_value: &Unknown) {}

pub fn format_with_on_before(
    schema: &Schema<impl std::marker::Send + std::marker::Sync + 'static>,
    value: &Unknown,
    on_before: Option<&dyn Fn(&Unknown)>,
) {
    if let Some(hook) = on_before {
        hook(value);
    }
    let _ = schema.formatter()(value);
}

impl<T> Schema<T> {
    pub fn formatter_with_on_before<F>(
        &self,
        on_before: F,
    ) -> std::boxed::Box<
        dyn Fn(&crate::Unknown) -> String + std::marker::Send + std::marker::Sync + 'static,
    >
    where
        T: Send + Sync + 'static,
        F: Fn(&Unknown) + Send + Sync + 'static,
    {
        let base = self.formatter();
        std::boxed::Box::new(move |value| {
            on_before(value);
            base(value)
        })
    }
}

impl<T> Schema<T> {
    /// Alias of [`Schema::formatter()`] with an optional `onBefore` hook: when
    /// a hook is supplied it runs on the value before the formatter renders it,
    /// matching the pin's `toFormatter({ onBefore })`.
    pub fn to_formatter<F>(
        &self,
        on_before: Option<F>,
    ) -> Box<dyn Fn(&Unknown) -> String + Send + Sync + 'static>
    where
        F: Fn(&Unknown) + Send + Sync + 'static,
    {
        let base = self.formatter();
        match on_before {
            Some(hook) => Box::new(move |value| {
                hook(value);
                base(value)
            }),
            None => base,
        }
    }
}

// Final re-check marker for elm-effect-rs-t113: compile only, no behavioral change.
