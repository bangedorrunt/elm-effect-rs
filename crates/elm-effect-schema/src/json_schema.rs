//! Schema to JSON Schema compiler: core compilation plus the
//! `toJsonSchemaDocument` document rules.
//!
//! Compiles the port's `SchemaKind` tree to JSON Schema documents: scalar
//! types, constrained strings/numbers/arrays/objects, typed enums and consts,
//! compacted literal unions, `oneOf`/`anyOf`/`allOf` unions, tuple
//! `prefixItems`, record `patternProperties`/`propertyNames` key shapes, and
//! `$defs` hoisting for identifier-annotated schemas (escaped, suffixed on
//! collision, shared across roots).
//!
//! `CompileOptions` carries the document-level knobs the rows pin down:
//! `onExcessProperty` (open by default, like the pin; `"error"` closes),
//! `generateDescriptions` (the `expected` message becomes the description),
//! and `includeAnnotationKey` (custom annotation keys ride only when the
//! predicate accepts them; standard keys always ride).
//!
//! Checks the compiler cannot spell as a keyword (`isFinite`, unknown
//! filters, custom callbacks, non-finite bounds) degrade to the unconstrained
//! base, the same pass-through the decoder uses for a check it does not know.
//! String length checks export the pin's loose code-unit bounds
//! (`isMinLength(n)` is `minLength: n - 1`); code-point checks stay exact.
//! A check that carries its own annotations, or that collides with a keyword
//! already on the document, lands in `allOf` instead of inline.
//!
//! Out of scope: the encoded-side projection (`fromJsonString`, `Class` and
//! `Error` encoded identifiers, `encodeTo` rewrites), custom annotation
//! values (pairs are strings, so objects/arrays/booleans have no spelling),
//! template-literal lowering (the kind holds no parts), multi-root reference
//! registries for mutual recursion, and constructors the port does not have
//! (`Date`, `URL`, `BigInt`, `Symbol`, `Uint8Array`, `Duration`, `Option`,
//! `Enum`, `Undefined`, `TemplateLiteral`). Their compiler rules live here
//! behind opaque-kind mappings with in-module tests; the constructors stay
//! excused.

use super::representation_doc::Document;
use super::{Field, Optionality, Schema, SchemaError, SchemaKind, Unknown, kind_type_name};
use std::collections::BTreeMap;

/// Several roots compiled against one definition map: shared identifiers are
/// emitted once and every use is a `$ref`.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiDocument {
    /// One compiled schema per root, in input order.
    pub schemas: Vec<serde_json::Value>,
    /// Definitions hoisted from identifier annotations, shared by all roots.
    pub definitions: serde_json::Map<String, serde_json::Value>,
}

/// What an object does with a key no other keyword claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnExcessProperty {
    /// Closed: `additionalProperties: false` (the pin's `"error"` mode).
    Error,
    /// Open: `additionalProperties: true` (the pin's default).
    #[default]
    Ignore,
}

/// Document-level knobs for [`compile_with_options`].
#[derive(Debug, Clone, Copy)]
pub struct CompileOptions {
    /// Closed (`Error`) or open (`Ignore`) objects.
    pub on_excess_property: OnExcessProperty,
    /// The `expected` message becomes the `description` when no description
    /// is present.
    pub generate_descriptions: bool,
    /// Custom annotation keys ride the document only when this accepts them.
    /// Standard keys ride always.
    pub include_annotation_key: Option<fn(&str) -> bool>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            on_excess_property: OnExcessProperty::Ignore,
            generate_descriptions: false,
            include_annotation_key: None,
        }
    }
}

/// Annotation keys that ride the document without any predicate. The pin
/// always includes its standard keys; anything else needs
/// `includeAnnotationKey`.
fn is_standard_annotation_key(key: &str) -> bool {
    matches!(
        key,
        "title" | "description" | "default" | "examples" | "readOnly" | "writeOnly"
    )
}

/// Human-facing keywords a check may have produced on a document already. A
/// second check touching one of these, or carrying its own annotations onto a
/// document that already has them, goes to `allOf` instead of inline.
fn is_check_keyword(key: &str) -> bool {
    matches!(
        key,
        "minLength"
            | "maxLength"
            | "pattern"
            | "format"
            | "minimum"
            | "maximum"
            | "exclusiveMinimum"
            | "exclusiveMaximum"
            | "multipleOf"
            | "minItems"
            | "maxItems"
            | "uniqueItems"
            | "minProperties"
            | "maxProperties"
            | "contentEncoding"
            | "contentMediaType"
    )
}

/// Compile one schema to a JSON Schema document, hoisting identifier
/// annotations into a `$defs` entry and emitting a `$ref` at each use.
pub fn compile(schema: &Schema<Unknown>) -> serde_json::Value {
    compile_with_options(schema, &CompileOptions::default())
}

/// [`compile`] with explicit [`CompileOptions`].
pub fn compile_with_options(
    schema: &Schema<Unknown>,
    options: &CompileOptions,
) -> serde_json::Value {
    let mut cx = Compiler {
        options,
        defs: serde_json::Map::new(),
    };
    let body = cx.doc_of_schema(schema);
    cx.finish(body)
}

/// Compile several roots against one shared definition map: an identifier
/// used by more than one root is emitted once and referenced everywhere.
pub fn compile_multi(roots: &[Schema<Unknown>]) -> MultiDocument {
    compile_multi_with_options(roots, &CompileOptions::default())
}

/// [`compile_multi`] with explicit [`CompileOptions`].
pub fn compile_multi_with_options(
    roots: &[Schema<Unknown>],
    options: &CompileOptions,
) -> MultiDocument {
    let mut cx = Compiler {
        options,
        defs: serde_json::Map::new(),
    };
    let schemas = roots.iter().map(|root| cx.doc_of_schema(root)).collect();
    let definitions = std::mem::take(&mut cx.defs);
    MultiDocument {
        schemas,
        definitions,
    }
}

/// One peeled wrapper: annotation pairs or a named check, outermost first.
#[derive(Debug, Clone)]
enum Layer {
    Annotate(Vec<(String, String)>),
    Check {
        name: String,
        args: Vec<String>,
        annotation: Vec<(String, String)>,
    },
}

/// The compiler state: options plus the shared definition map.
struct Compiler<'a> {
    options: &'a CompileOptions,
    defs: serde_json::Map<String, serde_json::Value>,
}

impl<'a> Compiler<'a> {
    /// Attach `$defs` when identifiers were hoisted. A primitive root holds no
    /// identifiers, so the object arm is the only one that fires; anything
    /// else keeps the definitions rather than dropping them.
    fn finish(&mut self, body: serde_json::Value) -> serde_json::Value {
        if self.defs.is_empty() {
            return body;
        }
        match body {
            serde_json::Value::Object(mut map) => {
                map.insert(
                    "$defs".to_string(),
                    serde_json::Value::Object(std::mem::take(&mut self.defs)),
                );
                serde_json::Value::Object(map)
            }
            other => serde_json::json!({"allOf": [other], "$defs": std::mem::take(&mut self.defs)}),
        }
    }

    /// Escape a definition name for a JSON Pointer `$ref`: `~` then `/`.
    fn escape_pointer(name: &str) -> String {
        name.replace('~', "~0").replace('/', "~1")
    }

    /// Hoist `doc` under `id`, suffixing on collision with a different
    /// document (`id_1`, `id_2`, ...) and reusing the entry when the same
    /// shape repeats. Answers the `$ref` for the stored entry.
    fn intern(&mut self, id: &str, doc: serde_json::Value) -> serde_json::Value {
        if let Some(existing) = self.defs.get(id) {
            if *existing == doc {
                return Self::reference(id);
            }
            let mut counter = 1;
            loop {
                let candidate = format!("{id}_{counter}");
                match self.defs.get(&candidate) {
                    Some(existing) if *existing == doc => {
                        return Self::reference(&candidate);
                    }
                    Some(_) => counter += 1,
                    None => {
                        self.defs.insert(candidate.clone(), doc);
                        return Self::reference(&candidate);
                    }
                }
            }
        }
        self.defs.insert(id.to_string(), doc);
        Self::reference(id)
    }

    /// A `$ref` for a stored definition name, pointer-escaped.
    fn reference(id: &str) -> serde_json::Value {
        serde_json::json!({"$ref": format!("#/$defs/{}", Self::escape_pointer(id))})
    }

    /// Whether the predicate (or the standard set) keeps an annotation pair.
    /// `expected` and `identifier` never ride inline: one is a codec message,
    /// the other hoists.
    fn keep_pair(&self, key: &str) -> bool {
        if key == "expected" || key == "identifier" {
            return false;
        }
        if is_standard_annotation_key(key) {
            return true;
        }
        self.options
            .include_annotation_key
            .is_some_and(|predicate| predicate(key))
    }

