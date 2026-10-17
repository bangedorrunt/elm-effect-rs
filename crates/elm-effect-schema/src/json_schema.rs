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

use super::{Field, Optionality, Schema, SchemaKind, Unknown, kind_type_name};

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