    /// Merge annotation pairs into a document: standard and accepted keys
    /// ride, `expected`/`identifier` never do, and the outer pair wins on
    /// collision. With `generateDescriptions`, a lone `expected` becomes the
    /// description.
    fn merge_pairs(&self, doc: &mut serde_json::Value, pairs: &[(String, String)]) {
        let map = match doc.as_object_mut() {
            Some(map) => map,
            None => return,
        };
        for (key, value) in pairs {
            if self.keep_pair(key) {
                map.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
        }
        if self.options.generate_descriptions
            && !map.contains_key("description")
            && let Some((_, expected)) = pairs.iter().find(|(key, _)| key == "expected")
        {
            map.insert(
                "description".to_string(),
                serde_json::Value::String(expected.clone()),
            );
        }
    }

    /// The document for a schema: peel annotation and check wrappers into
    /// layers, compile the shaped core (structs, tuples and unions read the
    /// schema's stored members so key annotations survive), then apply the
    /// layers innermost-first.
    fn doc_of_schema(&mut self, schema: &Schema<Unknown>) -> serde_json::Value {
        let mut layers: Vec<Layer> = Vec::new();
        let mut kind = schema.kind();
        loop {
            match kind {
                SchemaKind::Annotated { pairs, inner } => {
                    layers.push(Layer::Annotate(pairs));
                    kind = *inner;
                }
                SchemaKind::Check {
                    base,
                    name,
                    args,
                    annotation,
                } => {
                    layers.push(Layer::Check {
                        name,
                        args,
                        annotation,
                    });
                    kind = *base;
                }
                _ => break,
            }
        }
        let base_type = base_type_of(&kind);
        let mut doc = self.doc_of_core(schema, &kind);
        for layer in layers.iter().rev() {
            match layer {
                Layer::Annotate(pairs) => {
                    if let Some((_, id)) = pairs.iter().find(|(key, _)| key == "identifier") {
                        let id = id.clone();
                        self.merge_pairs(&mut doc, pairs);
                        return self.intern(&id, doc);
                    }
                    self.merge_pairs(&mut doc, pairs);
                }
                Layer::Check {
                    name,
                    args,
                    annotation,
                } => self.apply_check(&mut doc, base_type, name, args, annotation),
            }
        }
        // Annotations the pin drops for these shapes: the open value and the
        // shapes that describe nothing (`Void`, `Undefined`) or that the pin
        // documents bare (`ObjectKeyword`) carry no annotations.
        if matches!(
            strip_kind(&kind),
            SchemaKind::Json
                | SchemaKind::Opaque("unknown")
                | SchemaKind::Opaque("void")
                | SchemaKind::Opaque("undefined")
                | SchemaKind::Opaque("objectKeyword")
        ) {
            doc = self.doc_of_kind(&kind);
        }
        doc
    }

    /// The shaped core: structs, tuples and unions read the schema's stored
    /// members (full schemas, key annotations included); everything else
    /// compiles from the kind alone.
    fn doc_of_core(&mut self, schema: &Schema<Unknown>, kind: &SchemaKind) -> serde_json::Value {
        match kind {
            SchemaKind::Struct(fields) if schema.children.len() == fields.len() => {
                let fields: Vec<(String, Schema<Unknown>)> = schema
                    .children
                    .iter()
                    .map(|(name, field)| (name.clone(), (**field).clone()))
                    .collect();
                self.struct_document(&fields, None)
            }
            SchemaKind::Object if schema.struct_fields.is_some() => {
                let fields = schema.struct_fields.clone().unwrap_or_default();
                self.fields_document(&fields)
            }
            SchemaKind::Tuple(items) if tuple_stored(schema, items.len()) => {
                let elements = schema.tuple_elements.clone().unwrap_or_default();
                self.tuple_document(&elements, None)
            }
            SchemaKind::TupleWithRest(items, rest) if tuple_stored(schema, items.len()) => {
                let elements = schema.tuple_elements.clone().unwrap_or_default();
                self.tuple_document(&elements, Some((**rest).clone()))
            }
            SchemaKind::Union { mode, .. } if schema.union_members.is_some() => {
                let members = schema.union_members.clone().unwrap_or_default();
                let docs = members
                    .iter()
                    .map(|member| self.member_doc(member))
                    .collect::<Vec<_>>();
                union_document(docs, mode)
            }
            _ => self.doc_of_kind(kind),
        }
    }

    /// A struct/tuple/union member: its own document plus its key annotations
    /// as an `allOf` entry. Key annotations ride the occurrence, never the
    /// root (a top-level `annotateKey` is dropped).
    fn member_doc(&mut self, member: &Schema<Unknown>) -> serde_json::Value {
        let mut doc = self.doc_of_schema(member);
        if let Some(pairs) = member.resolve_annotations_key()
            && !pairs.is_empty()
            && let serde_json::Value::Object(map) = &mut doc
        {
            map.insert(
                "allOf".to_string(),
                serde_json::Value::Array(vec![serde_json::Value::Object(
                    pairs
                        .into_iter()
                        .map(|(key, value)| (key, serde_json::Value::String(value)))
                        .collect(),
                )]),
            );
        }
        doc
    }

    /// An object document for named fields from full schemas: non-absent
    /// fields are required, and a plain struct is closed (or open when the
    /// options ignore excess properties). No fields is the pin's empty-struct
    /// shape: anything but null.
    fn struct_document(
        &mut self,
        fields: &[(String, Schema<Unknown>)],
        open: Option<&SchemaKind>,
    ) -> serde_json::Value {
        if fields.is_empty() && open.is_none() {
            return serde_json::json!({"not": {"type": "null"}});
        }
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, field) in fields {
            let unwrapped = unwrap_optional_kind(&field.kind());
            let mut doc = self.doc_of_kind(&unwrapped);
            if let Some(pairs) = field.resolve_annotations_key()
                && !pairs.is_empty()
                && let serde_json::Value::Object(map) = &mut doc
            {
                map.insert(
                    "allOf".to_string(),
                    serde_json::Value::Array(vec![serde_json::Value::Object(
                        pairs
                            .into_iter()
                            .map(|(key, value)| (key, serde_json::Value::String(value)))
                            .collect(),
                    )]),
                );
            }
            properties.insert(name.clone(), doc);
            if !is_absent_tolerant_kind(&field.kind()) {
                required.push(serde_json::Value::String(name.clone()));
            }
        }
        let mut doc = serde_json::json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
        });
        if !required.is_empty() {
            doc["required"] = serde_json::Value::Array(required);
        }
        match open {
            // No catch-all named: closed in error mode, open in ignore mode.
            None => {
                doc["additionalProperties"] = serde_json::Value::Bool(
                    self.options.on_excess_property == OnExcessProperty::Ignore,
                );
            }
            // An open catch-all stays implicit; anything else is spelled out.
            Some(rest) if is_open_record_rest(rest) => {}
            Some(rest) if is_never_kind(rest) => {
                doc["additionalProperties"] = serde_json::Value::Bool(false);
            }
            Some(rest) => {
                doc["additionalProperties"] = self.doc_of_kind(rest);
            }
        }
        doc
    }

    /// An object document for fields declared through the `Field` API: the
    /// optionality lives on the field, so an `Optional` field is an
    /// absent-or-null `anyOf` while `OptionalKey` stays a plain property the
    /// `required` list leaves out.
    fn fields_document(&mut self, fields: &[Field]) -> serde_json::Value {
        if fields.is_empty() {
            return serde_json::json!({"not": {"type": "null"}});
        }
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in fields {
            let mut doc = self.doc_of_schema(field.inner());
            // An inner schema that already spells the null branch (an
            // `optional` re-typed to `Unknown`) stands as it is; a plain
            // inner shape gains the absent-or-null union.
            if field.optionality() == Optionality::Optional
                && !matches!(strip_kind(&field.inner().kind()), SchemaKind::Optional(_))
            {
                doc = serde_json::json!({"anyOf": [doc, {"type": "null"}]});
            }
            if let Some(pairs) = field.inner().resolve_annotations_key()
                && !pairs.is_empty()
                && let serde_json::Value::Object(map) = &mut doc
            {
                map.insert(
                    "allOf".to_string(),
                    serde_json::Value::Array(vec![serde_json::Value::Object(
                        pairs
                            .into_iter()
                            .map(|(key, value)| (key, serde_json::Value::String(value)))
                            .collect(),
                    )]),
                );
            }
            properties.insert(field.name().to_string(), doc);
            if field.optionality() == Optionality::Required {
                required.push(serde_json::Value::String(field.name().to_string()));
            }
        }
        let mut doc = serde_json::json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
        });
        if !required.is_empty() {
            doc["required"] = serde_json::Value::Array(required);
        }
        doc["additionalProperties"] =
            serde_json::Value::Bool(self.options.on_excess_property == OnExcessProperty::Ignore);
        doc
    }

    /// A tuple document from full element schemas: no elements is
    /// `items: false`; otherwise `prefixItems` with the leading required
    /// count as `minItems` and the full count as `maxItems` (plus `items`
    /// for a trailing catch-all, which always keeps its `minItems`).
    fn tuple_document(
        &mut self,
        elements: &[Schema<Unknown>],
        rest: Option<SchemaKind>,
    ) -> serde_json::Value {
        if elements.is_empty() && rest.is_none() {
            return serde_json::json!({"type": "array", "items": false});
        }
        let compiled: Vec<serde_json::Value> =
            elements.iter().map(|item| self.member_doc(item)).collect();
        let leading = elements
            .iter()
            .take_while(|item| !is_absent_tolerant_kind(&item.kind()))
            .count();
        match rest {
            None => {
                let count = compiled.len();
                let mut doc = serde_json::json!({
                    "type": "array",
                    "prefixItems": compiled,
                    "maxItems": count,
                });
                if leading > 0 {
                    doc["minItems"] =
                        serde_json::Value::Number(serde_json::Number::from(leading as u64));
                }
                doc
            }
            Some(rest) => {
                let items = self.doc_of_kind(&rest);
                let mut doc = serde_json::json!({
                    "type": "array",
                    "prefixItems": compiled,
                    "items": items,
                });
                if leading > 0 {
                    doc["minItems"] =
                        serde_json::Value::Number(serde_json::Number::from(leading as u64));
                }
                doc
            }
        }
    }

    /// Apply one named check to a document already compiled from its base. A
    /// check with its own annotations, or one colliding with a keyword already
    /// present (other than a type change), becomes an `allOf` entry; anything
    /// else merges inline. Checks with no keyword degrade to the base.
    fn apply_check(
        &mut self,
        doc: &mut serde_json::Value,
        base_type: Option<&str>,
        name: &str,
        args: &[String],
        annotation: &[(String, String)],
    ) {
        let pairs: Vec<(String, String)> = annotation
            .iter()
            // Exclusivity flags ride the check's annotation beside the bounds
            // but are control input, not document content.
            .filter(|(key, _)| key != "exclusiveMinimum" && key != "exclusiveMaximum")
            .filter(|(key, _)| self.keep_pair(key))
            .cloned()
            .collect();
        match check_fragment(
            base_type,
            name,
            args,
            &pairs,
            is_exclusive(annotation, "exclusiveMinimum"),
            is_exclusive(annotation, "exclusiveMaximum"),
        ) {
            CheckFragment::Skip => {}
            CheckFragment::Inline(keywords) => {
                let dirty = doc.get("allOf").is_some()
                    || doc
                        .as_object()
                        .is_some_and(|map| map.keys().any(|key| is_check_keyword(key)));
                let collides = keywords.iter().any(|(key, value)| {
                    key != "type" && doc.get(key).is_some_and(|current| current != value)
                });
                if (!pairs.is_empty() && dirty) || collides {
                    push_entry(doc, flat_entry(pairs, keywords));
                } else if let serde_json::Value::Object(map) = doc {
                    for (key, value) in keywords {
                        map.insert(key, value);
                    }
                    for (key, value) in pairs {
                        map.insert(key, serde_json::Value::String(value));
                    }
                }
            }
            CheckFragment::Entries { inline, entries } => {
                if let serde_json::Value::Object(map) = doc {
                    for (key, value) in inline {
                        map.insert(key, value);
                    }
                }
                for entry in entries {
                    push_entry(doc, entry);
                }
            }
        }
    }

    /// One kind as a document, for positions that only carry a kind (array
    /// items, record keys and values, check bases, rest shapes).
    fn doc_of_kind(&mut self, kind: &SchemaKind) -> serde_json::Value {
        match kind {
            SchemaKind::String => serde_json::json!({"type": "string"}),
            SchemaKind::Number => serde_json::json!({"type": "number"}),
            SchemaKind::Boolean => serde_json::json!({"type": "boolean"}),
            SchemaKind::Json => serde_json::json!({}),
            SchemaKind::Object => serde_json::json!({"type": "object"}),
            SchemaKind::Opaque("null") => serde_json::json!({"type": "null"}),
            SchemaKind::Opaque("never") => serde_json::json!({"not": {}}),
            SchemaKind::Opaque("void") | SchemaKind::Opaque("undefined") => {
                serde_json::json!({"type": "null"})
            }
            SchemaKind::Opaque("objectKeyword") => {
                serde_json::json!({"anyOf": [{"type": "array"}, {"type": "object"}]})
            }
            // Declarations the port has no constructor for yet: their
            // documents are fixed by the pin, so the compiler spells them.
            SchemaKind::Opaque("date") | SchemaKind::Opaque("url") => {
                serde_json::json!({"type": "string"})
            }
            // A number written as text decodes from its string wire form, so
            // the document describes the encoded side.
            SchemaKind::Opaque("numberFromString") | SchemaKind::Opaque("finiteFromString") => {
                serde_json::json!({"type": "string"})
            }
            SchemaKind::Opaque("uint8Array") => serde_json::json!({
                "type": "string",
                "format": "byte",
                "contentEncoding": "base64"
            }),
            SchemaKind::Opaque("bigint") => serde_json::json!({
                "type": "string",
                "pattern": "^-?\\d+$"
            }),
            // Duration is a tagged union of four closed shapes; the port has
            // no Duration constructor yet, so the compiler spells the fixed
            // document for the opaque kind directly.
            SchemaKind::Opaque("duration") => serde_json::json!({
                "anyOf": [
                    {
                        "type": "object",
                        "properties": {"_tag": {"type": "string", "enum": ["Infinity"]}},
                        "required": ["_tag"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {"_tag": {"type": "string", "enum": ["NegativeInfinity"]}},
                        "required": ["_tag"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "_tag": {"type": "string", "enum": ["Nanos"]},
                            "value": {"type": "string", "pattern": "^-?\\d+$"}
                        },
                        "required": ["_tag", "value"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "_tag": {"type": "string", "enum": ["Millis"]},
                            "value": {"type": "integer"}
                        },
                        "required": ["_tag", "value"],
                        "additionalProperties": false
                    }
                ]
            }),
            SchemaKind::Opaque("symbol") => serde_json::json!({
                "type": "string",
                "pattern": "^Symbol\\(([\\s\\S]*)\\)$"
            }),
            // Anything else opaque (a custom declaration, a suspended shape,
            // a re-typing) degrades to the open document.
            SchemaKind::Opaque(_) => serde_json::json!({}),
            SchemaKind::Array(items) => {
                serde_json::json!({"type": "array", "items": self.doc_of_kind(items)})
            }
            SchemaKind::Optional(inner) => {
                serde_json::json!({"anyOf": [self.doc_of_kind(inner), {"type": "null"}]})
            }
            SchemaKind::OptionalKey(inner) => self.doc_of_kind(inner),
            SchemaKind::Rest(inner) => {
                serde_json::json!({"prefixItems": [self.doc_of_kind(inner)]})
            }
            SchemaKind::Struct(fields) => self.struct_document_kinds(fields, None),
            SchemaKind::StructWithRest(fields, rest) => {
                self.struct_with_rest_document(fields, rest)
            }
            SchemaKind::Record(key, value) => self.record_document(key, value),
            SchemaKind::Tuple(items) => self.tuple_kinds_document(items, None),
            SchemaKind::TupleWithRest(items, rest) => {
                self.tuple_kinds_document(items, Some((**rest).clone()))
            }
            SchemaKind::Literal(value) => literal_document(value),
            SchemaKind::Literals(values) => literals_document(values),
            SchemaKind::Union { members, mode } => union_document(
                members
                    .iter()
                    .map(|member| self.doc_of_kind(member))
                    .collect(),
                mode,
            ),
            SchemaKind::Reference(name) => Self::reference(name),
            // Kind-only positions carry no key annotations; a bare wrapper
            // compiles through, hoisting an identifier it carries.
            SchemaKind::Annotated { pairs, inner } => {
                let mut doc = self.doc_of_kind(inner);
                if let Some((_, id)) = pairs.iter().find(|(key, _)| key == "identifier") {
                    let id = id.clone();
                    self.merge_pairs(&mut doc, pairs);
                    return self.intern(&id, doc);
                }
                self.merge_pairs(&mut doc, pairs);
                doc
            }
            SchemaKind::Check {
                base, name, args, ..
            } => {
                let base_type = base_type_of(base);
                let mut doc = self.doc_of_kind(base);
                self.apply_check(&mut doc, base_type, name, args, &[]);
                doc
            }
        }
    }

    /// An object document for named fields from kinds alone (no key
    /// annotations available on this path).
    fn struct_document_kinds(
        &mut self,
        fields: &[(String, SchemaKind)],
        open: Option<&SchemaKind>,
    ) -> serde_json::Value {
        if fields.is_empty() && open.is_none() {
            return serde_json::json!({"not": {"type": "null"}});
        }
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, kind) in fields {
            properties.insert(name.clone(), self.doc_of_kind(&unwrap_optional_kind(kind)));
            if !is_absent_tolerant_kind(kind) {
                required.push(serde_json::Value::String(name.clone()));
            }
        }
        let mut doc = serde_json::json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
        });
        if !required.is_empty() {
            doc["required"] = serde_json::Value::Array(required);
        }
        match open {
            None => {
                doc["additionalProperties"] = serde_json::Value::Bool(
                    self.options.on_excess_property == OnExcessProperty::Ignore,
                );
            }
            Some(rest) if is_open_record_rest(rest) => {}
            Some(rest) if is_never_kind(rest) => {
                doc["additionalProperties"] = serde_json::Value::Bool(false);
            }
            Some(rest) => {
                doc["additionalProperties"] = self.doc_of_kind(rest);
            }
        }
        doc
    }

    /// A struct plus its catch-all shape: a plain string index becomes an
    /// `allOf` object entry, a conjunctive key becomes `propertyNames` with
    /// the value as `additionalProperties`, and a union of records merges
    /// every member's key and value shapes.
    fn struct_with_rest_document(
        &mut self,
        fields: &[(String, SchemaKind)],
        rest: &SchemaKind,
    ) -> serde_json::Value {
        // An open catch-all is an open struct, which is what the import
        // produces for one and what round-trips back to it.
        if is_open_record_rest(rest) {
            return self.struct_document_kinds(fields, None);
        }
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, kind) in fields {
            properties.insert(name.clone(), self.doc_of_kind(&unwrap_optional_kind(kind)));
            if !is_absent_tolerant_kind(kind) {
                required.push(serde_json::Value::String(name.clone()));
            }
        }
        let mut doc = serde_json::json!({
            "type": "object",
        });
        if !properties.is_empty() {
            doc["properties"] = serde_json::Value::Object(properties);
        }
        if !required.is_empty() {
            doc["required"] = serde_json::Value::Array(required);
        }
        match strip_kind(rest) {
            SchemaKind::Union { members, .. }
                if members
                    .iter()
                    .all(|member| matches!(strip_kind(member), SchemaKind::Record(..))) =>
            {
                self.merge_rest_union(&mut doc, members);
            }
            SchemaKind::Record(key, value) => {
                self.apply_rest_record(&mut doc, key, value);
            }
            _ => {
                doc["additionalProperties"] = self.doc_of_kind(rest);
            }
        }
        doc
    }

    /// Merge one rest record into a struct-with-rest document: a plain string
    /// key is an `allOf` object entry, a single selector pattern is a closed
    /// `patternProperties` entry, and anything else is `propertyNames` with
    /// the value as `additionalProperties` (naming the declared properties
    /// as an enum branch beside the key shape).
    fn apply_rest_record(
        &mut self,
        doc: &mut serde_json::Value,
        key: &SchemaKind,
        value: &SchemaKind,
    ) {
        let value_doc = self.doc_of_kind(value);
        let (base, checks) = split_checks(key);
        match &base {
            SchemaKind::String if checks.is_empty() => {
                let mut entry = serde_json::json!({"type": "object"});
                if !is_open_value_kind(value) {
                    entry["additionalProperties"] = value_doc;
                }
                doc["allOf"] = serde_json::Value::Array(vec![entry]);
            }
            SchemaKind::String if checks.len() == 1 && is_selector_check(&checks[0]) => {
                let pattern = selector_pattern(&checks[0]).unwrap_or_default();
                let mut patterns = serde_json::Map::new();
                patterns.insert(pattern, value_doc);
                doc["allOf"] = serde_json::Value::Array(vec![serde_json::json!({
                    "type": "object",
                    "patternProperties": serde_json::Value::Object(patterns),
                    "additionalProperties": false
                })]);
            }
            _ => {
                let key_doc = self.record_key_document(&base, &checks);
                let mut branches = Vec::new();
                if let Some(names) = doc
                    .get("properties")
                    .and_then(|properties| properties.as_object())
                {
                    let names: Vec<serde_json::Value> = names
                        .keys()
                        .map(|name| serde_json::Value::String(name.clone()))
                        .collect();
                    if !names.is_empty() {
                        branches.push(serde_json::json!({"enum": names}));
                    }
                }
                branches.push(key_doc);
                doc["propertyNames"] = serde_json::json!({"anyOf": branches});
                doc["additionalProperties"] = value_doc;
            }
        }
    }

    /// Merge a union of rest records (several index signatures): every key
    /// shape joins `propertyNames`, every value joins `additionalProperties`,
    /// and the last plain string index also lands as an `allOf` object entry.
    fn merge_rest_union(&mut self, doc: &mut serde_json::Value, members: &[SchemaKind]) {
        let mut key_docs = Vec::new();
        let mut value_docs = Vec::new();
        let mut all_of: Option<serde_json::Value> = None;
        for member in members {
            if let SchemaKind::Record(key, value) = strip_kind(member) {
                let (base, checks) = split_checks(key);
                if matches!(base, SchemaKind::String) && checks.is_empty() {
                    key_docs.push(serde_json::json!({"type": "string"}));
                    let value_doc = self.doc_of_kind(value);
                    value_docs.push(value_doc.clone());
                    let mut entry = serde_json::json!({"type": "object"});
                    if !is_open_value_kind(value) {
                        entry["additionalProperties"] = value_doc;
                    }
                    all_of = Some(entry);
                } else {
                    key_docs.push(self.record_key_document(&base, &checks));
                    value_docs.push(self.doc_of_kind(value));
                }
            }
        }
        doc["propertyNames"] = serde_json::json!({"anyOf": key_docs});
        doc["additionalProperties"] = serde_json::json!({"anyOf": value_docs});
        if let Some(entry) = all_of {
            doc["allOf"] = serde_json::Value::Array(vec![entry]);
        }
    }

    /// A record document: literal keys expand to properties, a single
    /// selector pattern becomes `patternProperties`, and anything else is
    /// `propertyNames` (or a bare `additionalProperties` for a plain string
    /// key). In ignore mode a record that would need `propertyNames`
    /// degrades to an open object.
    fn record_document(&mut self, key: &SchemaKind, value: &SchemaKind) -> serde_json::Value {
        // An exact-value key names its properties.
        if let Some(names) = exact_key_names_kind(key) {
            let value_doc = self.doc_of_kind(value);
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();
            for name in &names {
                properties.insert(name.clone(), value_doc.clone());
                required.push(serde_json::Value::String(name.clone()));
            }
            return serde_json::json!({
                "type": "object",
                "properties": serde_json::Value::Object(properties),
                "required": required,
                "additionalProperties": false
            });
        }
        // A value nothing validates against closes the object, unless it
        // carries annotations of its own, which ride the value document.
        if is_never_kind(value) {
            let value_doc = self.doc_of_kind(value);
            if value_doc == serde_json::json!({"not": {}}) {
                return serde_json::json!({"type": "object", "additionalProperties": false});
            }
            return serde_json::json!({
                "type": "object",
                "additionalProperties": value_doc
            });
        }
        let (base, checks) = split_checks(key);
        match &base {
            SchemaKind::String if checks.is_empty() => {
                if is_open_value_kind(value) {
                    return serde_json::json!({"type": "object"});
                }
                serde_json::json!({
                    "type": "object",
                    "additionalProperties": self.doc_of_kind(value)
                })
            }
            SchemaKind::String if checks.len() == 1 && is_selector_check(&checks[0]) => {
                let pattern = selector_pattern(&checks[0]).unwrap_or_default();
                let mut patterns = serde_json::Map::new();
                patterns.insert(pattern, self.doc_of_kind(value));
                serde_json::json!({
                    "type": "object",
                    "patternProperties": serde_json::Value::Object(patterns),
                    "additionalProperties": false
                })
            }
            SchemaKind::String => {
                if self.options.on_excess_property == OnExcessProperty::Ignore {
                    return serde_json::json!({"type": "object", "additionalProperties": true});
                }
                let key_doc = self.record_key_document(&base, &checks);
                serde_json::json!({
                    "type": "object",
                    "propertyNames": key_doc,
                    "additionalProperties": self.doc_of_kind(value)
                })
            }
            SchemaKind::Opaque("symbol") if checks.is_empty() => {
                let mut patterns = serde_json::Map::new();
                patterns.insert(
                    "^Symbol\\(([\\s\\S]*)\\)$".to_string(),
                    self.doc_of_kind(value),
                );
                serde_json::json!({
                    "type": "object",
                    "patternProperties": serde_json::Value::Object(patterns),
                    "additionalProperties": false
                })
            }
            _ => serde_json::json!({"type": "object"}),
        }
    }

    /// A record key that is not a property selector: the open string shape
    /// plus every representable check keyword, innermost check first. A key
    /// with checks but no representable keyword keeps the open shape, and a
    /// custom filter with nothing to spell degrades to it too.
    fn record_key_document(
        &mut self,
        base: &SchemaKind,
        checks: &[(String, Vec<String>)],
    ) -> serde_json::Value {
        let mut doc = match base {
            SchemaKind::String => serde_json::json!({"type": "string"}),
            _ => self.doc_of_kind(base),
        };
        for (name, args) in checks.iter().rev() {
            self.apply_check(&mut doc, Some("string"), name, args, &[]);
        }
        doc
    }

    /// A tuple document from kinds alone (no key annotations on this path).
    /// No elements is `items: false`.
    fn tuple_kinds_document(
        &mut self,
        items: &[SchemaKind],
        rest: Option<SchemaKind>,
    ) -> serde_json::Value {
        if items.is_empty() && rest.is_none() {
            return serde_json::json!({"type": "array", "items": false});
        }
        let compiled: Vec<serde_json::Value> = items
            .iter()
            .map(|item| self.doc_of_kind(&unwrap_optional_kind(item)))
            .collect();
        let leading = items
            .iter()
            .take_while(|item| !is_absent_tolerant_kind(item))
            .count();
        match rest {
            None => {
                let count = compiled.len();
                let mut doc = serde_json::json!({
                    "type": "array",
                    "prefixItems": compiled,
                    "maxItems": count,
                });
                if leading > 0 {
                    doc["minItems"] =
                        serde_json::Value::Number(serde_json::Number::from(leading as u64));
                }
                doc
            }
            Some(rest) => {
                let items_doc = self.doc_of_kind(&rest);
                let mut doc = serde_json::json!({
                    "type": "array",
                    "prefixItems": compiled,
                    "items": items_doc,
                });
                if leading > 0 {
                    doc["minItems"] =
                        serde_json::Value::Number(serde_json::Number::from(leading as u64));
                }
                doc
            }
        }
    }
}

/// Whether a tuple's stored element schemas line up with its kind: only then
/// do the full schemas (key annotations included) drive the document.
fn tuple_stored(schema: &Schema<Unknown>, len: usize) -> bool {
    schema
        .tuple_elements
        .as_ref()
        .is_some_and(|elements| elements.len() == len)
}

/// Push an `allOf` entry onto a document, extending an existing `allOf`.
fn push_entry(doc: &mut serde_json::Value, entry: serde_json::Map<String, serde_json::Value>) {
    let map = match doc.as_object_mut() {
        Some(map) => map,
        None => return,
    };
    match map.get_mut("allOf") {
        Some(serde_json::Value::Array(entries)) => {
            entries.push(serde_json::Value::Object(entry));
        }
        _ => {
            map.insert(
                "allOf".to_string(),
                serde_json::Value::Array(vec![serde_json::Value::Object(entry)]),
            );
        }
    }
}

/// A flat `allOf` entry: the annotation pairs plus the check keywords.
fn flat_entry(
    pairs: Vec<(String, String)>,
    keywords: Vec<(String, serde_json::Value)>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut entry = serde_json::Map::new();
    for (key, value) in pairs {
        entry.insert(key, serde_json::Value::String(value));
    }
    for (key, value) in keywords {
        entry.insert(key, value);
    }
    entry
}

/// What one named check contributes to its base's document.
enum CheckFragment {
    /// No keyword: the base stands as it is.
    Skip,
    /// Keywords that merge inline, or become a flat `allOf` entry on
    /// collision or annotation.
    Inline(Vec<(String, serde_json::Value)>),
    /// Keywords that merge inline beside complete `allOf` entries, pushed
    /// as they are.
    Entries {
        inline: Vec<(String, serde_json::Value)>,
        entries: Vec<serde_json::Map<String, serde_json::Value>>,
    },
}

/// Whether a check argument list carries an exclusivity flag, which the port
/// keeps beside the bounds in the check's annotation.
fn is_exclusive(annotation: &[(String, String)], key: &str) -> bool {
    annotation
        .iter()
        .any(|(seen, value)| seen == key && value == "true")
}

/// One named check as document keywords. The base type decides which keyword
/// family a shared check name lands in (`isMinLength` is `minLength` on a
/// string, `minItems` on an array).
fn check_fragment(
    base_type: Option<&str>,
    name: &str,
    args: &[String],
    annotation: &[(String, String)],
    exclusive_minimum: bool,
    exclusive_maximum: bool,
) -> CheckFragment {
    match (base_type, name) {
        (Some("string"), "isMinLength") => {
            // A length counts UTF-16 units, so a shorter code-point string
            // can still satisfy it: the loose bound stays one below.
            match number_arg(args) {
                Some(value) => {
                    CheckFragment::Inline(vec![("minLength".to_string(), loose_lower(value))])
                }
                None => CheckFragment::Skip,
            }
        }
        (Some("string"), "isMaxLength") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("maxLength".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isMinCodePoints") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("minLength".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isMaxCodePoints") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("maxLength".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isBetweenLength") => match (number_arg(args), second_arg(args)) {
            (Some(min), Some(max)) => CheckFragment::Inline(vec![
                ("minLength".to_string(), loose_lower(min)),
                ("maxLength".to_string(), max),
            ]),
            _ => CheckFragment::Skip,
        },
        // A code-point range exports as two entries, which is the shape the
        // pin uses for it.
        (Some("string"), "isBetweenCodePoints") => {
            match (number_arg(args), second_arg(args)) {
                (Some(min), Some(max)) => {
                    let mut first = serde_json::Map::new();
                    for (key, value) in annotation {
                        first.insert(key.clone(), serde_json::Value::String(value.clone()));
                    }
                    first.insert("minLength".to_string(), min);
                    CheckFragment::Entries {
                        inline: Vec::new(),
                        entries: vec![
                            first,
                            [("maxLength".to_string(), max)].into_iter().collect(),
                        ],
                    }
                }
                _ => CheckFragment::Skip,
            }
        }
        (Some("string"), "isPattern") => match args
            .first()
            .and_then(|arg| regexp_pattern(arg))
        {
            Some(pattern) => CheckFragment::Inline(vec![(
                "pattern".to_string(),
                serde_json::Value::String(pattern),
            )]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isStartingWith") => match args.first() {
            Some(prefix) => CheckFragment::Inline(vec![(
                "pattern".to_string(),
                serde_json::Value::String(format!("^{}", escape_regex(prefix))),
            )]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isEndingWith") => match args.first() {
            Some(suffix) => CheckFragment::Inline(vec![(
                "pattern".to_string(),
                serde_json::Value::String(format!("{}$", escape_regex(suffix))),
            )]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isIncluding") => match args.first() {
            Some(part) => CheckFragment::Inline(vec![(
                "pattern".to_string(),
                serde_json::Value::String(escape_regex(part)),
            )]),
            None => CheckFragment::Skip,
        },
        (Some("string"), "isTrimmed") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String("^\\S[\\s\\S]*\\S$|^\\S$|^$".to_string()),
        )]),
        (Some("string"), "isLowercased") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String("^[^A-Z]*$".to_string()),
        )]),
        (Some("string"), "isUppercased") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String("^[^a-z]*$".to_string()),
        )]),
        (Some("string"), "isCapitalized") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String("^(?:[^a-z][\\s\\S]*)?$".to_string()),
        )]),
        (Some("string"), "isUncapitalized") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String("^(?:[^A-Z][\\s\\S]*)?$".to_string()),
        )]),
        (Some("string"), "isUUID") => CheckFragment::Inline(vec![
            (
                "format".to_string(),
                serde_json::Value::String("uuid".to_string()),
            ),
            (
                "pattern".to_string(),
                serde_json::Value::String(
                    "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|[fF]{8}-[fF]{4}-[fF]{4}-[fF]{4}-[fF]{12})$"
                        .to_string(),
                ),
            ),
        ]),
        (Some("string"), "isGUID") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String(
                "^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$"
                    .to_string(),
            ),
        )]),
        (Some("string"), "isBase64") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String(
                "^([0-9a-zA-Z+/]{4})*(([0-9a-zA-Z+/]{2}==)|([0-9a-zA-Z+/]{3}=))?$".to_string(),
            ),
        )]),
        (Some("string"), "isBase64Url") => CheckFragment::Inline(vec![(
            "pattern".to_string(),
            serde_json::Value::String(
                "^([0-9a-zA-Z-_]{4})*(([0-9a-zA-Z-_]{2}(==)?)|([0-9a-zA-Z-_]{3}(=)?))?$"
                    .to_string(),
            ),
        )]),
        (Some("number"), "isInt") => CheckFragment::Inline(vec![(
            "type".to_string(),
            serde_json::Value::String("integer".to_string()),
        )]),
        // A 32-bit range always rides an `allOf` entry: flat when bare,
        // nested under its own `allOf` when the check carries annotations.
        (Some("number"), "isInt32") => CheckFragment::Entries {
            inline: vec![(
                "type".to_string(),
                serde_json::Value::String("integer".to_string()),
            )],
            entries: vec![int32_entry(
                annotation,
                serde_json::json!(2147483647),
                serde_json::json!(-2147483648),
            )],
        },
        (Some("number"), "isUint32") => CheckFragment::Entries {
            inline: vec![(
                "type".to_string(),
                serde_json::Value::String("integer".to_string()),
            )],
            entries: vec![int32_entry(
                annotation,
                serde_json::json!(4294967295u32),
                serde_json::json!(0),
            )],
        },
        (Some("number"), "isGreaterThanOrEqualTo") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("minimum".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("number"), "isLessThanOrEqualTo") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("maximum".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("number"), "isGreaterThan") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("exclusiveMinimum".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("number"), "isLessThan") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("exclusiveMaximum".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("number"), "isBetween") => match (number_arg(args), second_arg(args)) {
            (Some(min), Some(max)) => {
                let mut keywords = Vec::new();
                keywords.push((
                    if exclusive_minimum {
                        "exclusiveMinimum"
                    } else {
                        "minimum"
                    }
                    .to_string(),
                    min,
                ));
                keywords.push((
                    if exclusive_maximum {
                        "exclusiveMaximum"
                    } else {
                        "maximum"
                    }
                    .to_string(),
                    max,
                ));
                CheckFragment::Inline(keywords)
            }
            _ => CheckFragment::Skip,
        },
        (Some("number"), "isMultipleOf") => match args
            .first()
            .and_then(|arg| arg.parse::<f64>().ok())
            .filter(|divisor| divisor.is_finite())
            .map(|divisor| num_value(divisor.abs()))
        {
            Some(value) => CheckFragment::Inline(vec![("multipleOf".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("array"), "isMinLength") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("minItems".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("array"), "isMaxLength") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("maxItems".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("array"), "isBetweenLength") => match (number_arg(args), second_arg(args)) {
            (Some(min), Some(max)) => CheckFragment::Inline(vec![
                ("minItems".to_string(), min),
                ("maxItems".to_string(), max),
            ]),
            _ => CheckFragment::Skip,
        },
        (Some("array"), "isUnique") => {
            CheckFragment::Inline(vec![("uniqueItems".to_string(), serde_json::Value::Bool(true))])
        }
        (Some("object"), "isMinProperties") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("minProperties".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("object"), "isMaxProperties") => match number_arg(args) {
            Some(value) => CheckFragment::Inline(vec![("maxProperties".to_string(), value)]),
            None => CheckFragment::Skip,
        },
        (Some("object"), "isBetweenProperties") => {
            match (number_arg(args), second_arg(args)) {
                (Some(min), Some(max)) => CheckFragment::Inline(vec![
                    ("minProperties".to_string(), min),
                    ("maxProperties".to_string(), max),
                ]),
                _ => CheckFragment::Skip,
            }
        }
        // `isFinite` is implied by `type: number`; exclusivity-annotated
        // ranges without bounds, and unknown filters, have no keyword and
        // degrade to the base.
        _ => CheckFragment::Skip,
    }
}

/// An `allOf` entry for a 32-bit range check: the bounds flat when the check
/// is bare, nested under their own `allOf` beside the check's annotations.
fn int32_entry(
    annotation: &[(String, String)],
    maximum: serde_json::Value,
    minimum: serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    let mut entry = serde_json::Map::new();
    for (key, value) in annotation {
        entry.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    if entry.is_empty() {
        entry.insert("maximum".to_string(), maximum);
        entry.insert("minimum".to_string(), minimum);
    } else {
        entry.insert(
            "allOf".to_string(),
            serde_json::Value::Array(vec![serde_json::json!({
                "maximum": maximum,
                "minimum": minimum
            })]),
        );
    }
    entry
}

/// The loose lower bound for a UTF-16 length check: one below, floored at
/// zero, keeping integer form.
fn loose_lower(value: serde_json::Value) -> serde_json::Value {
    let Some(number) = value.as_f64() else {
        return value;
    };
    num_value((number - 1.0).max(0.0))
}

/// A whole number keeps its integer form so a document round-trips textually
/// (`2`, not `2.0`); anything else goes through the float conversion.
fn num_value(number: f64) -> serde_json::Value {
    if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
        return serde_json::Value::Number(serde_json::Number::from(number as i64));
    }
    serde_json::Number::from_f64(number)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

/// A check argument printed back to JSON. Whole values keep their integer
/// form; non-finite bounds have no keyword and refuse the check.
fn number_arg(args: &[String]) -> Option<serde_json::Value> {
    let parsed: f64 = args.first()?.parse().ok()?;
    if !parsed.is_finite() {
        return None;
    }
    Some(num_value(parsed))
}

/// The second bound of a binary range check, without panicking on a short
/// argument list.
fn second_arg(args: &[String]) -> Option<serde_json::Value> {
    number_arg(args.get(1..).unwrap_or(&[]))
}

/// A `new RegExp("source", "flags")` check argument back to an emit pattern.
/// Only the Unicode flag (plus the inert `g`/`y`) is representable: anything
/// else degrades to no pattern. An unanchored source is anchored, which is
/// the shape the pin exports for it.
fn regexp_pattern(arg: &str) -> Option<String> {
    let inner = arg.strip_prefix("new RegExp(")?.strip_suffix(')')?;
    // Split on the last comma: flags never contain one, sources may. The
    // source stays verbatim (escapes intact), which is what the pin exports.
    let (source_raw, flags_raw) = inner.rsplit_once(',')?;
    let source = strip_quotes(source_raw.trim())?;
    let flags = strip_quotes(flags_raw.trim())?;
    if !flags.contains('u') || !flags.chars().all(|flag| matches!(flag, 'g' | 'u' | 'y')) {
        return None;
    }
    if source.is_empty() {
        return None;
    }
    if source.starts_with('^') {
        Some(source)
    } else {
        Some(format!("^(?:{source})"))
    }
}

/// One surrounding pair of double quotes off, nothing unescaped.
fn strip_quotes(text: &str) -> Option<String> {
    text.strip_prefix('"')?
        .strip_suffix('"')
        .map(str::to_string)
}

/// Escape literal text for a regexp: every syntax character takes a
/// backslash, which is what `isStartingWith`/`isEndingWith`/`isIncluding`
/// export.
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for char in text.chars() {
        if matches!(
            char,
            '\\' | '^' | '$' | '.' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}'
        ) {
            out.push('\\');
        }
        out.push(char);
    }
    out
}

/// The JSON Schema type a kind refines, deciding which keyword family a
/// shared check name lands in. Tuples refine arrays and struct-with-rests
/// refine objects, which the core helper does not name.
fn base_type_of(kind: &SchemaKind) -> Option<&'static str> {
    kind_type_name(strip_kind(kind)).or_else(|| match strip_kind(kind) {
        SchemaKind::Tuple(_) | SchemaKind::TupleWithRest(..) => Some("array"),
        SchemaKind::StructWithRest(..) => Some("object"),
        _ => None,
    })
}
/// Whether a kind is a wrapper the required computation looks through: an
/// absent-tolerant position, whether spelled `optional` or `optionalKey`.
fn is_absent_tolerant_kind(kind: &SchemaKind) -> bool {
    matches!(
        strip_kind(kind),
        SchemaKind::Optional(_) | SchemaKind::OptionalKey(_)
    )
}

/// Walk through annotation and check wrappers to the shaped base.
fn strip_kind(kind: &SchemaKind) -> &SchemaKind {
    let mut current = kind;
    loop {
        match current {
            SchemaKind::Annotated { inner, .. } => current = inner,
            SchemaKind::Check { base, .. } => current = base,
            _ => return current,
        }
    }
}

/// Unwrap one absent-tolerant layer for element compilation.
fn unwrap_optional_kind(kind: &SchemaKind) -> SchemaKind {
    match strip_kind(kind) {
        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => (**inner).clone(),
        _ => kind.clone(),
    }
}

/// Whether a record value shape is the open-JSON catch-all, which an object
/// document leaves implicit rather than spelling out.
fn is_open_value_kind(kind: &SchemaKind) -> bool {
    matches!(strip_kind(kind), SchemaKind::Json)
}

/// Whether a kind is the shape nothing validates against.
fn is_never_kind(kind: &SchemaKind) -> bool {
    matches!(strip_kind(kind), SchemaKind::Opaque("never"))
}

/// Whether a struct catch-all is the open-JSON shape the import produces for
/// an object with no `additionalProperties` keyword.
fn is_open_record_rest(kind: &SchemaKind) -> bool {
    match strip_kind(kind) {
        SchemaKind::Record(key, value) => {
            matches!(strip_kind(key), SchemaKind::String) && is_open_value_kind(value)
        }
        _ => false,
    }
}

/// A record key split into its base shape plus the checks layered on it,
/// outermost first.
fn split_checks(kind: &SchemaKind) -> (SchemaKind, Vec<(String, Vec<String>)>) {
    let mut checks = Vec::new();
    let mut current = kind;
    loop {
        match current {
            SchemaKind::Check {
                base, name, args, ..
            } => {
                checks.push((name.clone(), args.clone()));
                current = base;
            }
            SchemaKind::Annotated { inner, .. } => {
                current = inner;
            }
            _ => return (current.clone(), checks),
        }
    }
}

/// Whether one record-key check is a property selector on its own: a
/// representable pattern or an anchored literal affix. Conjunctive keys and
/// case-folding checks go through `propertyNames` instead.
fn is_selector_check(check: &(String, Vec<String>)) -> bool {
    match check.0.as_str() {
        "isPattern" => check
            .1
            .first()
            .is_some_and(|arg| regexp_pattern(arg).is_some()),
        "isStartingWith" | "isEndingWith" => true,
        _ => false,
    }
}

/// The `patternProperties` key for a selector check: the pattern it exports.
fn selector_pattern(check: &(String, Vec<String>)) -> Option<String> {
    match check.0.as_str() {
        "isPattern" => check.1.first().and_then(|arg| regexp_pattern(arg)),
        "isStartingWith" => check
            .1
            .first()
            .map(|prefix| format!("^{}", escape_regex(prefix))),
        "isEndingWith" => check
            .1
            .first()
            .map(|suffix| format!("{}$", escape_regex(suffix))),
        _ => None,
    }
}

/// The key names an exact-value key schema admits (`Record(Literals([...]))`),
/// deduped and in declaration order.
fn exact_key_names_kind(kind: &SchemaKind) -> Option<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for value in exact_values_kind(kind)? {
        if let Unknown::Text(name) = value
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    Some(names)
}

/// The exact values a kind admits, when it finitely admits any. Anything else
/// admits none.
fn exact_values_kind(kind: &SchemaKind) -> Option<Vec<Unknown>> {
    match strip_kind(kind) {
        SchemaKind::Literal(value) => Some(vec![value.clone()]),
        SchemaKind::Literals(values) => Some(values.clone()),
        _ => None,
    }
}

/// One exact value as a typed enum document: strings, numbers and booleans
/// carry their type beside the value, which is the shape the pin exports for
/// literals. Anything else degrades to `const`.
fn literal_document(value: &Unknown) -> serde_json::Value {
    match value {
        Unknown::Text(_) => serde_json::json!({"type": "string", "enum": [value.to_json()]}),
        Unknown::Bool(_) => serde_json::json!({"type": "boolean", "enum": [value.to_json()]}),
        Unknown::Number(number) => {
            serde_json::json!({"type": "number", "enum": [num_value(*number)]})
        }
        _ => serde_json::json!({"const": value.to_json()}),
    }
}

/// Several exact values: empty is the shape nothing validates against, one
/// type is a single typed enum, and mixed types split into an `anyOf`.
fn literals_document(values: &[Unknown]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::json!({"not": {}});
    }
    let mut strings = Vec::new();
    let mut numbers = Vec::new();
    let mut booleans = Vec::new();
    let mut rest = Vec::new();
    for value in values {
        match value {
            Unknown::Text(_) => strings.push(value.to_json()),
            Unknown::Number(number) => numbers.push(num_value(*number)),
            Unknown::Bool(_) => booleans.push(value.to_json()),
            _ => rest.push(value.to_json()),
        }
    }
    let mut groups = Vec::new();
    if !strings.is_empty() {
        groups.push(serde_json::json!({"type": "string", "enum": strings}));
    }
    if !numbers.is_empty() {
        groups.push(serde_json::json!({"type": "number", "enum": numbers}));
    }
    if !booleans.is_empty() {
        groups.push(serde_json::json!({"type": "boolean", "enum": booleans}));
    }
    for value in rest {
        groups.push(serde_json::json!({"const": value}));
    }
    if groups.len() == 1 {
        return groups.into_iter().next().expect("one group");
    }
    serde_json::json!({"anyOf": groups})
}

/// A union document: members that are all plain enums of one type with
/// disjoint values compact into a single typed enum (duplicates stay split,
/// which is why the disjointness matters); `oneOf` survives only when every
/// branch pair is exactly disjoint, otherwise it loosens to `anyOf`.
fn union_document(members: Vec<serde_json::Value>, mode: &str) -> serde_json::Value {
    if !members.is_empty()
        && members.iter().all(|member| {
            member.as_object().is_some_and(|map| {
                map.len() == 2 && map.contains_key("type") && map.contains_key("enum")
            })
        })
    {
        let first_type = members[0].get("type").cloned();
        let mut merged: Vec<serde_json::Value> = Vec::new();
        let mut disjoint = true;
        for member in &members {
            if member.get("type") != first_type.as_ref() {
                disjoint = false;
                break;
            }
            let values: Vec<serde_json::Value> = member
                .get("enum")
                .and_then(|values| values.as_array())
                .cloned()
                .unwrap_or_default();
            if values.iter().any(|value| merged.contains(value)) {
                disjoint = false;
                break;
            }
            merged.extend(values);
        }
        if disjoint {
            return serde_json::json!({"type": first_type, "enum": merged});
        }
    }
    let key = match mode {
        "oneOf" if members_exact(&members) => "oneOf",
        "oneOf" => "anyOf",
        "allOf" => "allOf",
        _ => "anyOf",
    };
    serde_json::json!({key: members})
}

/// Whether every union branch pair is exactly disjoint: different types, or
/// different consts, or disjoint enums. Anything looser overlaps, and an
/// approximated `oneOf` over overlapping branches loosens to `anyOf`.
fn members_exact(members: &[serde_json::Value]) -> bool {
    for (index, first) in members.iter().enumerate() {
        for second in &members[index + 1..] {
            if !pair_exact(first, second) {
                return false;
            }
        }
    }
    true
}

/// Whether two branches are exactly disjoint.
fn pair_exact(first: &serde_json::Value, second: &serde_json::Value) -> bool {
    let first_type = first.get("type");
    let second_type = second.get("type");
    if first_type.is_some() && second_type.is_some() && first_type != second_type {
        return true;
    }
    if let (Some(a), Some(b)) = (first.get("const"), second.get("const")) {
        return a != b;
    }
    if let (Some(a), Some(b)) = (first.get("enum"), second.get("enum")) {
        let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else {
            return false;
        };
        return a.iter().all(|value| !b.contains(value));
    }
    false
}

#[cfg(test)]
mod declaration_tests {
    use super::*;

    /// A schema for a hand-built kind: the declarations the port has no
    /// constructor for still compile through their opaque-kind mappings.
    fn kind_schema(kind: SchemaKind) -> Schema<Unknown> {
        Schema::new(Ok, |value| Ok(value.clone()), kind)
    }

    #[test]
    fn opaque_declarations_compile_to_their_fixed_documents() {
        for (name, expected) in [
            ("date", serde_json::json!({"type": "string"})),
            ("url", serde_json::json!({"type": "string"})),
            (
                "uint8Array",
                serde_json::json!({
                    "type": "string",
                    "format": "byte",
                    "contentEncoding": "base64"
                }),
            ),
            (
                "bigint",
                serde_json::json!({"type": "string", "pattern": "^-?\\d+$"}),
            ),
            (
                "symbol",
                serde_json::json!({
                    "type": "string",
                    "pattern": "^Symbol\\(([\\s\\S]*)\\)$"
                }),
            ),
            ("undefined", serde_json::json!({"type": "null"})),
            ("opaque Declaration", serde_json::json!({})),
        ] {
            assert_eq!(
                compile(&kind_schema(SchemaKind::Opaque(name))),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn duration_compiles_to_its_tagged_branches() {
        // Row 588: the port has no Duration constructor, so the opaque kind
        // spells the pin's fixed four-branch document.
        let schema = kind_schema(SchemaKind::Opaque("duration"));
        assert_eq!(
            compile(&schema),
            serde_json::json!({
                "anyOf": [
                    {
                        "type": "object",
                        "properties": {
                            "_tag": {"type": "string", "enum": ["Infinity"]}
                        },
                        "required": ["_tag"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "_tag": {"type": "string", "enum": ["NegativeInfinity"]}
                        },
                        "required": ["_tag"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "_tag": {"type": "string", "enum": ["Nanos"]},
                            "value": {"type": "string", "pattern": "^-?\\d+$"}
                        },
                        "required": ["_tag", "value"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": {
                            "_tag": {"type": "string", "enum": ["Millis"]},
                            "value": {"type": "integer"}
                        },
                        "required": ["_tag", "value"],
                        "additionalProperties": false
                    }
                ]
            })
        );
    }

    #[test]
    fn symbol_record_keys_become_a_pattern() {        let schema = kind_schema(SchemaKind::Record(
            Box::new(SchemaKind::Opaque("symbol")),
            Box::new(SchemaKind::Number),
        ));
        assert_eq!(
            compile(&schema),
            serde_json::json!({
                "type": "object",
                "patternProperties": {
                    "^Symbol\\(([\\s\\S]*)\\)$": {"type": "number"}
                },
                "additionalProperties": false
            })
        );
    }
}

// ---------- Representation documents (`toJsonSchemaDocument`) ----------
//
// Compiles a serializable representation `Document` (the pin's
// `SchemaRepresentation.toJsonSchemaDocument`) to a JSON Schema document with
// the `draft-2020-12` dialect and a top-level `definitions` map. This is the
// document-level twin of the `Schema`-level compiler above: it reads `Value`
// nodes instead of `SchemaKind`, so shapes the kind cannot see (template
// literals, enums, unique symbols, declarations, check callbacks) compile
// here.
//
// A `Filter` (or `FilterGroup`) carrying a `toJsonSchema` annotation names a
// callback in `CompileDocumentOptions::callbacks`: the annotation value is the
// callback id, and the callback answers the fragment plus whether it is a
// safe, looser approximation. A bare `Filter` with no callback and no known
// builtin identity is approximate and contributes nothing. The two builtins
// the carried rows need (`isInt`, `isPattern`) compile without a callback,
// from their representation payloads. Approximation loosens a `oneOf` union to
// `anyOf` and disqualifies record-key patterns; everywhere else it is
// invisible.
//
// Out of scope, with the rows that need them excused: `referencePolicy`
// naming, fallback-definition aliasing for equal schemas, encoded-side
// projections, and the `fromJsonSchemaDocument` import round trip.

/// The input a `toJsonSchema` callback sees: the current `type` keyword when
/// the base compiled to exactly one, plus the compiled
/// `representation.schemas` dependencies.
pub struct DocCallbackInput {
    pub base_type: Option<String>,
    pub schemas: Vec<serde_json::Value>,
}

/// A callback's answer: the fragment plus whether it is a safe, looser
/// approximation of the check.
pub struct DocCallbackOutput {
    pub fragment: serde_json::Value,
    pub approximate: bool,
}

impl DocCallbackOutput {
    /// An exact fragment.
    pub fn exact(fragment: serde_json::Value) -> Self {
        Self {
            fragment,
            approximate: false,
        }
    }

    /// A safe, looser fragment: it accepts everything the check accepts.
    pub fn approximate(fragment: serde_json::Value) -> Self {
        Self {
            fragment,
            approximate: true,
        }
    }
}

/// A `toJsonSchema` check callback: exact or approximate fragment for one
/// check, or an error message which fails the compilation at the check.
pub type DocCallback = Box<dyn Fn(&DocCallbackInput) -> Result<DocCallbackOutput, String>>;

/// Document-level knobs for [`compile_document_with_options`].
pub struct CompileDocumentOptions<'a> {
    /// Closed (`Error`) or open (`Ignore`) objects and index signatures.
    pub on_excess_property: OnExcessProperty,
    /// Custom annotation keys ride the document only when this accepts them.
    /// Standard keys always ride.
    pub include_annotation_key: Option<fn(&str) -> bool>,
    /// Check callbacks by the id a `toJsonSchema` annotation names.
    pub callbacks: &'a BTreeMap<String, DocCallback>,
}

/// Compile one representation document with default options: open objects,
/// no custom annotation keys, no callbacks.
pub fn compile_document(document: &Document) -> Result<serde_json::Value, SchemaError> {
    let empty: BTreeMap<String, DocCallback> = BTreeMap::new();
    compile_document_with_options(
        document,
        &CompileDocumentOptions {
            on_excess_property: OnExcessProperty::Ignore,
            include_annotation_key: None,
            callbacks: &empty,
        },
    )
}

/// [`compile_document`] with explicit [`CompileDocumentOptions`].
pub fn compile_document_with_options(
    document: &Document,
    options: &CompileDocumentOptions,
) -> Result<serde_json::Value, SchemaError> {
    let mut compiler = DocCompiler {
        options,
        references: &document.references,
        definitions: BTreeMap::new(),
        def_nodes: BTreeMap::new(),
        in_progress: Vec::new(),
        exactness: Vec::new(),
    };
    for key in document.references.keys() {
        compiler.compile_definition(key, &format!("[\"references\"][\"{key}\"]"))?;
    }
    let root = compiler.fresh_deps();
    let schema = compiler.recur(&document.representation, "[\"representation\"]", root)?;
    let mut definitions = serde_json::Map::new();
    for key in document.references.keys() {
        if let Some((compiled, _)) = compiler.definitions.get(key) {
            definitions.insert(key.clone(), compiled.clone());
        }
    }
    Ok(serde_json::json!({
        "dialect": "draft-2020-12",
        "schema": schema,
        "definitions": serde_json::Value::Object(definitions),
    }))
}

/// Exactness answers whether a compiled fragment represents its check
/// exactly. A `oneOf` union with an approximate branch loosens to `anyOf`; a
/// cycle is exact unless it can reach `false`.
#[derive(Debug, Clone)]
enum ExactNode {
    Exact(bool),
    Deps(Vec<usize>),
}

/// The document compiler state: options, the shared references, compiled
/// definitions plus their exactness nodes, and the exactness graph.
struct DocCompiler<'a> {
    options: &'a CompileDocumentOptions<'a>,
    references: &'a BTreeMap<String, serde_json::Value>,
    definitions: BTreeMap<String, (serde_json::Value, usize)>,
    def_nodes: BTreeMap<String, usize>,
    in_progress: Vec<String>,
    exactness: Vec<ExactNode>,
}

fn doc_error(path: impl Into<String>, message: impl Into<String>) -> SchemaError {
    SchemaError::InvalidJson {
        path: path.into(),
        message: message.into(),
    }
}

/// One named segment below a document path: `["representation"]` below root.
fn doc_seg(path: &str, key: &str) -> String {
    format!("{path}[\"{key}\"]")
}

/// One indexed segment below a document path: `[0]` below its array.
fn doc_idx(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

/// The JSON Schema types a check callback can observe as the base type.
fn is_json_schema_type(name: &str) -> bool {
    matches!(
        name,
        "string" | "number" | "boolean" | "array" | "object" | "null" | "integer"
    )
}

/// Check keywords a check fragment may carry inline: applicators and
/// dependent keywords change meaning when moved across schema objects, so
/// they stay out.
const INLINEABLE_CHECK_KEYWORDS: &str = "|type|format|pattern|multipleOf|minimum|maximum|\
     exclusiveMinimum|exclusiveMaximum|minLength|maxLength|minItems|maxItems|uniqueItems|\
     minProperties|maxProperties|propertyNames|contentEncoding|contentMediaType|";

/// Pure annotation keywords: everything else a fragment carries goes to
/// `allOf`. `format` and `content*` can affect validation, so they are not
/// pure.
const PROMOTABLE_ANNOTATION_KEYWORDS: &str =
    "|title|description|default|examples|readOnly|writeOnly|";

/// Annotation keys the compiler itself owns: never emitted as annotations.
fn is_doc_directive_key(key: &str) -> bool {
    matches!(
        key,
        "~sentinels"
            | "~structural"
            | "representation"
            | "arbitraryConstraint"
            | "toJsonSchema"
            | "toCode"
            | "toEquivalence"
            | "toFormatter"
            | "toCodec"
            | "toCodecJson"
            | "toCodecStringTree"
            | "toCodecIso"
            | "toCodecArbitrary"
            | "~identifier"
            | "title"
            | "description"
            | "default"
            | "examples"
            | "readOnly"
            | "writeOnly"
            | "format"
            | "contentEncoding"
            | "contentMediaType"
            | "contentSchema"
    )
}

/// Whether every key of a fragment is in the allowed pipe-delimited set.
fn has_only_keywords(fragment: &serde_json::Value, allowed: &str) -> bool {
    fragment
        .as_object()
        .is_some_and(|map| map.keys().all(|key| allowed.contains(&format!("|{key}|"))))
}

/// Whether merging `right_keys` into `left` collides: `$ref` never merges,
/// and neither does a key `left` already spells differently.
fn has_no_collisions(left: &serde_json::Value, right_keys: &[String]) -> bool {
    left.get("$ref")
        .and_then(serde_json::Value::as_str)
        .is_none()
        && right_keys.iter().all(|key| left.get(key).is_none())
}

/// Bounds that tighten to the larger value when repeated on one object.
fn is_lower_bound_keyword(key: &str) -> bool {
    matches!(
        key,
        "minimum" | "exclusiveMinimum" | "minLength" | "minItems" | "minProperties"
    )
}

/// Bounds that tighten to the smaller value when repeated on one object.
fn is_upper_bound_keyword(key: &str) -> bool {
    matches!(
        key,
        "maximum" | "exclusiveMaximum" | "maxLength" | "maxItems" | "maxProperties"
    )
}

impl<'a> DocCompiler<'a> {
    /// A fresh exactness dependency cell.
    fn fresh_deps(&mut self) -> usize {
        let id = self.exactness.len();
        self.exactness.push(ExactNode::Deps(Vec::new()));
        id
    }

    /// Mark the enclosing compilation approximate.
    fn mark_approximate(&mut self, deps: usize) {
        let id = self.exactness.len();
        self.exactness.push(ExactNode::Exact(false));
        if let Some(ExactNode::Deps(members)) = self.exactness.get_mut(deps) {
            members.push(id);
        }
    }

    /// Whether an exactness node is exact. A cycle holds unless it reaches
    /// `false`.
    fn is_exact(&self, id: usize, seen: &mut Vec<usize>) -> bool {
        if seen.contains(&id) {
            return true;
        }
        seen.push(id);
        match self.exactness.get(id) {
            Some(ExactNode::Exact(exact)) => *exact,
            Some(ExactNode::Deps(members)) => {
                let members = members.clone();
                members.iter().all(|member| self.is_exact(*member, seen))
            }
            None => true,
        }
    }

    /// A `$ref` for a definition name: JSON Pointer escaping, then URI
    /// fragment encoding (`~` to `~0`, `/` to `~1`, then `encodeURI`).
    fn definition_reference(name: &str) -> serde_json::Value {
        serde_json::json!({ "$ref": format!("#/$defs/{}", encode_uri_token(&name.replace('~', "~0").replace('/', "~1"))) })
    }

    /// Compile one named definition, registering its exactness cell first so
    /// recursive references share it.
    fn compile_definition(
        &mut self,
        key: &str,
        path: &str,
    ) -> Result<(serde_json::Value, usize), SchemaError> {
        if let Some(entry) = self.definitions.get(key) {
            return Ok(entry.clone());
        }
        if let Some(node) = self.def_nodes.get(key) {
            return Ok((Self::definition_reference(key), *node));
        }
        let body =
            self.references.get(key).cloned().ok_or_else(|| {
                doc_error(doc_seg(path, "$ref"), format!("Invalid reference {key}"))
            })?;
        let node = self.fresh_deps();
        self.def_nodes.insert(key.to_string(), node);
        self.in_progress.push(key.to_string());
        let compiled = self.recur(&body, path, node);
        self.in_progress.pop();
        let compiled = compiled?;
        self.definitions
            .insert(key.to_string(), (compiled.clone(), node));
        Ok((compiled, node))
    }

    /// Compile one representation node plus its checks and annotations.
    fn recur(
        &mut self,
        doc: &serde_json::Value,
        path: &str,
        deps: usize,
    ) -> Result<serde_json::Value, SchemaError> {
        let tag = doc
            .get("_tag")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| doc_error(path, "a representation carries a string `_tag`"))?;
        if tag == "Reference" {
            let key = doc
                .get("$ref")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| doc_error(path, "a `Reference` carries a `$ref`"))?;
            let (_, node) = self.compile_definition(key, path)?;
            if let Some(ExactNode::Deps(members)) = self.exactness.get_mut(deps) {
                members.push(node);
            }
            return Ok(Self::definition_reference(key));
        }
        let local = self.fresh_deps();
        if let Some(ExactNode::Deps(members)) = self.exactness.get_mut(deps) {
            members.push(local);
        }
        let mut output = self.compile_node(doc, tag, path, local)?;
        if let Some(ordinary) = self.collect_annotations(doc.get("annotations")) {
            let map = output
                .as_object_mut()
                .ok_or_else(|| doc_error(path, "a representation compiles to an object"))?;
            for (key, value) in ordinary {
                map.insert(key, value);
            }
        }
        let checks: Vec<serde_json::Value> = doc
            .get("checks")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, check) in checks.iter().enumerate() {
            let check_path = doc_idx(&doc_seg(path, "checks"), index);
            let base_type = output
                .get("type")
                .and_then(serde_json::Value::as_str)
                .filter(|name| is_json_schema_type(name))
                .map(str::to_string);
            if let Some((fragment, inline)) =
                self.compile_check(check, base_type, &check_path, local)?
            {
                output = append_doc_schema(output, fragment, inline);
            }
        }
        Ok(output)
    }

    /// The standard annotation keys a node or element carries: strings stay
    /// strings, `default`/`examples`/`contentSchema` ride as JSON, and custom
    /// keys ride only when the predicate accepts them. Directive keys
    /// (`toJsonSchema`, `representation`, `~identifier`, ...) never ride.
    fn collect_annotations(
        &self,
        annotations: Option<&serde_json::Value>,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let map = annotations?.as_object()?;
        let mut out = serde_json::Map::new();
        for (key, value) in map {
            let keep = match key.as_str() {
                "title" | "description" | "format" | "contentEncoding" | "contentMediaType" => {
                    value.is_string()
                }
                "default" | "contentSchema" => true,
                "examples" => value.is_array(),
                "readOnly" | "writeOnly" => value.is_boolean(),
                _ => {
                    !is_doc_directive_key(key)
                        && self
                            .options
                            .include_annotation_key
                            .is_some_and(|predicate| predicate(key))
                }
            };
            if keep {
                out.insert(key.clone(), value.clone());
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// One check to its fragment: a named callback first, a known builtin
    /// identity next, a bare `Filter` as approximate with no fragment, and a
    /// callback-free `FilterGroup` as the `allOf` of its children.
    fn compile_check(
        &mut self,
        check: &serde_json::Value,
        base_type: Option<String>,
        path: &str,
        deps: usize,
    ) -> Result<Option<(serde_json::Value, bool)>, SchemaError> {
        let tag = check
            .get("_tag")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| doc_error(path, "a check carries a string `_tag`"))?;
        let annotations = check
            .get("annotations")
            .and_then(serde_json::Value::as_object);
        let callback_id = annotations
            .and_then(|map| map.get("toJsonSchema"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if let Some(id) = callback_id {
            let callback = self
                .options
                .callbacks
                .get(&id)
                .ok_or_else(|| doc_error(path, format!("Unknown JSON Schema callback {id}")))?;
            let schemas = check
                .get("representation")
                .and_then(|identity| identity.get("schemas"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut compiled = Vec::with_capacity(schemas.len());
            for (index, schema) in schemas.iter().enumerate() {
                compiled.push(self.recur(
                    schema,
                    &doc_idx(&doc_seg(&doc_seg(path, "representation"), "schemas"), index),
                    deps,
                )?);
            }
            let output = callback(&DocCallbackInput {
                base_type,
                schemas: compiled,
            })
            .map_err(|message| doc_error(path, message))?;
            if output.approximate {
                self.mark_approximate(deps);
            }
            return Ok(Some(self.finish_callback_fragment(
                check,
                &output.fragment,
                tag,
            )));
        }
        if tag == "Filter"
            && let Some((fragment, approximate)) = self.builtin_fragment(check)
        {
            if approximate {
                self.mark_approximate(deps);
            }
            return Ok(Some(self.finish_callback_fragment(check, &fragment, tag)));
        }
        if tag == "Filter" {
            self.mark_approximate(deps);
            return Ok(None);
        }
        let children = check
            .get("checks")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| doc_error(path, "a `FilterGroup` carries `checks`"))?;
        let mut schemas = Vec::new();
        for (index, child) in children.iter().enumerate() {
            let child_path = doc_idx(&doc_seg(path, "checks"), index);
            if let Some((fragment, _)) =
                self.compile_check(child, base_type.clone(), &child_path, deps)?
            {
                schemas.push(fragment);
            }
        }
        if schemas.is_empty() {
            return Ok(None);
        }
        let mut group = serde_json::Map::new();
        group.insert("allOf".to_string(), serde_json::Value::Array(schemas));
        if let Some(ordinary) = self.collect_annotations(check.get("annotations")) {
            for (key, value) in ordinary {
                group.insert(key, value);
            }
        }
        Ok(Some((serde_json::Value::Object(group), false)))
    }

    /// A callback fragment plus the check's own ordinary annotations, and
    /// whether it merges inline: only annotation-free or purely-annotated
    /// `Filter` fragments whose keywords are all inlineable do.
    fn finish_callback_fragment(
        &self,
        check: &serde_json::Value,
        fragment: &serde_json::Value,
        tag: &str,
    ) -> (serde_json::Value, bool) {
        let ordinary = self.collect_annotations(check.get("annotations"));
        let mut schema = fragment.clone();
        if let Some(pairs) = &ordinary
            && let Some(map) = schema.as_object_mut()
        {
            for (key, value) in pairs {
                map.insert(key.clone(), value.clone());
            }
        }
        let allowed = if ordinary.is_some() {
            format!("{INLINEABLE_CHECK_KEYWORDS}{PROMOTABLE_ANNOTATION_KEYWORDS}")
        } else {
            INLINEABLE_CHECK_KEYWORDS.to_string()
        };
        let inline = tag == "Filter"
            && has_only_keywords(&schema, &allowed)
            && ordinary.as_ref().is_none_or(|pairs| {
                pairs
                    .keys()
                    .all(|key| PROMOTABLE_ANNOTATION_KEYWORDS.contains(&format!("|{key}|")))
            });
        (schema, inline)
    }

    /// The two builtins the carried rows spell without a callback. `isInt`
    /// contributes the safe-integer range beside `integer`; `isPattern`
    /// contributes its pattern only when every flag is representable
    /// (`d`/`g`/`u`, with sticky `y` anchored), and degrades to an
    /// approximate empty fragment otherwise.
    fn builtin_fragment(&self, check: &serde_json::Value) -> Option<(serde_json::Value, bool)> {
        let identity = check.get("representation")?;
        let id = identity.get("id")?.as_str()?;
        match id {
            "effect/schema/isInt" => Some((
                serde_json::json!({
                    "type": "integer",
                    "minimum": -9007199254740991i64,
                    "maximum": 9007199254740991i64,
                }),
                false,
            )),
            "effect/schema/isPattern" => {
                let payload = identity.get("payload")?;
                let source = payload.get("source")?.as_str()?;
                let flags = payload.get("flags")?.as_str().unwrap_or("u");
                let (core, sticky) = match flags.strip_suffix('y') {
                    Some(core) => (core, true),
                    None => (flags, false),
                };
                let exportable = core.ends_with('u')
                    && core[..core.len() - 1]
                        .chars()
                        .all(|flag| flag == 'd' || flag == 'g');
                if !exportable {
                    return Some((serde_json::json!({}), true));
                }
                Some((
                    serde_json::json!({
                        "pattern": if sticky {
                            format!("^(?:{source})")
                        } else {
                            source.to_string()
                        },
                    }),
                    false,
                ))
            }
            _ => None,
        }
    }

    /// One node without its checks or annotations.
    #[allow(clippy::too_many_lines)]
    fn compile_node(
        &mut self,
        doc: &serde_json::Value,
        tag: &str,
        path: &str,
        deps: usize,
    ) -> Result<serde_json::Value, SchemaError> {
        match tag {
            "Any" | "Unknown" => Ok(serde_json::json!({})),
            "ObjectKeyword" => {
                Ok(serde_json::json!({ "anyOf": [{ "type": "object" }, { "type": "array" }] }))
            }
            "Void" | "Undefined" | "Null" => Ok(serde_json::json!({ "type": "null" })),
            "BigInt" => Ok(serde_json::json!({
                "type": "string",
                "allOf": [{ "pattern": "^-?\\d+$" }],
            })),
            "Symbol" => Ok(serde_json::json!({
                "type": "string",
                "allOf": [{ "pattern": "^Symbol\\(([\\s\\S]*)\\)$" }],
            })),
            // A globally registered symbol spells its string form; a local
            // one has no JSON spelling. The port reads the persisted
            // `{ description, global }` shape, since a live symbol has no
            // `Value` spelling.
            "UniqueSymbol" => {
                let symbol = doc
                    .get("symbol")
                    .and_then(serde_json::Value::as_object)
                    .ok_or_else(|| doc_error(path, "a `UniqueSymbol` carries a `symbol`"))?;
                let global = symbol
                    .get("global")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let description = symbol
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if global {
                    Ok(serde_json::json!({
                        "type": "string",
                        "enum": [format!("Symbol({description})")],
                    }))
                } else {
                    Ok(serde_json::json!({ "not": {} }))
                }
            }
            "Declaration" => {
                self.mark_approximate(deps);
                Ok(serde_json::json!({}))
            }
            "Suspend" => {
                let thunk = doc
                    .get("thunk")
                    .ok_or_else(|| doc_error(path, "a `Suspend` carries a `thunk`"))?;
                self.recur(thunk, &doc_seg(path, "thunk"), deps)
            }
            "Never" => Ok(serde_json::json!({ "not": {} })),
            "String" => Ok(serde_json::json!({ "type": "string" })),
            // Numbers cross the JSON boundary with their non-finite spellings
            // beside them: the artificial encoding the pin exports.
            "Number" => Ok(serde_json::json!({
                "anyOf": [
                    { "type": "number" },
                    { "type": "string", "enum": ["NaN", "Infinity", "-Infinity"] },
                ],
            })),
            "Boolean" => Ok(serde_json::json!({ "type": "boolean" })),
            "Literal" => {
                let literal = doc
                    .get("literal")
                    .ok_or_else(|| doc_error(path, "a `Literal` carries a `literal`"))?;
                Ok(match literal_scalar(literal, path)? {
                    LiteralScalar::Text(text) => {
                        serde_json::json!({ "type": "string", "enum": [text] })
                    }
                    LiteralScalar::Number(number) => {
                        serde_json::json!({ "type": "number", "enum": [number] })
                    }
                    LiteralScalar::Bool(flag) => {
                        serde_json::json!({ "type": "boolean", "enum": [flag] })
                    }
                    LiteralScalar::BigInt(digits) => {
                        serde_json::json!({ "type": "string", "enum": [digits] })
                    }
                    LiteralScalar::NonFinite(_) => {
                        return Err(doc_error(path, "a literal is a finite JSON scalar"));
                    }
                })
            }
            "Enum" => {
                let members = doc
                    .get("enums")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| doc_error(path, "an `Enum` carries `enums`"))?;
                if members.is_empty() {
                    return Ok(serde_json::json!({ "not": {} }));
                }
                let mut types = Vec::with_capacity(members.len());
                for (index, member) in members.iter().enumerate() {
                    let member_path = doc_idx(&doc_seg(path, "enums"), index);
                    let pair = member.as_array().ok_or_else(|| {
                        doc_error(&member_path, "an enum member is a name/value pair")
                    })?;
                    let title = pair
                        .first()
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| {
                            doc_error(&member_path, "an enum member is a name/value pair")
                        })?;
                    let value = pair.get(1).ok_or_else(|| {
                        doc_error(&member_path, "an enum member is a name/value pair")
                    })?;
                    let value_path = doc_idx(&member_path, 1);
                    match literal_scalar(value, &value_path)? {
                        LiteralScalar::Text(text) => types.push(serde_json::json!({
                            "type": "string", "enum": [text], "title": title,
                        })),
                        LiteralScalar::Number(number) => types.push(serde_json::json!({
                            "type": "number", "enum": [number], "title": title,
                        })),
                        LiteralScalar::Bool(flag) => types.push(serde_json::json!({
                            "type": "boolean", "enum": [flag], "title": title,
                        })),
                        LiteralScalar::BigInt(_) => {
                            return Err(doc_error(
                                &value_path,
                                "an enum member is a string or a finite number",
                            ));
                        }
                        LiteralScalar::NonFinite(name) => {
                            return Err(doc_error(
                                &value_path,
                                format!("Invalid numeric enum value {name}"),
                            ));
                        }
                    }
                }
                Ok(serde_json::json!({ "anyOf": types }))
            }
            "TemplateLiteral" => {
                let parts = doc
                    .get("parts")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| doc_error(path, "a `TemplateLiteral` carries `parts`"))?;
                let mut pattern = String::from("^");
                for (index, part) in parts.iter().enumerate() {
                    pattern.push_str(&template_part_pattern(
                        part,
                        &doc_idx(&doc_seg(path, "parts"), index),
                    )?);
                }
                pattern.push('$');
                Ok(serde_json::json!({ "type": "string", "pattern": pattern }))
            }
            "Arrays" => self.compile_arrays(doc, path, deps),
            "Objects" => self.compile_objects(doc, path, deps),
            "Union" => {
                let branches = self.fresh_deps();
                if let Some(ExactNode::Deps(members)) = self.exactness.get_mut(deps) {
                    members.push(branches);
                }
                let members = doc
                    .get("types")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| doc_error(path, "a `Union` carries `types`"))?;
                if members.is_empty() {
                    return Ok(serde_json::json!({ "not": {} }));
                }
                let mut types = Vec::with_capacity(members.len());
                for (index, member) in members.iter().enumerate() {
                    types.push(self.recur(
                        member,
                        &doc_idx(&doc_seg(path, "types"), index),
                        branches,
                    )?);
                }
                let mode = doc
                    .get("options")
                    .and_then(|options| options.get("mode"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("anyOf");
                if mode == "anyOf"
                    && types.len() > 1
                    && let Some(compacted) = compact_doc_enums(&types)
                {
                    return Ok(compacted);
                }
                if mode == "oneOf" && !self.is_exact(branches, &mut Vec::new()) {
                    return Ok(serde_json::json!({ "anyOf": types }));
                }
                Ok(serde_json::json!({ mode: types }))
            }
            other => Err(doc_error(
                path,
                format!("no document compiler for `{other}`"),
            )),
        }
    }

    /// A tuple document: `prefixItems` with the leading required count as
    /// `minItems`, a lone rest as `items`, and an empty open tuple as the bare
    /// array shape.
    fn compile_arrays(
        &mut self,
        doc: &serde_json::Value,
        path: &str,
        deps: usize,
    ) -> Result<serde_json::Value, SchemaError> {
        let elements = doc
            .get("elements")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| doc_error(path, "an `Arrays` carries `elements`"))?;
        let rest = doc
            .get("rest")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| doc_error(path, "an `Arrays` carries `rest`"))?;
        if rest.len() > 1 {
            return Err(doc_error(
                doc_seg(path, "rest"),
                "Invalid schema representation document",
            ));
        }
        let mut out = serde_json::json!({ "type": "array" });
        let mut required = elements.len();
        let mut prefix = Vec::with_capacity(elements.len());
        for (index, element) in elements.iter().enumerate() {
            let element_path = doc_idx(&doc_seg(path, "elements"), index);
            if element.get("isOptional") == Some(&serde_json::Value::Bool(true)) {
                required -= 1;
            }
            let inner = element
                .get("type")
                .ok_or_else(|| doc_error(&element_path, "an element carries a `type`"))?;
            let mut compiled = self.recur(inner, &doc_seg(&element_path, "type"), deps)?;
            if let Some(ordinary) = self.collect_annotations(element.get("annotations")) {
                compiled = append_doc_schema(compiled, serde_json::Value::Object(ordinary), false);
            }
            prefix.push(compiled);
        }
        if prefix.is_empty() {
            out["items"] = serde_json::Value::Bool(false);
        } else {
            out["prefixItems"] = serde_json::Value::Array(prefix);
            out["maxItems"] = serde_json::Value::from(elements.len());
            if required > 0 {
                out["minItems"] = serde_json::Value::from(required);
            }
        }
        if let Some(rest) = rest.first() {
            out.as_object_mut().and_then(|map| map.remove("maxItems"));
            let compiled = self.recur(rest, &doc_idx(&doc_seg(path, "rest"), 0), deps)?;
            if !compiled.as_object().is_some_and(serde_json::Map::is_empty) {
                out["items"] = compiled;
            } else {
                out.as_object_mut().and_then(|map| map.remove("items"));
            }
        }
        Ok(out)
    }

    /// An object document: declared properties with `required`, index
    /// signatures as `additionalProperties`, single-selector keys as
    /// `patternProperties`, and anything else as `propertyNames` in error
    /// mode (open mode degrades to an open object).
    #[allow(clippy::too_many_lines)]
    fn compile_objects(
        &mut self,
        doc: &serde_json::Value,
        path: &str,
        deps: usize,
    ) -> Result<serde_json::Value, SchemaError> {
        let properties = doc
            .get("propertySignatures")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| doc_error(path, "an `Objects` carries `propertySignatures`"))?;
        let index = doc
            .get("indexSignatures")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| doc_error(path, "an `Objects` carries `indexSignatures`"))?;
        if properties.is_empty() && index.is_empty() {
            return Ok(serde_json::json!({ "not": { "type": "null" } }));
        }
        let mut out = serde_json::json!({ "type": "object" });
        if !properties.is_empty() {
            let mut compiled = serde_json::Map::new();
            let mut required = Vec::new();
            for (index, property) in properties.iter().enumerate() {
                let property_path = doc_idx(&doc_seg(path, "propertySignatures"), index);
                let name = property
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        doc_error(
                            doc_seg(&property_path, "name"),
                            "Invalid schema representation document",
                        )
                    })?;
                let inner = property
                    .get("type")
                    .ok_or_else(|| doc_error(&property_path, "a property carries a `type`"))?;
                let mut schema = self.recur(inner, &doc_seg(&property_path, "type"), deps)?;
                if let Some(ordinary) = self.collect_annotations(property.get("annotations")) {
                    schema = append_doc_schema(schema, serde_json::Value::Object(ordinary), false);
                }
                compiled.insert(name.to_string(), schema);
                if property.get("isOptional") != Some(&serde_json::Value::Bool(true)) {
                    required.push(serde_json::Value::String(name.to_string()));
                }
            }
            out["properties"] = serde_json::Value::Object(compiled);
            if !required.is_empty() {
                out["required"] = serde_json::Value::Array(required);
            }
        }
        let mut pattern_properties = serde_json::Map::new();
        let mut unconstrained: Vec<serde_json::Value> = Vec::new();
        let mut index_values: Vec<serde_json::Value> = Vec::new();
        let mut unrepresentable = false;
        for (index, signature) in index.iter().enumerate() {
            let signature_path = doc_idx(&doc_seg(path, "indexSignatures"), index);
            let parameter = signature.get("parameter").ok_or_else(|| {
                doc_error(&signature_path, "an index signature carries a `parameter`")
            })?;
            let target = signature
                .get("type")
                .ok_or_else(|| doc_error(&signature_path, "an index signature carries a `type`"))?;
            let mut value = self.recur(target, &doc_seg(&signature_path, "type"), deps)?;
            if value == serde_json::json!({ "not": {} }) {
                value = serde_json::Value::Bool(false);
            }
            index_values.push(value.clone());
            match self.index_patterns(
                parameter,
                &doc_seg(&signature_path, "parameter"),
                &mut Vec::new(),
            )? {
                None => {
                    unrepresentable = true;
                    self.mark_approximate(deps);
                }
                Some(patterns) if patterns.is_empty() => unconstrained.push(value),
                Some(patterns) => {
                    for pattern in patterns {
                        match pattern_properties.get(&pattern) {
                            None => {
                                pattern_properties.insert(pattern, value.clone());
                            }
                            Some(previous) => {
                                let merged = if previous == &serde_json::Value::Bool(false)
                                    || value == serde_json::Value::Bool(false)
                                {
                                    serde_json::Value::Bool(false)
                                } else {
                                    append_doc_schema(previous.clone(), value.clone(), false)
                                };
                                pattern_properties.insert(pattern, merged);
                            }
                        }
                    }
                }
            }
        }
        if !pattern_properties.is_empty() {
            out["patternProperties"] = serde_json::Value::Object(pattern_properties);
        }
        if unrepresentable && self.options.on_excess_property == OnExcessProperty::Error {
            let mut names: Vec<serde_json::Value> = Vec::new();
            if !properties.is_empty() {
                names.push(serde_json::json!({
                    "enum": properties.iter().filter_map(|property| {
                        property.get("name").cloned()
                    }).collect::<Vec<_>>(),
                }));
            }
            for (index, signature) in index.iter().enumerate() {
                let signature_path = doc_idx(&doc_seg(path, "indexSignatures"), index);
                names.push(self.recur(
                    signature.get("parameter").ok_or_else(|| {
                        doc_error(&signature_path, "an index signature carries a `parameter`")
                    })?,
                    &doc_seg(&signature_path, "parameter"),
                    deps,
                )?);
            }
            out["propertyNames"] = if names.len() == 1 {
                names.into_iter().next().expect("one name")
            } else {
                serde_json::json!({ "anyOf": names })
            };
            out["additionalProperties"] = if index_values.len() == 1 {
                index_values.into_iter().next().expect("one value")
            } else {
                serde_json::json!({ "anyOf": index_values })
            };
            if !unconstrained.is_empty() {
                out["allOf"] = serde_json::Value::Array(
                    unconstrained
                        .into_iter()
                        .map(|value| serde_json::json!({ "type": "object", "additionalProperties": value }))
                        .collect(),
                );
            }
        } else if unconstrained.is_empty() {
            out["additionalProperties"] = serde_json::Value::Bool(
                unrepresentable || self.options.on_excess_property != OnExcessProperty::Error,
            );
        } else if unconstrained.len() == 1
            && properties.is_empty()
            && pattern_properties_is_empty(&out)
        {
            out["additionalProperties"] = unconstrained.into_iter().next().expect("one value");
            if out["additionalProperties"]
                .as_object()
                .is_some_and(serde_json::Map::is_empty)
            {
                out.as_object_mut()
                    .and_then(|map| map.remove("additionalProperties"));
            }
        } else if !unconstrained.is_empty() {
            out["allOf"] = serde_json::Value::Array(
                unconstrained
                    .into_iter()
                    .map(|value| serde_json::json!({ "type": "object", "additionalProperties": value }))
                    .collect(),
            );
        }
        Ok(out)
    }

    /// The `patternProperties` selectors an index-signature parameter admits:
    /// an unconstrained string admits none, an exact pattern check or template
    /// admits one, and a union admits every member's. Anything else is
    /// unrepresentable (`None`), and a non-string, non-template, non-union
    /// parameter is invalid.
    fn index_patterns(
        &mut self,
        parameter: &serde_json::Value,
        path: &str,
        seen: &mut Vec<String>,
    ) -> Result<Option<Vec<String>>, SchemaError> {
        let tag = parameter
            .get("_tag")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| doc_error(path, "a representation carries a string `_tag`"))?;
        match tag {
            "Reference" => {
                let key = parameter
                    .get("$ref")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| doc_error(path, "a `Reference` carries a `$ref`"))?;
                if !self.references.contains_key(key) {
                    return Err(doc_error(
                        doc_seg(path, "$ref"),
                        format!("Invalid reference {key}"),
                    ));
                }
                self.compile_definition(key, path)?;
                if seen.contains(&key.to_string()) {
                    return Ok(None);
                }
                seen.push(key.to_string());
                let body = self.references.get(key).cloned().expect("checked above");
                let patterns =
                    self.index_patterns(&body, &format!("[\"references\"][\"{key}\"]"), seen);
                seen.pop();
                patterns
            }
            "String" => {
                let checks: Vec<serde_json::Value> = parameter
                    .get("checks")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if checks.is_empty() {
                    return Ok(Some(Vec::new()));
                }
                if checks.len() != 1 {
                    return Ok(None);
                }
                let check = &checks[0];
                let is_filter =
                    check.get("_tag").and_then(serde_json::Value::as_str) == Some("Filter");
                let has_callback = check
                    .get("annotations")
                    .and_then(|annotations| annotations.get("toJsonSchema"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| self.options.callbacks.contains_key(id));
                let has_builtin = check
                    .get("representation")
                    .and_then(|identity| identity.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| {
                        matches!(id, "effect/schema/isInt" | "effect/schema/isPattern")
                    });
                if !is_filter || (!has_callback && !has_builtin) {
                    return Ok(None);
                }
                let local = self.fresh_deps();
                let schema = self.recur(parameter, path, local)?;
                if !self.is_exact(local, &mut Vec::new()) {
                    return Ok(None);
                }
                match (schema.get("type"), schema.get("pattern")) {
                    (
                        Some(serde_json::Value::String(kind)),
                        Some(serde_json::Value::String(pattern)),
                    ) if kind == "string"
                        && has_only_keywords(
                            &schema,
                            "|type|pattern|title|description|default|examples|readOnly|writeOnly|",
                        ) =>
                    {
                        Ok(Some(vec![pattern.clone()]))
                    }
                    _ => Ok(None),
                }
            }
            "TemplateLiteral" => {
                let checks_empty = parameter
                    .get("checks")
                    .and_then(serde_json::Value::as_array)
                    .is_none_or(Vec::is_empty);
                if !checks_empty {
                    return Ok(None);
                }
                let parts = parameter
                    .get("parts")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| doc_error(path, "a `TemplateLiteral` carries `parts`"))?;
                let mut pattern = String::from("^");
                for (index, part) in parts.iter().enumerate() {
                    pattern.push_str(&template_part_pattern(
                        part,
                        &doc_idx(&doc_seg(path, "parts"), index),
                    )?);
                }
                pattern.push('$');
                Ok(Some(vec![pattern]))
            }
            "Union" => {
                let members = parameter
                    .get("types")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| doc_error(path, "a `Union` carries `types`"))?;
                if members.is_empty()
                    || parameter
                        .get("checks")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|checks| !checks.is_empty())
                    || parameter
                        .get("options")
                        .and_then(|options| options.get("mode"))
                        .and_then(serde_json::Value::as_str)
                        == Some("oneOf")
                {
                    return Ok(None);
                }
                let mut patterns = Vec::new();
                for (index, member) in members.iter().enumerate() {
                    match self.index_patterns(
                        member,
                        &doc_idx(&doc_seg(path, "types"), index),
                        seen,
                    )? {
                        None => return Ok(None),
                        Some(member_patterns) if member_patterns.is_empty() => {
                            return Ok(Some(Vec::new()));
                        }
                        Some(member_patterns) => patterns.extend(member_patterns),
                    }
                }
                Ok(Some(patterns))
            }
            _ => Err(doc_error(path, "Invalid schema representation document")),
        }
    }
}

/// Whether an object document carries no `patternProperties` entry.
fn pattern_properties_is_empty(doc: &serde_json::Value) -> bool {
    doc.get("patternProperties").is_none()
}

/// A literal or enum value the document spells: plain JSON scalars plus the
/// persisted typed wrappers for ambiguous values (non-finite numbers keep
/// their names, bigints keep their digits).
enum LiteralScalar {
    Text(String),
    Number(serde_json::Number),
    Bool(bool),
    BigInt(String),
    NonFinite(String),
}

fn literal_scalar(value: &serde_json::Value, path: &str) -> Result<LiteralScalar, SchemaError> {
    match value {
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            if let Some(wrapper) = value.as_object() {
                let kind = wrapper
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let body = wrapper.get("value");
                match (kind, body) {
                    ("string", Some(serde_json::Value::String(text))) => {
                        return Ok(LiteralScalar::Text(text.clone()));
                    }
                    ("number", Some(serde_json::Value::String(name)))
                        if matches!(name.as_str(), "NaN" | "Infinity" | "-Infinity") =>
                    {
                        return Ok(LiteralScalar::NonFinite(name.clone()));
                    }
                    ("number", Some(serde_json::Value::Number(number))) => {
                        return Ok(LiteralScalar::Number(number.clone()));
                    }
                    ("bigint", Some(serde_json::Value::String(digits))) => {
                        return Ok(LiteralScalar::BigInt(digits.clone()));
                    }
                    _ => {}
                }
            }
            Err(doc_error(path, "a literal is a JSON scalar"))
        }
        serde_json::Value::Bool(flag) => Ok(LiteralScalar::Bool(*flag)),
        serde_json::Value::Number(number) => Ok(LiteralScalar::Number(number.clone())),
        serde_json::Value::String(text) => Ok(LiteralScalar::Text(text.clone())),
    }
}

/// One template-literal part as its pattern: literals escape, `String` and
/// `Number` contribute their character classes, and templates and unions
/// nest. Anything else is invalid.
fn template_part_pattern(part: &serde_json::Value, path: &str) -> Result<String, SchemaError> {
    let tag = part
        .get("_tag")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| doc_error(path, "a representation carries a string `_tag`"))?;
    match tag {
        "Literal" => {
            let literal = part
                .get("literal")
                .ok_or_else(|| doc_error(path, "a `Literal` carries a `literal`"))?;
            match literal_scalar(literal, path)? {
                LiteralScalar::Text(text) => Ok(escape_regex(&text)),
                LiteralScalar::Number(number) => Ok(escape_regex(&number.to_string())),
                LiteralScalar::Bool(flag) => Ok(escape_regex(&flag.to_string())),
                LiteralScalar::BigInt(digits) => Ok(escape_regex(&digits)),
                LiteralScalar::NonFinite(_) => {
                    Err(doc_error(path, "a literal is a finite JSON scalar"))
                }
            }
        }
        "String" => Ok("[\\s\\S]*?".to_string()),
        "Number" => Ok("[+-]?\\d*\\.?\\d+(?:[Ee][+-]?\\d+)?".to_string()),
        "TemplateLiteral" => {
            let parts = part
                .get("parts")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| doc_error(path, "a `TemplateLiteral` carries `parts`"))?;
            let mut pattern = String::new();
            for (index, nested) in parts.iter().enumerate() {
                pattern.push_str(&template_part_pattern(
                    nested,
                    &doc_idx(&doc_seg(path, "parts"), index),
                )?);
            }
            Ok(pattern)
        }
        "Union" => {
            let members = part
                .get("types")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| doc_error(path, "a `Union` carries `types`"))?;
            let mut members_out = Vec::with_capacity(members.len());
            for (index, member) in members.iter().enumerate() {
                members_out.push(template_part_pattern(
                    member,
                    &doc_idx(&doc_seg(path, "types"), index),
                )?);
            }
            Ok(format!("(?:{})", members_out.join("|")))
        }
        _ => Err(doc_error(path, "Invalid schema representation document")),
    }
}

/// Merge a check fragment into its base: an empty base takes the fragment, an
/// empty fragment keeps the base, a `number`/`integer` type absorbs through
/// the artificial number encoding, an inline check tightens bounds in place,
/// and anything else lands in `allOf`.
fn append_doc_schema(
    mut left: serde_json::Value,
    right: serde_json::Value,
    inline_check: bool,
) -> serde_json::Value {
    if left.as_object().is_some_and(serde_json::Map::is_empty) {
        return right;
    }
    let right_keys: Vec<String> = right
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    if right_keys.is_empty() {
        return left;
    }
    let left_type = left
        .get("type")
        .and_then(serde_json::Value::as_str)
        .filter(|kind| *kind == "number" || *kind == "integer")
        .map(str::to_string);
    let is_number_encoding = is_number_encoding(&left);
    if left_type.is_some() || is_number_encoding {
        let extracted = extract_number_type(&right);
        if let Some(number_type) = extracted.0 {
            let merged_type = if left_type.as_deref() == Some("integer") || number_type == "integer"
            {
                "integer"
            } else {
                "number"
            };
            let mut base = left.clone();
            if let Some(map) = base.as_object_mut() {
                map.insert(
                    "type".to_string(),
                    serde_json::Value::String(merged_type.to_string()),
                );
                if is_number_encoding {
                    map.remove("anyOf");
                }
            }
            let extracted_keys: Vec<String> = extracted
                .1
                .as_object()
                .map(|map| map.keys().cloned().collect())
                .unwrap_or_default();
            if extracted_keys.is_empty() {
                return base;
            }
            if has_only_keywords(&extracted.1, PROMOTABLE_ANNOTATION_KEYWORDS)
                && has_no_collisions(&base, &extracted_keys)
            {
                let mut merged = base;
                if let (Some(map), Some(extra)) = (merged.as_object_mut(), extracted.1.as_object())
                {
                    for (key, value) in extra {
                        map.insert(key.clone(), value.clone());
                    }
                }
                return merged;
            }
            return append_doc_schema(base, extracted.1, inline_check);
        }
    }
    if inline_check && let Some(merged) = merge_inline_check(&left, &right, &right_keys) {
        return merged;
    }
    let members: Vec<serde_json::Value> = if right
        .get("allOf")
        .and_then(serde_json::Value::as_array)
        .is_some()
        && right_keys.len() == 1
    {
        right
            .get("allOf")
            .cloned()
            .and_then(|entries| entries.as_array().cloned())
            .unwrap_or_default()
    } else {
        vec![right]
    };
    if left
        .get("allOf")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        left.as_object_mut().and_then(|map| {
            map.get_mut("allOf").and_then(|entries| {
                entries
                    .as_array_mut()
                    .map(|entries| entries.extend(members))
            })
        });
        return left;
    }
    if left
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        let mut entries = vec![left];
        entries.extend(members);
        return serde_json::json!({ "allOf": entries });
    }
    if let Some(map) = left.as_object_mut() {
        map.insert("allOf".to_string(), serde_json::Value::Array(members));
    }
    left
}

/// Whether a document is the artificial number encoding: a `number` branch
/// beside the non-finite string spellings.
fn is_number_encoding(doc: &serde_json::Value) -> bool {
    let Some(members) = doc.get("anyOf").and_then(serde_json::Value::as_array) else {
        return false;
    };
    if members.len() != 2
        || members[0].get("type") != Some(&serde_json::Value::String("number".to_string()))
    {
        return false;
    }
    let special = &members[1];
    special.get("type") == Some(&serde_json::Value::String("string".to_string()))
        && special
            .get("enum")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|values| {
                ["NaN", "Infinity", "-Infinity"]
                    .iter()
                    .all(|name| values.contains(&serde_json::Value::String(name.to_string())))
            })
}

/// Pull a `number`/`integer` type out of a fragment, recursing through
/// `allOf` members. Members left without a type stay behind.
fn extract_number_type(fragment: &serde_json::Value) -> (Option<String>, serde_json::Value) {
    let mut number_type = match fragment.get("type").and_then(serde_json::Value::as_str) {
        Some("number" | "integer") => fragment
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        _ => None,
    };
    let mut out = fragment.clone();
    if number_type.is_some()
        && let Some(map) = out.as_object_mut()
    {
        map.remove("type");
    }
    if let Some(members) = out
        .get("allOf")
        .and_then(serde_json::Value::as_array)
        .cloned()
    {
        let mut kept = Vec::new();
        let mut changed = false;
        for member in members {
            let (member_type, rest) = extract_number_type(&member);
            if let Some(member_type) = member_type {
                changed = true;
                if number_type.is_none() || member_type == "integer" {
                    number_type = Some(member_type);
                }
            }
            if !rest.as_object().is_some_and(serde_json::Map::is_empty) {
                kept.push(rest);
            }
        }
        if changed {
            if let Some(map) = out.as_object_mut() {
                map.remove("allOf");
                if !kept.is_empty() {
                    map.insert("allOf".to_string(), serde_json::Value::Array(kept));
                }
            }
        }
    }
    (number_type, out)
}

/// Tighten repeated bounds in place and drop an inclusive or exclusive bound
/// implied by the other on the same side. Anything else (a `$ref` base or a
/// non-bound collision) refuses, so the caller falls back to `allOf`.
fn merge_inline_check(
    left: &serde_json::Value,
    right: &serde_json::Value,
    right_keys: &[String],
) -> Option<serde_json::Value> {
    if left
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        return None;
    }
    let mut out = left.clone();
    let map = out.as_object_mut()?;
    for (key, value) in right.as_object()? {
        map.insert(key.clone(), value.clone());
    }
    for key in right_keys {
        if left.get(key).is_none() {
            continue;
        }
        let (Some(first), Some(second)) = (
            left.get(key).and_then(serde_json::Value::as_f64),
            right.get(key).and_then(serde_json::Value::as_f64),
        ) else {
            return None;
        };
        let tightened = if is_lower_bound_keyword(key) {
            first.max(second)
        } else if is_upper_bound_keyword(key) {
            first.min(second)
        } else {
            return None;
        };
        // Whole bounds keep their integer form, which is what the pin prints.
        map.insert(key.clone(), num_value(tightened));
    }
    if let (Some(minimum), Some(exclusive)) = (
        map.get("minimum").and_then(serde_json::Value::as_f64),
        map.get("exclusiveMinimum")
            .and_then(serde_json::Value::as_f64),
    ) {
        map.remove(if exclusive >= minimum {
            "minimum"
        } else {
            "exclusiveMinimum"
        });
    }
    if let (Some(maximum), Some(exclusive)) = (
        map.get("maximum").and_then(serde_json::Value::as_f64),
        map.get("exclusiveMaximum")
            .and_then(serde_json::Value::as_f64),
    ) {
        map.remove(if exclusive <= maximum {
            "maximum"
        } else {
            "exclusiveMaximum"
        });
    }
    Some(out)
}

/// Compact same-type literal enums with disjoint values into one typed enum.
fn compact_doc_enums(members: &[serde_json::Value]) -> Option<serde_json::Value> {
    if members.iter().any(|member| {
        member.as_object().is_none_or(|map| {
            map.len() != 2 || !map.contains_key("type") || !map.contains_key("enum")
        })
    }) {
        return None;
    }
    let first_type = members[0].get("type").cloned();
    let mut merged = Vec::new();
    for member in members {
        if member.get("type") != first_type.as_ref() {
            return None;
        }
        let values: Vec<serde_json::Value> = member
            .get("enum")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if values.is_empty() || values.iter().any(|value| merged.contains(value)) {
            return None;
        }
        merged.extend(values);
    }
    Some(serde_json::json!({ "type": first_type, "enum": merged }))
}

/// Format a definition name as a URI fragment reference: JSON Pointer
/// escaping, then `encodeURI`, with `#` encoded.
fn encode_uri_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for char in token.chars() {
        match char {
            'A'..='Z'
            | 'a'..='z'
            | '0'..='9'
            | ';'
            | ','
            | '/'
            | '?'
            | ':'
            | '@'
            | '&'
            | '='
            | '+'
            | '$'
            | '-'
            | '_'
            | '.'
            | '!'
            | '~'
            | '*'
            | '\''
            | '('
            | ')' => out.push(char),
            '#' => out.push_str("%23"),
            _ => {
                let mut buffer = [0u8; 4];
                for byte in char.encode_utf8(&mut buffer).bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}
