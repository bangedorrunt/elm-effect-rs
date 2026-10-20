//! Effect Schema core: decode `unknown` into typed values, encode back.
//!
//! The unknown side is JSON-shaped (`Unknown`); the typed side is plain
//! Rust. `serde_json` serves the boring byte cases; the spec is Effect
//! Schema (parsers, issues with paths, strict never-lossy decoding).

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;

/// The largest integer a double holds exactly, and the bound every
/// integer-valued check in the port measures against: the pin's
/// `Number.isSafeInteger` bound. Every safe integer is exactly representable.
const MAX_SAFE_INTEGER: f64 = 9007199254740991.0;

pub mod transform;

pub mod differ_json_patch;

pub mod tagged;
pub mod tagged_union;

pub mod class;

pub mod built_in_revivers;
pub mod checks;
pub mod filters;
pub use built_in_revivers::*;
pub mod json_schema;
pub mod representation;
pub mod representation_doc;

pub mod refinements;

pub use class::{ClassMeta, ClassValue, ExcessProperty};
pub use differ_json_patch::{Differ, DifferError, JsonPatch, PatchError, PatchOp, apply, diff};
pub use filters::{FilterIssue, FilterOutcome, FilterSpec};
pub use tagged::{Tagged, TaggedError, catch_tag};
pub use tagged_union::{TaggedUnion, TaggedUnionError, tagged_union, to_tagged_union};

/// An untyped value: the decode source and encode target.
#[derive(Debug, Clone, PartialEq)]
pub enum Unknown {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Unknown>),
    Object(BTreeMap<String, Unknown>),
}

impl Unknown {
    pub fn kind(&self) -> &'static str {
        match self {
            Unknown::Null => "null",
            Unknown::Bool(_) => "boolean",
            Unknown::Number(_) => "number",
            Unknown::Text(_) => "string",
            Unknown::Array(_) => "array",
            Unknown::Object(_) => "object",
        }
    }
}

/// One placement-aware failure: what was expected, what arrived, where.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SchemaError {
    /// A value of the wrong shape arrived.
    #[error("expected {expected} at {path}, got {got}")]
    TypeMismatch {
        path: String,
        expected: &'static str,
        got: &'static str,
    },
    /// The same refusal where the expected side is only known once the schema
    /// exists: a union's member list, which no `&'static str` can hold. It
    /// prints exactly like `TypeMismatch`, so a caller matching on the message
    /// sees one shape; it exists as its own variant because the alternative is
    /// handing every one of the forty static sites a `String` for the sake of
    /// this one.
    #[error("expected {expected} at {path}, got {got}")]
    TypeMismatchDynamic {
        path: String,
        expected: String,
        got: &'static str,
    },
    /// A constructor default failed (`withConstructorDefault`): the default's
    /// own message, at the occurrence it was filling.
    #[error("{message} at {path}")]
    DefaultFailed { path: String, message: String },
    /// Invalid JSON bytes or JSON that is not UTF-8 text.
    #[error("invalid JSON at {path}: {message}")]
    InvalidJson { path: String, message: String },
    /// A value this schema cannot represent on the wire: the pin's
    /// encoding-side issue. `try_encode` returns it; the total `encode`
    /// panics, so a caller that may hold such a value must use the fallible
    /// one.
    #[error("cannot encode at {path}: expected {expected}")]
    EncodingFailed { path: String, expected: String },
    /// A refinement answered rather than passed (`makeFilter`): the filter's own
    /// message, at the path the filter pointed at.
    #[error("{message} at {path}")]
    FilterFailed { path: String, message: String },
    /// More than one issue at once, which is what a decode that collects every
    /// issue reports. It prints one member per line, the shape the pin's
    /// composite has.
    #[error("{}", composite_message(.issues))]
    Composite { issues: Vec<SchemaError> },
}

/// A composite's members, one message per line.
fn composite_message(issues: &[SchemaError]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join("\n")
}

/// Maps a wire value to the value a check attached on top of a schema reads.
/// See [`Schema::checked`] and the `check_view` field for what sets one.
pub type CheckView = std::sync::Arc<dyn Fn(&Unknown) -> Unknown + Send + Sync>;

/// A two-way codec between `Unknown` and `T`.
///
/// Clone because the pin's schemas are values you pass around, and because a
/// self-referential schema (the `suspend` recursion every toFormatter Struct
/// row uses) needs to hand out copies of itself from a closure.
#[derive(Clone)]
pub struct Schema<T> {
    decode: std::sync::Arc<dyn Fn(Unknown) -> Result<T, SchemaError> + Send + Sync>,
    encode: std::sync::Arc<dyn Fn(&T) -> Result<Unknown, SchemaError> + Send + Sync>,
    kind: SchemaKind,
    /// Named child schemas, so a formatter can recurse into them. Kinds alone
    /// cannot carry this: the pin's `suspend` is lazily resolved at format
    /// time, so a nested struct must be reachable as a schema, not as a shape.
    /// Empty for every constructor that does not build a composite.
    children: Vec<(String, std::sync::Arc<Schema<Unknown>>)>,
    /// How to render a value of this schema. Each constructor sets this, so a
    /// wrapper like `as_unknown` can simply delegate to the schema it wraps
    /// instead of having to reconstruct the shape.
    /// None until a composite constructor sets one, which tells `formatter()`
    /// to walk the kind instead.
    formatter: Option<std::sync::Arc<dyn Fn(&Unknown) -> String + Send + Sync>>,
    /// The relation `to_equivalence` answers with (`Schema.toEquivalence`).
    /// Each constructor that knows its own comparison rule installs one while
    /// the schema is built, reading its children's relations as it goes; a
    /// schema built without one compares the two values' wire forms. `suspend`
    /// is the one constructor that cannot decide eagerly, because the schema it
    /// resolves is the one being built.
    equivalence: Option<Equivalence<T>>,
    /// The schema a re-typing wraps. `as_unknown` keeps its own kind opaque so
    /// the JSON Schema document does not change, but a repr, and a check built
    /// on top, must see the schema underneath. Captured as a kind plus its
    /// reprs rather than as a schema, because the wrapped schema has an
    /// arbitrary `T` and re-typing it here would recurse forever.
    wrapped: Option<Box<Wrapped>>,
    /// What a check attached on top of this schema reads: the wire value mapped
    /// to the form the check is written against. The pin's `check` lands on the
    /// target side of a link, so `FiniteFromString.check(isInt())` reads the
    /// number its transform produced, not the text it consumed; a schema whose
    /// decoded value already is its wire value leaves this `None` and the check
    /// reads what arrived. A projection is a pure mapping over `Unknown` (it
    /// never fails: a wire value the transform cannot read is returned as it
    /// is, and the codec refuses it a moment later).
    check_view: Option<CheckView>,
    /// The construction path (`make`): a composite that fills constructor
    /// defaults sets this, because building a value is a different job from
    /// decoding one. A schema without it constructs by decoding.
    construct: Option<std::sync::Arc<dyn Fn(Unknown) -> Result<T, SchemaError> + Send + Sync>>,
    /// A constructor default (`withConstructorDefault`): evaluated at a field
    /// occurrence whose input is absent, on the construction path only. The
    /// value it produces is constructed through this same schema, so nested
    /// defaults fill recursively.
    constructor_default:
        Option<std::sync::Arc<dyn Fn() -> Result<Unknown, SchemaError> + Send + Sync>>,
    /// A decoding default (`withDecodingDefault`/`withDecodingDefaultType`):
    /// what an absent input means. A missing key at a struct occurrence, and a
    /// standalone null, decode to this instead of failing.
    decoding_default: Option<SchemaDefault<T>>,
    /// `{ encodingStrategy: "omit" }` on a decoding default: encoding leaves
    /// the key out, so a defaulted value does not travel back out.
    omit_on_encode: bool,
    /// The declared fields of a struct shape, kept so a derivation
    /// (`mapFields`/`encodeKeys`) rebuilds from the real field list instead of
    /// guessing one from the wire shape. None for every other shape.
    struct_fields: Option<Vec<Field>>,
    /// The declared positional elements of a tuple shape, for the same reason
    /// (`mapElements`). None for every other shape.
    tuple_elements: Option<Vec<Schema<Unknown>>>,
    /// The declared members of a union shape, for the same reason
    /// (`mapMembers`). None for every other shape, and for a union built by
    /// `union`, which tries two shapes without keeping the list.
    union_members: Option<Vec<Schema<Unknown>>>,
    /// The element schema an array was built from (`Schema.Array(...).value`).
    /// Kept as an erased schema rather than in `children`, which the object
    /// path reads as named fields.
    array_element: Option<std::sync::Arc<Schema<Unknown>>>,
    /// The schema an `Option` was built from (`Schema.Option(...).value`),
    /// erased because the box's value type is the field's own `T`. None for
    /// every other shape.
    option_value: Option<std::sync::Arc<Schema<Unknown>>>,
    /// The success and failure schemas a boxed `Result` was built from
    /// (`Schema.Result(...).success`/`.failure`), erased for the same reason.
    result_sides: Option<(
        std::sync::Arc<Schema<Unknown>>,
        std::sync::Arc<Schema<Unknown>>,
    )>,
    /// Key-level annotations (`annotateKey`). The pin keeps these in the AST's
    /// `context` slot rather than on the node, so a schema has two annotation
    /// slots and only one of them is read by `resolve_annotations`.
    key_annotations: Vec<(String, String)>,
    /// The class this schema describes (`Schema.Class` and its relatives), when
    /// it is one: the struct-shaped codec plus the identity, discriminant and
    /// constructor policy a class adds. None for every other schema.
    class_meta: Option<Box<crate::class::ClassMeta>>,
}

/// The two spellings of a decoding default. `withDecodingDefault` takes a wire
/// value, which is decoded through the schema; `withDecodingDefaultType` takes
/// one already decoded, used as it is. Both may fail, and a failure is a decode
/// failure at the occurrence being filled.
pub enum SchemaDefault<T> {
    Wire(std::sync::Arc<dyn Fn() -> Result<Unknown, SchemaError> + Send + Sync>),
    Decoded(std::sync::Arc<dyn Fn() -> Result<T, SchemaError> + Send + Sync>),
}

// Hand-written because a derive would demand `T: Clone` for a pair of `Arc`s
// that are cloneable whatever `T` is.
impl<T> Clone for SchemaDefault<T> {
    fn clone(&self) -> Self {
        match self {
            SchemaDefault::Wire(produce) => SchemaDefault::Wire(produce.clone()),
            SchemaDefault::Decoded(produce) => SchemaDefault::Decoded(produce.clone()),
        }
    }
}

/// What a re-typing hides: the schema it wrapped, plus its reprs.
#[derive(Clone)]
struct Wrapped {
    kind: SchemaKind,
    children: Vec<(String, Arc<Schema<Unknown>>)>,
    code: String,
    ty: String,
}

/// An equivalence relation on a schema's typed side: what
/// `Schema.toEquivalence` answers. The pin derives one per AST node behind a
/// `memoize`, so a second ask hands back the same relation; this port builds it
/// once, while the schema itself is being constructed, and `to_equivalence` is
/// then a lookup.
pub type Equivalence<T> = Arc<dyn Fn(&T, &T) -> bool + Send + Sync>;

/// The pin's `Equivalence.strictEqual` (`===`), which `Never` answers with:
/// identity for the two shapes `Unknown` holds by reference, value equality for
/// the rest. The distinction is the row's whole point, so it is made here rather
/// than approximated by `==`.
fn strict_identity_equivalence() -> Equivalence<Unknown> {
    Arc::new(|a: &Unknown, b: &Unknown| match (a, b) {
        (Unknown::Object(_), Unknown::Object(_)) | (Unknown::Array(_), Unknown::Array(_)) => {
            std::ptr::eq(a, b)
        }
        _ => a == b,
    })
}

/// The subset of ECMAScript regex syntax the pin's JSON Schema patterns use, and
/// nothing more. A pattern outside the subset is unenforced rather than
/// approximate: the pin itself reports two of them as unsupported, and guessing
/// at a translation would silently accept or reject the wrong values.
///
/// Supported: literals, `.`, `^`, `$`, `*`, `+`, `[...]` with ranges of
/// characters, and `|` is deliberately absent because no pin pattern uses it.
#[derive(Debug, Clone)]
enum Pattern {
    /// One literal character.
    Char(char),
    /// Any character.
    Any,
    /// A character class, negated or not.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    /// A quantified atom: `+` is `(1, None)`, `*` is `(0, None)` and `?` is
    /// `(0, Some(1))`.
    Repeat(Box<Pattern>, usize, Option<usize>),
}

/// Compile a pattern, or say what about it is outside the supported subset.
fn compile_pattern(source: &str) -> Option<Vec<Pattern>> {
    let mut out = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(current) = chars.next() {
        let atom = match current {
            '.' => Pattern::Any,
            '^' | '$' => Pattern::Char(current),
            '[' => {
                let negated = chars.peek() == Some(&'^');
                if negated {
                    chars.next();
                }
                let mut ranges = Vec::new();
                loop {
                    let Some(start) = chars.next() else {
                        return None;
                    };
                    if start == ']' {
                        break;
                    }
                    let end = match chars.peek() {
                        Some(&'-') => {
                            chars.next();
                            match chars.next() {
                                Some(end) if end != ']' => end,
                                // A trailing `-` before `]` is a literal dash.
                                _ => start,
                            }
                        }
                        _ => start,
                    };
                    ranges.push((start, end));
                }
                Pattern::Class { negated, ranges }
            }
            other => Pattern::Char(other),
        };
        // A quantifier attaches to the atom it follows. Counted forms
        // (`{2,3}`) are still outside the subset and read as literal braces.
        let quantified = match chars.peek() {
            Some('*') => Some(Pattern::Repeat(Box::new(atom.clone()), 0, None)),
            Some('+') => Some(Pattern::Repeat(Box::new(atom.clone()), 1, None)),
            Some('?') => Some(Pattern::Repeat(Box::new(atom.clone()), 0, Some(1))),
            _ => None,
        };
        match quantified {
            Some(repeated) => {
                chars.next();
                out.push(repeated);
            }
            None => out.push(atom),
        }
    }
    Some(out)
}

/// Whether a value satisfies a compiled pattern. Anchors match at the ends, and
/// every other atom is matched greedily and in order, which is correct for the
/// anchored, single-alternation patterns this subset compiles.
fn pattern_matches(pattern: &[Pattern], value: &str) -> bool {
    fn walk(pattern: &[Pattern], value: &[char], anchored_start: bool, anchored_end: bool) -> bool {
        if pattern.is_empty() {
            return value.is_empty() || !anchored_end;
        }
        match &pattern[0] {
            Pattern::Char('^') if anchored_start => walk(&pattern[1..], value, false, anchored_end),
            Pattern::Char('$') if anchored_end => pattern.len() == 1 && value.is_empty(),
            Pattern::Repeat(atom, minimum, maximum) => {
                // How many the atom could take here, bounded by the quantifier.
                let mut available = 0usize;
                while available < value.len()
                    && maximum.map_or(true, |limit| available < limit)
                    && atom_matches(atom, value[available])
                {
                    available += 1;
                }
                // Greedy, with backtracking: the longest run first, then one
                // character shorter, never below the quantifier's minimum.
                let mut taken = available;
                loop {
                    if taken >= *minimum
                        && walk(&pattern[1..], &value[taken..], anchored_start, anchored_end)
                    {
                        return true;
                    }
                    if taken <= *minimum {
                        return false;
                    }
                    taken -= 1;
                }
            }
            atom => {
                if value.is_empty() {
                    // An unanchored end lets a shorter pattern match a longer
                    // value, which is the case for a pattern with no `$`.
                    return pattern.len() == 1 && !anchored_end;
                }
                if !atom_matches(atom, value[0]) {
                    return false;
                }
                walk(&pattern[1..], &value[1..], anchored_start, anchored_end)
            }
        }
    }
    let value: Vec<char> = value.chars().collect();
    walk(pattern, &value, true, false)
}

fn atom_matches(atom: &Pattern, value: char) -> bool {
    match atom {
        Pattern::Char(expected) => *expected == value,
        Pattern::Any => true,
        Pattern::Class { negated, ranges } => {
            let in_class = ranges
                .iter()
                .any(|(start, end)| value >= *start && value <= *end);
            in_class != *negated
        }
        // A quantifier is never handed here: `walk` takes a Repeat apart itself,
        // which is what gives it its backtracking. A nested one would be a
        // pattern the compiler does not produce (`a**`), so it matches nothing.
        Pattern::Repeat(..) => false,
    }
}

thread_local! {
    /// Per-THREAD, not per-process, and that is the whole point: two imports
    /// running at once must not swap each other's definitions. A process-global
    /// slot passed a single-threaded run and failed roughly one run in three
    /// under `cargo test`, because the test harness runs tests on several
    /// threads and one thread's `from_json_schema` overwrote another's `$defs`
    /// mid-import.
    static ROOT_DEFS: std::cell::RefCell<Option<serde_json::Map<String, serde_json::Value>>> =
        const { std::cell::RefCell::new(None) };
}

/// Marks an annotation value that prints as raw JSON rather than as a string.
/// The pin's `contentSchema` carries an object, so `annotate({ "contentSchema":
/// { "type": "number" } })` is the repr, not a quoted string. The marker is a
/// control character no annotation value could contain, so a legitimate value is
/// never mistaken for it.
const RAW_ANNOTATION: char = '\u{1}';

/// One refinement in a chain: its name, its arguments, and the annotation pairs
/// the pin attaches to it. `decompose` lowers a schema's `Check` chain into a
/// list of these so an intersection can dedupe and reorder them, and `rebuild`
/// raises the list back into a chain.
#[derive(Debug, Clone, PartialEq)]
struct Check {
    name: String,
    args: Vec<String>,
    annotation: Vec<(String, String)>,
    /// The members of a `makeFilterGroup`, kept beside the rendered argument so
    /// a decode evaluates them instead of reading them back out of the printed
    /// repr. Empty for every ordinary check.
    group: Vec<Check>,
}

/// Insert annotation pairs into a list, replacing by key rather than
/// appending. A document cannot carry the same key twice, and neither can an
/// intersection: the later value wins, in the earlier key's position.
fn merge_annotation(annotation: &mut Vec<(String, String)>, pairs: Vec<(String, String)>) {
    for (key, value) in pairs {
        match annotation.iter_mut().find(|(seen, _)| *seen == key) {
            Some(slot) => slot.1 = value,
            None => annotation.push((key, value)),
        }
    }
}

/// One exact value as the pin's repr prints it: a JSON scalar, with a whole
/// number keeping no `.0`, because that is what JavaScript prints.
fn render_value(value: &Unknown) -> String {
    match value {
        Unknown::Number(number) => format_number(*number),
        other => other.to_json().to_string(),
    }
}

/// A pattern as the JS RegExp literal the pin's repr prints. The bare pattern
/// text is not what appears in a check's argument.
fn regex_argument(pattern: &str) -> String {
    format!("new RegExp({}, \"u\")", json_key(pattern))
}

/// The message a pattern check carries, naming the pattern it matches.
fn regex_message(pattern: &str) -> String {
    format!("a string matching the RegExp {pattern}")
}

/// Render one check as the pin prints it inside a chain or a filter group.
/// A JSON value the way the pin's repr prints one inside an annotation: objects
/// and arrays carry a space after each comma and around their braces. Compact
/// JSON would read `{"type":"number"}`, and the pin's row asserts the spaced form.
fn spaced_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(key, value)| format!("{}: {}", json_key(key), spaced_json(value)))
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(spaced_json).collect();
            format!("[{}]", inner.join(", "))
        }
        other => other.to_string(),
    }
}

/// An annotation value as the pin's repr prints it: raw JSON when marked, a
/// quoted string otherwise.
fn render_annotation_value(value: &str) -> String {
    match value.strip_prefix(RAW_ANNOTATION) {
        Some(raw) => match serde_json::from_str(raw) {
            Ok(parsed) => spaced_json(&parsed),
            // A value that is not JSON after all falls back to the string the
            // caller stored, so a malformed annotation cannot panic a repr.
            Err(_) => json_key(raw),
        },
        None => json_key(value),
    }
}

fn render_check(check: &Check) -> String {
    let mut out = format!("Schema.{}({})", check.name, check.args.join(", "));
    if !check.annotation.is_empty() {
        let pairs = check
            .annotation
            .iter()
            .map(|(key, value)| format!("{}: {}", json_key(key), render_annotation_value(value)))
            .collect::<Vec<_>>()
            .join(", ");
        out = format!("{out}.annotate({{ {pairs} }})");
    }
    out
}

/// Whether a base carries no named type, which is the open JSON value. A
/// keyword-only member (an `allOf` member with `propertyNames` but no `type`)
/// needs the object checks applied to it.
fn base_kind_is_open(schema: &Schema<Unknown>) -> bool {
    matches!(schema.repr_kind(), SchemaKind::Json)
}

/// Build the schema for a filtered set of exact values: none is `Never`, one is
/// a `Literal`, several are a `Literals`.
fn literal_union(values: Vec<Unknown>) -> Schema<Unknown> {
    match values.len() {
        0 => Schema::as_unknown(Schema::never()),
        1 => Schema::as_unknown(Schema::literal(
            values.into_iter().next().expect("one value"),
        )),
        _ => {
            let kind = SchemaKind::Literals(values.clone());
            Schema::as_unknown(Schema::new(
                move |unknown| {
                    if values.iter().any(|value| *value == unknown) {
                        Ok(unknown)
                    } else {
                        mismatch("$", "one of the literals", &unknown)
                    }
                },
                |value| Ok(value.clone()),
                kind,
            ))
        }
    }
}

/// Split a kind into its base shape, the checks layered on top of it, and the
/// annotation pairs riding the base. A `Check` nesting is read from the outside
/// in, so the outermost check is the last one in the list, which is the order a
/// repr prints them.
fn decompose(kind: &SchemaKind) -> (SchemaKind, Vec<Check>, Vec<(String, String)>) {
    match kind {
        SchemaKind::Annotated { pairs, inner } => {
            let (base, checks, mut annotation) = decompose(inner);
            merge_annotation(&mut annotation, pairs.clone());
            (base, checks, annotation)
        }
        SchemaKind::Check {
            base,
            name,
            args,
            annotation,
        } => {
            let (kind, mut checks, leading) = decompose(base);
            checks.push(Check {
                name: name.clone(),
                args: args.clone(),
                annotation: annotation.clone(),
                group: Vec::new(),
            });
            (kind, checks, leading)
        }
        other => (other.clone(), Vec::new(), Vec::new()),
    }
}

/// The key names an exact-value key schema admits (`Record(Literals([...]))`),
/// deduped and in the order they were declared. A key schema that fixes no
/// values of its own (`String`, a checked pattern) names no keys: there is
/// nothing to require of an object under it.
fn exact_key_names(key: &Schema<Unknown>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for value in exact_values(&key.repr_kind()).unwrap_or_default() {
        if let Unknown::Text(name) = value {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// The exact values a kind admits, when it admits finitely many. A single
/// `Literal` admits one; a `Literals` admits its list; anything else admits
/// none, which is what makes an intersection with a refinement a filter.
fn exact_values(kind: &SchemaKind) -> Option<Vec<Unknown>> {
    match kind {
        SchemaKind::Literal(value) => Some(vec![value.clone()]),
        SchemaKind::Literals(values) => Some(values.clone()),
        _ => None,
    }
}

/// Keep only the values that satisfy every check. This is the whole of a
/// literal refinement: `allOf: [{ enum: ["a", "ab"] }]` next to a `minLength: 2`
/// keeps `"ab"` and drops `"a"`.
fn filter_by_checks(values: Vec<Unknown>, checks: &[Check]) -> Vec<Unknown> {
    values
        .into_iter()
        .filter(|value| {
            checks.iter().all(|check| {
                check_holds_arg(&check.name, check.args.first().map(String::as_str), value)
            })
        })
        .collect()
}

/// Whether a tuple element is optional, which the pin spells two ways: an
/// `optionalKey` position may be absent, and an `optional` position may be
/// absent or null. Both make the positions after them a required-after-optional.
fn is_optional_element(kind: &SchemaKind) -> bool {
    matches!(kind, SchemaKind::OptionalKey(_) | SchemaKind::Optional(_))
}

/// The decoder for one position of a tuple with a rest list, which is the
/// pin's `getParser` split: a leading position has its own schema, a position
/// inside the repeatable window uses the first rest schema, and a position at
/// or past the tail threshold uses the trailing one that lines up with it.
fn rest_decoder_at<'a>(
    elements: &'a [Arc<Schema<Unknown>>],
    rest: &'a [Arc<Schema<Unknown>>],
    tail_threshold: usize,
    index: usize,
) -> &'a Arc<Schema<Unknown>> {
    if index < elements.len() {
        &elements[index]
    } else if index >= tail_threshold {
        &rest[index - tail_threshold + 1]
    } else {
        &rest[0]
    }
}

/// Whether an exact value fits a base kind. A base that names a type admits
/// only values of that type, so an `enum` member of another type is dropped by
/// the intersection rather than kept: `{ type: "boolean", allOf: [{ const: 1 }] }`
/// is `never`, and a mixed enum under a string base keeps its strings.
/// The JSON Schema keywords the port cannot lower, each with the pin's own
/// sentence for refusing it. A keyword that is absent from this table is one the
/// port either implements or safely ignores (`custom`, `minContains`), and the
/// difference matters: refusing a keyword the pin ignores would break documents
/// the pin accepts.
const UNSUPPORTED_KEYWORDS: [(&str, &str); 6] = [
    (
        "$dynamicRef",
        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"$dynamicRef\": Effect has no equivalent constraint.",
    ),
    (
        "contains",
        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"contains\": Effect has no equivalent constraint.",
    ),
    (
        "dependentRequired",
        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"dependentRequired\": Effect has no equivalent constraint.",
    ),
    (
        "dependentSchemas",
        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"dependentSchemas\": Effect has no equivalent constraint.",
    ),
    (
        "unevaluatedItems",
        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"unevaluatedItems\": Effect has no equivalent constraint.",
    ),
    (
        "unevaluatedProperties",
        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"unevaluatedProperties\": Effect has no equivalent constraint.",
    ),
];

/// The JSON Schema type a kind names, which is what an intersection compares.
///
/// Two kinds can name the same type and still differ in shape: a `Record` and a
/// `Struct` are both objects, and a `Tuple` and an `Array` are both arrays. That
/// is why an intersection cannot compare the kinds directly, and why the kinds
/// that name no single type answer `None`.
pub(crate) fn kind_type_name(kind: &SchemaKind) -> Option<&'static str> {
    match kind {
        SchemaKind::String => Some("string"),
        SchemaKind::Number => Some("number"),
        SchemaKind::Boolean => Some("boolean"),
        SchemaKind::Object | SchemaKind::Struct(_) | SchemaKind::Record(..) => Some("object"),
        SchemaKind::Array(_) | SchemaKind::Tuple(_) | SchemaKind::Rest(_) => Some("array"),
        SchemaKind::Opaque("null") => Some("null"),
        _ => None,
    }
}

fn value_fits_kind(kind: &SchemaKind, value: &Unknown) -> bool {
    match kind {
        SchemaKind::String => matches!(value, Unknown::Text(_)),
        SchemaKind::Number => matches!(value, Unknown::Number(_)),
        SchemaKind::Boolean => matches!(value, Unknown::Bool(_)),
        SchemaKind::Opaque("null") => matches!(value, Unknown::Null),
        // A base with no type of its own (the open JSON value, a literal union,
        // a struct) admits whatever the values are.
        _ => true,
    }
}

/// Raise a base kind, its checks and its leading annotations back into one kind.
/// The decode of the result is the base's decode plus every check's predicate,
/// so a rebuilt schema enforces what the schemas it was built from enforced.
fn rebuild(
    kind: SchemaKind,
    checks: Vec<Check>,
    annotation: Vec<(String, String)>,
) -> Schema<Unknown> {
    let mut out = new_kinded(kind);
    if !annotation.is_empty() {
        // The annotation rides the base, so a repr prints
        // `String.annotate(..).check(..)` and not `String.check(..).annotate(..)`.
        out = Schema::annotated_with(
            out,
            annotation
                .into_iter()
                .map(|(key, value)| (key, value))
                .collect(),
        );
    }
    for check in checks.iter() {
        // A filter group carries its members beside the rendered argument, and
        // they are what a decode enforces, so the group goes through the
        // constructor that knows about them.
        if check.group.is_empty() {
            out = Schema::checked(
                out,
                &check.name,
                check.args.iter().map(String::as_str).collect(),
                check
                    .annotation
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str()))
                    .collect(),
            );
        } else {
            out = Schema::checked_group(
                out,
                &check.name,
                check.args.iter().map(String::as_str).collect(),
                check
                    .annotation
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str()))
                    .collect(),
                &check.group,
            );
        }
    }
    out
}

/// A bare base kind with the decode that kind implies, for rebuilding a chain
/// that `decompose` took apart. Only the kinds an import can produce are here.
fn new_kinded(kind: SchemaKind) -> Schema<Unknown> {
    match kind {
        SchemaKind::String => Schema::as_unknown(Schema::string()),
        SchemaKind::Number => Schema::as_unknown(Schema::number()),
        SchemaKind::Boolean => Schema::as_unknown(Schema::boolean()),
        other => Schema::as_unknown(Schema::new(
            move |value| Ok(value),
            |value| Ok(value.clone()),
            other,
        )),
    }
}

/// Which side of a transform a Standard JSON Schema document describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandardJsonSchemaSide {
    Input,
    Output,
}

/// The `target` option of `~standard.jsonSchema.input/output`: only the tuple
/// prefix keyword and the definition slot differ between drafts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StandardJsonSchemaTarget {
    #[default]
    Draft2020_12,
    Draft07,
}

/// A schema carrying Standard JSON Schema metadata (`toStandardJSONSchemaV1`):
/// the same codec, plus `input`/`output` documents per target draft.
pub struct StandardJsonSchemaV1<T> {
    schema: Schema<T>,
}

/// Wrap a schema without changing its codec (`toStandardJSONSchemaV1`).
pub fn to_standard_json_schema_v1<T>(schema: Schema<T>) -> StandardJsonSchemaV1<T> {
    StandardJsonSchemaV1 { schema }
}

/// Something a Standard Schema V1 adapter can validate: a plain schema, or a
/// JSON-Schema-documented one (the pin composes both standards).
pub trait IntoV1Schema<T> {
    /// The schema validation runs against.
    fn into_v1_schema(self) -> Schema<T>;
}

impl<T> IntoV1Schema<T> for Schema<T> {
    fn into_v1_schema(self) -> Schema<T> {
        self
    }
}

impl<T> IntoV1Schema<T> for StandardJsonSchemaV1<T> {
    fn into_v1_schema(self) -> Schema<T> {
        self.schema
    }
}

/// Which issues `validate` returns: every issue, or only the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum V1Errors {
    /// Collect every issue (the default).
    #[default]
    All,
    /// Stop at the first issue.
    First,
}

/// One path segment of a Standard Schema V1 issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// An object key.
    Key(String),
    /// An array index.
    Index(usize),
}

/// One Standard Schema V1 issue: a message plus a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Issue {
    /// Human-readable failure message.
    pub message: String,
    /// Path to the failing value.
    pub path: Vec<PathSegment>,
}

/// Standard Schema V1 validation outcome: a value or a list of issues.
#[derive(Debug, Clone, PartialEq)]
pub enum V1Result<T> {
    /// Validation succeeded.
    Value(T),
    /// Validation failed.
    Issues(Vec<V1Issue>),
}

/// A schema adapted to Standard Schema V1 (`toStandardSchemaV1`): `validate`
/// answers `{ value }` or `{ issues }`, with per-issue messages and paths.
/// Async schemas, missing-dependency effects, and `message` annotation routing
/// need engine machinery this crate does not have; those rows stay excused.
pub struct StandardSchemaV1<T> {
    schema: Schema<T>,
    errors: V1Errors,
}

/// Adapt a schema (or a JSON-Schema-documented one) to Standard Schema V1.
pub fn to_standard_schema_v1<T, S: IntoV1Schema<T>>(inner: S) -> StandardSchemaV1<T> {
    StandardSchemaV1 {
        schema: inner.into_v1_schema(),
        errors: V1Errors::All,
    }
}

impl<T> StandardSchemaV1<T> {
    /// Only return the first issue (`parseOptions: { errors: "first" }`).
    pub fn with_errors(mut self, errors: V1Errors) -> Self {
        self.errors = errors;
        self
    }

    /// Validate `input`, answering a value or issues with messages and paths.
    pub fn validate(&self, input: Unknown) -> V1Result<T> {
        match self.schema.decode_collect(input) {
            Ok(value) => V1Result::Value(value),
            Err(errors) => {
                let mut issues = Vec::new();
                for error in errors {
                    flatten_issue(&error, &mut issues);
                    if self.errors == V1Errors::First {
                        break;
                    }
                }
                V1Result::Issues(issues.into_iter().map(|error| v1_issue(&error)).collect())
            }
        }
    }

    /// The input JSON Schema document for `target` (survives composition).
    pub fn input(&self, target: StandardJsonSchemaTarget) -> serde_json::Value
    where
        T: Send + Sync + 'static,
    {
        self.schema.standard_json_schema_input(target)
    }

    /// The output JSON Schema document for `target` (survives composition).
    pub fn output(&self, target: StandardJsonSchemaTarget) -> serde_json::Value
    where
        T: Send + Sync + 'static,
    {
        self.schema.standard_json_schema_output(target)
    }
}

/// Re-root an error path under `prefix` for every variant (`at` only covers
/// `TypeMismatch`/`DefaultFailed`; the collect walk needs all of them).
fn repath_under(error: SchemaError, prefix: &str) -> SchemaError {
    let join = |path: String| match path.strip_prefix('$') {
        Some(rest) => format!("{prefix}{rest}"),
        None => format!("{prefix}{path}"),
    };
    match error {
        SchemaError::TypeMismatch {
            path,
            expected,
            got,
        } => SchemaError::TypeMismatch {
            path: join(path),
            expected,
            got,
        },
        SchemaError::TypeMismatchDynamic {
            path,
            expected,
            got,
        } => SchemaError::TypeMismatchDynamic {
            path: join(path),
            expected,
            got,
        },
        SchemaError::DefaultFailed { path, message } => SchemaError::DefaultFailed {
            path: join(path),
            message,
        },
        SchemaError::InvalidJson { path, message } => SchemaError::InvalidJson {
            path: join(path),
            message,
        },
        SchemaError::EncodingFailed { path, expected } => SchemaError::EncodingFailed {
            path: join(path),
            expected,
        },
        SchemaError::FilterFailed { path, message } => SchemaError::FilterFailed {
            path: join(path),
            message,
        },
        SchemaError::Composite { issues } => SchemaError::Composite {
            issues: issues
                .into_iter()
                .map(|issue| repath_under(issue, prefix))
                .collect(),
        },
    }
}

/// A composite's members, in order; every other error is already one issue.
fn flatten_issue(error: &SchemaError, out: &mut Vec<SchemaError>) {
    match error {
        SchemaError::Composite { issues } => {
            for issue in issues {
                flatten_issue(issue, out);
            }
        }
        _ => out.push(error.clone()),
    }
}

/// A `SchemaError` as a Standard Schema issue: the pin's message with a
/// leading `Expected ` unless it already reads like a sentence.
fn v1_issue(error: &SchemaError) -> V1Issue {
    let message = match error {
        SchemaError::TypeMismatch { expected, .. } => sentence(expected),
        SchemaError::TypeMismatchDynamic { expected, .. } => sentence(expected),
        SchemaError::FilterFailed { message, .. }
        | SchemaError::DefaultFailed { message, .. }
        | SchemaError::InvalidJson { message, .. } => sentence(message),
        SchemaError::EncodingFailed { expected, .. } => {
            sentence(&format!("cannot encode {expected}"))
        }
        SchemaError::Composite { .. } => unreachable!("flattened before mapping"),
    };
    V1Issue {
        message,
        path: parse_v1_path(&error_path(error)),
    }
}

/// `Expected X` unless the message already starts uppercase.
fn sentence(message: &str) -> String {
    match message.chars().next() {
        Some(first) if first.is_ascii_lowercase() => format!("Expected {message}"),
        _ => message.to_owned(),
    }
}

/// The path a `SchemaError` carries (`$`, `$.a`, `$[0]` shapes).
fn error_path(error: &SchemaError) -> String {
    match error {
        SchemaError::TypeMismatch { path, .. }
        | SchemaError::TypeMismatchDynamic { path, .. }
        | SchemaError::DefaultFailed { path, .. }
        | SchemaError::InvalidJson { path, .. }
        | SchemaError::EncodingFailed { path, .. }
        | SchemaError::FilterFailed { path, .. } => path.clone(),
        SchemaError::Composite { .. } => unreachable!("flattened before mapping"),
    }
}

/// Split `$`, `$.a.b`, `$[0]`, `$.a[1].b` into segments.
fn parse_v1_path(path: &str) -> Vec<PathSegment> {
    let mut out = Vec::new();
    let mut rest = path.strip_prefix('$').unwrap_or(path);
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('.') {
            let end = tail.find(['.', '[']).unwrap_or(tail.len());
            out.push(PathSegment::Key(tail[..end].to_owned()));
            rest = &tail[end..];
        } else if let Some(tail) = rest.strip_prefix('[') {
            let end = tail.find(']').unwrap_or(tail.len());
            if let Ok(index) = tail[..end].parse::<usize>() {
                out.push(PathSegment::Index(index));
            }
            rest = tail.get(end + 1..).unwrap_or("");
        } else {
            break;
        }
    }
    out
}

impl<T> StandardJsonSchemaV1<T> {
    /// The wrapped schema: decode/encode behave exactly as before.
    pub fn schema(&self) -> &Schema<T> {
        &self.schema
    }

    /// The input document for `target` (default `draft-2020-12`).
    pub fn input(&self, target: StandardJsonSchemaTarget) -> serde_json::Value
    where
        T: Send + Sync + 'static,
    {
        self.schema.standard_json_schema_input(target)
    }

    /// The output document for `target`.
    pub fn output(&self, target: StandardJsonSchemaTarget) -> serde_json::Value
    where
        T: Send + Sync + 'static,
    {
        self.schema.standard_json_schema_output(target)
    }
}

/// Build the root document for one side, hoisting repeated `identifier`
/// annotations into the draft's definition slot.
fn standard_json_schema_document(
    kind: &SchemaKind,
    side: StandardJsonSchemaSide,
    target: StandardJsonSchemaTarget,
) -> serde_json::Value {
    let mut defs = serde_json::Map::new();
    let mut body = standard_doc_for_kind(kind, side, target, &mut defs);
    if !defs.is_empty()
        && let serde_json::Value::Object(ref mut map) = body
    {
        map.insert(
            match target {
                StandardJsonSchemaTarget::Draft2020_12 => "$defs",
                StandardJsonSchemaTarget::Draft07 => "definitions",
            }
            .to_string(),
            serde_json::Value::Object(defs),
        );
    }
    body
}

/// One kind as a Standard JSON Schema fragment. Checks and non-identifier
/// annotations ride the codec, not the document; an `identifier` hoists the
/// inner shape into the root definitions and emits a `$ref`.
fn standard_doc_for_kind(
    kind: &SchemaKind,
    side: StandardJsonSchemaSide,
    target: StandardJsonSchemaTarget,
    defs: &mut serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    match kind {
        SchemaKind::Annotated { pairs, inner } => {
            if let Some((_, id)) = pairs.iter().find(|(key, _)| key == "identifier") {
                let doc = standard_doc_for_kind(inner, side, target, defs);
                defs.entry(id.clone()).or_insert(doc);
                let reference = match target {
                    StandardJsonSchemaTarget::Draft2020_12 => format!("#/$defs/{id}"),
                    StandardJsonSchemaTarget::Draft07 => format!("#/definitions/{id}"),
                };
                return serde_json::json!({"$ref": reference});
            }
            standard_doc_for_kind(inner, side, target, defs)
        }
        SchemaKind::Check { base, .. } => standard_doc_for_kind(base, side, target, defs),
        // ponytail: only the transforms this port's rows use branch by side.
        SchemaKind::Opaque("finiteFromString") | SchemaKind::Opaque("numberFromString") => {
            match side {
                StandardJsonSchemaSide::Input => serde_json::json!({"type": "string"}),
                StandardJsonSchemaSide::Output => serde_json::json!({"type": "number"}),
            }
        }
        SchemaKind::String => serde_json::json!({"type": "string"}),
        SchemaKind::Number => serde_json::json!({"type": "number"}),
        SchemaKind::Boolean => serde_json::json!({"type": "boolean"}),
        SchemaKind::Tuple(elements) => {
            let items: Vec<serde_json::Value> = elements
                .iter()
                .map(|element| standard_doc_for_kind(element, side, target, defs))
                .collect();
            let count = items.len();
            let mut doc =
                serde_json::json!({"type": "array", "minItems": count, "maxItems": count});
            doc[match target {
                StandardJsonSchemaTarget::Draft2020_12 => "prefixItems",
                StandardJsonSchemaTarget::Draft07 => "items",
            }] = serde_json::Value::Array(items);
            doc
        }
        other => other.to_json_schema(),
    }
}

/// The wire shape of a schema: drives JSON Schema documents.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaKind {
    String,
    Number,
    Boolean,
    Object,
    Array(Box<SchemaKind>),
    Optional(Box<SchemaKind>),
    /// Named fields in declaration order (`Struct`).
    Struct(Vec<(String, SchemaKind)>),
    /// A string-keyed mapping (`Record`): key shape then value shape.
    Record(Box<SchemaKind>, Box<SchemaKind>),
    /// Positional elements (`Tuple`).
    Tuple(Vec<SchemaKind>),
    /// Absorbs the remaining elements of a `Tuple`.
    Rest(Box<SchemaKind>),
    /// A refinement (`check`): a predicate over the decoded value. The `base`
    /// is the schema being refined, so `Number.check(isFinite)` is
    /// `Check { base: Number, name: "isFinite", .. }`. The annotation carries
    /// the pairs the pin attaches, usually `expected` but sometimes
    /// `identifier` or `description`.
    Check {
        base: Box<SchemaKind>,
        name: String,
        args: Vec<String>,
        annotation: Vec<(String, String)>,
    },
    /// A combinator whose wire shape is not introspectable.
    Opaque(&'static str),
    /// Any JSON value (`Json`).
    Json,
    /// A struct field whose key may be absent (`optionalKey` shape). Distinct
    /// from `Optional`: `optional` also accepts an explicit null, and the
    /// pin's JSON Schema import emits `optionalKey` for a non-required
    /// property, so the two are not interchangeable in a repr.
    OptionalKey(Box<SchemaKind>),
    /// A struct plus a catch-all shape (`StructWithRest`): the named fields
    /// and the shape that absorbs every other key.
    StructWithRest(Vec<(String, SchemaKind)>, Box<SchemaKind>),
    /// Positional elements with a rest list (`TupleWithRest`): the leading
    /// positions, then the repeatable schema, then the fixed trailing
    /// positions the pin's rest list carries after the first entry.
    TupleWithRest(Vec<SchemaKind>, Vec<SchemaKind>),
    /// Several exact values (`Literals`), which is what an enum of plain
    /// scalars imports to. An enum containing null is a `Union` instead, because
    /// `null` is a schema of its own and not a literal.
    Literals(Vec<Unknown>),
    /// A named reference to a root definition (`$ref`). Its repr is the
    /// definition's NAME, not its shape: the pin keeps a reference a reference,
    /// so the schema prints as `A` and the definition prints once, separately,
    /// carrying `identifier: A`. Annotations riding the reference render as a
    /// `suspend` so a repr can still name the generic the pin names.
    Reference(String),
    /// A single exact value (`Literal`).
    Literal(Unknown),
    /// Template spans (`TemplateLiteral`): literal anchors and span shapes
    /// kept as full kinds so identifiers on a part survive the round trip.
    /// The parser (built from [`template_part_of_kind`]) drives decoding;
    /// parts the parser cannot spell (`Int`, `Digits`, `NonEmptyString`)
    /// ride as their base shape with the constraint in the parser only.
    TemplateLiteral(Vec<SchemaKind>),
    /// A union of two or more shapes (`Union`). `mode` is either `"anyOf"` or
    /// `"oneOf"`; they differ in whether a value matching more than one member
    /// is accepted, and the repr carries the difference.
    Union {
        members: Vec<SchemaKind>,
        mode: &'static str,
    },
    /// Annotation pairs wrapped around another shape. Generalized beyond
    /// title/description because the pin annotates with `expected`, `identifier`
    /// and `description` alike, and only `expected` is not a doc field.
    Annotated {
        pairs: Vec<(String, String)>,
        inner: Box<SchemaKind>,
    },
}

impl SchemaKind {
    /// The JSON Schema document for a struct's named fields: every field
    /// required, because a struct's repr cannot say otherwise. An import walks
    /// the other way, so `StructWithRest` composes this with its catch-all.
    fn struct_document(fields: &[(String, SchemaKind)]) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<serde_json::Value> = Vec::new();
        for (name, kind) in fields {
            properties.insert(name.clone(), kind.to_json_schema());
            required.push(serde_json::Value::String(name.clone()));
        }
        serde_json::json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
            "required": required,
        })
    }

    /// The document for a struct built from a runtime field list, whose fields
    /// live beside the kind rather than inside it. An `optionalKey` field is a
    /// property the `required` list leaves out, which is the only way a JSON
    /// Schema document can say the key may be absent while an explicit null
    /// still decodes.
    fn struct_document_from_fields(fields: &[Field]) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<serde_json::Value> = Vec::new();
        for field in fields {
            properties.insert(
                field.name.clone(),
                field.schema.repr_kind().to_json_schema(),
            );
            if field.optionality() == Optionality::Required {
                required.push(serde_json::Value::String(field.name.clone()));
            }
        }
        serde_json::json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
            "required": required,
        })
    }

    fn to_json_schema(&self) -> serde_json::Value {
        match self {
            SchemaKind::String => serde_json::json!({"type": "string"}),
            SchemaKind::Number => serde_json::json!({"type": "number"}),
            SchemaKind::Boolean => serde_json::json!({"type": "boolean"}),
            SchemaKind::Object => serde_json::json!({"type": "object"}),
            SchemaKind::Array(items) => {
                serde_json::json!({"type": "array", "items": items.to_json_schema()})
            }
            SchemaKind::Optional(inner) => {
                serde_json::json!({"anyOf": [inner.to_json_schema(), {"type": "null"}]})
            }
            SchemaKind::Struct(fields) => Self::struct_document(fields),
            SchemaKind::Record(key, _value) => {
                let mut doc = serde_json::json!({"type": "object"});
                // A checked key is a `patternProperties` key, which is the only
                // way a repr can say the key shape.
                if !matches!(&**key, SchemaKind::String) {
                    doc["patternProperties"] = key.to_json_schema();
                }
                doc
            }
            SchemaKind::Tuple(elements) => {
                let items: Vec<serde_json::Value> =
                    elements.iter().map(|k| k.to_json_schema()).collect();
                serde_json::json!({"type": "array", "prefixItems": items, "items": false})
            }
            SchemaKind::Rest(inner) => {
                serde_json::json!({"prefixItems": [inner.to_json_schema()]})
            }
            SchemaKind::Check { base, .. } => base.to_json_schema(),
            SchemaKind::Json => serde_json::json!({}),
            SchemaKind::Reference(_) => serde_json::json!({"$comment": "reference"}),
            SchemaKind::OptionalKey(inner) => inner.to_json_schema(),
            SchemaKind::StructWithRest(fields, rest) => {
                let mut doc = Self::struct_document(fields);
                doc["additionalProperties"] = rest.to_json_schema();
                doc
            }
            // JSON Schema cannot pin trailing positions after a variadic
            // segment, so the repeatable schema is all this document can say.
            // The pin refuses the whole document instead; the port keeps the
            // repeatable part and leaves the trailing positions open.
            SchemaKind::TupleWithRest(elements, rest) => {
                let mut doc = serde_json::json!({"type": "array"});
                doc["prefixItems"] = serde_json::Value::Array(
                    elements.iter().map(SchemaKind::to_json_schema).collect(),
                );
                doc["items"] = rest[0].to_json_schema();
                doc
            }
            SchemaKind::Literal(value) => value.to_json(),
            SchemaKind::Literals(values) => {
                serde_json::Value::Array(values.iter().map(Unknown::to_json).collect())
            }
            SchemaKind::Union { .. } => serde_json::json!({}),
            SchemaKind::TemplateLiteral(parts) => {
                match crate::json_schema::template_pattern_of_kinds(parts) {
                    Some(pattern) => {
                        serde_json::json!({"type": "string", "pattern": pattern})
                    }
                    None => serde_json::json!({"type": "string"}),
                }
            }
            // `never` is the one shape a JSON Schema document spells as a
            // complement: `{ not: {} }` admits nothing, which is what makes
            // an `optionalKey(Never)` field survive a document round trip.
            SchemaKind::Opaque("never") => serde_json::json!({"not": {}}),
            SchemaKind::Opaque(name) => serde_json::json!({"$comment": name}),
            SchemaKind::Annotated { pairs, inner } => {
                let mut doc = inner.to_json_schema();
                if let serde_json::Value::Object(ref mut map) = doc {
                    for (key, value) in pairs {
                        map.insert(key.clone(), serde_json::Value::String(value.clone()));
                    }
                }
                doc
            }
        }
    }
}

/// A decoded struct: field names in declaration order, values still untyped.
///
/// The pin's `Schema.Struct` decodes to a plain object, so the port keeps the
/// decoded form as an `Unknown` object rather than forcing a Rust struct. That
/// is what lets one `struct_` call serve any field set, which is what the
/// formatter rows need: they assert on the rendering, not on a Rust type.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StructValue {
    fields: Vec<(String, Unknown)>,
}

impl StructValue {
    /// A field's decoded value, or `None` when the name was never declared.
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

    /// The decoded value back as an `Unknown` object.
    pub fn to_unknown(&self) -> Unknown {
        Unknown::Object(self.fields.iter().cloned().collect())
    }
}

impl Schema<StructValue> {
    /// A struct of `field -> value` schemas (`Schema.Struct`). Each field
    /// decodes through its own schema; a missing or extra field is a
    /// `TypeMismatch` naming the path. The field schemas are kept as
    /// `children` so a formatter can recurse into them, which is what the
    /// pin's nested `Struct` rows assert.
    pub fn struct_<const N: usize>(fields: [(&str, Schema<Unknown>); N]) -> Self {
        Self::struct_of(
            fields
                .into_iter()
                .map(|(name, schema)| (name.to_string(), schema))
                .collect(),
        )
    }

    /// The same struct shape with a field list built at runtime, which is what
    /// an imported JSON Schema document needs: its field count is not known
    /// until the document is read.
    pub fn struct_of(fields: Vec<(String, Schema<Unknown>)>) -> Self {
        let schemas: Vec<(String, Arc<Schema<Unknown>>)> = fields
            .into_iter()
            .map(|(name, schema)| (name, Arc::new(schema)))
            .collect();
        let expected: Vec<String> = schemas.iter().map(|(name, _)| name.clone()).collect();
        let decode_fields = schemas.clone();
        let kind = SchemaKind::Struct(
            schemas
                .iter()
                .map(|(name, schema)| (name.clone(), schema.repr_kind()))
                .collect(),
        );
        let mut schema = Self::new(
            move |unknown| {
                let object = match unknown {
                    Unknown::Object(map) => map,
                    other => return mismatch("$", "object", &other),
                };
                for name in &expected {
                    if !object.contains_key(name) {
                        return Err(SchemaError::TypeMismatch {
                            path: format!("$.{name}"),
                            expected: "a declared field",
                            got: "absent",
                        });
                    }
                }
                let mut decoded = Vec::with_capacity(decode_fields.len());
                for (name, field) in &decode_fields {
                    let value = object.get(name).cloned().unwrap_or(Unknown::Null);
                    match field.decode(value) {
                        Ok(value) => decoded.push((name.clone(), value)),
                        Err(error) => return Err(error),
                    }
                }
                Ok(StructValue { fields: decoded })
            },
            move |value| {
                let object: BTreeMap<String, Unknown> = value.fields.iter().cloned().collect();
                Ok(Unknown::Object(object))
            },
            kind,
        );
        schema.children = schemas.clone();
        let children = schemas;
        schema.formatter = Some(std::sync::Arc::new(move |value: &Unknown| {
            // A Struct with NO declared fields falls back to the plain
            // JSON-ish rendering, which is what the pin does: its "empty" row
            // asserts `{"a":1}` and `[1,2,3]` unspaced. Only a Struct whose
            // fields are declared takes the spaced path.
            if children.is_empty() {
                return format_value(value);
            }
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = children
                .iter()
                .map(|(name, field)| {
                    let item = map.get(name).cloned().unwrap_or(Unknown::Null);
                    let key = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}: {}", field.formatter()(&item))
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }));
        schema
    }
}

/// A schema with the parse options it was created with
/// (`decodeUnknownExit`'s creation-time options). A per-call option still wins
/// for that call, and the next call without one goes back to the bound options.
pub struct Decoder<T> {
    schema: Schema<T>,
    options: transform::ParseOptions,
}

impl<T> Decoder<T> {
    /// Decode with the bound options.
    pub fn decode(&self, input: Unknown) -> Result<T, SchemaError> {
        self.schema.decode_with(input, &self.options)
    }

    /// Decode with options for this call only.
    pub fn decode_with(
        &self,
        input: Unknown,
        options: &transform::ParseOptions,
    ) -> Result<T, SchemaError> {
        self.schema.decode_with(input, options)
    }

    /// The options this decoder was created with.
    pub fn options(&self) -> transform::ParseOptions {
        self.options
    }
}

/// The encoding twin of [`Decoder`]: a schema with the options it was created
/// with, overridable for one call.
pub struct Encoder<T> {
    schema: Schema<T>,
    options: transform::ParseOptions,
}

impl<T> Encoder<T> {
    /// Encode with the bound options.
    pub fn encode(&self, value: &T) -> Result<Unknown, SchemaError> {
        self.schema.encode_with(value, &self.options)
    }

    /// Encode with options for this call only.
    pub fn encode_with(
        &self,
        value: &T,
        options: &transform::ParseOptions,
    ) -> Result<Unknown, SchemaError> {
        self.schema.encode_with(value, options)
    }

    /// The options this encoder was created with.
    pub fn options(&self) -> transform::ParseOptions {
        self.options
    }
}

impl<T> Schema<T> {
    pub(crate) fn new(
        decode: impl Fn(Unknown) -> Result<T, SchemaError> + Send + Sync + 'static,
        encode: impl Fn(&T) -> Result<Unknown, SchemaError> + Send + Sync + 'static,
        kind: SchemaKind,
    ) -> Self {
        Self {
            decode: std::sync::Arc::new(decode),
            encode: std::sync::Arc::new(encode),
            kind,
            children: Vec::new(),
            formatter: None,
            equivalence: None,
            wrapped: None,
            check_view: None,
            construct: None,
            constructor_default: None,
            decoding_default: None,
            omit_on_encode: false,
            struct_fields: None,
            tuple_elements: None,
            union_members: None,
            array_element: None,
            option_value: None,
            result_sides: None,
            key_annotations: Vec::new(),
            class_meta: None,
        }
    }

    /// Attach the class identity a `Class`-family schema carries. Crate-private:
    /// `crate::class` is the only place a class schema is assembled.
    pub(crate) fn with_class_meta(mut self, meta: crate::class::ClassMeta) -> Self {
        self.class_meta = Some(Box::new(meta));
        self
    }

    /// Install the relation `to_equivalence` answers with. Each constructor
    /// calls this with the rule its own shape implies, reading its children's
    /// relations as it builds them.
    pub(crate) fn with_equivalence(mut self, relation: Equivalence<T>) -> Self {
        self.equivalence = Some(relation);
        self
    }

    /// The relation on this schema's values (`Schema.toEquivalence`): a struct's
    /// field-by-field walk, an array's element walk, a union's member
    /// selection, a Record's key-filtered entries. A schema whose constructor
    /// installed a richer rule (an `overrideToEquivalence`, a class's field
    /// list) answers with it instead.
    ///
    /// A schema built with no rule of its own compares the two values' wire
    /// forms, which is all a shape with no comparison to make can say: an
    /// imported reference, a template literal, a schema adapted by `convert`.
    pub fn to_equivalence(&self) -> Equivalence<T>
    where
        T: 'static,
    {
        match &self.equivalence {
            Some(relation) => relation.clone(),
            None => {
                let encode = self.encode.clone();
                Arc::new(
                    move |a: &T, b: &T| matches!((encode(a), encode(b)), (Ok(x), Ok(y)) if x == y),
                )
            }
        }
    }

    /// `overrideToEquivalence`: replace this schema's derived relation with
    /// `build()`'s. The pin spells the override as a function-valued
    /// annotation and calls it once behind a `memoize`; this port's `annotate`
    /// carries string pairs only, so the factory form is its own constructor
    /// and is called once, here.
    pub fn override_to_equivalence(
        mut schema: Schema<T>,
        build: impl Fn() -> Equivalence<T> + Send + Sync + 'static,
    ) -> Schema<T> {
        schema.equivalence = Some(build());
        schema
    }

    /// The construction kernel, for the class builder: a class constructs
    /// through its source struct so constructor defaults fill, and the class's
    /// own wrapper runs the discriminant and excess policies first.
    pub(crate) fn with_construct(
        mut self,
        construct: Arc<dyn Fn(Unknown) -> Result<T, SchemaError> + Send + Sync>,
    ) -> Self {
        self.construct = Some(construct);
        self
    }

    /// `annotateKey`: key-level annotations, read back by
    /// [`resolve_annotations_key`]. They ride the codec untouched, so a struct
    /// using this schema as a field keeps them at the occurrence.
    ///
    /// [`resolve_annotations_key`]: Self::resolve_annotations_key
    pub fn annotate_key<S: Into<String>>(schema: Schema<T>, pairs: Vec<(S, S)>) -> Schema<T> {
        let mut out = schema;
        out.key_annotations = pairs
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        out
    }

    /// `resolveAnnotationsKey`: this node's key annotations, or `None` when it
    /// carries none. The pin reads `ast.context?.annotations`, so the lookup
    /// does not walk into a base or a check.
    pub fn resolve_annotations_key(&self) -> Option<Vec<(String, String)>> {
        (!self.key_annotations.is_empty()).then(|| self.key_annotations.clone())
    }

    /// `resolveAnnotations`: a schema that has checks answers with the LAST
    /// check's annotations, and every other schema answers with its own. A
    /// check that carries none leaves the answer `None` rather than falling
    /// back to the base, which is the pin's rule and the reason an annotated
    /// base under an unannotated check resolves to nothing.
    pub fn resolve_annotations(&self) -> Option<Vec<(String, String)>> {
        let mut node = self.repr_kind();
        let mut own: Option<Vec<(String, String)>> = None;
        loop {
            match node {
                // An annotation is not a check, so the walk keeps going: only
                // a check stops it.
                SchemaKind::Annotated { pairs, inner } => {
                    if own.is_none() && !pairs.is_empty() {
                        own = Some(pairs);
                    }
                    node = *inner;
                }
                // The outermost check is the last one applied.
                SchemaKind::Check { annotation, .. } => {
                    return if annotation.is_empty() {
                        None
                    } else {
                        Some(annotation)
                    };
                }
                _ => return own,
            }
        }
    }

    /// The JSON Schema document describing this schema's wire shape.
    pub fn json_schema(&self) -> serde_json::Value {
        // A struct built from a runtime field list carries `Object` as its kind
        // and the fields beside it, so its document is written from the
        // declared list: an `optionalKey` field is a property `required`
        // leaves out, which is what makes the key absent-or-null.
        if self.kind == SchemaKind::Object {
            if let Some(fields) = &self.struct_fields {
                return SchemaKind::struct_document_from_fields(fields);
            }
        }
        self.kind.to_json_schema()
    }

    /// The input-side Standard JSON Schema document (`~standard.jsonSchema.input`):
    /// the shape `decode` accepts. Only transforms differ from [`json_schema`](Self::json_schema).
    pub fn standard_json_schema_input(&self, target: StandardJsonSchemaTarget) -> serde_json::Value
    where
        T: Send + Sync + 'static,
    {
        standard_json_schema_document(&self.repr_kind(), StandardJsonSchemaSide::Input, target)
    }

    /// The output-side Standard JSON Schema document (`~standard.jsonSchema.output`):
    /// the shape `encode` produces. Only transforms differ from `input`.
    pub fn standard_json_schema_output(&self, target: StandardJsonSchemaTarget) -> serde_json::Value
    where
        T: Send + Sync + 'static,
    {
        standard_json_schema_document(&self.repr_kind(), StandardJsonSchemaSide::Output, target)
    }

    /// The declared fields of a struct shape, in declaration order, or `None`
    /// for every other shape.
    pub fn fields(&self) -> Option<&[Field]> {
        self.struct_fields.as_deref()
    }

    /// The declared elements of a tuple shape, in position order, or `None` for
    /// every other shape.
    pub fn elements(&self) -> Option<&[Schema<Unknown>]> {
        self.tuple_elements.as_deref()
    }

    /// The exact value a `Literal` shape admits, or `None` for every other
    /// shape. A derivation that rebuilds a member list needs it: the member it
    /// receives is the `Literal`, and the value is not in the wire shape.
    pub fn literal_value(&self) -> Option<&Unknown> {
        match &self.kind {
            SchemaKind::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// The declared members of a union shape (`Union.members`). An exact-value
    /// shape (`Literals`) answers one `Literal` member per value, which is how
    /// the pin builds its list. `None` for every other shape.
    pub fn members(&self) -> Option<Vec<Schema<Unknown>>> {
        // Read through a re-typing: `as_unknown` keeps the member list, so a
        // wrapped union must answer with it rather than with the wrapper's own
        // opaque kind.
        match self.repr_kind() {
            SchemaKind::Union { .. } => self.union_members.clone(),
            SchemaKind::Literals(values) => {
                Some(values.into_iter().map(Schema::<Unknown>::literal).collect())
            }
            _ => None,
        }
    }

    /// Decode an unknown value, or a typed error naming the path.
    pub fn decode(&self, unknown: Unknown) -> Result<T, SchemaError> {
        (self.decode)(unknown)
    }

    /// Encode, or the reason this schema cannot represent the value.
    pub fn try_encode(&self, value: &T) -> Result<Unknown, SchemaError> {
        (self.encode)(value)
    }

    /// Decode under explicit parse options (`decodeUnknown` with options):
    /// `errors` decides how many issues a composite reports, and
    /// `on_excess_property` what happens to keys the schema does not model.
    pub fn decode_with(
        &self,
        input: Unknown,
        options: &transform::ParseOptions,
    ) -> Result<T, SchemaError> {
        options.scoped(|| self.decode(input))
    }

    /// Construction under explicit parse options (`make` with options).
    pub fn construct_with(
        &self,
        input: Unknown,
        options: &transform::ParseOptions,
    ) -> Result<T, SchemaError> {
        options.scoped(|| self.construct(input))
    }

    /// Encode under explicit parse options (`encodeUnknown` with options).
    pub fn encode_with(
        &self,
        value: &T,
        options: &transform::ParseOptions,
    ) -> Result<Unknown, SchemaError> {
        options.scoped(|| self.try_encode(value))
    }

    /// Bind this schema to the options a decoder is created with
    /// (`decodeUnknown` with creation-time options).
    pub fn decoder(self, options: transform::ParseOptions) -> Decoder<T> {
        Decoder {
            schema: self,
            options,
        }
    }

    /// Bind this schema to the options an encoder is created with.
    pub fn encoder(self, options: transform::ParseOptions) -> Encoder<T> {
        Encoder {
            schema: self,
            options,
        }
    }

    /// Decode collecting every issue: struct-shaped schemas with named
    /// children accumulate one issue per failing field (missing keys refuse
    /// like the pin's own missing-key check); everything else yields at most
    /// one. `decode` stays fail-fast; this is the pin's `errors: "all"`
    /// reading, and [`Self::construct_collect`] and
    /// [`Schema::encode_collect`] complete the trio.
    pub fn decode_collect(&self, input: Unknown) -> Result<T, Vec<SchemaError>> {
        self.collect(input, CollectOp::Decode)
    }

    /// Build collecting every issue: the `make` reading of
    /// [`Self::decode_collect`], so a struct reports one issue per field
    /// construction refuses instead of stopping at the first.
    pub fn construct_collect(&self, input: Unknown) -> Result<T, Vec<SchemaError>> {
        self.collect(input, CollectOp::Construct)
    }

    fn collect(&self, input: Unknown, op: CollectOp) -> Result<T, Vec<SchemaError>> {
        if !self.children.is_empty() {
            if let Unknown::Object(map) = &input {
                let mut issues = Vec::new();
                for (name, field) in &self.children {
                    match map.get(name) {
                        Some(value) => {
                            if let Err(errors) = field.collect(value.clone(), op) {
                                for error in errors {
                                    issues.push(repath_under(error, &format!("$.{name}")));
                                }
                            }
                        }
                        None => issues.push(SchemaError::TypeMismatch {
                            path: format!("$.{name}"),
                            expected: "a declared field",
                            got: "absent",
                        }),
                    }
                }
                if !issues.is_empty() {
                    return Err(issues);
                }
            }
        } else if let Some(fields) = &self.struct_fields {
            if let Unknown::Object(map) = &input {
                let mut issues = Vec::new();
                for field in fields {
                    match map.get(&field.name) {
                        None if field.decoding_default.is_some()
                            || field.schema.decoding_default.is_some() =>
                        {
                            let default = field
                                .decoding_default
                                .as_ref()
                                .or(field.schema.decoding_default.as_ref())
                                .expect("checked just above");
                            let path = format!("$.{}", field.name);
                            if let Err(error) = field.schema.absent_value(default, &path) {
                                issues.push(error);
                            }
                        }
                        None if field.optionality != Optionality::Required => {}
                        Some(Unknown::Null) if field.optionality == Optionality::Optional => {}
                        None => issues.push(SchemaError::TypeMismatch {
                            path: format!("$.{}", field.name),
                            expected: "required field",
                            got: "missing",
                        }),
                        Some(value) => {
                            if let Err(errors) = field.schema.collect(value.clone(), op) {
                                for error in errors {
                                    issues.push(repath_under(error, &format!("$.{}", field.name)));
                                }
                            }
                        }
                    }
                }
                if !issues.is_empty() {
                    return Err(issues);
                }
            }
        }
        match op {
            CollectOp::Decode => self.decode(input).map_err(|error| vec![error]),
            CollectOp::Construct => self.construct(input).map_err(|error| vec![error]),
        }
    }

    /// Build a value (`make`'s path): the same validation as [`decode`],
    /// except that a composite fills its constructor defaults and constructs a
    /// present nested composite recursively. A schema with no construction
    /// kernel decodes unchanged, so this differs observably only where a
    /// `withConstructorDefault` is involved.
    ///
    /// [`decode`]: Self::decode
    pub fn construct(&self, unknown: Unknown) -> Result<T, SchemaError> {
        match &self.construct {
            Some(construct) => construct(unknown),
            None => (self.decode)(unknown),
        }
    }

    /// `makeOption`: construction as an `Option`, `None` when it would fail.
    ///
    /// A constructor default still only fills at a field occurrence, so asking
    /// a defaulted schema to make an absent value answers `None` instead of
    /// running the default. That is the pin's rule and the reason this is not
    /// `construct` with a default fallback.
    pub fn make_option(&self, unknown: Unknown) -> Option<T> {
        self.construct(unknown).ok()
    }

    /// Decode JSON text through the unknown representation.
    pub fn decode_json(&self, json: &str) -> Result<T, SchemaError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| SchemaError::InvalidJson {
                path: "$".to_string(),
                message: error.to_string(),
            })?;
        self.decode(Unknown::from_json(value))
    }

    /// Encode to canonical JSON text (total when decode is total).
    pub fn encode_json(&self, value: &T) -> Result<String, SchemaError> {
        serde_json::to_string(&self.try_encode(value)?.to_json()).map_err(|error| {
            SchemaError::InvalidJson {
                path: "$".to_string(),
                message: error.to_string(),
            }
        })
    }

    /// Wrap a schema with a title and/or description (`annotation`
    /// shapes): the wire shape is unchanged; the JSON Schema document
    /// carries the metadata.
    pub fn annotated(schema: Schema<T>, title: Option<&str>, description: Option<&str>) -> Schema<T>
    where
        T: Send + Sync + 'static,
    {
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Some(title) = title {
            pairs.push(("title".to_string(), title.to_string()));
        }
        if let Some(description) = description {
            pairs.push(("description".to_string(), description.to_string()));
        }
        Self::annotated_with(schema, pairs)
    }

    /// Annotations under their own keys, for the ones that are not a doc field.
    /// The pin's `expected` on a JSON catch-all is the case that needs this.
    pub fn annotated_with<S: Into<String>>(schema: Schema<T>, pairs: Vec<(S, S)>) -> Schema<T>
    where
        T: Send + Sync + 'static,
    {
        let kind = SchemaKind::Annotated {
            pairs: pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
            inner: Box::new(schema.repr_kind()),
        };
        Schema {
            decode: schema.decode,
            encode: schema.encode,
            kind,
            children: schema.children,
            array_element: schema.array_element,
            // An annotation rides the same codec, so the accessors a boxed
            // schema answers must survive it.
            option_value: schema.option_value.clone(),
            result_sides: schema.result_sides.clone(),
            formatter: schema.formatter.clone(),
            // An annotation rides the same codec, so the relation underneath is
            // still the one for these values: an `overrideToEquivalence` applied
            // before the annotation must survive it.
            equivalence: schema.equivalence.clone(),
            // An annotation rides the same codec, so a constructor default on
            // the schema underneath still applies to construction.
            construct: schema.construct.clone(),
            constructor_default: schema.constructor_default.clone(),
            // The wrapped reprs describe the schema underneath, so carrying them
            // would make `to_code` answer for the unwrapped schema and drop the
            // annotation from the repr.
            wrapped: None,
            // An annotation rides the same codec, so a check on top of it reads
            // exactly what a check underneath it would have read.
            check_view: schema.check_view.clone(),
            // A decoding default and its encoding strategy ride the codec, so an
            // annotated schema must keep answering for them.
            decoding_default: schema.decoding_default.clone(),
            omit_on_encode: schema.omit_on_encode,
            struct_fields: schema.struct_fields.clone(),
            tuple_elements: schema.tuple_elements.clone(),
            union_members: schema.union_members.clone(),
            // An annotation wraps the codec, so the key annotations underneath
            // must survive the wrapping or a field occurrence would lose them.
            key_annotations: schema.key_annotations.clone(),
            // An annotation over a class schema keeps the class: the pin's
            // `A.annotate({})` is still a schema with fields and a `make`.
            class_meta: schema.class_meta.clone(),
        }
    }

    /// Re-type a schema to `Unknown` through itself: decode validates
    /// and normalizes, then the value returns to unknown form. This is
    /// how heterogeneous struct fields share one `Field` type.
    pub fn as_unknown(schema: Schema<T>) -> Schema<Unknown>
    where
        T: Send + Sync + 'static,
    {
        Self::as_unknown_shared(Arc::new(schema))
    }

    /// [`as_unknown`](Self::as_unknown) for a schema already behind an `Arc`,
    /// which is the only way to re-type a `Schema<T>` when `T: Clone` does not
    /// hold: this crate's `Clone` is derived, so cloning the schema itself
    /// would demand the value type be cloneable. The array's `value` accessor
    /// needs exactly that.
    pub(crate) fn as_unknown_shared(schema: Arc<Schema<T>>) -> Schema<Unknown>
    where
        T: Send + Sync + 'static,
    {
        // Capture the wrapped schema's reprs BEFORE it is shared below:
        // re-typing it here would recurse forever. The reprs go through
        // `repr_kind`/`to_code`/`to_type` rather than the raw fields, so
        // re-typing an already re-typed schema keeps the shape underneath
        // instead of recording the `Opaque("unknown")` marker as its shape. A
        // nested `Array` of a re-typed child renders `Schema.Array(Schema.Unknown)`
        // otherwise.
        let wrapped = Some(Box::new(Wrapped {
            kind: schema.repr_kind(),
            children: schema.children.clone(),
            code: schema.to_code(),
            ty: schema.to_type(),
        }));
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        let construct_schema = schema.clone();
        let inner_render = schema.formatter.clone();
        let inner_kind = schema.repr_kind();
        let inner_children = schema.children.clone();
        let bridge = schema.clone();
        // A decoding default survives the re-typing in the form this level
        // speaks, which is the wire form: this wrapper's own decode is the
        // inner encode, so a decoded default crosses over the same way.
        let inner_default = schema
            .decoding_default
            .as_ref()
            .map(|default| match default {
                SchemaDefault::Wire(produce) => SchemaDefault::Wire(produce.clone()),
                SchemaDefault::Decoded(produce) => {
                    let produce = produce.clone();
                    SchemaDefault::Wire(Arc::new(move || {
                        produce().and_then(|value| bridge.try_encode(&value))
                    }))
                }
            });
        let mut out = Schema::new(
            move |unknown| {
                decode_schema
                    .decode(unknown)
                    .and_then(|value| encode_schema.try_encode(&value))
            },
            |value| Ok(value.clone()),
            SchemaKind::Opaque("unknown"),
        );
        // A re-typing does not change the shape, so the wrapper renders exactly
        // the way the schema it wraps does. Without this the nested Struct rows
        // lose their spacing, because every field has to pass through
        // as_unknown to share one Unknown type.
        out.formatter = Some(Arc::new(move |value: &Unknown| match &inner_render {
            Some(render) => render(value),
            None => format_root(&inner_kind, &inner_children, value),
        }));
        out.wrapped = wrapped;
        // The re-typing shares the inner codec's view of a value, so a check
        // built on top of it reads what the schema underneath reads.
        out.check_view = schema.check_view.clone();
        // The re-typing shares the inner codec, so it must share the inner
        // construction too: a field schema wrapped as `Unknown` is exactly
        // where `make` looks for a constructor default.
        out.constructor_default = schema.constructor_default.clone();
        // The same for a decoding default: it is a property of the codec, and
        // the struct walk looks for it through the re-typing.
        out.decoding_default = inner_default;
        out.omit_on_encode = schema.omit_on_encode;
        // A key annotation is read at the occurrence the schema is used at, and
        // every struct field goes through this re-typing.
        out.key_annotations = schema.key_annotations.clone();
        // A class field is used through the re-typing too, so the class's
        // identity has to survive it or a nested class field would not
        // recognize an instance.
        out.class_meta = schema.class_meta.clone();
        // A nested struct must stay reachable for accumulating validation:
        // `decode_collect` recurses through `children`, and every struct
        // field passes through this re-typing.
        out.children = schema.children.clone();
        // A derivation needs the field list through the re-typing too, which is
        // how every struct field is spelled.
        out.struct_fields = schema.struct_fields.clone();
        out.tuple_elements = schema.tuple_elements.clone();
        out.union_members = schema.union_members.clone();
        out.construct = Some(Arc::new(move |unknown| {
            let built = construct_schema.construct(unknown)?;
            construct_schema.try_encode(&built)
        }));
        out
    }

    /// The kind a repr should render, seeing through a re-typing. `as_unknown`
    /// keeps its own kind opaque on purpose, so a constructor that copies a
    /// child's kind (`record`, `checked`) must copy the one underneath or the
    /// repr prints `Schema.Unknown` for a perfectly visible child.
    /// Swap a schema's kind, keeping the decode, encode, children and formatter.
    /// Only for a kind that describes exactly the same wire shape, which is the
    /// case for a record whose key is checked rather than passed through.
    pub(crate) fn with_kind(mut self, kind: SchemaKind) -> Self {
        self.kind = kind;
        self.wrapped = None;
        self
    }

    pub(crate) fn repr_kind(&self) -> SchemaKind {
        match &self.wrapped {
            Some(wrapped) => wrapped.kind.clone(),
            None => self.kind.clone(),
        }
    }

    /// `{ encodingStrategy: "omit" }`: encoding leaves out the key this schema
    /// fills back in, so a decoding default does not travel outwards.
    pub fn omit_encoding(mut self) -> Self {
        self.omit_on_encode = true;
        self
    }

    /// What an absent input decodes to: the default as given, wire values
    /// through this schema, or the default's own failure rooted at `path`.
    fn absent_value(&self, default: &SchemaDefault<T>, path: &str) -> Result<T, SchemaError> {
        match default {
            SchemaDefault::Wire(produce) => {
                let wire = produce().map_err(|error| error.at(path))?;
                self.decode(wire).map_err(|error| error.at(path))
            }
            SchemaDefault::Decoded(produce) => produce().map_err(|error| error.at(path)),
        }
    }

    /// Add a constructor default (`withConstructorDefault`): the closure runs
    /// when the value is absent at a field occurrence, a missing object key or
    /// an array slot past the end, and only on the construction path
    /// (`make`). Decoding and encoding ignore it. The produced value is then
    /// constructed through this same schema, so a defaulted `Struct` fills its
    /// own nested defaults before it is handed back.
    pub fn with_constructor_default(
        mut self,
        default: impl Fn() -> Result<Unknown, SchemaError> + Send + Sync + 'static,
    ) -> Self {
        self.constructor_default = Some(std::sync::Arc::new(default));
        self
    }

    /// A lazily-built schema for recursive shapes (`suspend` shape): the
    /// factory resolves once, on the first call that needs it.
    pub fn suspend<U: Clone + Send + Sync + 'static>(
        factory: impl Fn() -> Schema<U> + Send + Sync + 'static,
    ) -> Schema<U> {
        // The pin's `Suspend` memoizes its thunk on the first call
        // (SchemaAST.ts:3863), and its rows count the factory's calls rather
        // than trusting them: a union whose winner short-circuits never
        // resolves the member at all, and two decodes that both reach it
        // resolve it exactly once. Resolving per call diverges on the second
        // count, and would rebuild the whole recursive subtree per value node.
        let resolved: Arc<std::sync::OnceLock<Schema<U>>> = Arc::new(std::sync::OnceLock::new());
        let factory = Arc::new(factory);
        let decode_resolved = Arc::clone(&resolved);
        let decode_factory = Arc::clone(&factory);
        let encode_resolved = Arc::clone(&resolved);
        let encode_factory = Arc::clone(&factory);
        let render_resolved = Arc::clone(&resolved);
        let render_factory = Arc::clone(&factory);
        let mut out = Schema::new(
            move |unknown| {
                decode_resolved
                    .get_or_init(|| decode_factory())
                    .decode(unknown)
            },
            move |value| {
                encode_resolved
                    .get_or_init(|| encode_factory())
                    .try_encode(value)
            },
            SchemaKind::Opaque("suspend"),
        );
        // Rendering resolves on the same cell, so a recursive shape terminates
        // on the value's depth: the resolved schema is reached again through
        // its own formatter, never through a second factory call.
        out.formatter = Some(Arc::new(move |value| {
            render_resolved.get_or_init(|| render_factory()).formatter()(value)
        }));
        out
    }

    /// Check a decoded value with a predicate (`refinement` shape): a
    /// failing predicate is a typed mismatch naming the check.
    pub fn refine(
        self,
        check: &'static str,
        predicate: impl Fn(&T) -> bool + Send + Sync + 'static,
    ) -> Self
    where
        T: Send + Sync + 'static,
    {
        let inner = Arc::new(self);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| {
                let value = decode_inner.decode(unknown)?;
                if predicate(&value) {
                    Ok(value)
                } else {
                    Err(SchemaError::TypeMismatch {
                        path: "$".to_string(),
                        expected: check,
                        got: "a value failing the check",
                    })
                }
            },
            move |value| encode_inner.try_encode(value),
            SchemaKind::Opaque("refinement"),
        )
    }

    /// `decodeTo`: `from`'s wire form, this schema's value type, and a pair
    /// that may fail in either direction.
    ///
    /// [`convert`](Self::convert) is the total version of this, and the reason
    /// this one exists: a transform whose encode can refuse (`ByteSizeFromNumber`
    /// refusing a byte count with no safe-integer wire form) cannot use it.
    /// `from` stays the wire side, so a failure on the way in is still reported
    /// at the schema that refused the value.
    ///
    /// The value is cloned on the way out because a `Transformation` owns what
    /// it is handed, while `try_encode` only lends its value.
    pub fn decode_to<A>(
        from: Schema<A>,
        transformation: crate::transform::Transformation<A, T>,
    ) -> Schema<T>
    where
        A: Send + Sync + 'static,
        T: Clone + Send + Sync + 'static,
    {
        let kind = from.repr_kind();
        let transformation = std::sync::Arc::new(transformation);
        let decode_step = transformation.clone();
        let encode_step = transformation;
        let from = std::sync::Arc::new(from);
        let decode_from = from.clone();
        let encode_from = from;
        Schema::new(
            move |unknown| {
                decode_from
                    .decode(unknown)
                    .and_then(|wire| decode_step.forward(wire))
            },
            move |value| {
                encode_step
                    .backward(value.clone())
                    .and_then(|wire| encode_from.try_encode(&wire))
            },
            kind,
        )
    }

    /// Adapt a schema to a new type through a total isomorphism pair.
    pub fn convert<A, D, E>(schema: Schema<A>, decode: D, encode: E) -> Schema<T>
    where
        A: Send + Sync + 'static,
        T: Send + Sync + 'static,
        D: Fn(A) -> Result<T, SchemaError> + Send + Sync + 'static,
        E: Fn(&T) -> A + Send + Sync + 'static,
    {
        let kind = schema.repr_kind();
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        Schema::new(
            move |unknown| decode_schema.decode(unknown).and_then(&decode),
            move |value| encode_schema.try_encode(&encode(value)),
            kind,
        )
    }
}

impl<T: Send + Sync + 'static> Schema<T> {
    /// `Schema.fromJsonString`: the wire form is JSON text, the value is
    /// whatever `inner` decodes. A failure inside the parsed document keeps the
    /// inner schema's own error, so a nested occurrence reads at the path the
    /// caller expects; only the text itself, invalid JSON, is this schema's.
    pub fn from_json_string(inner: Schema<T>) -> Self {
        Self::from_json_string_with(inner, |_, value| value, |_, value| Some(value), None)
    }

    /// [`from_json_string`](Self::from_json_string) with the pin's options:
    /// a `reviver` run over every parsed value before decoding, a `replacer`
    /// run over every encoded value before printing (`None` drops the key, the
    /// pin's `undefined`), and `space` indentation.
    ///
    /// The hooks are called with the key path's last segment (`""` at the
    /// root); the replacer sees each object's own values, not a re-walked tree.
    pub fn from_json_string_with<RV, RP>(
        inner: Schema<T>,
        reviver: RV,
        replacer: RP,
        space: Option<u8>,
    ) -> Self
    where
        RV: Fn(&str, Unknown) -> Unknown + Send + Sync + 'static,
        RP: Fn(&str, Unknown) -> Option<Unknown> + Send + Sync + 'static,
    {
        let inner = Arc::new(inner);
        let decode_inner = Arc::clone(&inner);
        let encode_inner = Arc::clone(&inner);
        let reviver: Arc<dyn Fn(&str, Unknown) -> Unknown + Send + Sync> = Arc::new(reviver);
        let replacer: Arc<dyn Fn(&str, Unknown) -> Option<Unknown> + Send + Sync> =
            Arc::new(replacer);
        Schema::new(
            move |unknown| {
                let Unknown::Text(source) = unknown else {
                    return mismatch("$", "a JSON string", &unknown);
                };
                let parsed: serde_json::Value =
                    serde_json::from_str(&source).map_err(|error| SchemaError::InvalidJson {
                        path: "$".to_string(),
                        message: error.to_string(),
                    })?;
                let revived = revive(&reviver, "", parsed);
                decode_inner.decode(Unknown::from_json(revived))
            },
            move |value| {
                let encoded = encode_inner.try_encode(value)?;
                let replaced = replacer_at(&replacer, "", web_json(&encoded))
                    .unwrap_or(serde_json::Value::Null);
                let printed =
                    print_json(&replaced, space).map_err(|error| SchemaError::EncodingFailed {
                        path: "$".to_string(),
                        expected: error.to_string(),
                    })?;
                Ok(Unknown::Text(printed))
            },
            SchemaKind::Opaque("fromJsonString"),
        )
    }
}

/// The pin's `JSON.parse(text, reviver)`: the hook runs on the parsed value at
/// each key, innermost first, so a parent sees what its children revived.
fn revive(
    reviver: &Arc<dyn Fn(&str, Unknown) -> Unknown + Send + Sync>,
    key: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    let revived = match value {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| revive(reviver, &index.to_string(), item))
                .collect(),
        ),
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|(name, item)| {
                    let item = revive(reviver, &name, item);
                    (name, item)
                })
                .collect(),
        ),
        other => other,
    };
    reviver(key, Unknown::from_json(revived)).to_json()
}

/// The pin's `JSON.stringify(value, replacer)`: one call per key, the hook's
/// value then walked for keys of its own. `None` is the pin's `undefined`: an
/// object loses the key, an array element prints as `null`.
fn replacer_at(
    replacer: &Arc<dyn Fn(&str, Unknown) -> Option<Unknown> + Send + Sync>,
    key: &str,
    value: serde_json::Value,
) -> Option<serde_json::Value> {
    let kept = web_json(&replacer(key, Unknown::from_json(value))?);
    Some(match kept {
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .filter_map(|(name, item)| {
                    replacer_at(replacer, &name, item).map(|kept| (name, kept))
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    replacer_at(replacer, &index.to_string(), item)
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect(),
        ),
        other => other,
    })
}

/// `Unknown` as a JSON value with the JavaScript number spelling: an integral
/// number in the safe range is written as an integer (`1`, not `1.0`), and a
/// non-finite one as `null`, which is what `JSON.stringify` writes. The wire
/// side of this port means a JSON number, so printing has to say it that way;
/// `Unknown::to_json` keeps serde's own spelling and is left alone.
fn web_json(unknown: &Unknown) -> serde_json::Value {
    match unknown {
        Unknown::Null => serde_json::Value::Null,
        Unknown::Bool(boolean) => serde_json::Value::Bool(*boolean),
        Unknown::Number(number) => {
            let max_safe = 9007199254740991.0_f64;
            if number.fract() == 0.0 && number.abs() <= max_safe {
                serde_json::Value::Number(serde_json::Number::from(*number as i64))
            } else {
                serde_json::Number::from_f64(*number)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            }
        }
        Unknown::Text(text) => serde_json::Value::String(text.clone()),
        Unknown::Array(items) => serde_json::Value::Array(items.iter().map(web_json).collect()),
        Unknown::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), web_json(value)))
                .collect(),
        ),
    }
}

/// Print a parsed document: compact unless `space` asks for indentation, the
/// pin's `JSON.stringify` default and its `space` option.
///
/// The indent is `space` spaces and never a tab (the pin passes the number
/// through as-is); an empty object or array stays on one line, which is what
/// `JSON.stringify({}, null, 2)` prints.
fn print_json(value: &serde_json::Value, space: Option<u8>) -> Result<String, serde_json::Error> {
    let Some(width) = space else {
        return serde_json::to_string(value);
    };
    let indent = " ".repeat(width as usize);
    let mut out = String::new();
    write_indented(value, &indent, 0, &mut out)?;
    Ok(out)
}

/// One node of the indented print. Scalars go back through `serde_json` so
/// string escaping stays its job, never this function's.
fn write_indented(
    value: &serde_json::Value,
    indent: &str,
    depth: usize,
    out: &mut String,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push('\n');
                write_indent_into(out, indent, depth + 1);
                write_indented(item, indent, depth + 1, out)?;
            }
            out.push('\n');
            write_indent_into(out, indent, depth);
            out.push(']');
        }
        serde_json::Value::Object(fields) if !fields.is_empty() => {
            out.push('{');
            for (index, (name, item)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push('\n');
                write_indent_into(out, indent, depth + 1);
                out.push_str(&serde_json::to_string(name)?);
                out.push_str(": ");
                write_indented(item, indent, depth + 1, out)?;
            }
            out.push('\n');
            write_indent_into(out, indent, depth);
            out.push('}');
        }
        other => out.push_str(&serde_json::to_string(other)?),
    }
    Ok(())
}

/// `depth` levels of `indent`.
fn write_indent_into(out: &mut String, indent: &str, depth: usize) {
    for _ in 0..depth {
        out.push_str(indent);
    }
}

impl SchemaError {
    /// Re-root an error path under a prefix (`$[0]` + `[1]` = `$[0][1]`).
    fn at(self, prefix: &str) -> Self {
        let rooted = |path: String| match path.strip_prefix('$') {
            Some(rest) => format!("{prefix}{rest}"),
            None => format!("{prefix}{path}"),
        };
        match self {
            SchemaError::TypeMismatch {
                path,
                expected,
                got,
            } => SchemaError::TypeMismatch {
                path: rooted(path),
                expected,
                got,
            },
            // A failed check carries a path like any other refusal, so a check
            // on a field has to land at that field and not at the root.
            SchemaError::TypeMismatchDynamic {
                path,
                expected,
                got,
            } => SchemaError::TypeMismatchDynamic {
                path: rooted(path),
                expected,
                got,
            },
            SchemaError::DefaultFailed { path, message } => SchemaError::DefaultFailed {
                path: rooted(path),
                message,
            },
            // A composite from a nested collecting walk carries one issue per
            // member, each with its own root-relative path.
            SchemaError::Composite { issues } => SchemaError::Composite {
                issues: issues.into_iter().map(|issue| issue.at(prefix)).collect(),
            },
            other => other,
        }
    }
}

/// The pin's refusal for positions past a tuple's end: one excess property per
/// position, all of them under `errors: "all"`. Tuple overflow is always a
/// refusal, so `onExcessProperty` does not enter into it.
fn excess_issues(mut paths: impl Iterator<Item = String>) -> Option<SchemaError> {
    let first = paths.next()?;
    let issue = |path: String| SchemaError::TypeMismatch {
        path,
        expected: "no excess property",
        got: "excess property",
    };
    if transform::ParseOptions::current().errors == transform::Errors::First {
        return Some(issue(first));
    }
    let mut issues = vec![issue(first)];
    issues.extend(paths.map(issue));
    Some(SchemaError::Composite { issues })
}

fn mismatch<T>(path: &str, expected: &'static str, got: &Unknown) -> Result<T, SchemaError> {
    Err(SchemaError::TypeMismatch {
        path: path.to_string(),
        expected,
        got: got.kind(),
    })
}

/// The same refusal for a message that is built once the schema exists, so it
/// cannot be a `&'static str`.
fn mismatch_dynamic<T>(path: &str, expected: String, got: &Unknown) -> Result<T, SchemaError> {
    Err(SchemaError::TypeMismatchDynamic {
        path: path.to_string(),
        expected,
        got: got.kind(),
    })
}

/// The JavaScript `String(number)` spelling, which is the wire form of
/// `NumberFromString`: the non-finite values have names there.
pub(crate) fn js_number_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else if value == 0.0 {
        // `String(-0)` is "0" in JavaScript: the sign of a zero is not part of
        // the spelling, so negative zero prints as zero.
        "0".to_string()
    } else {
        format!("{value}")
    }
}

/// The source-side read of `finite_from_string`: a text that parses to a finite
/// number. Shared by decode and by the construction path's fallback.
fn decode_finite_from_string(unknown: Unknown) -> Result<f64, SchemaError> {
    match unknown {
        Unknown::Text(ref text) => {
            let trimmed = text.trim();
            match trimmed.parse::<f64>() {
                Ok(number) if number.is_finite() => Ok(number),
                _ => mismatch("$", "a finite number string", &unknown),
            }
        }
        other => mismatch("$", "a finite number string", &other),
    }
}

impl Schema<Unknown> {
    /// Any value at all (`Any` / `Unknown` shapes): decode always succeeds.
    pub fn any() -> Self {
        Self::new(Ok, |value| Ok(value.clone()), SchemaKind::Opaque("any"))
    }

    /// Anything decodes (`Unknown` shape): identity, like `any`.
    pub fn unknown() -> Self {
        Self::new(Ok, |value| Ok(value.clone()), SchemaKind::Opaque("unknown"))
    }

    /// No value decodes (`Never` shape): decode always fails. Nothing encodes
    /// either, since a schema with no value has no wire value to hand back, so
    /// the refusal travels on `try_encode` instead of coming back as the input.
    pub fn never() -> Self {
        Self::new(
            |unknown| mismatch("$", "never", &unknown),
            |_| {
                Err(SchemaError::EncodingFailed {
                    path: "$".to_string(),
                    expected: "no value: Never has none".to_string(),
                })
            },
            SchemaKind::Opaque("never"),
        )
        .with_equivalence(strict_identity_equivalence())
    }

    /// Any object or array, and nothing else (`ObjectKeyword` shape). The pin
    /// also accepts a function; the port's `Unknown` has no function value, so
    /// the refusal names the two shapes it can hold.
    pub fn object_keyword() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Object(_) | Unknown::Array(_) => Ok(unknown),
                other => mismatch("$", "object | array", &other),
            },
            |value| Ok(value.clone()),
            SchemaKind::Opaque("objectKeyword"),
        )
    }

    /// One exact value (`Literal` shape): equality decides. The kind carries the
    /// value, because a repr prints `Schema.Literal("a")` and an opaque kind
    /// would print the bare constructor.
    ///
    /// A non-finite number is refused, as the pin throws for one: `NaN` and the
    /// infinities compare false against themselves, so such a literal could
    /// never match anything and would be a schema with no members.
    pub fn literal(expected: Unknown) -> Self {
        if let Unknown::Number(number) = expected {
            assert!(
                number.is_finite(),
                "A numeric literal must be finite, got {}",
                js_number(number)
            );
        }
        let kind = SchemaKind::Literal(expected.clone());
        Self::new(
            move |unknown| {
                if unknown == expected {
                    Ok(unknown)
                } else {
                    mismatch("$", "that literal", &unknown)
                }
            },
            |value| Ok(value.clone()),
            kind,
        )
    }

    /// Fill an absent input with a default before decoding
    /// (`withDecodingDefault`): the default is wire-shaped, so it is decoded
    /// through this schema, and it may fail, which is a decode failure at the
    /// place it was filling. Encoding passes values through unless
    /// [`omit_encoding`] was asked for.
    ///
    /// [`omit_encoding`]: Schema::omit_encoding
    pub fn with_decoding_default<U: Send + Sync + 'static>(
        schema: Schema<U>,
        default: impl Fn() -> Result<Unknown, SchemaError> + Send + Sync + 'static,
    ) -> Schema<U> {
        Self::with_default(schema, SchemaDefault::Wire(Arc::new(default)))
    }

    /// `withDecodingDefaultType`: the default is a value of the decoded type
    /// already, so it is used as it is instead of being decoded again.
    pub fn with_decoding_default_type<U: Send + Sync + 'static>(
        schema: Schema<U>,
        default: impl Fn() -> Result<U, SchemaError> + Send + Sync + 'static,
    ) -> Schema<U> {
        Self::with_default(schema, SchemaDefault::Decoded(Arc::new(default)))
    }

    fn with_default<U: Send + Sync + 'static>(
        schema: Schema<U>,
        default: SchemaDefault<U>,
    ) -> Schema<U> {
        let schema = Arc::new(schema);
        let decode_schema = schema.clone();
        let decode_default = default.clone();
        let mut out = Schema::new(
            move |unknown| match unknown {
                Unknown::Null => decode_schema.absent_value(&decode_default, "$"),
                value => decode_schema.decode(value),
            },
            move |value| schema.try_encode(value),
            SchemaKind::Opaque("withDecodingDefault"),
        );
        out.decoding_default = Some(default);
        out
    }

    /// A literal-to-literal map (`Literal.transform`, and the per-member form of
    /// `Literals([..]).transform([..])`): decoding maps an input literal to its
    /// output, encoding maps the output back to the input. Either direction
    /// refuses a value outside the map, so this is one of the schemas whose
    /// encode can fail, which is what [`try_encode`] is for.
    ///
    /// [`try_encode`]: Schema::try_encode
    pub fn literal_transform(pairs: Vec<(Unknown, Unknown)>) -> Self {
        assert!(
            !pairs.is_empty(),
            "a literal transform maps at least one pair"
        );
        let kind = match pairs.as_slice() {
            [(from, _)] => SchemaKind::Literal(from.clone()),
            _ => SchemaKind::Literals(pairs.iter().map(|(from, _)| from.clone()).collect()),
        };
        // A refusal names the values this schema does describe, the way the
        // pin's `Expected "a" | "b"` does.
        let targets = pairs
            .iter()
            .map(|(_, to)| format_value(to))
            .collect::<Vec<_>>()
            .join(" | ");
        let decode_pairs = pairs.clone();
        let encode_pairs = pairs;
        Self::new(
            move |unknown| match decode_pairs.iter().find(|(from, _)| *from == unknown) {
                Some((_, to)) => Ok(to.clone()),
                None => mismatch("$", "one of the source literals", &unknown),
            },
            move |value| match encode_pairs.iter().find(|(_, to)| to == value) {
                Some((from, _)) => Ok(from.clone()),
                None => Err(SchemaError::EncodingFailed {
                    path: "$".to_string(),
                    expected: format!("one of {targets}"),
                }),
            },
            kind,
        )
    }

    /// Try the first schema, fall back to the second (`Union` shape).
    /// Encode always uses the first schema.
    pub fn union(first: Schema<Unknown>, second: Schema<Unknown>) -> Self {
        let kind = SchemaKind::Opaque("union");
        let first = Arc::new(first);
        let second = Arc::new(second);
        let decode_first = first.clone();
        let decode_second = second.clone();
        let encode_first = first.clone();
        Schema::new(
            move |unknown| match decode_first.decode(unknown.clone()) {
                ok @ Ok(_) => ok,
                Err(_) => decode_second.decode(unknown),
            },
            move |value| encode_first.try_encode(value),
            kind,
        )
    }
}

impl Schema<()> {
    /// Anything at all decodes, yielding the unit value (`Void` shape). The port
    /// stands `Unknown::Null` in for the pin's `undefined` on the way out, since
    /// a value-less result is what it has to represent one.
    pub fn void() -> Self {
        Self::new(
            |_| Ok(()),
            |_| Ok(Unknown::Null),
            SchemaKind::Opaque("void"),
        )
    }

    /// Only null (`Null` shape).
    pub fn null() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Null => Ok(()),
                other => mismatch("$", "null", &other),
            },
            |_| Ok(Unknown::Null),
            SchemaKind::Opaque("null"),
        )
    }
}

impl Schema<String> {
    /// A string whose type is open to literal autocomplete
    /// (`StringForLiteralAutocomplete`). The runtime behavior is `String`'s and
    /// the pin's AST is the string AST, so this is that schema under the name
    /// the pin's surface gives it.
    pub fn string_for_literal_autocomplete() -> Self {
        Self::string()
    }

    /// Any text.
    pub fn string() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Text(text) => Ok(text),
                other => mismatch("$", "string", &other),
            },
            |value| Ok(Unknown::Text(value.clone())),
            SchemaKind::String,
        )
    }

    /// `Schema.Trim`: the value is whitespace-trimmed on the way in, and the
    /// encoded side refuses text that is not already trimmed.
    ///
    /// The asymmetry is the pin's: trimming is a decode-only convenience, so a
    /// padded value cannot travel back out through the encoder without the
    /// caller trimming it first.
    pub fn trim() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Text(text) => Ok(text.trim().to_string()),
                other => mismatch("$", "string", &other),
            },
            |value| {
                if value.trim() == value.as_str() {
                    Ok(Unknown::Text(value.clone()))
                } else {
                    Err(SchemaError::EncodingFailed {
                        path: "$".to_string(),
                        expected: "a string with no leading or trailing whitespace".to_string(),
                    })
                }
            },
            SchemaKind::Opaque("trim"),
        )
    }
}

impl Schema<Unknown> {
    /// `Schema.UnknownFromJsonString`: the wire is a JSON document in text, the
    /// value is the document parsed, and encoding prints it back compactly.
    ///
    /// The parse is this schema's own step, so an unparseable text fails here
    /// rather than in a wrapped schema: there is nothing below to fail.
    pub fn unknown_from_json_string() -> Self {
        Schema::from_json_string(Schema::unknown())
    }
}

impl Schema<f64> {
    /// Any number.
    pub fn number() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Number(number) => Ok(number),
                other => mismatch("$", "number", &other),
            },
            |value| Ok(Unknown::Number(*value)),
            SchemaKind::Number,
        )
    }

    /// A finite number written as text (`FiniteFromString`): `"1.5"`
    /// decodes to `1.5`. Non-text, blank, unparseable, and non-finite
    /// (`NaN`, infinities) inputs fail; encoding renders the number back
    /// to text.
    /// A number built from a string (`NumberFromString`): the JS `Number()`
    /// spelling is the wire form, so `NaN`, `Infinity`, `+Infinity` and
    /// `-Infinity` are values here rather than refusals, unlike
    /// [`finite_from_string`](Self::finite_from_string).
    pub fn number_from_string() -> Self {
        let mut schema = Self::new(
            |unknown| match unknown {
                Unknown::Text(ref text) => {
                    text.trim()
                        .parse::<f64>()
                        .map_err(|_| SchemaError::TypeMismatch {
                            path: "$".to_string(),
                            expected: "a number string",
                            got: "unparseable",
                        })
                }
                other => mismatch("$", "a number string", &other),
            },
            |value| Ok(Unknown::Text(js_number_text(*value))),
            SchemaKind::Opaque("numberFromString"),
        );
        // The same view `finite_from_string` gives a check, over the wider set
        // of spellings this one reads (`"Infinity"` included).
        schema.check_view = Some(std::sync::Arc::new(|wire| match wire {
            Unknown::Text(text) => text
                .trim()
                .parse::<f64>()
                .map(Unknown::Number)
                .unwrap_or_else(|_| wire.clone()),
            _ => wire.clone(),
        }));
        schema
    }

    pub fn finite_from_string() -> Self {
        let mut schema = Self::new(
            decode_finite_from_string,
            |value| Ok(Unknown::Text(value.to_string())),
            SchemaKind::Opaque("finiteFromString"),
        );
        // A check on top of this transform reads the number, not the text it
        // came from, which is what makes `isInt` refuse `"1.2"`.
        schema.check_view = Some(std::sync::Arc::new(|wire| {
            decode_finite_from_string(wire.clone())
                .map(Unknown::Number)
                .unwrap_or_else(|_| wire.clone())
        }));
        // Construction takes a finite number as already constructed, which is
        // the pin's `descriptor.isConstructed(input)` shortcut and why `make`
        // accepts `{ a: 1 }` for this field, and otherwise reads the source
        // side: a string parses the same way decode parses it.
        schema.construct = Some(std::sync::Arc::new(|unknown| match unknown {
            Unknown::Number(number) if number.is_finite() => Ok(number),
            other => decode_finite_from_string(other),
        }));
        schema
    }
}

impl Schema<bool> {
    /// Either boolean.
    pub fn boolean() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Bool(boolean) => Ok(boolean),
                other => mismatch("$", "boolean", &other),
            },
            |value| Ok(Unknown::Bool(*value)),
            SchemaKind::Boolean,
        )
    }
}

impl Schema<BTreeMap<String, Unknown>> {
    /// Any object, fields left untyped.
    pub fn object() -> Self {
        Self::new(
            |unknown| match unknown {
                Unknown::Object(fields) => Ok(fields),
                other => mismatch("$", "object", &other),
            },
            |fields| Ok(Unknown::Object(fields.clone())),
            SchemaKind::Object,
        )
    }

    /// One required text field of an object.
    pub fn field(name: &str, inner: Schema<String>) -> Schema<String> {
        let decode_name = name.to_string();
        let encode_name = name.to_string();
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Object(mut fields) => match fields.remove(&decode_name) {
                    Some(value) => decode_inner.decode(value).map_err(|error| match error {
                        SchemaError::TypeMismatch { expected, got, .. } => {
                            SchemaError::TypeMismatch {
                                path: format!("$.{decode_name}"),
                                expected,
                                got,
                            }
                        }
                        SchemaError::TypeMismatchDynamic { expected, got, .. } => {
                            SchemaError::TypeMismatchDynamic {
                                path: format!("$.{decode_name}"),
                                expected,
                                got,
                            }
                        }
                        other => other,
                    }),
                    None => Err(SchemaError::TypeMismatch {
                        path: format!("$.{decode_name}"),
                        expected: "required field",
                        got: "missing",
                    }),
                },
                other => mismatch("$", "object", &other),
            },
            move |value| {
                let mut fields = BTreeMap::new();
                fields.insert(encode_name.clone(), encode_inner.try_encode(value)?);
                Ok(Unknown::Object(fields))
            },
            SchemaKind::Opaque("field"),
        )
    }

    /// An optional text field (`optionalKey` shape): missing or null
    /// decodes to `None`; encoding `None` omits the field.
    pub fn optional_field(name: &str, inner: Schema<String>) -> Schema<Option<String>> {
        let decode_name = name.to_string();
        let encode_name = name.to_string();
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Object(fields) => match fields.get(&decode_name) {
                    None | Some(Unknown::Null) => Ok(None),
                    Some(value) => decode_inner.decode(value.clone()).map(Some),
                },
                other => mismatch("$", "object", &other),
            },
            move |value| match value {
                None => Ok(Unknown::Object(BTreeMap::new())),
                Some(value) => {
                    let mut fields = BTreeMap::new();
                    fields.insert(encode_name.clone(), encode_inner.try_encode(value)?);
                    Ok(Unknown::Object(fields))
                }
            },
            SchemaKind::Opaque("optional field"),
        )
    }
}

impl<T: Send + Sync + 'static> Schema<Vec<T>> {
    /// Homogeneous arrays: every element decoded, failures name `[i]`.
    pub fn array(inner: Schema<T>) -> Schema<Vec<T>> {
        Self::array_with(inner, true)
    }

    /// `Schema.NonEmptyArray(inner)`: an array with at least one element.
    ///
    /// The pin builds it as a tuple of one required head plus a rest, so an
    /// empty array reads as a missing key at position 0 rather than a length
    /// check. Everything else is the plain array, and the element schema stays
    /// readable through [`element`](Self::element).
    pub fn non_empty_array(inner: Schema<T>) -> Schema<Vec<T>> {
        Self::array_with(inner, false)
    }

    /// The one array codec, with the emptiness rule as its only variable.
    fn array_with(inner: Schema<T>, allow_empty: bool) -> Schema<Vec<T>> {
        // The pin's empty-array refusal is the tuple shape's missing key, so it
        // names position 0 rather than a length constraint.
        fn empty() -> SchemaError {
            SchemaError::TypeMismatch {
                path: "$[0]".to_string(),
                expected: "an element",
                got: "missing",
            }
        }
        // The child's shape, not its raw kind: a re-typed child keeps its shape
        // in `wrapped`, so reading `kind` here would render the element as
        // `Schema.Unknown`.
        let kind = SchemaKind::Array(Box::new(inner.repr_kind()));
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let render = inner.formatter.clone();
        let inner_kind = inner.repr_kind();
        let mut out = Self::new(
            move |unknown| match unknown {
                Unknown::Array(items) => {
                    if !allow_empty && items.is_empty() {
                        return Err(empty());
                    }
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(index, item)| {
                            decode_inner.decode(item).map_err(|error| match error {
                                SchemaError::TypeMismatch { expected, got, .. } => {
                                    SchemaError::TypeMismatch {
                                        path: format!("$[{index}]"),
                                        expected,
                                        got,
                                    }
                                }
                                SchemaError::TypeMismatchDynamic { expected, got, .. } => {
                                    SchemaError::TypeMismatchDynamic {
                                        path: format!("$[{index}]"),
                                        expected,
                                        got,
                                    }
                                }
                                other => other,
                            })
                        })
                        .collect()
                }
                other => mismatch("$", "array", &other),
            },
            move |values: &Vec<T>| {
                if !allow_empty && values.is_empty() {
                    return Err(empty());
                }
                Ok(Unknown::Array(
                    values
                        .iter()
                        .map(|value| encode_inner.try_encode(value))
                        .collect::<Result<Vec<_>, _>>()?,
                ))
            },
            kind,
        );
        out.array_element = Some(Arc::new(Schema::as_unknown_shared(inner.clone())));
        // Each element renders through its own schema, so an array of Structs
        // keeps the struct spacing the pin emits (`[{ "a": 1 }]`, not `[{"a":1}]`).
        out.formatter = Some(Arc::new(move |value: &Unknown| {
            let Unknown::Array(items) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = items
                .iter()
                .map(|item| match &render {
                    Some(r) => r(item),
                    None => format_root(&inner_kind, &[], item),
                })
                .collect();
            format!("[{}]", inner.join(", "))
        }));
        out
    }

    /// `Schema.Array(...).value` at the pin: the element schema the array was
    /// built from, so a caller can inspect the element type without probing the
    /// codec with a sample value. Annotations ride the same schema, so the
    /// accessor keeps answering through one.
    pub fn element(&self) -> Option<&Schema<Unknown>> {
        self.array_element.as_deref()
    }
}

impl<A: Send + Sync + 'static, B: Send + Sync + 'static> Schema<(A, B)> {
    /// Fixed-length two-element arrays (`Tuple` shape): length and order
    /// are both checked; element failures name `[0]` / `[1]`.
    pub fn tuple2(first: Schema<A>, second: Schema<B>) -> Schema<(A, B)> {
        let first = Arc::new(first);
        let second = Arc::new(second);
        let decode_first = first.clone();
        let decode_second = second.clone();
        let encode_first = first.clone();
        let encode_second = second.clone();
        Schema::new(
            move |unknown| match unknown {
                Unknown::Array(items) => {
                    let mut items = items.into_iter();
                    let a = decode_first
                        .decode(items.next().unwrap_or(Unknown::Null))
                        .map_err(|error| error.at("$[0]"))?;
                    let b = decode_second
                        .decode(items.next().unwrap_or(Unknown::Null))
                        .map_err(|error| error.at("$[1]"))?;
                    Ok((a, b))
                }
                other => mismatch("$", "tuple", &other),
            },
            move |(a, b)| {
                Ok(Unknown::Array(vec![
                    encode_first.try_encode(a)?,
                    encode_second.try_encode(b)?,
                ]))
            },
            SchemaKind::Opaque("tuple"),
        )
    }
}

/// One field of a struct shape: required or optional with a decoder.
///
/// Two optional flavors: [`OptionalKey`](Optionality::OptionalKey) (exact:
/// the key may be absent, but an explicit null still decodes the inner
/// schema) and [`Optional`](Optionality::Optional) (absent or null both
/// skip the inner schema).
/// A named set of literal values (`Schema.Enum`): the pairs as declared and the
/// union of literals they decode through. Built by
/// [`Schema::enumeration`]; the names stay reachable because the pin exposes
/// them (`schema.enums.Apple`), and a caller that only needs the codec can take
/// [`schema`](Enumeration::schema) and drop the rest.
pub struct Enumeration {
    pairs: Vec<(String, Unknown)>,
    schema: Schema<Unknown>,
}

impl Enumeration {
    /// The named values, in declaration order.
    pub fn enums(&self) -> &[(String, Unknown)] {
        &self.pairs
    }

    /// The value one name holds, if the set has that name.
    pub fn value(&self, name: &str) -> Option<&Unknown> {
        self.pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The schema the values decode and encode through.
    pub fn schema(&self) -> &Schema<Unknown> {
        &self.schema
    }

    /// The schema by value, for the common `let schema = ...schema()` shape.
    pub fn into_schema(self) -> Schema<Unknown> {
        self.schema
    }
}

impl Clone for Enumeration {
    /// The pairs clone plainly; the schema is shared rather than rebuilt, so a
    /// copy of a set decodes exactly what the original does.
    fn clone(&self) -> Self {
        Self {
            pairs: self.pairs.clone(),
            schema: self.schema.clone(),
        }
    }
}

/// Which entry point a collecting walk serves. Decoding and construction both
/// take the wire value and answer the value, so one walk covers both.
#[derive(Clone, Copy)]
enum CollectOp {
    Decode,
    Construct,
}

#[derive(Clone)]
pub struct Field {
    name: String,
    schema: Arc<Schema<Unknown>>,
    optionality: Optionality,
    /// A field-level default (`withDecodingDefaultKey`): a missing key fills it,
    /// while an explicit null still goes through `schema` (which is what makes
    /// the `Key` spelling exact rather than absorbing everything absent).
    decoding_default: Option<SchemaDefault<Unknown>>,
    omit_on_encode: bool,
    /// `OptionFromOptionalKey` & co.: the decoded `None` marker (this port's
    /// `Unknown::Null`) leaves the key out on encode, while a present value
    /// travels. Unlike `omit_on_encode` this depends on the value.
    omit_none_on_encode: bool,
}

/// How optional a struct field is on the decoding axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optionality {
    /// The key must be present.
    Required,
    /// Exact optional: the key may be absent, but an explicit null still
    /// decodes the inner schema.
    OptionalKey,
    /// The key may be absent or null; both skip the inner schema.
    Optional,
}

impl Field {
    /// A required field decoded with `schema`.
    pub fn required(name: &str, schema: Schema<Unknown>) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::Required,
            decoding_default: None,
            omit_on_encode: false,
            omit_none_on_encode: false,
        }
    }

    /// A field whose missing key fills a wire-shaped default
    /// (`withDecodingDefaultKey`): `default` is decoded through the field's
    /// schema, so it is written the way the wire spells it. A key that is
    /// present still decodes normally, null included.
    pub fn decoding_default(
        name: &str,
        schema: Schema<Unknown>,
        default: impl Fn() -> Result<Unknown, SchemaError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::Required,
            decoding_default: Some(SchemaDefault::Wire(Arc::new(default))),
            omit_on_encode: false,
            omit_none_on_encode: false,
        }
    }

    /// The same, spelled with a value of the decoded type
    /// (`withDecodingDefaultTypeKey`).
    pub fn decoding_default_type(
        name: &str,
        schema: Schema<Unknown>,
        default: impl Fn() -> Result<Unknown, SchemaError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::Required,
            decoding_default: Some(SchemaDefault::Decoded(Arc::new(default))),
            omit_on_encode: false,
            omit_none_on_encode: false,
        }
    }

    /// `{ encodingStrategy: "omit" }` on a field default: encoding drops the
    /// key even though the field is decoded.
    pub fn omit_encoding(mut self) -> Self {
        self.omit_on_encode = true;
        self
    }

    /// A `None` marker leaves the key out on encode, while a present value
    /// still travels (`OptionFromOptionalKey` and its relatives).
    pub fn omit_none_encoding(mut self) -> Self {
        self.omit_none_on_encode = true;
        self
    }

    /// `Schema.OptionFromOptionalKey(inner)` at a struct field: an absent key
    /// decodes to `None`, a present null still goes through `inner` (so a
    /// schema with no null refuses it), and a `None` leaves the key out again.
    pub fn option_from_optional_key(name: &str, inner: Schema<Unknown>) -> Self {
        Self::decoding_default_type(name, inner, || Ok(Unknown::Null)).omit_none_encoding()
    }

    /// `Schema.OptionFromOptional(inner)`: absence and null are both `None`,
    /// and a `None` leaves the key out on encode.
    pub fn option_from_optional(name: &str, inner: Schema<Unknown>) -> Self {
        Self::decoding_default_type(name, inner, || Ok(Unknown::Null))
            .make_nullable()
            .omit_none_encoding()
    }

    /// `Schema.OptionFromOptionalNullOr(inner, { onNoneEncoding })`: absence,
    /// null and the port's null stand-in for undefined are all `None`.
    /// `omit_none` picks the pin's two encodings: true leaves the key out,
    /// false carries the null stand-in (`onNoneEncoding: null`).
    pub fn option_from_optional_null_or(
        name: &str,
        inner: Schema<Unknown>,
        omit_none: bool,
    ) -> Self {
        let field = Self::decoding_default_type(name, inner, || Ok(Unknown::Null)).make_nullable();
        if omit_none {
            field.omit_none_encoding()
        } else {
            field
        }
    }

    /// An optional field: missing or null decodes without error.
    pub fn optional(name: &str, schema: Schema<Unknown>) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::Optional,
            decoding_default: None,
            omit_on_encode: false,
            omit_none_on_encode: false,
        }
    }

    /// An exact-optional field: the key may be absent, but an explicit
    /// null still decodes the inner schema.
    pub fn optional_key(name: &str, schema: Schema<Unknown>) -> Self {
        Self {
            name: name.to_string(),
            schema: Arc::new(schema),
            optionality: Optionality::OptionalKey,
            decoding_default: None,
            omit_on_encode: false,
            omit_none_on_encode: false,
        }
    }

    /// Reverse `optional`/`optional_key`: the field becomes required.
    pub fn require(mut self) -> Self {
        self.optionality = Optionality::Required;
        self
    }

    /// The key this field reads.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How optional the key is on the decoding axis.
    pub fn optionality(&self) -> Optionality {
        self.optionality
    }

    /// The schema this field's value decodes with.
    pub fn inner(&self) -> &Schema<Unknown> {
        &self.schema
    }

    /// Swap the field's schema (`evolve`'s `(field) => ...`): the key and its
    /// optionality stay, the codec changes.
    pub fn with_schema(mut self, schema: Schema<Unknown>) -> Self {
        self.schema = Arc::new(schema);
        self
    }

    /// Rename the key (`evolveKeys`/`renameKeys`).
    pub fn renamed(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// `Schema.optionalKey` over a field's schema: the key may be absent, while
    /// an explicit null still decodes the value schema.
    pub fn make_optional_key(mut self) -> Self {
        self.optionality = Optionality::OptionalKey;
        self
    }

    /// `Schema.optional`/`NullOr` over a field's schema: a null value decodes to
    /// none. The key itself stays required, which is what tells this apart from
    /// [`optional`](Field::optional).
    pub fn make_nullable(self) -> Self {
        let inner = (*self.schema).clone();
        Self {
            schema: Arc::new(Schema::as_unknown(Schema::optional(inner))),
            ..self
        }
    }

    /// Make every field required (`required` over a struct shape).
    pub fn require_all(fields: Vec<Field>) -> Vec<Field> {
        fields.into_iter().map(Field::require).collect()
    }

    /// A tag field (`tag`/`tagDefaultOmit`): absent keys fill the default
    /// on decode (and construction), and the key is omitted on encode. The
    /// default is a literal, so decoding it through the field's own schema is
    /// the identity, and it rides the same field-default path as the rest.
    pub fn tag(name: &str, value: Unknown) -> Self {
        let literal = value.clone();
        Self {
            name: name.to_string(),
            schema: Arc::new(Schema::literal(value)),
            optionality: Optionality::Required,
            decoding_default: Some(SchemaDefault::Wire(Arc::new(move || Ok(literal.clone())))),
            omit_on_encode: true,
            omit_none_on_encode: false,
        }
    }
}

/// A transformer over a tuple's element list, the shape `mapElements` takes.
/// As with the field transformers, the pin spells each helper out and the port
/// builds them from these.
pub type ElementTransform = Box<dyn Fn(Vec<Schema<Unknown>>) -> Vec<Schema<Unknown>>>;

/// `Tuple.map(f)`: every element through `f`.
pub fn element_map(
    f: impl Fn(Schema<Unknown>) -> Schema<Unknown> + Copy + 'static,
) -> ElementTransform {
    Box::new(move |elements| elements.into_iter().map(f).collect())
}

/// `Tuple.appendElement`.
pub fn element_append(element: Schema<Unknown>) -> ElementTransform {
    Box::new(move |mut elements| {
        elements.push(element.clone());
        elements
    })
}

/// `Tuple.appendElements`.
pub fn element_append_all(more: Vec<Schema<Unknown>>) -> ElementTransform {
    Box::new(move |mut elements| {
        elements.extend(more.clone());
        elements
    })
}

/// `Tuple.pick`: the listed positions, in the order given.
pub fn element_pick(positions: Vec<usize>) -> ElementTransform {
    Box::new(move |elements| {
        positions
            .iter()
            .filter_map(|position| elements.get(*position).cloned())
            .collect()
    })
}

/// `Tuple.omit`: every position except the listed ones.
pub fn element_omit(positions: Vec<usize>) -> ElementTransform {
    Box::new(move |elements| {
        elements
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !positions.contains(index))
            .map(|(_, element)| element)
            .collect()
    })
}

/// `Tuple.evolve`: a function per position, `None` leaving that position alone.
/// Wider than `Tuple.map` in the same way `Struct.evolve` is wider than
/// `Struct.map`.
pub fn element_evolve(
    transforms: Vec<Option<fn(Schema<Unknown>) -> Schema<Unknown>>>,
) -> ElementTransform {
    Box::new(move |elements| {
        elements
            .into_iter()
            .enumerate()
            .map(
                |(index, element)| match transforms.get(index).copied().flatten() {
                    Some(transform) => transform(element),
                    None => element,
                },
            )
            .collect()
    })
}

/// `Tuple.renameIndices`: `order[new position]` names the old position that
/// moves there; a position the list does not reach keeps its own index.
pub fn element_rename_indices(order: Vec<usize>) -> ElementTransform {
    Box::new(move |elements| {
        (0..elements.len())
            .map(|index| {
                let source = order
                    .get(index)
                    .copied()
                    .filter(|source| *source < elements.len())
                    .unwrap_or(index);
                elements[source].clone()
            })
            .collect()
    })
}

/// `Tuple.map(Schema.optionalKey)`: every element's position may be absent.
pub fn element_make_optional_key(element: Schema<Unknown>) -> Schema<Unknown> {
    Schema::optional_key_shape(element)
}

/// `Tuple.map(Schema.NullOr)`: every element may be null.
pub fn element_make_nullable(element: Schema<Unknown>) -> Schema<Unknown> {
    Schema::as_unknown(Schema::optional(element))
}

/// A transformer over a struct's field list, the shape `mapFields` takes. The
/// pin spells each of these as its own helper (`Struct.evolve`,
/// `Struct.map`, `Struct.mapPick`, ...); the port builds them all from these
/// four, since each one is a different way to say "replace the field list".
pub type FieldTransform = Box<dyn Fn(Vec<Field>) -> Vec<Field>>;

/// `Struct.map(f)`: every field through `f`.
pub fn field_map(f: impl Fn(Field) -> Field + Copy + 'static) -> FieldTransform {
    Box::new(move |fields| fields.into_iter().map(f).collect())
}

/// `Struct.evolve`/`Struct.evolveKeys`/`Struct.renameKeys`/`Struct.rename`: the
/// named fields through their own function, the rest untouched. A name with no
/// field to match simply does nothing, which is what the pin does too.
pub fn field_evolve(pairs: Vec<(&'static str, fn(Field) -> Field)>) -> FieldTransform {
    Box::new(move |fields| {
        fields
            .into_iter()
            .map(
                |field| match pairs.iter().find(|(name, _)| *name == field.name()) {
                    Some((_, transform)) => transform(field),
                    None => field,
                },
            )
            .collect()
    })
}

/// `Struct.evolveEntries`: every `(key, value)` pair through one function, so a
/// transform can rename the key and change the codec at once.
pub fn field_evolve_entries(f: impl Fn(Field) -> Field + Copy + 'static) -> FieldTransform {
    field_map(f)
}

/// `Struct.mapPick`: only the named fields through `f`.
pub fn field_map_pick(
    names: Vec<&'static str>,
    f: impl Fn(Field) -> Field + Copy + 'static,
) -> FieldTransform {
    Box::new(move |fields| {
        fields
            .into_iter()
            .map(|field| {
                if names.contains(&field.name()) {
                    f(field)
                } else {
                    field
                }
            })
            .collect()
    })
}

/// `Struct.mapOmit`: every field except the named ones through `f`.
pub fn field_map_omit(
    names: Vec<&'static str>,
    f: impl Fn(Field) -> Field + Copy + 'static,
) -> FieldTransform {
    Box::new(move |fields| {
        fields
            .into_iter()
            .map(|field| {
                if names.contains(&field.name()) {
                    field
                } else {
                    f(field)
                }
            })
            .collect()
    })
}

/// One walk over a struct-shape input, shared by the decode path and the
/// construction path. The one difference: a missing field whose schema carries
/// a constructor default fills on construction and stays absent on decode,
/// while a decoding default is the mirror image and fills on both (that is what
/// makes it a default for the wire rather than for a constructed value).
/// One field's contribution to a struct walk: the pair to insert, or `None`
/// when the field is absent and may be. Split out of [`walk_struct`] so the
/// loop can accumulate a failure instead of returning it.
fn walk_field(
    field: &Field,
    input: &mut BTreeMap<String, Unknown>,
    construct: bool,
) -> Result<Option<(String, Unknown)>, SchemaError> {
    match input.remove(&field.name) {
        None if construct && field.schema.constructor_default.is_some() => {
            let default = field
                .schema
                .constructor_default
                .as_ref()
                .expect("checked just above");
            let produced = default().map_err(|error| error.at(&format!("$.{}", field.name)))?;
            let built = field
                .schema
                .construct(produced)
                .map_err(|error| error.at(&format!("$.{}", field.name)))?;
            Ok(Some((field.name.clone(), built)))
        }
        // A decoding default fills an absent key: the field's own
        // (`withDecodingDefaultKey`) or the one the field's schema carries
        // (`withDecodingDefault` at a struct occurrence).
        None if field.decoding_default.is_some() || field.schema.decoding_default.is_some() => {
            let default = field
                .decoding_default
                .as_ref()
                .or(field.schema.decoding_default.as_ref())
                .expect("checked just above");
            let path = format!("$.{}", field.name);
            let filled = field.schema.absent_value(default, &path)?;
            Ok(Some((field.name.clone(), filled)))
        }
        None if field.optionality != Optionality::Required => Ok(None),
        Some(Unknown::Null) if field.optionality == Optionality::Optional => Ok(None),
        None => Err(SchemaError::TypeMismatch {
            path: format!("$.{}", field.name),
            expected: "required field",
            got: "missing",
        }),
        Some(value) => {
            let step = if construct {
                field.schema.construct(value)
            } else {
                field.schema.decode(value)
            };
            let decoded = step.map_err(|error| error.at(&format!("$.{}", field.name)))?;
            Ok(Some((field.name.clone(), decoded)))
        }
    }
}

fn walk_struct(
    fields: &[Field],
    unknown: Unknown,
    construct: bool,
) -> Result<BTreeMap<String, Unknown>, SchemaError> {
    let options = transform::ParseOptions::current();
    let Unknown::Object(mut input) = unknown else {
        return mismatch("$", "object", &unknown);
    };
    let mut out = BTreeMap::new();
    let mut issues = Vec::new();
    for field in fields {
        match walk_field(field, &mut input, construct) {
            Ok(Some((name, value))) => {
                out.insert(name, value);
            }
            Ok(None) => {}
            Err(error) if options.errors == transform::Errors::First => return Err(error),
            Err(error) => issues.push(error),
        }
    }
    // Undeclared keys are the pin's `onExcessProperty` decision: dropped by
    // default (`"ignore"`), refused under `"error"`, and absorbed by
    // `struct_with_rest` instead of ever reaching this walk.
    if options.on_excess_property == ExcessProperty::Reject {
        for key in input.keys() {
            let issue = SchemaError::TypeMismatch {
                path: format!("$.{key}"),
                expected: "no excess property",
                got: "excess property",
            };
            if options.errors == transform::Errors::First {
                return Err(issue);
            }
            issues.push(issue);
        }
    }
    if issues.is_empty() {
        Ok(out)
    } else {
        Err(SchemaError::Composite { issues })
    }
}

impl Schema<BTreeMap<String, Unknown>> {
    /// Encode collecting every issue: the encoding half of
    /// [`Schema::decode_collect`]. One issue per absent required field and per
    /// field its own encoder refuses; a value with no issues goes through the
    /// normal encoder, so an accepted result has exactly one spelling.
    pub fn encode_collect(
        &self,
        value: &BTreeMap<String, Unknown>,
    ) -> Result<Unknown, Vec<SchemaError>> {
        if let Some(fields) = &self.struct_fields {
            let mut issues = Vec::new();
            for field in fields {
                // An `omit` field is not part of what this schema describes on
                // the way out, so encoding is not asked for it.
                let travels = !(field.omit_on_encode || field.schema.omit_on_encode);
                match value.get(&field.name) {
                    None if !travels => {}
                    None if field.optionality != Optionality::Required => {}
                    None => issues.push(SchemaError::TypeMismatch {
                        path: format!("$.{}", field.name),
                        expected: "required field",
                        got: "missing",
                    }),
                    Some(Unknown::Null) if field.optionality == Optionality::Optional => {}
                    Some(present) => {
                        if let Err(error) = field.schema.try_encode(present) {
                            issues.push(repath_under(error, &format!("$.{}", field.name)));
                        }
                    }
                }
            }
            if !issues.is_empty() {
                return Err(issues);
            }
        }
        self.try_encode(value).map_err(|error| vec![error])
    }

    /// A struct shape (`Struct`/`pick`/`Partial` shapes): every required
    /// field must decode; optional fields may be missing or null. Extra
    /// input fields pass through untouched.
    ///
    /// The shape has two walks. `decode` reads data that is already there and
    /// never fills a constructor default, which is the pin's rule that
    /// `withConstructorDefault` applies to construction only. The construction
    /// walk (`make`) fills an absent field whose schema carries one.
    pub fn struct_shape(fields: Vec<Field>) -> Self {
        let omitted: Vec<String> = fields
            .iter()
            .filter(|field| field.omit_on_encode || field.schema.omit_on_encode)
            .map(|field| field.name.clone())
            .collect();
        // A `None` marker is dropped key by key, so a field holding a value
        // still travels (`OptionFromOptionalKey` and friends).
        let omitted_none: Vec<String> = fields
            .iter()
            .filter(|field| field.omit_none_on_encode)
            .map(|field| field.name.clone())
            .collect();
        // Presence is a property of the declared field, so the ENCODE side
        // enforces it too: the pin encodes `Struct({ a: String })` from `{}`
        // and fails with `Missing key at ["a"]`. A field the wire drops anyway
        // (`omit`) is not asked for, because its key is not part of what this
        // schema describes on the way out.
        let required: Vec<String> = fields
            .iter()
            .filter(|field| field.optionality() == Optionality::Required)
            .filter(|field| !omitted.iter().any(|name| *name == field.name))
            .map(|field| field.name.clone())
            .collect();
        let declared: Vec<String> = fields.iter().map(|field| field.name.clone()).collect();
        let decode_fields = fields.clone();
        let declared_fields = Some(fields.clone());
        let construct_fields = fields;
        let mut schema = Self::new(
            move |unknown| walk_struct(&decode_fields, unknown, false),
            move |output| {
                for name in &required {
                    if !output.contains_key(name) {
                        return Err(SchemaError::TypeMismatch {
                            path: format!("$.{name}"),
                            expected: "required field",
                            got: "missing",
                        });
                    }
                }
                let mut filtered = output.clone();
                for name in &omitted {
                    filtered.remove(name);
                }
                for name in &omitted_none {
                    if matches!(filtered.get(name), Some(Unknown::Null)) {
                        filtered.remove(name);
                    }
                }
                // A key the field list does not declare is the call site's
                // excess-property decision: the pin's encoder drops it unless
                // the call site asked for an error, and the pin's decode walk
                // above already dropped it on the way in.
                match transform::ParseOptions::current().on_excess_property {
                    ExcessProperty::Reject => {
                        if let Some(key) = filtered.keys().find(|key| !declared.contains(key)) {
                            return Err(SchemaError::TypeMismatch {
                                path: format!("$.{key}"),
                                expected: "no excess property",
                                got: "excess property",
                            });
                        }
                    }
                    _ => filtered.retain(|key, _| declared.contains(key)),
                }
                Ok(Unknown::Object(filtered))
            },
            SchemaKind::Object,
        );
        schema.construct = Some(std::sync::Arc::new(move |unknown| {
            walk_struct(&construct_fields, unknown, true)
        }));
        schema.struct_fields = declared_fields;
        schema
    }

    /// A record shape whose keys are also checked (`patternProperties` shape).
    /// The wire shape is the same as `record`, so the key schema lives in the
    /// kind for a repr and is enforced by checking the key.
    pub fn record2(key: Schema<Unknown>, value: Schema<Unknown>) -> Self {
        Self::record2_excess(key, value, ExcessProperty::Ignore)
    }

    /// [`record2`](Self::record2) with an explicit answer for a key the key
    /// schema refuses: `Ignore` and `Strip` both drop it, because a Record has
    /// no source map to leave it in, and `Reject` names it as an excess
    /// property on either side. That is what the pin's `onExcessProperty`
    /// option decides at the call site.
    pub fn record2_excess(
        key: Schema<Unknown>,
        value: Schema<Unknown>,
        policy: ExcessProperty,
    ) -> Self {
        let kind = SchemaKind::Record(Box::new(key.repr_kind()), Box::new(value.repr_kind()));
        // An exact-value key schema (`Record(Literals([...]), ...)`) names a
        // fixed set of keys, and the pin requires every one of them. An
        // `optionalKey` value is how the pin spells "this key may be absent",
        // so with one there is nothing to require; a pattern key
        // (`patternProperties`) admits no fixed set in the first place.
        let required = match value.repr_kind() {
            SchemaKind::OptionalKey(_) => Vec::new(),
            _ => exact_key_names(&key),
        };
        Self::record_shape(Some(key), value, required, policy).with_kind(kind)
    }

    /// A record shape (`Record` shape) over an index signature: every value
    /// decodes with the value schema; keys pass through. Empty objects decode
    /// fine.
    ///
    /// An `optionalKey` value is refused here rather than silently accepted:
    /// under an index signature there is no key to be absent, so the pin
    /// refuses to build the shape at all. `Schema::optional` is the spelling
    /// that means "this value may be absent".
    pub fn record(value: Schema<Unknown>) -> Self {
        assert!(
            !matches!(value.repr_kind(), SchemaKind::OptionalKey(_)),
            "Cannot use `Schema.optionalKey` with index signatures, use `Schema.optional` instead."
        );
        Self::record_shape(None, value, Vec::new(), ExcessProperty::Ignore)
    }

    /// The record walk both constructors share. `required_keys` is empty for an
    /// index signature and holds the exact-value key schema's names otherwise.
    fn record_shape(
        key: Option<Schema<Unknown>>,
        value: Schema<Unknown>,
        required_keys: Vec<String>,
        policy: ExcessProperty,
    ) -> Self {
        let value = Arc::new(value);
        let render = value.formatter.clone();
        // The key schema decides both directions: a key it refuses is an entry
        // the record does not describe, so the policy is what says whether that
        // entry is dropped or named.
        fn excess_entry(policy: ExcessProperty, name: &str) -> Result<bool, SchemaError> {
            match policy {
                ExcessProperty::Reject => Err(SchemaError::TypeMismatch {
                    path: format!("$.{name}"),
                    expected: "no excess property",
                    got: "excess",
                }),
                _ => Ok(false),
            }
        }
        // A value that arrived here through a re-typing has an opaque kind of
        // its own, so the Record has to copy the kind underneath it.
        let value_kind = value.repr_kind();
        let key = key.map(Arc::new);
        let decode_key = key.clone();
        let encode_key = key.clone();
        let mut out = Self::new(
            move |unknown| match unknown {
                Unknown::Object(fields) => {
                    for name in &required_keys {
                        if !fields.contains_key(name) {
                            return Err(SchemaError::TypeMismatch {
                                path: format!("$.{name}"),
                                expected: "a declared key",
                                got: "missing",
                            });
                        }
                    }
                    let mut decoded = std::collections::BTreeMap::new();
                    for (name, item) in fields {
                        // A key the key schema refuses is an entry this record
                        // does not describe, so by default it is dropped rather
                        // than failing the whole record. That is what makes
                        // `Record(String.check(isPattern(/^a/)), Number)`
                        // select its keys instead of rejecting them.
                        if let Some(key) = &decode_key
                            && !key_names_entry(key, &name)
                        {
                            excess_entry(policy, &name)?;
                            continue;
                        }
                        let item = value
                            .decode(item)
                            .map_err(|error| error.at(&format!("$.{name}")))?;
                        decoded.insert(name, item);
                    }
                    Ok(decoded)
                }
                other => mismatch("$", "object", &other),
            },
            // Encoding walks the same key rule, so a map carrying a key the
            // key schema does not describe is dropped or named on the way out
            // exactly as it is on the way in.
            move |fields: &std::collections::BTreeMap<String, Unknown>| {
                if let Some(key) = &encode_key {
                    let mut encoded = std::collections::BTreeMap::new();
                    for (name, item) in fields {
                        if !key_names_entry(key, name) {
                            excess_entry(policy, name)?;
                            continue;
                        }
                        encoded.insert(name.clone(), item.clone());
                    }
                    return Ok(Unknown::Object(encoded));
                }
                Ok(Unknown::Object(fields.clone()))
            },
            SchemaKind::Record(
                Box::new(
                    key.as_ref()
                        .map(|key| key.repr_kind())
                        .unwrap_or(SchemaKind::String),
                ),
                Box::new(value_kind.clone()),
            ),
        );
        // A Record renders its entries spaced, the way the pin does, and each
        // entry renders through the value schema so a nested Record or Struct
        // recurses with the same rule.
        out.formatter = Some(Arc::new(move |value: &Unknown| {
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            // An empty object renders bare, the same rule the Struct follows.
            if map.is_empty() {
                return "{}".to_string();
            }
            let inner: Vec<String> = map
                .iter()
                .map(|(key, item)| {
                    let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!(
                        "{key}: {}",
                        match &render {
                            Some(r) => r(item),
                            None => format_root(&value_kind, &[], item),
                        }
                    )
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }));
        out
    }

    /// A subset of fields (`pick` shape): only the named fields decode.
    pub fn pick(fields: Vec<Field>, names: &[&str]) -> Self {
        let wanted: Vec<Field> = fields
            .into_iter()
            .filter(|field| names.contains(&field.name.as_str()))
            .collect();
        Self::struct_shape(wanted)
    }

    /// Derive a struct schema from this one's declared fields (`mapFields`):
    /// the transformer receives the field list and returns the list the derived
    /// shape should have. A schema that is not a struct shape has no field list
    /// to map, so asking one is a mistake rather than a no-op.
    pub fn map_fields(self, transform: impl Fn(Vec<Field>) -> Vec<Field>) -> Self {
        let fields = self
            .struct_fields
            .clone()
            .expect("mapFields needs a struct shape with declared fields");
        Self::struct_shape(transform(fields))
    }

    /// Rename keys on the encoded side (`encodeKeys`): decoding accepts the
    /// destination spelling and hands the field its source name, encoding does
    /// the reverse. Two destinations that collide, or a destination that lands
    /// on a field nobody is renaming, would make two fields share one wire key,
    /// so both are refused where the mapping is built.
    pub fn encode_keys(self, pairs: Vec<(&str, &str)>) -> Self {
        let mut destinations: Vec<String> = Vec::new();
        for (_, destination) in &pairs {
            if destinations.iter().any(|seen| seen == destination) {
                panic!("Duplicate encoded keys: {destination}");
            }
            destinations.push((*destination).to_string());
        }
        let sources: Vec<String> = pairs
            .iter()
            .map(|(source, _)| (*source).to_string())
            .collect();
        if let Some(fields) = self.struct_fields.as_ref() {
            for (_, destination) in &pairs {
                let collides = fields
                    .iter()
                    .any(|field| field.name == *destination && !sources.contains(&field.name));
                if collides {
                    panic!("Duplicate encoded keys: {destination}");
                }
            }
        }
        let inner = Arc::new(self);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let decode_pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(source, destination)| ((*source).to_string(), (*destination).to_string()))
            .collect();
        let encode_pairs = decode_pairs.clone();
        let mut out = Self::new(
            move |unknown| {
                let renamed = rename_keys(unknown, &decode_pairs, false);
                decode_inner.decode(renamed)
            },
            move |value| {
                let encoded = encode_inner.try_encode(value)?;
                Ok(rename_keys(encoded, &encode_pairs, true))
            },
            SchemaKind::Object,
        );
        out.struct_fields = inner.struct_fields.clone();
        out
    }

    /// Add fields derived from the decoded value (`extendTo`): decoding fills
    /// each added key from its getter, and encoding drops the key again so the
    /// wire stays the base's.
    ///
    /// A getter that answers `None` leaves its key absent, the pin's
    /// `Option.none()`: the field is missing rather than null. The added fields
    /// are part of the declared shape, so a later derivation (`mapFields`, the
    /// tagged-union index) sees them.
    pub fn extend_to(
        base: Schema<BTreeMap<String, Unknown>>,
        fields: Vec<Field>,
        derive: Vec<(String, DeriveFn)>,
    ) -> Self {
        let mut declared = base.struct_fields.clone().unwrap_or_default();
        declared.extend(fields);
        let inner = Arc::new(base);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let decode_derive = derive.clone();
        let encode_derive = derive;
        let mut out = Self::new(
            move |unknown| {
                let value = decode_inner.decode(unknown)?;
                let mut extended = value.clone();
                for (name, getter) in &decode_derive {
                    if let Some(filled) = getter(&value) {
                        extended.insert(name.clone(), filled);
                    }
                }
                Ok(extended)
            },
            move |value| {
                let mut stripped = value.clone();
                for (name, _) in &encode_derive {
                    stripped.remove(name);
                }
                encode_inner.try_encode(&stripped)
            },
            SchemaKind::Object,
        );
        out.struct_fields = Some(declared);
        out
    }
}

/// What an `extendTo` field derives from the decoded base value: `Some` fills
/// the key, `None` leaves it out.
pub type DeriveFn = Arc<dyn Fn(&BTreeMap<String, Unknown>) -> Option<Unknown> + Send + Sync>;

/// Move an object's keys between two spellings (`encodeKeys`):
/// `to_destination` picks the direction. Anything that is not an object passes
/// through, since the schema underneath is what refuses it.
fn rename_keys(unknown: Unknown, pairs: &[(String, String)], to_destination: bool) -> Unknown {
    let Unknown::Object(mut fields) = unknown else {
        return unknown;
    };
    for (source, destination) in pairs {
        let (from, to) = if to_destination {
            (source, destination)
        } else {
            (destination, source)
        };
        if let Some(value) = fields.remove(from) {
            fields.insert(to.clone(), value);
        }
    }
    Unknown::Object(fields)
}

impl<T: Send + Sync + 'static> Schema<Option<T>> {
    /// `Schema.Option(inner)`: the boxed option.
    ///
    /// The wire form is the tag object the pin's codec links to,
    /// `{ "_tag": "None" }` or `{ "_tag": "Some", value }`, while the value is
    /// an `Option`. `None` is a value here, not an absent key, which is what
    /// tells this apart from [`optional`](Self::optional): that one reads a
    /// null, this one refuses it.
    pub fn option(inner: Schema<T>) -> Schema<Option<T>> {
        let kind = SchemaKind::Union {
            members: vec![
                SchemaKind::Struct(vec![
                    (
                        "_tag".to_string(),
                        SchemaKind::Literal(Unknown::Text("Some".to_string())),
                    ),
                    ("value".to_string(), inner.repr_kind()),
                ]),
                SchemaKind::Struct(vec![(
                    "_tag".to_string(),
                    SchemaKind::Literal(Unknown::Text("None".to_string())),
                )]),
            ],
            mode: "anyOf",
        };
        let inner = Arc::new(inner);
        let erased = Some(Arc::new(Schema::as_unknown_shared(inner.clone())));
        let decode_inner = inner.clone();
        let encode_inner = inner;
        let mut schema = Schema::new(
            move |unknown| {
                let Unknown::Object(fields) = unknown else {
                    return mismatch("$", "Option", &unknown);
                };
                match fields.get("_tag") {
                    Some(Unknown::Text(tag)) if tag == "Some" => match fields.get("value") {
                        Some(value) => decode_inner
                            .decode(value.clone())
                            .map(Some)
                            .map_err(|error| error.at("$.value")),
                        // `Some` without a value is not a shape the box has.
                        None => Err(SchemaError::TypeMismatch {
                            path: "$.value".to_string(),
                            expected: "required field",
                            got: "missing",
                        }),
                    },
                    Some(Unknown::Text(tag)) if tag == "None" => Ok(None),
                    _ => mismatch("$", "Option", &Unknown::Object(fields)),
                }
            },
            move |value| {
                let mut fields = BTreeMap::new();
                match value {
                    None => {
                        fields.insert("_tag".to_string(), Unknown::Text("None".to_string()));
                    }
                    Some(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Some".to_string()));
                        fields.insert("value".to_string(), encode_inner.try_encode(value)?);
                    }
                }
                Ok(Unknown::Object(fields))
            },
            kind,
        );
        schema.option_value = erased;
        schema
    }

    /// `Schema.Option(...).value`: the schema the box wraps, readable through
    /// an annotation.
    pub fn value(&self) -> Option<&Schema<Unknown>> {
        self.option_value.as_deref()
    }

    /// `null` (or missing, by the caller's hand) decodes to `None`.
    pub fn optional(inner: Schema<T>) -> Schema<Option<T>> {
        // A value that arrived here through a re-typing carries an opaque kind
        // of its own, so the Optional has to copy the kind underneath it or a
        // repr prints `Schema.optional(Schema.Unknown)`.
        let kind = SchemaKind::Optional(Box::new(inner.repr_kind()));
        // The relation is the value schema's on the `Some` side and the tag on
        // the `None` side, which is the pin's `effect/schema/Option`
        // declaration rule: an absent value is equivalent only to another
        // absent one, so a `None` never matches a `Some`.
        let inner_relation = inner.to_equivalence();
        let inner = Arc::new(inner);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let mut schema = Schema::new(
            move |unknown| match unknown {
                Unknown::Null => Ok(None),
                other => decode_inner.decode(other).map(Some),
            },
            move |value| match value {
                None => Ok(Unknown::Null),
                Some(value) => encode_inner.try_encode(value),
            },
            kind,
        );
        schema.equivalence = Some(Arc::new(move |a: &Option<T>, b: &Option<T>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => inner_relation(a, b),
            _ => false,
        }));
        schema
    }
}

impl<A: Send + Sync + 'static, E: Send + Sync + 'static> Schema<Result<A, E>> {
    /// `Schema.Result(success, failure)`: the boxed result.
    ///
    /// The wire form is the tag object the pin's codec links to,
    /// `{ "_tag": "Success", success }` or `{ "_tag": "Failure", failure }`,
    /// while the value is a `Result`.
    pub fn result(success: Schema<A>, failure: Schema<E>) -> Schema<Result<A, E>> {
        let tag = |name: &str| {
            (
                "_tag".to_string(),
                SchemaKind::Literal(Unknown::Text(name.to_string())),
            )
        };
        let kind = SchemaKind::Union {
            members: vec![
                SchemaKind::Struct(vec![
                    tag("Success"),
                    ("success".to_string(), success.repr_kind()),
                ]),
                SchemaKind::Struct(vec![
                    tag("Failure"),
                    ("failure".to_string(), failure.repr_kind()),
                ]),
            ],
            mode: "anyOf",
        };
        let success = Arc::new(success);
        let failure = Arc::new(failure);
        let sides = Some((
            Arc::new(Schema::as_unknown_shared(success.clone())),
            Arc::new(Schema::as_unknown_shared(failure.clone())),
        ));
        let decode_success = success.clone();
        let decode_failure = failure.clone();
        let encode_success = success;
        let encode_failure = failure;
        let mut schema = Schema::new(
            move |unknown| {
                let Unknown::Object(fields) = unknown else {
                    return mismatch("$", "Result", &unknown);
                };
                let missing = |side: &str| SchemaError::TypeMismatch {
                    path: format!("$.{side}"),
                    expected: "required field",
                    got: "missing",
                };
                match fields.get("_tag") {
                    Some(Unknown::Text(tag)) if tag == "Success" => match fields.get("success") {
                        Some(value) => decode_success
                            .decode(value.clone())
                            .map(Ok)
                            .map_err(|error| error.at("$.success")),
                        None => Err(missing("success")),
                    },
                    Some(Unknown::Text(tag)) if tag == "Failure" => match fields.get("failure") {
                        Some(value) => decode_failure
                            .decode(value.clone())
                            .map(Err)
                            .map_err(|error| error.at("$.failure")),
                        None => Err(missing("failure")),
                    },
                    _ => mismatch("$", "Result", &Unknown::Object(fields)),
                }
            },
            move |value| {
                let mut fields = BTreeMap::new();
                match value {
                    Ok(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Success".to_string()));
                        fields.insert("success".to_string(), encode_success.try_encode(value)?);
                    }
                    Err(value) => {
                        fields.insert("_tag".to_string(), Unknown::Text("Failure".to_string()));
                        fields.insert("failure".to_string(), encode_failure.try_encode(value)?);
                    }
                }
                Ok(Unknown::Object(fields))
            },
            kind,
        );
        schema.result_sides = sides;
        schema
    }

    /// `Schema.Result(...).success`: the success schema the box wraps.
    pub fn success(&self) -> Option<&Schema<Unknown>> {
        self.result_sides
            .as_ref()
            .map(|(success, _)| success.as_ref())
    }

    /// `Schema.Result(...).failure`: the failure schema the box wraps.
    pub fn failure(&self) -> Option<&Schema<Unknown>> {
        self.result_sides
            .as_ref()
            .map(|(_, failure)| failure.as_ref())
    }
}

impl Schema<Unknown> {
    /// Import a JSON Schema document (`fromJsonSchemaDocument` shapes):
    /// objects with `type`, `properties`/`required`, `items`, `enum`,
    /// `const` and `anyOf` import; `$ref` and anything else is a typed
    /// error naming the gap.
    /// The schema-level annotations an imported document carries. `expected` is
    /// not one of them: that is the open-object marker, written by
    /// `annotated_json` and dropped again by an intersection.
    fn document_annotations(doc: &serde_json::Value) -> Vec<(String, String)> {
        // `identifier` rides with the doc fields because that is where the pin
        // puts it: a `$defs` entry carries one and the repr prints it as an
        // annotation on the schema.
        // The built-in JSON Schema annotations. `contentSchema` is the one that
        // is not a string, so it is lifted as its raw JSON rather than as text.
        [
            "title",
            "description",
            "identifier",
            "format",
            "contentEncoding",
            "contentMediaType",
            "contentSchema",
        ]
        .into_iter()
        .filter_map(|key| {
            doc.get(key).map(|value| {
                // A string annotation lifts its text; anything else lifts its
                // JSON, which is what `contentSchema`'s object needs.
                let text = match value {
                    serde_json::Value::String(text) => text.clone(),
                    // Anything else is JSON the repr must print as JSON, so
                    // it carries the marker rather than being quoted.
                    other => format!("{RAW_ANNOTATION}{}", other),
                };
                (key.to_string(), text)
            })
        })
        .collect()
    }

    /// Any JSON value annotated as open. The `expected` message always comes
    /// first, and the pin's other annotation keywords ride along after it.
    fn annotated_json(doc: &serde_json::Value) -> Schema<Unknown> {
        let mut pairs = vec![("expected".to_string(), "JSON value".to_string())];
        pairs.extend(Self::document_annotations(doc));
        Schema::annotated_with(Schema::json(), pairs)
    }

    /// The open-object catch-all: the annotated JSON value as a string-keyed
    /// record. Every open form in the import uses this exact shape, both as a
    /// whole object and as the rest of a struct, so it is named once rather
    /// than spelled at each use.
    fn json_catch_all() -> Schema<Unknown> {
        Schema::as_unknown(Schema::record(Schema::annotated_json(&serde_json::json!(
            {}
        ))))
    }

    /// Wrap a checked schema so the check is visible to a repr. `as_unknown`
    /// would otherwise hide it behind its opaque kind.
    fn as_unknown_checked(base: Schema<Unknown>, name: &str, expected: &str) -> Self {
        Schema::as_unknown(Schema::checked(
            base,
            name,
            Vec::new(),
            vec![("expected", expected)],
        ))
    }

    /// Apply every refinement the document carries to a base that already has
    /// one. A typed import runs this with its own base; a typeless one runs it
    /// over the open JSON value, which is what an `allOf` member is.
    /// The refinements are grouped by the keyword family they belong to, so a
    /// document cannot pick up a string check and a number check at once.
    fn refined_any(base: Schema<Unknown>, doc: &serde_json::Value) -> Self {
        let string_keywords = [
            "minLength",
            "maxLength",
            "minCodePoints",
            "maxCodePoints",
            "pattern",
        ];
        let number_keywords = [
            "minimum",
            "maximum",
            "exclusiveMinimum",
            "exclusiveMaximum",
            "multipleOf",
        ];
        let array_keywords = ["minItems", "maxItems", "uniqueItems"];
        if string_keywords
            .iter()
            .any(|keyword| doc.get(keyword).is_some())
        {
            Self::refined_text_on(base, doc)
        } else if number_keywords
            .iter()
            .any(|keyword| doc.get(keyword).is_some())
        {
            Self::refined_number(base, doc)
        } else if array_keywords
            .iter()
            .any(|keyword| doc.get(keyword).is_some())
        {
            Self::refined_array(base, doc)
        } else {
            base
        }
    }

    /// A string plus whatever length or pattern constraints the document
    /// carries. `maxLength` is a length in code points in the pin's import, so
    /// it renders as `isMaxCodePoints`; `minLength` is the one keyword whose
    /// filter depends on its value, which is why it is spelled out below.
    fn refined_text(doc: &serde_json::Value) -> Self {
        Self::refined_text_on(Schema::as_unknown(Schema::string()), doc)
    }

    /// The same as `refined_text`, for a base that already exists: an `allOf`
    /// member's checks land on the base it is intersected with rather than on a
    /// fresh string.
    fn refined_text_on(base: Schema<Unknown>, doc: &serde_json::Value) -> Self {
        let mut schema = base;
        // `minLength: 1` is the non-empty case the pin keeps as `isMinLength`,
        // and every other value counts code points as `isMinCodePoints`, which
        // is what makes a two-code-unit emoji one code point long. The two
        // filters carry different messages, and each names its own unit.
        if let Some(number) = doc.get("minLength").and_then(serde_json::Value::as_f64) {
            let printed = format_number(number);
            let (name, message) = if number == 1.0 {
                (
                    "isMinLength",
                    format!("a value with a length of at least {printed}"),
                )
            } else {
                (
                    "isMinCodePoints",
                    format!("a string with at least {printed} code points"),
                )
            };
            schema = Schema::as_unknown(Schema::checked(
                schema,
                name,
                vec![printed.as_str()],
                vec![("expected", message.as_str())],
            ));
        }
        for (keyword, name, message) in [
            (
                "maxLength",
                "isMaxCodePoints",
                "a string with at most {n} code points",
            ),
            (
                "minCodePoints",
                "isMinCodePoints",
                "a string with at least {n} code points",
            ),
            (
                "maxCodePoints",
                "isMaxCodePoints",
                "a string with at most {n} code points",
            ),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                // A code-point message names the count inline, so `{n}` is
                // filled here rather than appended after the format.
                let message = message.replace("{n}", &printed);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &message)],
                ));
            }
        }
        if let Some(pattern) = doc.get("pattern").and_then(serde_json::Value::as_str) {
            schema = Schema::as_unknown(Schema::checked(
                schema,
                "isPattern",
                vec![regex_argument(pattern).as_str()],
                vec![("expected", regex_message(pattern).as_str())],
            ));
        }
        schema
    }

    /// A pattern-keyed mapping (`patternProperties`): every key is checked
    /// against the pattern and every value decodes with the one pattern schema.
    /// The pin refuses more than one pattern, because a record's key shape is a
    /// single schema and two patterns would need an intersection it cannot spell.
    fn patterned_record(doc: &serde_json::Value) -> Result<Self, SchemaError> {
        let Some(serde_json::Value::Object(patterns)) = doc.get("patternProperties") else {
            return Ok(Schema::as_unknown(Schema::record(Schema::json())));
        };
        let mut members = patterns.iter();
        let Some((pattern, value)) = members.next() else {
            return Ok(Schema::as_unknown(Schema::record(Schema::json())));
        };
        if members.next().is_some() {
            return Err(SchemaError::TypeMismatch {
                path: "$.patternProperties".to_string(),
                expected: "at most one pattern",
                got: "more than one pattern",
            });
        }
        let key = Schema::as_unknown(Schema::checked(
            Schema::as_unknown(Schema::string()),
            "isPattern",
            vec![regex_argument(pattern).as_str()],
            vec![("expected", regex_message(pattern).as_str())],
        ));
        Ok(Schema::as_unknown(Schema::record2(
            key,
            Self::import(value)?,
        )))
    }

    /// The object-level checks: a bound on the number of entries, and a schema
    /// every key must satisfy. Both ride a record, which is the shape an open
    /// object already has, so they are checks rather than a different base.
    fn refined_object(base: Schema<Unknown>, doc: &serde_json::Value) -> Result<Self, SchemaError> {
        let mut schema = base;
        for (keyword, name, message) in [
            ("minProperties", "isMinProperties", "a value with at least"),
            ("maxProperties", "isMaxProperties", "a value with at most"),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &format!("{message} {printed} entry"))],
                ));
            }
        }
        if let Some(names) = doc.get("propertyNames") {
            // The key schema stands in the check's argument slot, which is where
            // a repr prints it: the check is over the object, but what it
            // constrains is each key.
            // A JSON object key is always a string, so a `propertyNames` value
            // that names no type constrains a string. An untyped import would
            // otherwise start from the open JSON value.
            // A key is a string, and the pin measures one in CODE POINTS, so a
            // `minLength` beside `propertyNames` is a code-point length. The
            // document is rewritten before importing rather than special-cased
            // afterwards, so the check's own message names the right unit.
            let names = match names {
                serde_json::Value::Object(map) => {
                    let mut map = map.clone();
                    if let Some(value) = map.remove("minLength") {
                        map.insert("minCodePoints".to_string(), value);
                    }
                    if let Some(value) = map.remove("maxLength") {
                        map.insert("maxCodePoints".to_string(), value);
                    }
                    serde_json::Value::Object(map)
                }
                other => other.clone(),
            };
            let inner = Self::import(&names)?;
            let (kind, checks, annotation) = decompose(&inner.repr_kind());
            let inner = if kind == SchemaKind::Json {
                let annotation: Vec<(String, String)> = annotation
                    .into_iter()
                    .filter(|(key, value)| !(key == "expected" && value == "JSON value"))
                    .collect();
                rebuild(SchemaKind::String, checks, annotation)
            } else {
                inner
            };
            schema = Schema::as_unknown(Schema::checked(
                schema,
                "isPropertyNames",
                vec![inner.to_code().as_str()],
                vec![(
                    "expected",
                    "an object with property names matching the schema",
                )],
            ));
        }
        Ok(schema)
    }

    /// A number plus whatever bound constraints the document carries. The
    /// pin distinguishes an exclusive from an inclusive bound with two check
    /// names, so each keyword maps to exactly one.
    fn refined_number(base: Schema<Unknown>, doc: &serde_json::Value) -> Self {
        let mut schema = base;
        for (keyword, name, message) in [
            (
                "minimum",
                "isGreaterThanOrEqualTo",
                "a value greater than or equal to",
            ),
            (
                "maximum",
                "isLessThanOrEqualTo",
                "a value less than or equal to",
            ),
            ("exclusiveMinimum", "isGreaterThan", "a value greater than"),
            ("exclusiveMaximum", "isLessThan", "a value less than"),
            (
                "multipleOf",
                "isMultipleOf",
                "a value that is a multiple of",
            ),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &format!("{message} {printed}"))],
                ));
            }
        }
        schema
    }

    /// A positional prefix (`prefixItems`) as a tuple. The pin's rule is that
    /// `minItems` marks the leading positions required, `maxItems` decides
    /// whether any position is open, and `items` names the open element:
    ///
    /// - `maxItems` above the prefix length keeps a rest, but that rest can hold
    ///   fewer elements than the prefix declares, so a max-length check rides
    ///   along;
    /// - `maxItems` equal to the prefix length, or `items: false`, closes the
    ///   tuple and drops the rest entirely.
    fn tuple_from_prefix(
        prefix: &[serde_json::Value],
        doc: &serde_json::Value,
    ) -> Result<Self, SchemaError> {
        let min = doc
            .get("minItems")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        let max = doc
            .get("maxItems")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize);
        let closed_by_items = matches!(doc.get("items"), Some(serde_json::Value::Bool(false)));

        let mut elements = Vec::with_capacity(prefix.len());
        for (index, element) in prefix.iter().enumerate() {
            let schema = Self::import(element)?;
            // A position below `minItems` must be present; the rest may be
            // absent, which is exactly `optionalKey` against `Optional`.
            elements.push(if index < min {
                schema
            } else {
                Schema::optional_key_shape(schema)
            });
        }

        // Open unless `maxItems` exactly equals the prefix length or `items`
        // closed it. Any other `maxItems` keeps the rest and carries a
        // max-length check, even a `maxItems` below the prefix length, because
        // the tuple alone would allow every declared position.
        let open = match max {
            Some(max) => max != prefix.len() && !closed_by_items,
            None => !closed_by_items,
        };
        if !open {
            // Neither `refined_array` nor a check: `maxItems` was spent closing
            // the tuple and `minItems` was spent marking the leading positions
            // required, so both are already represented in the shape.
            return Ok(Schema::as_unknown(Schema::tuple_of(elements)));
        }
        let rest = match doc.get("items") {
            Some(serde_json::Value::Bool(_)) | None => {
                Schema::annotated_with(Schema::json(), vec![("expected", "JSON value")])
            }
            Some(items) => Self::import(items)?,
        };
        let base = Schema::as_unknown(Schema::tuple_with_rest(elements, vec![rest]));
        // A rest that cannot hold every declared position also needs the
        // max-length check, because the tuple alone would allow them. The check
        // is attached whenever `maxItems` is named at all.
        let base = match max {
            Some(max) => Schema::as_unknown(Schema::checked(
                base,
                "isMaxLength",
                vec![format_number(max as f64).as_str()],
                vec![(
                    "expected",
                    &format!("a value with a length of at most {max}"),
                )],
            )),
            None => base,
        };
        // `refined_array` is deliberately not called here: it maps `maxItems` to
        // the same max-length check, and the open form already attached it.
        Ok(base)
    }

    /// An array plus whatever size or uniqueness constraints the document
    /// carries.
    fn refined_array(base: Schema<Unknown>, doc: &serde_json::Value) -> Self {
        let mut schema = base;
        for (keyword, name, message) in [
            (
                "minItems",
                "isMinLength",
                "a value with a length of at least",
            ),
            (
                "maxItems",
                "isMaxLength",
                "a value with a length of at most",
            ),
        ] {
            if let Some(number) = doc.get(keyword).and_then(serde_json::Value::as_f64) {
                let printed = format_number(number);
                schema = Schema::as_unknown(Schema::checked(
                    schema,
                    name,
                    vec![printed.as_str()],
                    vec![("expected", &format!("{message} {printed}"))],
                ));
            }
        }
        if doc.get("uniqueItems").and_then(serde_json::Value::as_bool) == Some(true) {
            schema = Schema::as_unknown(Schema::checked(
                schema,
                "isUnique",
                Vec::new(),
                vec![("expected", "an array with unique items")],
            ));
        }
        schema
    }

    pub fn from_json_schema(doc: &serde_json::Value) -> Result<Self, SchemaError> {
        // `$defs` is scoped to the root, so it is captured once here rather than
        // discovered at each use. A document with none still resets it, so a
        // second import does not read the first one's definitions.
        let defs = doc
            .get("$defs")
            .or_else(|| doc.get("definitions"))
            .and_then(serde_json::Value::as_object)
            .cloned();
        ROOT_DEFS.with(|slot| *slot.borrow_mut() = defs);
        Self::import(doc)
    }

    /// Only a primitive value can be named by `const` or `enum`: an object or
    /// an array has no literal form, and the pin refuses it with the keyword's
    /// path and the member's index rather than approximating one.
    fn require_primitive(path: String, value: &serde_json::Value) -> Result<(), SchemaError> {
        if matches!(
            value,
            serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::String(_)
        ) {
            return Ok(());
        }
        Err(SchemaError::TypeMismatch {
            path,
            expected: "a primitive value, because only primitive values are supported in \"const\" and \"enum\"",
            got: "a structured value",
        })
    }

    /// Keep only the exact values that satisfy the checks the document's other
    /// keywords contribute. A `const` or an `enum` beside a refinement is that
    /// refinement applied to each named value, which is exactly what an
    /// intersection does to a literal union.
    fn filter_exact_values(values: Vec<Unknown>, doc: &serde_json::Value) -> Vec<Unknown> {
        if values.is_empty() {
            return values;
        }
        let mut siblings = doc.clone();
        if let Some(object) = siblings.as_object_mut() {
            object.remove("const");
            object.remove("enum");
        }
        let refinement = Self::refined_any(Self::annotated_json(&siblings), &siblings);
        let (_, checks, _) = decompose(&refinement.repr_kind());
        filter_by_checks(values, &checks)
    }

    /// Intersect two imported schemas. Both are split into their base kind,
    /// their checks and their leading annotations; the member's checks are then
    /// appended to the base's, dropping any the base already has, and the
    /// member's annotations ride along with the checks it contributed.
    ///
    /// A schema-level annotation on the member attaches to the group of checks
    /// the member left behind: to the one check if it left exactly one, to the
    /// base's own annotation if it left none (its only check was a duplicate),
    /// and to a `makeFilterGroup` if it left several. That is the rule the pin's
    /// own intersection rows assert, and it is why the same `description` lands
    /// in three different places.
    ///
    /// A member that names no type is not a different base: it is a bare set of
    /// checks constraining whatever it is intersected with, so the base's kind
    /// stands and only the open-object marker drops from its annotations.
    fn intersect(base: Schema<Unknown>, member: Schema<Unknown>) -> Result<Self, SchemaError> {
        // A base that names no type is not a base at all: it is the open JSON
        // value, so whichever side names a type supplies the base, and its checks
        // and annotations come with it. This is the shape
        // `{ allOf: [{ type: "string", ... }] }` has, because the surrounding
        // keywords are empty once `allOf` is removed. Swapping only the kinds
        // would leave the open value's own annotation behind, which printed a
        // duplicate `"expected": "JSON value"` on the base.
        // A refinement intersected with a reference must deref it, but only
        // when the refinement carries checks: an annotating member still wants
        // the reference preserved (see :1442). When member_checks is empty, the
        // base keeps its Reference shape, and annotations propagate naturally.
        let (member_kind_tmp, member_checks_tmp, _member_annotation_tmp) =
            decompose(&member.repr_kind());
        let _ = member_kind_tmp;
        let base = if !member_checks_tmp.is_empty()
            && matches!(base.repr_kind(), SchemaKind::Reference(name))
        {
            // The closure capture needs `name` moved out:
            let name = match base.repr_kind() {
                SchemaKind::Reference(n) => n.clone(),
                _ => unreachable!("guarded above"),
            };
            ROOT_DEFS
                .with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .and_then(|defs| defs.get(&name))
                        .cloned()
                })
                .and_then(|def_doc| Schema::<Unknown>::import(&def_doc).ok())
                .unwrap_or(base)
        } else {
            base
        };
        let (base_kind, base_checks, base_annotation) = decompose(&base.repr_kind());
        let (member_kind, member_checks, member_annotation) = decompose(&member.repr_kind());
        let member_annotation: Vec<(String, String)> = member_annotation
            .into_iter()
            .filter(|(key, value)| !(key == "expected" && value == "JSON value"))
            .collect();
        // `never` swallows the intersection: nothing is both `never` and
        // anything else. A `false` member reaches here as `never`, and the pin
        // answers `Never` rather than refusing the document, which is also what
        // makes `{ allOf: [false, { type: "string" }] }` hold nothing instead
        // of failing to import.
        if base_kind == SchemaKind::Opaque("never") || member_kind == SchemaKind::Opaque("never") {
            return Ok(Schema::as_unknown(Schema::never()));
        }
        // A refinement intersected with a finite set of exact values keeps the
        // values that satisfy it. Either side may carry the values, so both are
        // tried. A side that names a different base has no compatible member.
        if let Some(values) = exact_values(&member_kind) {
            // The base's own type also decides: a value the base cannot hold is
            // not a surviving value, which is what makes a mixed enum under a
            // string base keep its strings alone.
            let kept: Vec<Unknown> = filter_by_checks(values, &base_checks)
                .into_iter()
                .filter(|value| value_fits_kind(&base_kind, value))
                // A base that carries exact values of its own decides too: the
                // intersection of two sets of values is the values in both, so
                // `{ const: 1 }` beside `{ enum: [1, 2] }` keeps 1 alone and
                // `{ const: 1 }` beside `{ const: 2 }` keeps nothing. Keeping
                // only one side is what let 2 through a document that names 1.
                .filter(|value| match exact_values(&base_kind) {
                    Some(base_values) => base_values.contains(value),
                    None => true,
                })
                .collect();
            // The member's own annotation rides the values that survived it.
            // The base's annotation went with the base kind the values
            // replaced, which is why only the member's description survives.
            let union = literal_union(kept);
            return Ok(if member_annotation.is_empty() {
                union
            } else {
                Schema::annotated_with(union, member_annotation)
            });
        }
        // The other direction: the base holds the values and the member holds
        // the refinement. A member whose own base is `never` — an `anyOf` with no
        // compatible member — leaves nothing.
        if let Some(values) = exact_values(&base_kind) {
            // The refining member's own type decides too, by the same rule the
            // other direction uses: a value it cannot hold does not survive.
            let kept = filter_by_checks(values, &member_checks)
                .into_iter()
                .filter(|value| value_fits_kind(&member_kind, value))
                .collect();
            return Ok(literal_union(kept));
        }
        // The base is the side that names a type, so an untyped one defers to
        // whichever member has one. `decompose` strips the annotation first,
        // which is why this looks at the decomposed kind and not the raw one:
        // the open value arrives wrapped in `Annotated`.
        let (base_kind, mut checks, mut annotation, member_kind, member_checks) =
            if base_kind == SchemaKind::Json && member_kind != SchemaKind::Json {
                (
                    member_kind,
                    member_checks,
                    member_annotation.clone(),
                    SchemaKind::Json,
                    base_checks,
                )
            } else {
                (
                    base_kind,
                    base_checks,
                    base_annotation,
                    member_kind,
                    member_checks,
                )
            };
        if member_kind != base_kind && member_kind != SchemaKind::Json {
            match (kind_type_name(&base_kind), kind_type_name(&member_kind)) {
                // Two different named types cannot both hold one value, so the
                // intersection holds none. The pin answers `Never` here rather
                // than refusing the document.
                (Some(base_type), Some(member_type)) if base_type != member_type => {
                    return Ok(Schema::as_unknown(Schema::never()));
                }
                // The same type in two different shapes is a merge this port
                // does not carry: two `prefixItems` lists, two `properties`
                // sets. Refusing names the gap; picking one side would admit
                // values the other refuses.
                _ => {
                    return Err(SchemaError::TypeMismatch {
                        path: "$.allOf".to_string(),
                        expected: "a member intersecting the same base",
                        got: "a member with a different base",
                    });
                }
            }
        }
        let surviving: Vec<Check> = member_checks
            .into_iter()
            .filter(|check| {
                !checks
                    .iter()
                    .any(|seen| seen.name == check.name && seen.args == check.args)
            })
            .collect();
        checks.extend(surviving.clone());
        match surviving.len() {
            // No surviving check of its own, so its annotation belongs to the
            // base, which is where the pin puts it.
            0 => merge_annotation(&mut annotation, member_annotation),
            // Exactly one, so the annotation rides that check.
            1 => {
                let last = checks.last_mut().expect("one check was just pushed");
                merge_annotation(&mut last.annotation, member_annotation);
            }
            // Several checks plus an annotation of the member's own is what the
            // pin groups into a `makeFilterGroup`; several checks with no
            // annotation stay a linear chain.
            _ if !member_annotation.is_empty() => {
                // The grouped checks come back OUT of the chain and stand as the
                // filter group's arguments, so the group replaces them rather
                // than following them. Their own annotations ride along inside
                // each argument, which is what the pin prints.
                let grouped: Vec<Check> = checks.split_off(checks.len() - surviving.len());
                checks.push(Check {
                    name: "makeFilterGroup".to_string(),
                    args: vec![format!(
                        "[{}]",
                        grouped
                            .iter()
                            .map(render_check)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )],
                    annotation: member_annotation,
                    // The members travel with the group so a decode can enforce
                    // them; the rendered argument above is what a repr prints.
                    group: grouped,
                });
            }
            _ => {}
        }
        Ok(rebuild(base_kind, checks, annotation))
    }
}

/// The root document's `$defs`, set once by `from_json_schema` and read by the
/// `$ref` branch anywhere below it.
///
/// A thread-local rather than a parameter because every recursive `import` call
/// site would otherwise have to carry it, and the root is the only document that
/// can name one: JSON Schema scopes `$defs` to the root, so a nested document's
/// own `$defs` is a different scope and the pin refuses it.
/// A pattern scope is OPEN when its catch-all is absent, `true`, or an
/// empty schema — all three mean "anything goes", which is what leaves an
/// unmatched key with no value shape. A non-empty schema is typed, and is
/// refused by a different rule rather than this one.
fn is_open_pattern_scope(doc: &serde_json::Value) -> bool {
    match doc.get("additionalProperties") {
        None | Some(serde_json::Value::Bool(true)) => true,
        Some(serde_json::Value::Object(fields)) => fields.is_empty(),
        _ => false,
    }
}

/// The definition name a reference points at, after the same validation
/// `resolve_ref` does. Used where the NAME is what matters rather than the
/// schema: building a `Reference` kind.
fn ref_name(reference: &str) -> Result<String, SchemaError> {
    if reference.is_empty() {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a reference naming a definition",
            got: "an empty reference",
        });
    }
    let Some(pointer) = reference.strip_prefix("#/$defs/") else {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a \"#/$defs/Name\" reference",
            got: "a reference of another shape",
        });
    };
    if pointer.is_empty() {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a reference naming a definition",
            got: "a reference with no name",
        });
    }
    if pointer.contains('/') {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a \"#/$defs/Name\" reference",
            got: "a reference that walks into a definition",
        });
    }
    Ok(pointer.replace("~1", "/").replace("~0", "~"))
}

/// A definition name as a legal identifier: `/` and `~` both become `_`. The pin
/// prints `A_B_C` for a definition named `A/B~C`, and prints the raw name as that
/// definition's `identifier`.
fn sanitize_definition_name(name: &str) -> String {
    name.replace(['/', '~'], "_")
}

/// An annotation keyword's value as text, so a reference's siblings can be
/// lifted without going through the document-annotation path.
fn annotation_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Resolve `$ref` against the root definitions, or refuse it the way the pin
/// does. The four refusals are distinct, because they are distinct mistakes: an
/// empty reference, one that is not `#/$defs/Name`, a name the root does not
/// define, and a scope nested somewhere other than the root.
fn resolve_ref(reference: &str) -> Result<serde_json::Value, SchemaError> {
    if reference.is_empty() {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a reference naming a definition",
            got: "an empty reference",
        });
    }
    let Some(pointer) = reference.strip_prefix("#/$defs/") else {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a \"#/$defs/Name\" reference",
            got: "a reference of another shape",
        });
    };
    if pointer.is_empty() {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a reference naming a definition",
            got: "a reference with no name",
        });
    }
    // A definition reference names ONE segment. A pointer that keeps walking,
    // like `#/$defs/outer/properties/inner`, is the wrong shape rather than a
    // missing definition, and the pin says so: the escape for a literal slash in
    // a name is `~1`, which arrives here as `~1` and never as `/`.
    if pointer.contains('/') {
        return Err(SchemaError::TypeMismatch {
            path: "$ref".to_string(),
            expected: "a \"#/$defs/Name\" reference",
            got: "a reference that walks into a definition",
        });
    }
    // `~1` and `~0` are the JSON Pointer escapes for `/` and `~`, applied in
    // that order, so a definition may be named with either character.
    let name = pointer.replace("~1", "/").replace("~0", "~");
    let found = ROOT_DEFS
        .with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|defs| defs.get(&name))
                .cloned()
        })
        .ok_or_else(|| SchemaError::TypeMismatchDynamic {
            path: "$ref".to_string(),
            // The dynamic half is the part that names which definition was
            // wanted, which is the whole point of the message.
            expected: format!("a definition named {name:?} for $ref {reference:?}"),
            got: "a definition the root does not define",
        })?;
    Ok(found)
}

impl Schema<Unknown> {
    fn import_object(doc: &serde_json::Value) -> Result<Self, SchemaError> {
        let get = |key: &str| doc.get(key);
        // An OPEN pattern scope cannot be imported whichever other keywords are
        // present: a key that matches no pattern has no value shape to land on,
        // and the pin refuses rather than guessing. `additionalProperties: false`
        // is what closes it, and the rule is checked before the named-keys path so
        // a document carrying both properties and an open pattern refuses too.
        // Open means absent, `true`, or an empty schema. A TYPED one is a
        // different refusal — "cannot combine typed additionalProperties with
        // other property schemas" — and this check must not shadow it, which is
        // why `is_open_pattern_scope` is exact about which shapes count as open.
        if doc.get("patternProperties").is_some() && is_open_pattern_scope(doc) {
            return Err(SchemaError::TypeMismatch {
                path: "$.patternProperties".to_string(),
                expected: "a closed pattern scope (\"additionalProperties\": false)",
                got: "an open one, whose unmatched keys have no value shape",
            });
        }
        let empty = serde_json::Map::new();
        let properties = match get("properties") {
            Some(serde_json::Value::Object(properties)) => properties,
            _ => &empty,
        };
        let required: Vec<&str> = match get("required") {
            Some(serde_json::Value::Array(names)) => {
                names.iter().filter_map(|name| name.as_str()).collect()
            }
            _ => Vec::new(),
        };
        let mut fields = Vec::new();
        for (name, property) in properties {
            let schema = Self::import(property)?;
            // A non-required property is an exact optional: the key
            // may be absent, but an explicit null still decodes the
            // inner schema. That is `optionalKey`, not `optional`.
            let schema = if required.iter().any(|wanted| *wanted == name.as_str()) {
                schema
            } else {
                Schema::optional_key_shape(schema)
            };
            fields.push((name.clone(), schema));
        }
        // The two refusals the pin makes for an object that names a key domain
        // it cannot spell. Both are index signatures over keys the named keys
        // already claimed, which is why neither is approximated: a typed
        // `additionalProperties` beside any other property schema, and a closed
        // patterned object that also names properties.
        let patterned = doc.get("patternProperties").is_some();
        let names_properties = !properties.is_empty() || !required.is_empty();
        match get("additionalProperties") {
            Some(serde_json::Value::Bool(false)) if patterned && names_properties => {
                return Err(SchemaError::TypeMismatch {
                    path: "$.schema".to_string(),
                    expected: "a closed patterned object with one pattern and no properties: Cannot import this closed patterned object: only one pattern without properties is supported",
                    got: "a closed patterned object that also declares properties",
                });
            }
            Some(additional) if !additional.is_boolean() && (names_properties || patterned) => {
                return Err(SchemaError::TypeMismatch {
                    path: "$.schema".to_string(),
                    expected: "no typed additionalProperties beside other property schemas: Cannot combine typed \"additionalProperties\" with other property schemas: Effect index signatures also check excluded keys",
                    got: "a typed additionalProperties beside declared keys",
                });
            }
            _ => {}
        }
        if fields.is_empty() {
            // A pattern-keyed mapping is a record of its own: the pattern
            // becomes the key schema and the one pattern schema the value. It
            // never reaches the named-keys path below, because a pattern key and
            // a named key are different shapes.
            if doc.get("patternProperties").is_some() {
                return Self::patterned_record(doc);
            }
            // No named keys: the whole object is a record, and what
            // `additionalProperties` says decides the value shape.
            return Ok(match get("additionalProperties") {
                // `false` closes it: the value shape is `never`.
                Some(serde_json::Value::Bool(false)) => {
                    Schema::as_unknown(Schema::record(Schema::never()))
                }
                // Absent or `true` opens it to any JSON value.
                Some(serde_json::Value::Bool(true)) | None => Self::json_catch_all(),
                // A typed catch-all on its own is a plain record.
                Some(schema) => Schema::as_unknown(Schema::record(Self::import(schema)?)),
            })
            .and_then(|base| Self::refined_object(base, doc));
        }
        match get("additionalProperties") {
            // `false` closes the object: nothing but the named keys.
            Some(serde_json::Value::Bool(false)) => {
                Ok(Schema::as_unknown(Schema::struct_of(fields)))
            }
            // A typed catch-all next to named keys is refused by the
            // pin, because an index signature also checks the keys
            // the named fields already claimed.
            Some(serde_json::Value::Bool(true)) | None => {
                let rest = Self::json_catch_all();
                Ok(Schema::as_unknown(Schema::struct_with_rest(fields, rest)))
            }
            Some(_) => Err(SchemaError::TypeMismatch {
                path: "$.additionalProperties".to_string(),
                expected: "a boolean, with no other property schemas",
                got: "a schema next to declared properties",
            }),
        }
    }

    fn import(doc: &serde_json::Value) -> Result<Self, SchemaError> {
        let get = |key: &str| doc.get(key);
        // A pattern the pin's own importer would refuse cannot be enforced
        // here, so the import fails at the keyword rather than compiling a
        // matcher that silently accepts or rejects the wrong values. An
        // escape is one such pattern (the matcher interprets no escape class,
        // and ECMAScript Unicode mode rejects an invalid identity escape) and
        // an unterminated class is the other.
        if let Some(pattern) = get("pattern").and_then(serde_json::Value::as_str) {
            if pattern.contains('\\') || compile_pattern(pattern).is_none() {
                return Err(SchemaError::TypeMismatch {
                    path: "$.pattern".to_string(),
                    expected: "a pattern ECMAScript Unicode mode accepts: Cannot import pattern using ECMAScript Unicode mode",
                    got: "a pattern outside the supported subset",
                });
            }
        }
        if let Some(serde_json::Value::String(r#ref)) = get("$ref") {
            // Siblings of a `$ref` are not folded into it: the pin refuses the
            // ones that would change what the reference means, and inlines the
            // reference itself.
            // A reference stays a reference. The pin prints the definition's NAME
            // as the schema's code and type, and prints the definition once,
            // separately, carrying `identifier: <name>` — inlining it here would
            // lose that and print the definition's shape where the pin prints its
            // name.
            // The definition must EXIST, which is a separate refusal from a
            // malformed reference: a name the root does not define is missing
            // data, not a bad shape.
            let resolved = resolve_ref(r#ref)?;
            let name = ref_name(r#ref)?;
            // A definition name is not always a legal identifier: `A/B~C` becomes
            // `A_B_C` in a repr, while the definition itself keeps the raw name
            // as its `identifier`. Both are needed, so the sanitised form is what
            // a `Reference` carries and the raw one what the identifier says. The
            // definition itself is carried too, so decoding does not depend on a
            // global thread-local that a later import can overwrite.
            let sanitized = sanitize_definition_name(&name);
            let annotations = ["title", "description", "identifier", "format"]
                .into_iter()
                .filter_map(|keyword| get(keyword).map(|value| (keyword, value)))
                .collect::<Vec<_>>();
            if annotations.is_empty() && get("allOf").is_none() {
                return Ok(Schema::as_unknown(Schema::reference(&sanitized, resolved)));
            }
            // allOf override: stop talking and hand off to the allOf visitor.
            // Siblings of a reference render through a `suspend`, which is how
            // the pin names the codec's type parameter while keeping the
            // annotation.
            let pairs = annotations
                .into_iter()
                .map(|(keyword, value)| (keyword.to_string(), annotation_text(value)))
                .collect::<Vec<_>>();
            if get("allOf").is_none() {
                return Ok(Schema::as_unknown(Schema::annotated_with(
                    Schema::as_unknown(Schema::reference(&sanitized, resolved)),
                    pairs,
                )));
            }
        }
        // A keyword the port cannot lower is refused by name, the way the pin
        // refuses it. Dropping one instead imports a schema weaker than the
        // document asks for, and the two are indistinguishable to whoever wrote
        // the document: an `unevaluatedProperties: false` that was ignored reads
        // exactly like one that is enforced.
        for (keyword, expected) in UNSUPPORTED_KEYWORDS {
            if get(keyword).is_some() {
                return Err(SchemaError::TypeMismatch {
                    path: format!("$.{keyword}"),
                    expected,
                    got: "a keyword this port cannot lower",
                });
            }
        }
        // `if`, `then` and `else` are only live when their value is something
        // other than `false`: a `false` branch applies to nothing, and the pin's
        // own row asserts the keyword is ignored there. `minContains` and
        // `maxContains` are absent from the table above for the same reason -
        // on their own they constrain nothing at all.
        for keyword in ["if", "then", "else"] {
            if get(keyword).is_some_and(|value| *value != serde_json::Value::Bool(false)) {
                let expected = match keyword {
                    "if" => {
                        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"if\": Effect has no equivalent constraint."
                    }
                    "then" => {
                        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"then\": Effect has no equivalent constraint."
                    }
                    _ => {
                        "a keyword with an equivalent constraint: Cannot import JSON Schema keyword \"else\": Effect has no equivalent constraint."
                    }
                };
                return Err(SchemaError::TypeMismatch {
                    path: format!("$.{keyword}"),
                    expected,
                    got: "a keyword this port cannot lower",
                });
            }
        }
        // `allOf` intersects. Each member contributes its checks to the base's
        // chain and its own annotations to the base, which is what the pin's
        // intersection rows assert.
        if let Some(all_of) = get("allOf") {
            let serde_json::Value::Array(members) = all_of else {
                return Err(SchemaError::TypeMismatch {
                    path: "$.allOf".to_string(),
                    expected: "an array",
                    got: "a non-array",
                });
            };
            // The surrounding keywords are themselves a member of the
            // intersection, so they are imported first and then intersected.
            let mut siblings = doc.clone();
            if let Some(object) = siblings.as_object_mut() {
                object.remove("allOf");
            }
            let mut base = Self::import(&siblings)?;
            for member in members {
                base = Self::intersect(base, Self::import(member)?)?;
            }
            return Ok(base);
        }
        // `not: {}` is "everything but anything", which is nothing. Any other
        // operand would need a complement the port cannot express, so it is
        // refused rather than approximated.
        if let Some(negated) = get("not") {
            if negated.as_object().is_some_and(|object| object.is_empty()) {
                return Ok(Schema::as_unknown(Schema::never()));
            }
            return Err(SchemaError::TypeMismatch {
                path: "$.not".to_string(),
                expected: "an empty schema, the only `not` this port can lower: Cannot import JSON Schema keyword \"not\": Effect has no equivalent constraint.",
                got: "a complement of a non-empty schema",
            });
        }
        // `enum` members are validated even beside a `const`, because the pin
        // collects both keywords' checks and reports the first bad one it finds;
        // the index in the path is the member's own.
        if let Some(serde_json::Value::Array(members)) = get("enum") {
            for (index, member) in members.iter().enumerate() {
                Self::require_primitive(format!("$.enum[{index}]"), member)?;
            }
        }
        if let Some(constants) = get("const") {
            Self::require_primitive("$.const".to_string(), constants)?;
            let values =
                Self::filter_exact_values(vec![Unknown::from_json(constants.clone())], doc);
            return Self::with_document_annotations(
                values
                    .into_iter()
                    .next()
                    .map(Schema::literal)
                    .ok_or_else(|| SchemaError::TypeMismatch {
                        path: "$.const".to_string(),
                        expected: "a value the document's own refinements admit",
                        got: "no value",
                    }),
                doc,
            );
        }
        if let Some(options) = get("enum") {
            let serde_json::Value::Array(members) = options else {
                return Err(SchemaError::TypeMismatch {
                    path: "$.enum".to_string(),
                    expected: "an array",
                    got: "a non-array",
                });
            };
            // Only primitives can be named: an object or array member has no
            // literal form, so the import refuses it with the member's index.
            let values: Vec<Unknown> = members.iter().cloned().map(Unknown::from_json).collect();
            // The document's own refinements constrain the values it names:
            // `{ enum: ["😀", "e\u{0301}"], minLength: 2 }` keeps the
            // two-code-point member and drops the one-code-point emoji, which
            // is the same filter an intersection applies to a literal union.
            let values = Self::filter_exact_values(values, doc);
            // Plain scalars collapse to one `Literals`; an enum containing null
            // keeps the `Union`, because `null` is a schema and not a literal.
            if values.iter().all(|value| value != &Unknown::Null) {
                let kind = SchemaKind::Literals(values.clone());
                return Self::with_document_annotations(
                    Ok(Schema::as_unknown(Schema::new(
                        move |unknown| {
                            if values.iter().any(|value| *value == unknown) {
                                Ok(unknown)
                            } else {
                                mismatch("$", "one of the literals", &unknown)
                            }
                        },
                        |value| Ok(value.clone()),
                        kind,
                    ))),
                    doc,
                );
            }
            let mut union: Option<Schema<Unknown>> = None;
            for value in values {
                let literal = Schema::literal(value);
                union = Some(match union {
                    None => literal,
                    Some(accumulated) => Schema::union(accumulated, literal),
                });
            }
            return Self::with_document_annotations(
                union.ok_or_else(|| SchemaError::TypeMismatch {
                    path: "$.enum".to_string(),
                    expected: "a non-empty array",
                    got: "an empty array",
                }),
                doc,
            );
        }
        // `anyOf` and `oneOf` build a union, and the document's other keywords
        // intersect it: a union meets a non-union by intersecting each member
        // on its own, which is how `{ properties: { id }, anyOf: [{ a }, { b }] }`
        // yields two structs that each carry `id`. `oneOf` is the same union
        // with a mode, and the repr carries it.
        for (keyword, mode) in [("anyOf", "anyOf"), ("oneOf", "oneOf")] {
            let Some(serde_json::Value::Array(options)) = get(keyword) else {
                continue;
            };
            let imported: Vec<Schema<Unknown>> =
                options.iter().map(Self::import).collect::<Result<_, _>>()?;
            if imported.is_empty() {
                return Err(SchemaError::TypeMismatch {
                    path: format!("$.{keyword}"),
                    expected: "a non-empty array",
                    got: "an empty array",
                });
            }
            let mut siblings = doc.clone();
            if let Some(object) = siblings.as_object_mut() {
                object.remove("anyOf");
                object.remove("oneOf");
            }
            // An annotation-only sibling constrains nothing, so the union
            // stands as the members imported it and the annotations ride the
            // union itself. A sibling that DOES constrain needs each union
            // lane pruned against it (`{ properties: { id }, anyOf: [{ a }, { b }] }`
            // yields two structs that each carry `id`), and the port carries no
            // lane pruning yet, so the import refuses rather than silently
            // dropping the member's own constraints.
            let annotated_only = siblings.as_object().is_some_and(|object| {
                object
                    .keys()
                    .all(|key| ["title", "description", "format"].contains(&key.as_str()))
            });
            if !annotated_only {
                return Err(SchemaError::TypeMismatch {
                    path: format!("$.{keyword}"),
                    expected: "a union with only annotations beside it, because intersecting a union with a sibling needs the lane pruning this port does not carry",
                    got: "a union beside constraining keywords",
                });
            }
            // `annotated_with` wraps rather than rebuilds, so a union's own
            // decode survives its annotation, which `decompose` would not keep.
            let pairs = Self::document_annotations(doc);
            return Ok(if pairs.is_empty() {
                Schema::union_with_mode(imported, mode)
            } else {
                Schema::annotated_with(Schema::union_with_mode(imported, mode), pairs)
            });
        }
        // A boolean schema is JSON Schema's shorthand: `false` admits nothing
        // and `true` admits anything.
        match doc {
            serde_json::Value::Bool(false) => return Ok(Schema::as_unknown(Schema::never())),
            serde_json::Value::Bool(true) => {
                return Ok(Self::annotated_json(&serde_json::json!({})));
            }
            _ => {}
        }
        match get("type") {
            // Nothing constrains the value, so it is any JSON value annotated as
            // the open catch-all. The pin's `annotate` pairs ride along.
            None => {
                // A document with no `type` still carries refinements, which is
                // how an `allOf` member like `{ minLength: 1 }` is written: it
                // constrains whatever it is intersected with rather than naming
                // a base of its own. The base stays the open JSON value.
                // An object keyword with no type still constrains an object, so
                // the object-level checks run here too: an `allOf` member
                // carrying only `propertyNames` is exactly that shape.
                let base = Self::refined_any(Self::annotated_json(doc), doc);
                if base_kind_is_open(&base) {
                    return Self::refined_object(base, doc);
                }
                Ok(base)
            }
            Some(serde_json::Value::String(type_name)) => {
                Self::with_document_annotations(
                    match type_name.as_str() {
                        "string" => Ok(Self::refined_text(doc)),
                        // The pin's JSON Schema import makes a `number` finite and an
                        // `integer` an int, so both carry a check where a bare string
                        // does not.
                        "number" => Ok(Self::refined_number(
                            Self::as_unknown_checked(
                                Schema::as_unknown(Schema::number()),
                                "isFinite",
                                "a finite number",
                            ),
                            doc,
                        )),
                        "integer" => Ok(Self::refined_number(
                            Self::as_unknown_checked(
                                Schema::as_unknown(Schema::number()),
                                "isInt",
                                "an integer",
                            ),
                            doc,
                        )),
                        "boolean" => Ok(Schema::as_unknown(Schema::boolean())),
                        "null" => Ok(Schema::as_unknown(Schema::null())),
                        "array" => match get("prefixItems") {
                            // `prefixItems` is a positional prefix, so it decodes to a
                            // tuple rather than an array. Everything else about the
                            // array comes from `minItems`, `maxItems` and `items`.
                            Some(serde_json::Value::Array(prefix)) => {
                                Self::tuple_from_prefix(prefix, doc)
                            }
                            _ => {
                                let items = match get("items") {
                                    Some(items) => Self::import(items)?,
                                    // An array with no `items` constrains each
                                    // element only to be a JSON value, which is
                                    // the annotated open catch-all rather than
                                    // the bare `Any`: the `expected` annotation
                                    // is what an open form carries everywhere
                                    // else in an import.
                                    None => Self::annotated_json(&serde_json::json!({})),
                                };
                                Ok(Self::refined_array(
                                    Schema::as_unknown(Schema::array(items)),
                                    doc,
                                ))
                            }
                        },
                        "object" => Self::import_object(doc),
                        _ => Err(SchemaError::TypeMismatch {
                            path: "$.type".to_string(),
                            expected: "a known type name",
                            got: "an unknown type name",
                        }),
                    },
                    doc,
                )
            }
            // An array of type names is a union of the types it names, each
            // carrying the document's own refinements, which is how
            // `{ type: ["string", "null"] }` becomes a two-member union.
            Some(serde_json::Value::Array(names)) => {
                let mut members: Vec<Schema<Unknown>> = Vec::new();
                for name in names {
                    let Some(name) = name.as_str() else {
                        return Err(SchemaError::TypeMismatch {
                            path: "$.type".to_string(),
                            expected: "a type name",
                            got: "a non-string member",
                        });
                    };
                    // Each member is the same document with its own type name,
                    // so a refinement the document carries applies to every
                    // member that can hold it. The document-level annotations
                    // are not one of those: they ride the union rather than
                    // each member, so they come out of the member's copy.
                    let mut single = doc.clone();
                    if let Some(object) = single.as_object_mut() {
                        object.insert(
                            "type".to_string(),
                            serde_json::Value::String(name.to_string()),
                        );
                        for key in ["title", "description", "format"] {
                            object.remove(key);
                        }
                    }
                    members.push(Self::import(&single)?);
                }
                if members.is_empty() {
                    return Err(SchemaError::TypeMismatch {
                        path: "$.type".to_string(),
                        expected: "at least one type name",
                        got: "an empty array",
                    });
                }
                // The annotation wraps rather than rebuilds, so the union's own
                // decode survives it.
                let pairs = Self::document_annotations(doc);
                let union = Schema::union_with_mode(members, "anyOf");
                Ok(if pairs.is_empty() {
                    union
                } else {
                    Schema::annotated_with(union, pairs)
                })
            }
            _ => Err(SchemaError::TypeMismatch {
                path: "$.type".to_string(),
                expected: "a type name",
                got: "a non-string",
            }),
        }
    }

    /// Lift a document's own schema-level annotations onto an imported schema.
    /// A typed base needs this as much as a bare one does, or a `description`
    /// beside `type: "number"` is silently dropped and the intersection row
    /// that expects it on the base cannot carry.
    fn with_document_annotations(
        result: Result<Self, SchemaError>,
        doc: &serde_json::Value,
    ) -> Result<Self, SchemaError> {
        let pairs = Self::document_annotations(doc);
        result.map(|schema| {
            if pairs.is_empty() {
                schema
            } else {
                // The annotation rides the BASE, not the whole chain, so a repr
                // prints `String.annotate(..).check(..)` rather than
                // `String.check(..).annotate(..)`. Decomposing and rebuilding is
                // how it gets there, which is also what an intersection does.
                let (kind, checks, mut annotation) = decompose(&schema.repr_kind());
                merge_annotation(&mut annotation, pairs);
                rebuild(kind, checks, annotation)
            }
        })
    }
}

impl Unknown {
    /// Build an `Unknown` from a JSON value.
    pub fn from_json(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Unknown::Null,
            serde_json::Value::Bool(boolean) => Unknown::Bool(boolean),
            serde_json::Value::Number(number) => Unknown::Number(number.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(text) => Unknown::Text(text),
            serde_json::Value::Array(items) => {
                Unknown::Array(items.into_iter().map(Unknown::from_json).collect())
            }
            serde_json::Value::Object(fields) => Unknown::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, Unknown::from_json(value)))
                    .collect(),
            ),
        }
    }

    /// This value back as a JSON value.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Unknown::Null => serde_json::Value::Null,
            Unknown::Bool(boolean) => serde_json::Value::Bool(*boolean),
            Unknown::Number(number) => serde_json::Number::from_f64(*number)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Unknown::Text(text) => serde_json::Value::String(text.clone()),
            Unknown::Array(items) => {
                serde_json::Value::Array(items.iter().map(Unknown::to_json).collect())
            }
            Unknown::Object(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_json()))
                    .collect(),
            ),
        }
    }
}

// Template literals

/// One part of a template-literal parser: a span (string-like capture) or a
/// literal anchor that must match verbatim. Not `Eq`: a literals part holds the
/// values it matches, and a number has no total equality.
#[derive(Clone, Debug, PartialEq)]
pub enum TemplatePart {
    /// Any string, possibly empty.
    AnyString,
    /// A non-empty string.
    NonEmptyString,
    /// A leading integer (`-`? digits).
    Int,
    /// A number as the pin's `Number` part reads one out of text: a sign,
    /// digits, an optional fraction and an optional exponent, so `a1`, `a+1`,
    /// `a1.2` and `a1.5e-45` are all this part and `a` is not.
    Number,
    /// A bigint: an optional `-` and digits. `a+1` and `a1.2` are not this
    /// part, which is what the pin's BigInt rows assert against them.
    Digits,
    /// A literal matched verbatim.
    Anchor(String),
    /// A set of literal values (`Literals([1, 2, 3])`): the part matches the
    /// text each value spells and captures the value itself, so the capture is
    /// the number `1` rather than the text `"1"`.
    Literals(Vec<Unknown>),
    /// A nested `TemplateLiteralParser` part: the whole head is one nested
    /// capture list, which is how the parser form nests.
    NestedParser(Box<TemplateParser>),
    /// Any one of these alternatives, tried in order: how a union in a part, and
    /// a nested `TemplateLiteral`, both arrive here. Each alternative is a whole
    /// sequence of parts. The capture follows the winner: a one-part
    /// alternative is that part and keeps its own capture, while a longer
    /// sequence is a nested template literal, whose value is the text it
    /// matched.
    OneOf(Vec<Vec<TemplatePart>>),
}

/// A template-literal parser: a sequence of spans and anchors matched
/// greedily with backtracking. Decoding yields one capture per part (anchors
/// echo themselves); encoding concatenates the captures back.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateParser {
    parts: Vec<TemplatePart>,
}

impl TemplateParser {
    /// Build from parts.
    pub fn new(parts: Vec<TemplatePart>) -> Self {
        Self { parts }
    }

    /// Parse the input, returning one capture per part, or `None`.
    pub fn parse(&self, input: &str) -> Option<Vec<Unknown>> {
        let mut captures = Vec::new();
        Self::match_parts(&self.parts, input, &mut captures).then_some(captures)
    }

    /// Whether the input matches the parts, keeping no captures. A
    /// `TemplateLiteral` validates a string and hands the same string back, so
    /// it needs this walk and none of its output.
    pub fn matches(&self, input: &str) -> bool {
        let mut captures = Vec::new();
        Self::match_parts(&self.parts, input, &mut captures)
    }

    /// The parts this parser was built from (`TemplateLiteral.parts`).
    pub fn parts(&self) -> &[TemplatePart] {
        &self.parts
    }

    /// Render captures back to a string (encoding axis).
    pub fn render(&self, captures: &[Unknown]) -> Option<String> {
        if captures.len() != self.parts.len() {
            return None;
        }
        let mut out = String::new();
        for capture in captures {
            render_capture(capture, &mut out)?;
        }
        Some(out)
    }

    /// Render captures back to a string, checking each one against the part it
    /// fills. The pin's parser encodes every capture through its own part
    /// schema, so a fraction in an integer part and an empty string in a
    /// non-empty part are refusals, and the path names the part's index.
    pub fn render_checked(&self, captures: &[Unknown]) -> Result<String, SchemaError> {
        let mut out = String::new();
        check_captures(&self.parts, captures, "", &mut out)?;
        Ok(out)
    }

    fn match_parts(parts: &[TemplatePart], input: &str, captures: &mut Vec<Unknown>) -> bool {
        let Some((first, rest)) = parts.split_first() else {
            return input.is_empty();
        };
        match first {
            TemplatePart::Anchor(anchor) => {
                let Some(tail) = input.strip_prefix(anchor.as_str()) else {
                    return false;
                };
                captures.push(Unknown::Text(anchor.clone()));
                if Self::match_parts(rest, tail, captures) {
                    return true;
                }
                captures.pop();
                false
            }
            span => {
                let min = match span {
                    &TemplatePart::NonEmptyString
                    | &TemplatePart::Int
                    | &TemplatePart::Number
                    | &TemplatePart::Digits => 1,
                    _ => 0,
                };
                let mut end = input.len();
                loop {
                    while end > 0 && !input.is_char_boundary(end) {
                        end -= 1;
                    }
                    if end < min {
                        return false;
                    }
                    let (head, tail) = input.split_at(end);
                    if let Some(capture) = check_span(span, head) {
                        captures.push(capture);
                        if Self::match_parts(rest, tail, captures) {
                            return true;
                        }
                        captures.pop();
                    }
                    if end == 0 {
                        return false;
                    }
                    end -= 1;
                }
            }
        }
    }
}

/// A number spelled the way JavaScript writes it, so a refusal names the value
/// the pin names: `Infinity`, `-Infinity` and `NaN` rather than Rust's `inf`.
fn js_number(number: f64) -> String {
    if number.is_nan() {
        "NaN".to_string()
    } else if number.is_infinite() {
        if number.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        }
    } else {
        number.to_string()
    }
}

/// Format an integer-valued float without a trailing `.0`.
fn trim_int(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

/// Write one capture as its text. A nested parser's capture is the list of its
/// own captures, so it renders as their concatenation.
fn render_capture(capture: &Unknown, out: &mut String) -> Option<()> {
    match capture {
        Unknown::Text(text) => out.push_str(text),
        Unknown::Number(number) => out.push_str(&trim_int(*number)),
        Unknown::Bool(boolean) => out.push_str(if *boolean { "true" } else { "false" }),
        Unknown::Array(items) => {
            for item in items {
                render_capture(item, out)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// The text one literal value spells, which is what a literals part matches on.
/// Distinct from `render_value`, which is the pin's repr printer.
fn literal_text(value: &Unknown) -> Option<String> {
    let mut out = String::new();
    render_capture(value, &mut out)?;
    Some(out)
}

/// Check one capture list against its parts, appending the text each capture
/// renders to. The pin's paths are part indexes: `[0]` at the top, and a nested
/// parser's own path hangs off its part's index.
fn check_captures(
    parts: &[TemplatePart],
    captures: &[Unknown],
    path: &str,
    out: &mut String,
) -> Result<(), SchemaError> {
    if parts.len() != captures.len() {
        return Err(SchemaError::EncodingFailed {
            path: path.to_string(),
            expected: "one capture per part".to_string(),
        });
    }
    for (index, (part, capture)) in parts.iter().zip(captures).enumerate() {
        let part_path = format!("{path}[{index}]");
        check_capture(part, capture, &part_path, out)?;
    }
    Ok(())
}

/// Check and render one capture against the part it fills.
fn check_capture(
    part: &TemplatePart,
    capture: &Unknown,
    path: &str,
    out: &mut String,
) -> Result<(), SchemaError> {
    let refuse = |expected: &str| SchemaError::EncodingFailed {
        path: path.to_string(),
        expected: expected.to_string(),
    };
    match *part {
        TemplatePart::Anchor(ref anchor) => match capture {
            Unknown::Text(text) if text == anchor => out.push_str(text),
            _ => return Err(refuse("the literal part itself")),
        },
        TemplatePart::AnyString => match capture {
            Unknown::Text(text) => out.push_str(text),
            _ => return Err(refuse("a string")),
        },
        TemplatePart::NonEmptyString => match capture {
            Unknown::Text(text) if !text.is_empty() => out.push_str(text),
            _ => return Err(refuse("a value with a length of at least 1")),
        },
        TemplatePart::Int => match capture {
            Unknown::Number(number) if number.fract() == 0.0 => out.push_str(&trim_int(*number)),
            _ => return Err(refuse("an integer")),
        },
        TemplatePart::Number => match capture {
            Unknown::Number(number) => out.push_str(&trim_int(*number)),
            _ => return Err(refuse("a number")),
        },
        TemplatePart::Digits => match capture {
            Unknown::Text(text) if parse_digits(text).is_some() => out.push_str(text),
            _ => return Err(refuse("a bigint")),
        },
        TemplatePart::Literals(ref values) => {
            let Some(value) = values.iter().find(|value| *value == capture) else {
                return Err(refuse("one of the literal values"));
            };
            out.push_str(&literal_text(value).ok_or_else(|| refuse("a literal value"))?);
        }
        TemplatePart::NestedParser(ref parser) => match capture {
            Unknown::Array(items) => check_captures(&parser.parts, items, path, out)?,
            _ => return Err(refuse("a list of captures")),
        },
        TemplatePart::OneOf(ref alternatives) => {
            let mut rendered = None;
            for alternative in alternatives {
                // One part is that part; a longer sequence is the pin's nested
                // `TemplateLiteral`, a string schema whose value is the text it
                // matched, so the text has to match the whole sequence.
                if alternative.len() == 1 {
                    let mut candidate = String::new();
                    if check_capture(&alternative[0], capture, path, &mut candidate).is_ok() {
                        rendered = Some(candidate);
                        break;
                    }
                } else if let Unknown::Text(text) = capture {
                    if TemplateParser::match_parts(alternative, text, &mut Vec::new()) {
                        rendered = Some(text.clone());
                        break;
                    }
                }
            }
            match rendered {
                Some(text) => out.push_str(&text),
                None => return Err(refuse("one of the part's alternatives")),
            }
        }
    }
    Ok(())
}

/// Check one span candidate, returning its capture. The capture follows the
/// part's own kind rather than the shape of the text: a literals part captures
/// the value it matched, and a nested parser captures its own list.
fn check_span(span: &TemplatePart, head: &str) -> Option<Unknown> {
    match *span {
        TemplatePart::AnyString => Some(Unknown::Text(head.to_string())),
        TemplatePart::NonEmptyString => (!head.is_empty()).then(|| Unknown::Text(head.to_string())),
        TemplatePart::Int => parse_int(head).map(Unknown::Number),
        TemplatePart::Number => parse_number(head).map(Unknown::Number),
        TemplatePart::Digits => parse_digits(head).map(Unknown::Text),
        TemplatePart::Literals(ref values) => values
            .iter()
            .find(|value| literal_text(value).as_deref() == Some(head))
            .cloned(),
        // The span is already the whole candidate, so the nested parser either
        // matches it entirely or does not.
        TemplatePart::NestedParser(ref parser) => parser.parse(head).map(Unknown::Array),
        TemplatePart::OneOf(ref alternatives) => alternatives.iter().find_map(|alternative| {
            let mut captures = Vec::new();
            if !TemplateParser::match_parts(alternative, head, &mut captures) {
                return None;
            }
            match captures.len() {
                1 => captures.pop(),
                _ => Some(Unknown::Text(head.to_string())),
            }
        }),
        TemplatePart::Anchor(_) => None,
    }
}

/// A number in the grammar the pin's `Number` part accepts: a sign, at least one
/// digit, an optional fraction with at least one digit, and an optional
/// exponent with at least one digit. Both cases of `e` count.
fn parse_number(head: &str) -> Option<f64> {
    let bytes = head.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        index += 1;
    }
    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == integer_start {
        return None;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == fraction_start {
            return None;
        }
    }
    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == exponent_start {
            return None;
        }
    }
    if index != bytes.len() {
        return None;
    }
    head.parse::<f64>().ok()
}

/// A bigint as text: an optional `-` and digits, and nothing else.
fn parse_digits(head: &str) -> Option<String> {
    let digits = head.strip_prefix('-').unwrap_or(head);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| head.to_string())
}

/// A leading integer: optional `-` plus digits, the longest valid prefix.
/// The caller enumerates lengths, so this only validates one candidate.
fn parse_int(head: &str) -> Option<f64> {
    let digits = head.strip_prefix('-').unwrap_or(head);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    head.parse::<f64>().ok()
}

impl Schema<String> {
    /// A template-literal part as the schema kind that represents it.
    /// `NonEmptyString`, `Int` and `Digits` ride as plain `String`: the
    /// length/numeric floor lives in the parser (which keeps the original
    /// parts), so decoding stays exact while the representation shows the
    /// base shape.
    pub fn template_kind_of_part(part: &TemplatePart) -> SchemaKind {
        match part {
            TemplatePart::Anchor(text) => SchemaKind::Literal(Unknown::Text(text.clone())),
            TemplatePart::Number => SchemaKind::Number,
            TemplatePart::AnyString
            | TemplatePart::NonEmptyString
            | TemplatePart::Int
            | TemplatePart::Digits => SchemaKind::String,
            TemplatePart::Literals(values) => match values.as_slice() {
                [single] => SchemaKind::Literal(single.clone()),
                _ => SchemaKind::Union {
                    members: values
                        .iter()
                        .map(|value| SchemaKind::Literal(value.clone()))
                        .collect(),
                    mode: "anyOf",
                },
            },
            TemplatePart::NestedParser(parser) => SchemaKind::TemplateLiteral(
                parser
                    .parts()
                    .iter()
                    .map(Self::template_kind_of_part)
                    .collect(),
            ),
            TemplatePart::OneOf(alternatives) => SchemaKind::Union {
                members: alternatives
                    .iter()
                    .map(|sequence| match sequence.as_slice() {
                        [single] => Self::template_kind_of_part(single),
                        _ => SchemaKind::TemplateLiteral(
                            sequence.iter().map(Self::template_kind_of_part).collect(),
                        ),
                    })
                    .collect(),
                mode: "anyOf",
            },
        }
    }

    /// A schema kind back to the template part it spells, stripping
    /// annotations (identifiers ride the kind for the round trip and stay
    /// out of the parser). `None` means the kind has no template spelling.
    pub fn template_part_of_kind(kind: &SchemaKind) -> Option<TemplatePart> {
        match kind {
            SchemaKind::Literal(Unknown::Text(text)) => Some(TemplatePart::Anchor(text.clone())),
            SchemaKind::Literal(Unknown::Number(number)) => {
                Some(TemplatePart::Anchor(number.to_string()))
            }
            SchemaKind::Literal(Unknown::Bool(flag)) => {
                Some(TemplatePart::Anchor(flag.to_string()))
            }
            SchemaKind::String => Some(TemplatePart::AnyString),
            SchemaKind::Number => Some(TemplatePart::Number),
            SchemaKind::Union { members, .. } => {
                let mut alternatives = Vec::with_capacity(members.len());
                for member in members {
                    alternatives.push(vec![Self::template_part_of_kind(member)?]);
                }
                Some(TemplatePart::OneOf(alternatives))
            }
            SchemaKind::TemplateLiteral(parts) => {
                let mut nested = Vec::with_capacity(parts.len());
                for part in parts {
                    nested.push(Self::template_part_of_kind(part)?);
                }
                Some(TemplatePart::OneOf(vec![nested]))
            }
            SchemaKind::Annotated { inner, .. } => Self::template_part_of_kind(inner),
            _ => None,
        }
    }

    /// A template literal from already-built part kinds (the revive path
    /// and explicitly annotated parts): identifiers and other annotations
    /// ride the kinds into the round trip while the parser (built from the
    /// unannotated spellings) drives decoding. `None` when a part has no
    /// template spelling.
    pub fn template_literal_from_kinds(parts: Vec<SchemaKind>) -> Option<Self> {
        let mut parser_parts = Vec::with_capacity(parts.len());
        for part in &parts {
            parser_parts.push(Self::template_part_of_kind(part)?);
        }
        let parser = TemplateParser::new(parser_parts);
        Some(Self::new(
            move |unknown| match unknown {
                Unknown::Text(text) => {
                    parser
                        .matches(&text)
                        .then_some(text)
                        .ok_or_else(|| SchemaError::TypeMismatch {
                            path: "$".to_string(),
                            expected: "a string matching template literal parts",
                            got: "no match",
                        })
                }
                other => mismatch("$", "string", &other),
            },
            move |value| Ok(Unknown::Text(value.clone())),
            SchemaKind::TemplateLiteral(parts),
        ))
    }

    /// A template literal (`Schema.TemplateLiteral`): a string schema that only
    /// admits text matching every part, and hands the same text back. Distinct
    /// from `template`, which is the pin's `TemplateLiteralParser` and yields
    /// one capture per part.
    pub fn template_literal(parts: Vec<TemplatePart>) -> Self {
        let kinds = parts.iter().map(Self::template_kind_of_part).collect();
        let parser = TemplateParser::new(parts);
        Self::new(
            move |unknown| match unknown {
                Unknown::Text(text) => {
                    parser
                        .matches(&text)
                        .then_some(text)
                        .ok_or_else(|| SchemaError::TypeMismatch {
                            path: "$".to_string(),
                            expected: "a string matching template literal parts",
                            got: "no match",
                        })
                }
                other => mismatch("$", "string", &other),
            },
            // Encoding is the identity on the text: the pin's TemplateLiteral
            // carries no transformation, so there is nothing to render back.
            // A text that would not decode cannot be built from a decode, and
            // `encode` has no failure channel to report one.
            move |value| Ok(Unknown::Text(value.clone())),
            SchemaKind::TemplateLiteral(kinds),
        )
    }
}

impl Schema<Vec<Unknown>> {
    /// A template-literal parser schema: decodes text into per-part
    /// captures, encodes captures back into text.
    pub fn template(parts: Vec<TemplatePart>) -> Self {
        let parser = TemplateParser::new(parts);
        let encode_parser = parser.clone();
        Self::new(
            move |unknown| match unknown {
                Unknown::Text(input) => {
                    parser
                        .parse(&input)
                        .ok_or_else(|| SchemaError::TypeMismatch {
                            path: "$".to_string(),
                            expected: "a string matching template literal parts",
                            got: "no match",
                        })
                }
                other => mismatch("$", "string", &other),
            },
            move |captures| Ok(Unknown::Text(encode_parser.render_checked(captures)?)),
            SchemaKind::Opaque("template"),
        )
    }
}

// Construction

impl Schema<std::collections::BTreeMap<String, Unknown>> {
    /// Build a value (`make`): construction, not decode. An absent field whose
    /// schema carries a constructor default fills in, and a present nested
    /// struct is constructed too, so nested defaults fill recursively.
    pub fn make(
        &self,
        input: std::collections::BTreeMap<String, Unknown>,
    ) -> Result<std::collections::BTreeMap<String, Unknown>, SchemaError> {
        self.construct(Unknown::Object(input))
    }
}

impl<T> Schema<T> {
    /// The value formatter for this schema (`Schema.toFormatter`).
    ///
    /// A Struct with declared fields renders each field as `"name": value`
    /// joined by ", ", and the same rule applies to every NESTED Struct, which
    /// is why this walks the schema tree rather than the value tree: the pin's
    /// nested output is `{ "a": "a", "as": [{ "a": "b" }] }`, with the spacing
    /// on the inner objects too. An EMPTY Struct has no declared fields, so it
    /// falls through to the plain JSON-ish rendering, which is what the pin
    /// does too (its "empty" row asserts `{"a":1}` unspaced).
    pub fn formatter(&self) -> Box<dyn Fn(&Unknown) -> String + Send + Sync + 'static> {
        match &self.formatter {
            // A composite constructor supplied its own renderer, because it has
            // child schemas the kind alone cannot reach.
            Some(render) => {
                let render = render.clone();
                Box::new(move |value| render(value))
            }
            // Everything else is described by its kind, so walk it.
            None => {
                let kind = self.repr_kind();
                let children = self.children.clone();
                Box::new(move |value| format_root(&kind, &children, value))
            }
        }
    }
}

// SchemaRepresentation: the code / type repr

impl Schema<Unknown> {
    /// A named reference to a root definition. The decode is the referenced
    /// definition's decode, resolved once at construction: a reference is a
    /// name, but it still has to decode what it names.
    pub fn reference(name: &str, definition: serde_json::Value) -> Self {
        let n = name.to_string();
        let definition = definition.clone();
        let decode = move |unknown: Unknown| match Self::import(&definition) {
            Ok(inner) => inner.decode(unknown),
            Err(_) => Ok(unknown),
        };
        Self::new(
            decode,
            |value: &Unknown| Ok(value.clone()),
            SchemaKind::Reference(name.to_string()),
        )
    }

    /// Any JSON value (`Json`). The wire shape is unconstrained, which is what
    /// makes it the catch-all for an open object's additional properties.
    pub fn json() -> Self {
        Self::new(Ok, |value| Ok(value.clone()), SchemaKind::Json)
    }

    /// A struct field whose key may be absent (`optionalKey`). Decode delegates
    /// to the inner schema: whether the key may be missing is a property of the
    /// enclosing struct, so it is enforced there and not here.
    pub fn optional_key_shape(inner: Schema<Unknown>) -> Self {
        let kind = SchemaKind::OptionalKey(Box::new(inner.repr_kind()));
        let decode = inner.decode.clone();
        let encode = inner.encode.clone();
        Self::new(
            move |unknown| decode(unknown),
            move |value| encode(value),
            kind,
        )
    }

    /// A struct plus a catch-all shape (`StructWithRest`): the named fields and
    /// the shape absorbing every key they do not claim.
    pub fn struct_with_rest(fields: Vec<(String, Schema<Unknown>)>, rest: Schema<Unknown>) -> Self {
        // A key the rest's key schema names is an identifier the rest owns, so
        // a fixed property of the same name would claim it twice. The pin
        // refuses that at construction; here the same guard stops the two
        // lookups from silently deciding between themselves. Only an
        // exact-value key schema names any key, which is what `exact_values`
        // reports.
        if let SchemaKind::Record(key, _) = rest.repr_kind() {
            for name in exact_values(&key)
                .into_iter()
                .flatten()
                .filter_map(|value| match value {
                    Unknown::Text(name) => Some(name),
                    _ => None,
                })
            {
                if fields.iter().any(|(field_name, _)| *field_name == name) {
                    panic!("Duplicate identifier: \"{name}\". ts(2300)");
                }
            }
        }
        let kind = SchemaKind::StructWithRest(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), field.repr_kind()))
                .collect(),
            Box::new(rest.repr_kind()),
        );
        let names: Vec<String> = fields.iter().map(|(name, _)| name.clone()).collect();
        // A field wrapped in `optionalKey` may be absent; a bare one may not.
        // The kind already carries the difference, so this does not need a
        // second list of required names.
        let required: Vec<String> = fields
            .iter()
            .filter(|(_, field)| !matches!(field.repr_kind(), SchemaKind::OptionalKey(_)))
            .map(|(name, _)| name.clone())
            .collect();
        let decoders: Vec<(String, Arc<Schema<Unknown>>)> = fields
            .into_iter()
            .map(|(name, field)| (name, Arc::new(field)))
            .collect();
        let rest_decode = Arc::new(rest);
        Self::new(
            move |unknown| {
                let object = match unknown {
                    Unknown::Object(map) => map,
                    other => return mismatch("$", "object", &other),
                };
                for name in &required {
                    if !object.contains_key(name) {
                        return Err(SchemaError::TypeMismatch {
                            path: format!("$.{name}"),
                            expected: "a declared field",
                            got: "absent",
                        });
                    }
                }
                let mut out = Vec::with_capacity(decoders.len());
                for (name, field) in &decoders {
                    match object.get(name) {
                        None => {}
                        Some(value) => {
                            let decoded = field
                                .decode(value.clone())
                                .map_err(|error| error.at(&format!("$.{name}")))?;
                            out.push((name.clone(), decoded));
                        }
                    }
                }
                // The keys the named fields do not claim are the catch-all's
                // to describe. Two rest shapes arrive here, and both are the
                // pin's: a record-shaped one (what the pin's rest list holds)
                // describes the sub-object those keys form, so it decodes that
                // object as a whole rather than one value at a time, which is
                // what makes an object-level check on the rest decide. A
                // value-shaped one describes each value on its own.
                let leftovers: std::collections::BTreeMap<String, Unknown> = object
                    .into_iter()
                    .filter(|(key, _)| !names.iter().any(|name| name == key))
                    .collect();
                if matches!(rest_decode.repr_kind(), SchemaKind::Record(..)) {
                    match rest_decode.decode(Unknown::Object(leftovers))? {
                        Unknown::Object(entries) => out.extend(entries),
                        other => {
                            return Err(SchemaError::TypeMismatch {
                                path: "$".to_string(),
                                expected: "an object-shaped catch-all",
                                got: other.kind(),
                            });
                        }
                    }
                } else {
                    for (key, value) in leftovers {
                        out.push((key.clone(), rest_decode.decode(value)?));
                    }
                }
                Ok(Unknown::Object(out.into_iter().collect()))
            },
            |value| Ok(value.clone()),
            kind,
        )
    }

    /// Derive a tuple schema from this one's elements (`mapElements`), the
    /// positional twin of `mapFields`.
    pub fn map_elements(
        self,
        transform: impl Fn(Vec<Schema<Unknown>>) -> Vec<Schema<Unknown>>,
    ) -> Self {
        let elements = self
            .tuple_elements
            .clone()
            .expect("mapElements needs a tuple shape with declared elements");
        Self::tuple_of(transform(elements))
    }

    /// Fixed-length positional elements (`Tuple`), built from a list whose
    /// length is only known once the document has been read. An optional
    /// element cannot be followed by a required one: the values after it have no
    /// position to occupy when it is absent, so the pin refuses such a tuple
    /// while it is being built and so does this.
    pub fn tuple_of(elements: Vec<Schema<Unknown>>) -> Self {
        let kinds: Vec<SchemaKind> = elements.iter().map(|e| e.repr_kind()).collect();
        // "Optional" has two spellings in the pin: an `optionalKey` position
        // may be absent, and an `optional` position may be absent or null.
        // Either one makes every position after it a required-after-optional,
        // so the rule counts both.
        let required = kinds
            .iter()
            .take_while(|kind| !is_optional_element(kind))
            .count();
        assert!(
            kinds[required..].iter().all(is_optional_element),
            "A required element cannot follow an optional element. ts(1257)"
        );
        let kind = SchemaKind::Tuple(kinds);
        let declared_elements = Some(elements.clone());
        let decoders: Vec<Arc<Schema<Unknown>>> = elements.into_iter().map(Arc::new).collect();
        let total = decoders.len();
        let construct_decoders = decoders.clone();
        let mut schema = Self::new(
            move |unknown| {
                let items = match unknown {
                    Unknown::Array(items) => items,
                    other => return mismatch("$", "array", &other),
                };
                // A position below the first optional one must be present, and
                // none may appear past the last declared position.
                if items.len() > total {
                    let excess =
                        excess_issues((total..items.len()).map(|index| format!("$[{index}]")))
                            .expect("the array is longer than the declared positions");
                    return Err(excess);
                }
                // A short tuple is a missing key at the first absent position,
                // the shape the pin reports (`Missing key at [1]`); only this
                // port's spelling of a missing key differs.
                if items.len() < required {
                    return Err(SchemaError::TypeMismatch {
                        path: format!("$[{}]", items.len()),
                        expected: "required field",
                        got: "missing",
                    });
                }
                let mut out = Vec::with_capacity(items.len());
                for (index, element) in decoders.iter().enumerate() {
                    // An absent position is absent: the decoded tuple is
                    // shorter, which is what lets an optional element's absence
                    // survive the round trip instead of hardening into a present
                    // null the element schema itself would have refused. The
                    // ordering rule guarantees the absent ones are all trailing.
                    let Some(value) = items.get(index) else {
                        break;
                    };
                    let decoded = element
                        .decode(value.clone())
                        .map_err(|error| error.at(&format!("$[{index}]")))?;
                    out.push(decoded);
                }
                Ok(Unknown::Array(out))
            },
            |value| Ok(value.clone()),
            kind,
        );
        schema.tuple_elements = declared_elements;
        // Construction fills an absent position whose element carries a
        // constructor default; decoding still answers null for an absent
        // optional position and refuses a missing required one.
        schema.construct = Some(Arc::new(move |unknown| {
            let Unknown::Array(items) = unknown else {
                return mismatch("$", "array", &unknown);
            };
            if items.len() > total {
                let excess = excess_issues((total..items.len()).map(|index| format!("$[{index}]")))
                    .expect("the array is longer than the declared positions");
                return Err(excess);
            }
            let mut out = Vec::with_capacity(total);
            for (index, element) in construct_decoders.iter().enumerate() {
                let path = format!("$[{index}]");
                match items.get(index) {
                    Some(value) => {
                        out.push(
                            element
                                .construct(value.clone())
                                .map_err(|error| error.at(&path))?,
                        );
                    }
                    None if element.constructor_default.is_some() => {
                        let default = element
                            .constructor_default
                            .as_ref()
                            .expect("checked just above");
                        let produced = default().map_err(|error| error.at(&path))?;
                        out.push(
                            element
                                .construct(produced)
                                .map_err(|error| error.at(&path))?,
                        );
                    }
                    None if index >= required => out.push(Unknown::Null),
                    None => {
                        return Err(SchemaError::TypeMismatch {
                            path,
                            expected: "a required element",
                            got: "missing",
                        });
                    }
                }
            }
            Ok(Unknown::Array(out))
        }));
        schema
    }

    /// Build a tuple (`make`): an absent position whose element carries a
    /// constructor default fills in; a missing required position still fails.
    /// The decoded form of a tuple here is `Unknown`, so this is the value it
    /// builds, exactly as `decode` returns it.
    pub fn make(&self, input: Vec<Unknown>) -> Result<Unknown, SchemaError> {
        self.construct(Unknown::Array(input))
    }

    /// Positional elements with a rest list (`TupleWithRest`): the leading
    /// positions, a repeatable schema, then the fixed trailing positions the
    /// pin's rest list carries after its first entry.
    ///
    /// `[Boolean, String]` reads "zero or more booleans, then a final string",
    /// so a short array is caught at the *trailing* index and an element that
    /// lands in the repeatable window has to be a boolean. Encoding walks the
    /// same split, which is what makes the round trip exact.
    pub fn tuple_with_rest(elements: Vec<Schema<Unknown>>, rest: Vec<Schema<Unknown>>) -> Self {
        assert!(
            !rest.is_empty(),
            "TupleWithRest needs at least one rest element"
        );
        // The leading positions are still positional, so the same rule applies
        // to them: no required element may follow an optional one, in either
        // spelling of optional.
        let kinds: Vec<SchemaKind> = elements.iter().map(|e| e.repr_kind()).collect();
        let required = kinds
            .iter()
            .take_while(|kind| !is_optional_element(kind))
            .count();
        assert!(
            kinds[required..].iter().all(is_optional_element),
            "A required element cannot follow an optional element. ts(1257)"
        );
        let rest_kinds: Vec<SchemaKind> = rest.iter().map(|e| e.repr_kind()).collect();
        // Only the first rest schema repeats; every later one is a fixed
        // trailing position, so an optional one there has no place to be.
        assert!(
            rest_kinds[1..]
                .iter()
                .all(|kind| !is_optional_element(kind)),
            "An optional element cannot follow a rest element. ts(1266)"
        );
        let kind = SchemaKind::TupleWithRest(kinds, rest_kinds);
        let decoders: Vec<Arc<Schema<Unknown>>> = elements.into_iter().map(Arc::new).collect();
        let rest_decoders: Vec<Arc<Schema<Unknown>>> = rest.into_iter().map(Arc::new).collect();
        let element_len = decoders.len();
        let tail_len = rest_decoders.len() - 1;
        let decode_elements = decoders.clone();
        let decode_rest = rest_decoders.clone();
        Self::new(
            move |unknown| {
                let items = match unknown {
                    Unknown::Array(items) => items,
                    other => return mismatch("$", "array", &other),
                };
                let threshold = element_len.max(items.len().saturating_sub(tail_len));
                let end = items.len().max(element_len + tail_len);
                let mut out = Vec::with_capacity(items.len());
                for index in 0..end {
                    let decoder = rest_decoder_at(&decode_elements, &decode_rest, threshold, index);
                    match items.get(index) {
                        Some(value) => {
                            let decoded = decoder
                                .decode(value.clone())
                                .map_err(|error| error.at(&format!("$[{index}]")))?;
                            out.push(decoded);
                        }
                        // An absent position is a missing key unless the schema
                        // calls it optional. A leading optional can be absent
                        // while a trailing position is still required, so the
                        // whole remainder is scanned before giving up on it.
                        None => {
                            let missing = (index..end).find(|other| {
                                !is_optional_element(
                                    &rest_decoder_at(
                                        &decode_elements,
                                        &decode_rest,
                                        threshold,
                                        *other,
                                    )
                                    .repr_kind(),
                                )
                            });
                            match missing {
                                Some(index) => {
                                    return Err(SchemaError::TypeMismatch {
                                        path: format!("$[{index}]"),
                                        expected: "required field",
                                        got: "missing",
                                    });
                                }
                                None => break,
                            }
                        }
                    }
                }
                Ok(Unknown::Array(out))
            },
            move |value: &Unknown| {
                let Unknown::Array(items) = value else {
                    return mismatch("$", "array", value);
                };
                // Encoding is the same split over the decoded length, so the
                // repeatable window is whatever the value leaves between the
                // leading positions and the trailing ones.
                let threshold = element_len.max(items.len().saturating_sub(tail_len));
                if items.len() < element_len + tail_len {
                    return Err(SchemaError::TypeMismatch {
                        path: format!("$[{}]", items.len()),
                        expected: "required field",
                        got: "missing",
                    });
                }
                let mut out = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let decoder = rest_decoder_at(&decoders, &rest_decoders, threshold, index);
                    let encoded = decoder
                        .try_encode(item)
                        .map_err(|error| error.at(&format!("$[{index}]")))?;
                    out.push(encoded);
                }
                Ok(Unknown::Array(out))
            },
            kind,
        )
    }

    /// `Schema.NullOr(inner)`: the inner schema or null, which the pin spells
    /// as a two-member union. [`optional`] is its struct-field twin and answers
    /// `None` for a null; this one keeps the null as a value of its own.
    ///
    /// [`optional`]: Schema::optional
    pub fn null_or(inner: Schema<Unknown>) -> Self {
        let null = Schema::as_unknown(Schema::<()>::null());
        Self::union_with_mode(vec![inner, null], "anyOf")
    }

    /// `Schema.ArrayEnsure(inner)`: one value or an array of them, decoded to
    /// an array (`ArrayEnsure(FiniteFromString)` reads `"1"` and `["1", "2"]`
    /// alike).
    ///
    /// The inner schema is tried first, so a value that is already an array is
    /// kept whole when the inner schema accepts it: `ArrayEnsure(Tuple([..]))`
    /// decodes `[1, 2]` as one tuple, not as two elements. An array the inner
    /// schema refuses is decoded element-wise. Encoding is the inverse, which
    /// is what makes the round trip exact: a one-element array unwraps to the
    /// single value, and every other array encodes element-wise.
    ///
    /// The pin builds this as `Union([one, many])` lowered through a
    /// `decodeTo`; the port needs only the order the union tries, so the two
    /// attempts live here and a value neither accepts reports the inner
    /// schema's refusal rather than the union's joined type text.
    pub fn array_ensure(inner: Schema<Unknown>) -> Self {
        let single = Arc::new(inner.clone());
        let single_decode = single.clone();
        let inner_encode = Arc::new(inner.clone());
        let many = Arc::new(Schema::as_unknown(Schema::array(inner.clone())));
        let many_decode = many.clone();
        let formatter = many.formatter.clone();
        let mut out = Self::new(
            move |unknown| match single_decode.decode(unknown.clone()) {
                Ok(value) => Ok(Unknown::Array(vec![value])),
                Err(refusal) => match &unknown {
                    Unknown::Array(_) => many_decode.decode(unknown),
                    _ => Err(refusal),
                },
            },
            move |value: &Unknown| {
                let Unknown::Array(items) = value else {
                    return mismatch("$", "array", value);
                };
                if let [only] = items.as_slice() {
                    return inner_encode.try_encode(only);
                }
                let mut out = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    out.push(
                        inner_encode
                            .try_encode(item)
                            .map_err(|error| error.at(&format!("$[{index}]")))?,
                    );
                }
                Ok(Unknown::Array(out))
            },
            SchemaKind::Array(Box::new(inner.repr_kind())),
        );
        out.formatter = formatter;
        out.array_element = Some(Arc::new(inner));
        out
    }

    /// Derive a union from this one's members (`mapMembers`), the member-list
    /// twin of `mapFields` and `mapElements`. A union keeps its mode, which is
    /// the only thing distinguishing `anyOf` from `oneOf`; an exact-value shape
    /// becomes a plain union of the transformed members, which is what the
    /// pin's `Literals.mapMembers` does.
    pub fn map_members(
        self,
        transform: impl Fn(Vec<Schema<Unknown>>) -> Vec<Schema<Unknown>>,
    ) -> Self {
        let mode = match &self.kind {
            SchemaKind::Union { mode, .. } => *mode,
            _ => "anyOf",
        };
        let members = self
            .members()
            .expect("mapMembers needs a union or an exact-value shape");
        Self::union_with_mode(transform(members), mode)
    }

    /// `Schema.flip`: the same codec with its two directions swapped, so what
    /// the schema accepted on the way in it now produces, and what it produced
    /// it now accepts.
    ///
    /// A flip does not change the shape, so the flipped schema answers for the
    /// shape underneath it: a flipped Union is still a Union whose options a
    /// derivation reads, and a check applied on top of a flipped schema lands
    /// on the side the flip made "in", which is the pin's
    /// "flip & check & flip should apply the check to the encoded side". The
    /// schema it flipped stays reachable through [`flip_source`].
    pub fn flip(schema: Schema<Unknown>) -> Self {
        let decode_schema = schema.clone();
        let encode_schema = schema.clone();
        let mut out = Self::new(
            move |unknown| decode_schema.try_encode(&unknown),
            move |value| encode_schema.decode(value.clone()),
            SchemaKind::Opaque("flip"),
        );
        // The wrapped reprs describe the schema underneath, exactly as a
        // re-typing does, so `repr_kind`/`to_code`/`to_type` keep answering for
        // the shape rather than for the flip itself.
        out.wrapped = Some(Box::new(Wrapped {
            kind: schema.repr_kind(),
            children: schema.children.clone(),
            code: schema.to_code(),
            ty: schema.to_type(),
        }));
        out.children = vec![("flipped".to_string(), Arc::new(schema))];
        out
    }

    /// `Schema.Enum(values)`: a named set of literal values, plus the union
    /// those literals decode through.
    ///
    /// The pin takes a TypeScript enum or a `const` object and keeps the names
    /// reachable as `schema.enums`; here the names arrive as pairs, so
    /// [`enums`](Enumeration::enums) hands them back in declaration order. Two
    /// names may hold the same value (the pin's own String-enum row has both
    /// `Banana` and `Cantaloupe` spelling `0`), so nothing is deduplicated.
    pub fn enumeration(pairs: Vec<(&str, Unknown)>) -> Enumeration {
        let pairs: Vec<(String, Unknown)> = pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        // The pin refuses a non-finite numeric member when the enum is built
        // (`A numeric enum value must be finite, got NaN`), because such a
        // member is unreachable: no wire value equals it.
        for (_, value) in &pairs {
            if let Unknown::Number(number) = value {
                if !number.is_finite() {
                    panic!(
                        "A numeric enum value must be finite, got {}",
                        js_number_text(*number)
                    );
                }
            }
        }
        // One `Literal` member per name, in declaration order. The union is a
        // real one (`union_with_mode`), not the two-member `Schema::union`
        // chain: only the former refuses with the members' own union text
        // (`Expected 0 | 1`), and that text is the pin's. An empty set names no
        // member at all and admits nothing, which is `never`.
        let members: Vec<Schema<Unknown>> = pairs
            .iter()
            .map(|(_, value)| Schema::literal(value.clone()))
            .collect();
        Enumeration {
            pairs,
            schema: Schema::union_with_mode(members, "anyOf"),
        }
    }

    /// `Schema.declare(predicate, expected)`: a shape with no wire form of its
    /// own, admitted only when the predicate says so, refused with the caller's
    /// own `expected` text.
    ///
    /// The pin's row declares a host class (`(u) => u instanceof File`); a port
    /// whose values are all wire values proves the same shape with a predicate
    /// it can state. The kind is the bare `declare` rather than the expected
    /// text, which is only known at run time and a `SchemaKind` holds a
    /// `&'static str`.
    pub fn declare(
        predicate: impl Fn(&Unknown) -> bool + Send + Sync + 'static,
        expected: &str,
    ) -> Self {
        let expected = expected.to_string();
        let predicate = Arc::new(predicate);
        Self::new(
            move |unknown| {
                if predicate(&unknown) {
                    Ok(unknown)
                } else {
                    Err(SchemaError::TypeMismatchDynamic {
                        path: "$".to_string(),
                        expected: expected.clone(),
                        got: unknown.kind(),
                    })
                }
            },
            |value| Ok(value.clone()),
            SchemaKind::Opaque("declare"),
        )
    }

    /// `Schema.Redacted(inner[, { label }])`: a box that hides its contents.
    ///
    /// The pin boxes the value in a `Redacted` instance; with no class identity
    /// here, the box is its wire shape, an object carrying `value` and, when
    /// the schema was built with one, a `label` that has to match. Two of the
    /// pin's messages are deliberate: a value that is not boxed at all is
    /// refused as `Redacted`, and a boxed value the inner schema refuses is
    /// reported at `$.value` as `a valid value` rather than with the inner
    /// schema's own text, because the pin hides which of its checks failed.
    pub fn redacted(inner: Schema<Unknown>, label: Option<&str>) -> Self {
        let label = label.map(|label| label.to_string());
        let inner_for_children = inner.clone();
        let decode_inner = Arc::new(inner.clone());
        let encode_inner = Arc::new(inner);
        let decode_label = label.clone();
        let mut out = Self::new(
            move |unknown| redacted_decode(&decode_inner, decode_label.as_deref(), &unknown),
            move |value| redacted_encode(&encode_inner, label.as_deref(), value),
            SchemaKind::Opaque("redacted"),
        );
        // The wrapped schema rides `children` under the name of the key it
        // occupies, so the accessor below reads it back: that is what the pin's
        // `schema.value` exposes.
        out.children = vec![("value".to_string(), Arc::new(inner_for_children))];
        out
    }

    /// `Schema.RedactedFromValue(inner)`: the box on the way out, the wrapped
    /// value on the way in. Decoding wraps whatever the inner schema accepts;
    /// encoding refuses anything that is not already a box and hands back the
    /// value inside it.
    pub fn redacted_from_value(inner: Schema<Unknown>) -> Self {
        let inner_for_children = inner.clone();
        let decode_inner = Arc::new(inner.clone());
        let encode_inner = Arc::new(inner);
        let mut out = Self::new(
            move |unknown| {
                let value = decode_inner.decode(unknown)?;
                let mut boxed = std::collections::BTreeMap::new();
                boxed.insert("value".to_string(), value);
                Ok(Unknown::Object(boxed))
            },
            move |value| unboxed_value(&encode_inner, None, value),
            SchemaKind::Opaque("redactedFromValue"),
        );
        out.children = vec![("value".to_string(), Arc::new(inner_for_children))];
        out
    }

    /// The exact-value shape (`Schema.Literals`): one `Literal` member per
    /// value. Kept as a shape of its own rather than folded to a single
    /// `Literal` the way the JSON Schema importer folds a one-value `enum`,
    /// because the member list is what a derivation reads.
    pub fn literals(values: Vec<Unknown>) -> Self {
        assert!(
            !values.is_empty(),
            "a literals shape holds at least one value"
        );
        let kind = SchemaKind::Literals(values.clone());
        Self::new(
            move |unknown| {
                if values.iter().any(|value| *value == unknown) {
                    Ok(unknown)
                } else {
                    mismatch("$", "one of the literals", &unknown)
                }
            },
            |value| Ok(value.clone()),
            kind,
        )
    }

    /// A union of two or more shapes (`Union`). `mode` is `"anyOf"` or
    /// `"oneOf"`; a `oneOf` accepts a value matching exactly one member, which
    /// is the only semantic difference, and the repr carries it.
    pub fn union_with_mode(members: Vec<Schema<Unknown>>, mode: &'static str) -> Self {
        // A union with no members admits nothing at all, which is exactly what
        // `never` is: both directions refuse rather than pretending a value
        // could resolve to a member that does not exist.
        if members.is_empty() {
            return Schema::as_unknown(Schema::never());
        }
        let kind = SchemaKind::Union {
            members: members.iter().map(|member| member.repr_kind()).collect(),
            mode,
        };
        let declared_members = Some(members.clone());
        let decoders: Vec<Arc<Schema<Unknown>>> = members.into_iter().map(Arc::new).collect();
        // The refusal names the members by their type, which is what the pin
        // prints for a union that resolved none: `Expected string | number`.
        let expected: String = decoders
            .iter()
            .map(|member| member.to_type())
            .collect::<Vec<_>>()
            .join(" | ");
        // `oneOf` is a decode rule, not a repr detail: a value two members both
        // accept is refused rather than resolved to whichever came first. That
        // costs the early exit an `anyOf` keeps, which is the whole difference
        // between the two modes.
        let one_of = mode == "oneOf";
        // The pin's Union relation selects a candidate member and answers with
        // THAT member's relation, which is why a member carrying its own
        // `toEquivalence` decides its values instead of a wire-form compare.
        // A member owns these values when its own decode accepts both; the
        // first such member wins, matching the decode order.
        let relation_members: Vec<(Arc<Schema<Unknown>>, Equivalence<Unknown>)> = decoders
            .iter()
            .map(|member| (Arc::clone(member), member.to_equivalence()))
            .collect();
        let mut schema = Self::new(
            move |unknown| {
                let candidates = union_candidates(&decoders, &unknown);
                if candidates.is_empty() {
                    return mismatch_dynamic("$", expected.clone(), &unknown);
                }
                // The pin hands a lone candidate's own failure back rather than
                // the union's summary: the input already said which member it
                // meant, so the useful message is that member's.
                if candidates.len() == 1 {
                    return decoders[candidates[0]].decode(unknown);
                }
                let mut decoded = None;
                let mut matches = 0usize;
                for index in candidates {
                    if let Ok(value) = decoders[index].decode(unknown.clone()) {
                        matches += 1;
                        decoded.get_or_insert(value);
                        if !one_of {
                            break;
                        }
                    }
                }
                if matches > 1 {
                    return mismatch("$", "exactly one member to match", &unknown);
                }
                match decoded {
                    Some(value) => Ok(value),
                    None => mismatch_dynamic("$", expected.clone(), &unknown),
                }
            },
            |value| Ok(value.clone()),
            kind,
        );
        schema.union_members = declared_members;
        schema.equivalence = Some(Arc::new(move |a: &Unknown, b: &Unknown| {
            for (member, relation) in &relation_members {
                if member.decode(a.clone()).is_ok() && member.decode(b.clone()).is_ok() {
                    return relation(a, b);
                }
            }
            false
        }));
        schema
    }
}

/// Where a discriminated union member looks for its tag: a struct key or a
/// tuple position. The pin's `collectSentinels` keeps the same pair.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SentinelKey {
    Field(String),
    Index(usize),
}

/// The literal pairs a member demands of its input (`collectSentinels`): the
/// required fields that are literals, the required elements that are, and, for
/// a nested union, only the pairs every one of its members demands.
fn collect_sentinels(kind: &SchemaKind) -> Vec<(SentinelKey, Unknown)> {
    match kind {
        SchemaKind::Struct(fields) | SchemaKind::StructWithRest(fields, _) => fields
            .iter()
            .filter_map(|(name, field)| match field {
                SchemaKind::Literal(value) => {
                    Some((SentinelKey::Field(name.clone()), value.clone()))
                }
                _ => None,
            })
            .collect(),
        SchemaKind::Tuple(elements) | SchemaKind::TupleWithRest(elements, _) => elements
            .iter()
            .enumerate()
            .filter_map(|(index, element)| match element {
                SchemaKind::Literal(value) => Some((SentinelKey::Index(index), value.clone())),
                _ => None,
            })
            .collect(),
        SchemaKind::Annotated { inner, .. } => collect_sentinels(inner),
        SchemaKind::Union { members, .. } => {
            let mut out = match members.first() {
                Some(first) => collect_sentinels(first),
                None => return Vec::new(),
            };
            for member in &members[1..] {
                let inner = collect_sentinels(member);
                out.retain(|(key, value)| inner.iter().any(|(k, v)| k == key && v == value));
            }
            out
        }
        _ => Vec::new(),
    }
}

const ALL_KINDS: [&str; 6] = ["null", "boolean", "number", "string", "array", "object"];

/// The runtime kinds a member with no sentinels can accept (`getCandidateTypes`).
/// A shape the port cannot see into is a fallback for every kind, so an
/// `Unknown`/`Void` member stays a candidate.
fn candidate_kinds(kind: &SchemaKind) -> Vec<&'static str> {
    match kind {
        SchemaKind::String | SchemaKind::TemplateLiteral(_) => vec!["string"],
        SchemaKind::Number => vec!["number"],
        SchemaKind::Boolean => vec!["boolean"],
        SchemaKind::Array(_) | SchemaKind::Tuple(_) | SchemaKind::TupleWithRest(..) => {
            vec!["array"]
        }
        SchemaKind::Object
        | SchemaKind::Record(..)
        | SchemaKind::Struct(_)
        | SchemaKind::StructWithRest(..) => {
            // A struct with no fields has nothing to exclude a kind by.
            if matches!(kind, SchemaKind::Struct(fields) if fields.is_empty()) {
                ALL_KINDS.to_vec()
            } else {
                vec!["object"]
            }
        }
        SchemaKind::Literal(value) => vec![value.kind()],
        SchemaKind::Literals(values) => {
            let mut out: Vec<&'static str> = Vec::new();
            for value in values {
                let kind = value.kind();
                if !out.contains(&kind) {
                    out.push(kind);
                }
            }
            out
        }
        SchemaKind::Opaque("null") => vec!["null"],
        SchemaKind::Annotated { inner, .. } => candidate_kinds(inner),
        SchemaKind::Union { members, .. } => {
            let mut out: Vec<&'static str> = Vec::new();
            for member in members {
                for kind in candidate_kinds(member) {
                    if !out.contains(&kind) {
                        out.push(kind);
                    }
                }
            }
            out
        }
        _ => ALL_KINDS.to_vec(),
    }
}

/// The value a sentinel key holds in the input, or `None` when the key is not
/// there. An absent key is neutral; an observed one decides.
fn sentinel_value(input: &Unknown, key: &SentinelKey) -> Option<Unknown> {
    match (input, key) {
        (Unknown::Object(map), SentinelKey::Field(name)) => map.get(name).cloned(),
        (Unknown::Array(items), SentinelKey::Index(index)) => items.get(*index).cloned(),
        _ => None,
    }
}

/// The members that demand `value` at `key`, or `None` when no member does.
fn sentinel_matches(
    by_sentinel: &[(SentinelKey, Vec<(Unknown, Vec<usize>)>)],
    key: &SentinelKey,
    value: &Unknown,
) -> Option<Vec<usize>> {
    let values = by_sentinel
        .iter()
        .find(|(candidate, _)| candidate == key)?
        .1
        .iter()
        .find(|(candidate, _)| candidate == value)?
        .1
        .clone();
    Some(values)
}

fn add_sentinel(
    by_sentinel: &mut Vec<(SentinelKey, Vec<(Unknown, Vec<usize>)>)>,
    key: &SentinelKey,
    value: &Unknown,
    index: usize,
) {
    let position = match by_sentinel
        .iter()
        .position(|(candidate, _)| candidate == key)
    {
        Some(position) => position,
        None => {
            by_sentinel.push((key.clone(), Vec::new()));
            by_sentinel.len() - 1
        }
    };
    let values = &mut by_sentinel[position].1;
    let value_position = match values.iter().position(|(candidate, _)| candidate == value) {
        Some(position) => position,
        None => {
            values.push((value.clone(), Vec::new()));
            values.len() - 1
        }
    };
    let indexes = &mut values[value_position].1;
    if !indexes.contains(&index) {
        indexes.push(index);
    }
}

/// A member's sentinels, read from the field/element lists a shape keeps beside
/// its kind. `struct_shape` records itself as a plain object, so the kind alone
/// would hide the tags it declares; `struct_of` keeps them in the kind.
pub(crate) fn member_sentinels(member: &Schema<Unknown>) -> Vec<(SentinelKey, Unknown)> {
    // A tagged class carries its discriminant in the class metadata rather than
    // in a field list, because the class strips `_tag` before its source struct
    // sees it. Without this the union index and the dispatcher cannot see the
    // tag the class demands, and a union of classes would try every member.
    if let Some(meta) = member.class_meta.as_deref() {
        if let Some(tag) = meta.discriminant() {
            return vec![(
                SentinelKey::Field("_tag".to_string()),
                Unknown::Text(tag.to_string()),
            )];
        }
    }
    if let Some(fields) = &member.struct_fields {
        return fields
            .iter()
            .filter(|field| field.optionality() == Optionality::Required)
            .filter_map(|field| match field.inner().repr_kind() {
                SchemaKind::Literal(value) => {
                    Some((SentinelKey::Field(field.name().to_string()), value))
                }
                _ => None,
            })
            .collect();
    }
    if let Some(elements) = &member.tuple_elements {
        return elements
            .iter()
            .take_while(|element| !is_optional_element(&element.repr_kind()))
            .enumerate()
            .filter_map(|(index, element)| match element.repr_kind() {
                SchemaKind::Literal(value) => Some((SentinelKey::Index(index), value)),
                _ => None,
            })
            .collect();
    }
    collect_sentinels(&member.repr_kind())
}

/// The members a decoded input can still match (`getCandidateIndex`, without
/// the constructor axis). A discriminated member is skipped when the tag it
/// demands is present and different, which is what keeps a tagged struct from
/// claiming every object.
fn union_candidates(members: &[Arc<Schema<Unknown>>], input: &Unknown) -> Vec<usize> {
    let mut otherwise: Vec<(&'static str, usize)> = Vec::new();
    let mut by_sentinel: Vec<(SentinelKey, Vec<(Unknown, Vec<usize>)>)> = Vec::new();
    let mut owners: Vec<(SentinelKey, Vec<usize>)> = Vec::new();
    let mut discriminated = 0usize;

    for (index, member) in members.iter().enumerate() {
        let kind = member.repr_kind();
        if matches!(kind, SchemaKind::Opaque("never")) {
            continue;
        }
        let sentinels = member_sentinels(member);
        if sentinels.is_empty() {
            for candidate in candidate_kinds(&kind) {
                otherwise.push((candidate, index));
            }
            continue;
        }
        discriminated += 1;
        for (key, value) in sentinels {
            add_sentinel(&mut by_sentinel, &key, &value, index);
            let position = match owners.iter().position(|(owned, _)| *owned == key) {
                Some(position) => position,
                None => {
                    owners.push((key, Vec::new()));
                    owners.len() - 1
                }
            };
            if !owners[position].1.contains(&index) {
                owners[position].1.push(index);
            }
        }
    }

    let fallback = |kind: &'static str| -> Vec<usize> {
        otherwise
            .iter()
            .filter(|(candidate, _)| *candidate == kind)
            .map(|(_, index)| *index)
            .collect()
    };

    // Without a discriminated member there is nothing to narrow by. An array
    // counts as an object keyword, which is how a tuple's positional tag works.
    if discriminated == 0 || !matches!(input, Unknown::Object(_) | Unknown::Array(_)) {
        return fallback(input.kind());
    }

    let mut selected = fallback(input.kind());
    // An observed key adds the members that demand its value ...
    for (key, _) in &by_sentinel {
        if let Some(value) = sentinel_value(input, key) {
            if let Some(indexes) = sentinel_matches(&by_sentinel, key, &value) {
                selected.extend(indexes);
            }
        }
    }

    // ... and rejects every selected member that owns the key without
    // demanding that value. A key a member never mentions leaves it alone,
    // which is what makes two members with different tags decidable.
    for (key, _) in &by_sentinel {
        let Some(value) = sentinel_value(input, key) else {
            continue;
        };
        let matching = sentinel_matches(&by_sentinel, key, &value).unwrap_or_default();
        let owned = owners
            .iter()
            .find(|(owned, _)| owned == key)
            .map(|(_, indexes)| indexes.clone())
            .unwrap_or_default();
        selected.retain(|index| !owned.contains(index) || matching.contains(index));
    }

    selected.sort_unstable();
    selected.dedup();
    selected
}

impl Schema<Unknown> {
    /// Wrap `base` in a refinement (`check`). The kind records the check so a
    /// repr can render it; decode enforces the predicate where this port knows
    /// it, and passes through where it does not, so an unknown check degrades
    /// to "unconstrained" rather than rejecting a valid value.
    pub fn checked(
        base: Schema<Unknown>,
        name: &str,
        args: Vec<&str>,
        annotation: Vec<(&str, &str)>,
    ) -> Self {
        // A re-typing keeps its own kind opaque, so the check must be built on
        // the kind underneath it or the repr would print `Schema.Unknown`.
        let base_kind = base.repr_kind();
        let kind = SchemaKind::Check {
            base: Box::new(base_kind),
            name: name.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            annotation: annotation
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        };
        let numeric: Vec<f64> = args.iter().filter_map(|arg| arg.parse().ok()).collect();
        let bounds = numeric.clone();
        // The first argument is kept in its printed form: some checks read it as
        // text (a RegExp literal for `isPattern`), not as a number.
        let first_arg = args.first().map(|arg| (*arg).to_string());
        let exclusive_minimum = annotation
            .iter()
            .any(|(key, value)| *key == "exclusiveMinimum" && *value == "true");
        let exclusive_maximum = annotation
            .iter()
            .any(|(key, value)| *key == "exclusiveMaximum" && *value == "true");
        let name_owned = name.to_string();
        // The encode closure needs the same three inputs the decode one uses,
        // so they are cloned rather than moved.
        let encode_name = name_owned.clone();
        let encode_bounds = bounds.clone();
        let encode_first_arg = first_arg.clone();
        let base_view = base.check_view.clone();
        let inner = Arc::new(base);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        // The check reads what the schema underneath produces (see
        // `check_value`), and a failure carries the check's own `expected` text
        // when it was given one.
        let decode_view = inner.check_view.clone();
        let encode_view = inner.check_view.clone();
        let decode_refusal = check_refusal(&annotation);
        let encode_refusal = decode_refusal.clone();
        let mut out = Self::new(
            move |unknown| {
                let checked = check_value(decode_view.as_ref(), &unknown);
                let holds = match name_owned.as_str() {
                    "isBetweenCodePoints" => {
                        let bmin = bounds.first().copied().unwrap_or(0.0).floor().max(0.0);
                        let bmax = bounds.get(1).copied().unwrap_or(0.0).floor().max(0.0);
                        if let Unknown::Text(text) = &checked {
                            let count = text.chars().count() as f64;
                            count >= bmin && count <= bmax
                        } else {
                            true
                        }
                    }
                    "isBetween" => {
                        between_holds(&bounds, exclusive_minimum, exclusive_maximum, &checked)
                    }
                    "isBetweenProperties" => between_properties_holds(
                        &bounds,
                        exclusive_minimum,
                        exclusive_maximum,
                        &checked,
                    ),
                    _ => check_holds_arg(&name_owned, first_arg.as_deref(), &checked),
                };
                if !holds {
                    return Err(SchemaError::TypeMismatchDynamic {
                        path: "$".to_string(),
                        expected: decode_refusal.clone(),
                        got: checked.kind(),
                    });
                }
                decode_inner.decode(unknown)
            },
            // The check is the source schema's, so it decides the encode side
            // too: a value that fails it has no wire form rather than one that
            // silently violates the refinement on the way out.
            move |value| {
                let checked = check_value(encode_view.as_ref(), value);
                let holds = match encode_name.as_str() {
                    "isBetweenCodePoints" => {
                        let bmin = encode_bounds
                            .first()
                            .copied()
                            .unwrap_or(0.0)
                            .floor()
                            .max(0.0);
                        let bmax = encode_bounds
                            .get(1)
                            .copied()
                            .unwrap_or(0.0)
                            .floor()
                            .max(0.0);
                        if let Unknown::Text(text) = &checked {
                            let count = text.chars().count() as f64;
                            count >= bmin && count <= bmax
                        } else {
                            true
                        }
                    }
                    "isBetween" => between_holds(
                        &encode_bounds,
                        exclusive_minimum,
                        exclusive_maximum,
                        &checked,
                    ),
                    "isBetweenProperties" => between_properties_holds(
                        &encode_bounds,
                        exclusive_minimum,
                        exclusive_maximum,
                        &checked,
                    ),
                    _ => check_holds_arg(&encode_name, encode_first_arg.as_deref(), &checked),
                };
                if !holds {
                    return Err(SchemaError::TypeMismatchDynamic {
                        path: "$".to_string(),
                        expected: encode_refusal.clone(),
                        got: checked.kind(),
                    });
                }
                encode_inner.try_encode(value)
            },
            kind,
        );
        out.formatter = inner.formatter.clone();
        out.wrapped = None;
        // A refinement of a transform reads what the transform produced too, so
        // a second check stacked on the first still sees the number.
        out.check_view = base_view;
        // The same rule for the shape: a check on a struct still has that
        // struct's fields, which is what a class built from the checked source
        // declares and what a derivation reads.
        out.struct_fields = inner.struct_fields.clone();
        out.tuple_elements = inner.tuple_elements.clone();
        out.union_members = inner.union_members.clone();
        out.class_meta = inner.class_meta.clone();
        out
    }

    /// A `makeFilterGroup`: several refinements behind one annotation. The
    /// rendered argument is what a repr prints, and the members are what a
    /// decode enforces, so a grouped range decides both its ends rather than
    /// passing everything the way a named check with no predicate would.
    pub(crate) fn checked_group(
        base: Schema<Unknown>,
        name: &str,
        args: Vec<&str>,
        annotation: Vec<(&str, &str)>,
        group: &[Check],
    ) -> Self {
        let members: Vec<(String, Option<String>)> = group
            .iter()
            .map(|member| {
                (
                    member.name.clone(),
                    member.args.first().map(|arg| arg.to_string()),
                )
            })
            .collect();
        // A group reads the same value its first member's check reads, and
        // refuses with the group's own annotation.
        let group_refusal = check_refusal(&annotation);
        let annotated = Self::checked(base, name, args, annotation);
        let inner = Arc::new(annotated);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let group_view = inner.check_view.clone();
        let mut out = Self::new(
            move |unknown| {
                for (member_name, member_arg) in &members {
                    let checked = check_value(group_view.as_ref(), &unknown);
                    if !check_holds_arg(member_name, member_arg.as_deref(), &checked) {
                        return Err(SchemaError::TypeMismatchDynamic {
                            path: "$".to_string(),
                            expected: group_refusal.clone(),
                            got: checked.kind(),
                        });
                    }
                }
                decode_inner.decode(unknown)
            },
            move |value| encode_inner.try_encode(value),
            inner.repr_kind(),
        );
        out.formatter = inner.formatter.clone();
        out.wrapped = None;
        out
    }

    /// An `isPropertyNames` check: the record's keys are the thing being
    /// refined, so the key schema travels with the check and every key of the
    /// decoded object is decided by it. Rendering the key schema into the
    /// argument is what a repr prints; carrying the schema itself is what a
    /// decode needs, which is why this is its own constructor rather than a
    /// `checked` call with a rendered argument.
    pub fn checked_property_names(
        base: Schema<Unknown>,
        key: Schema<Unknown>,
        annotation: Vec<(&str, &str)>,
    ) -> Self {
        let rendered = key.to_code();
        let annotated = Self::checked(base, "isPropertyNames", vec![rendered.as_str()], annotation);
        let key = Arc::new(key);
        let decode_key = key.clone();
        let inner = Arc::new(annotated);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let mut out = Self::new(
            move |unknown| {
                if let Unknown::Object(fields) = &unknown {
                    for name in fields.keys() {
                        if let Err(error) = decode_key.decode(Unknown::Text(name.clone())) {
                            return Err(error.at(&format!("$.{name}")));
                        }
                    }
                }
                decode_inner.decode(unknown)
            },
            move |value| encode_inner.try_encode(value),
            inner.repr_kind(),
        );
        out.formatter = inner.formatter.clone();
        out.wrapped = None;
        out
    }
}

/// Decimal places a plain decimal number shows, which is the precision JS's
/// `Number.prototype.toString` would print. A value JS prints in exponent
/// notation reports zero, and the pin names the rows where that matters.
fn decimal_places(value: f64) -> u32 {
    let text = format!("{value}");
    match text.split_once('.') {
        Some((_, fraction)) => fraction.chars().take_while(|c| c.is_ascii_digit()).count() as u32,
        None => 0,
    }
}

/// Round to `places` decimal places, the way JS's `toFixed` does. A precision
/// past f64's range has no scale to divide by, so the value is returned as it
/// stands: the pin names rows at 1e-323, where the operands are already exactly
/// representable or not at all, and rounding there would only produce NaN.
fn round_to(value: f64, places: u32) -> f64 {
    let scale = 10f64.powi(places as i32);
    if !scale.is_finite() {
        return value;
    }
    (value * scale).round() / scale
}

/// Whether the decoded value satisfies a named refinement. Only the checks the
/// pin's JSON Schema import actually emits are implemented; anything else
/// passes, so an unimplemented refinement is unenforced rather than wrong.
pub(crate) fn check_holds_named(name: &str, arg: Option<&str>, value: &Unknown) -> bool {
    check_holds_arg(name, arg, value)
}

pub(crate) fn check_holds_arg(name: &str, arg: Option<&str>, value: &Unknown) -> bool {
    // `isIncluding` has no numeric argument, so it is matched on the text it
    // carries before the bound below is parsed.
    if name == "isIncluding" {
        let Some(needle) = arg else {
            return true;
        };
        return matches!(value, Unknown::Text(text) if text.contains(needle));
    }
    // The two string-format checks the pin spells as a fixed pattern. They take
    // no argument, and the pin's own expected text names what they want.
    if name == "isStringBigInt" {
        return matches!(value, Unknown::Text(text) if is_string_big_int_text(text));
    }
    if name == "isStringSymbol" {
        return matches!(value, Unknown::Text(text) if is_string_symbol_text(text));
    }
    // `isPattern`'s argument is the JS RegExp literal the pin's repr prints, so
    // the pattern has to be read back out of it rather than matched as written.
    if name == "isPattern" {
        let Some(inner) = arg.and_then(|arg| {
            arg.strip_prefix("new RegExp(")
                .and_then(|rest| rest.strip_suffix(", \"u\")"))
                .or_else(|| {
                    arg.strip_prefix("new RegExp(")
                        .and_then(|r| r.strip_suffix(')'))
                })
        }) else {
            return true;
        };
        let pattern = inner.trim_matches('"');
        return match compile_pattern(pattern) {
            Some(compiled) => match value {
                Unknown::Text(text) => pattern_matches(&compiled, text),
                _ => false,
            },
            // Outside the supported subset, so unenforced rather than guessed.
            None => true,
        };
    }
    let limit = arg.and_then(|arg| arg.parse::<f64>().ok());
    check_holds(name, limit, value)
}

/// The value a check reads: the wire value through the schema's own projection
/// when it has one (see [`Schema::checked`]), and the wire value itself when it
/// has none. Shared by every place a named check is evaluated, so "a check
/// reads what the transform produced" is one rule rather than three.
pub(crate) fn check_value(view: Option<&CheckView>, wire: &Unknown) -> Unknown {
    match view {
        Some(view) => view(wire),
        None => wire.clone(),
    }
}

/// The message a failed check carries: the check's own `expected` annotation,
/// or the pin's fallback for a check that carries none.
fn check_refusal(annotation: &[(&str, &str)]) -> String {
    annotation
        .iter()
        .find(|(key, _)| *key == "expected")
        .map(|(_, value)| (*value).to_string())
        .unwrap_or_else(|| "a value satisfying the refinement".to_string())
}

/// The pin's `^-?\d+$`: ASCII digits with an optional leading `-`, and no
/// leading `+` (SchemaAST.ts:4862).
fn is_string_big_int_text(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// The pin's `^Symbol\(([\s\S]*)\)$`: `Symbol(` then anything (newlines
/// included) then `)`. The inner text is greedy, so `Symbol(a))` matches.
fn is_string_symbol_text(text: &str) -> bool {
    text.strip_prefix("Symbol(")
        .is_some_and(|rest| rest.ends_with(')'))
}

/// The boxed shape a `Redacted` value has here: an object carrying `value`.
/// The pin tests for a `Redacted` instance; a port whose values are all wire
/// values tests the shape instead, which is the same rule for everything the
/// port can hold.
fn redacted_box(value: &Unknown) -> Option<&std::collections::BTreeMap<String, Unknown>> {
    match value {
        Unknown::Object(map) if map.contains_key("value") => Some(map),
        _ => None,
    }
}

/// What a value that is not a box is refused as.
fn not_redacted(value: &Unknown) -> SchemaError {
    SchemaError::TypeMismatchDynamic {
        path: "$".to_string(),
        expected: "Redacted".to_string(),
        got: value.kind(),
    }
}

/// What a boxed value the wrapped schema refuses is reported as: the pin hides
/// which of its checks failed, and reports the failure at the box.
fn invalid_redacted_value(got: &Unknown) -> SchemaError {
    SchemaError::TypeMismatchDynamic {
        path: "$.value".to_string(),
        expected: "a valid value".to_string(),
        got: got.kind(),
    }
}

/// The declared label, when there is one, has to be the label the box carries.
fn check_redacted_label(
    label: Option<&str>,
    map: &std::collections::BTreeMap<String, Unknown>,
) -> Result<(), SchemaError> {
    let Some(expected) = label else {
        return Ok(());
    };
    if matches!(map.get("label"), Some(Unknown::Text(text)) if text == expected) {
        return Ok(());
    }
    Err(SchemaError::TypeMismatchDynamic {
        path: "$.label".to_string(),
        // The pin checks the label with `Literal(label)`, so the expected side
        // is the literal's own spelling, quotes and all.
        expected: format!("{expected:?}"),
        got: map.get("label").map_or("absent", Unknown::kind),
    })
}

/// Decode into a box: the same shape check, label check and inner decode the
/// pin's `Redacted` performs, with the box rebuilt around the decoded value.
fn redacted_decode(
    inner: &Schema<Unknown>,
    label: Option<&str>,
    value: &Unknown,
) -> Result<Unknown, SchemaError> {
    let Some(map) = redacted_box(value) else {
        return Err(not_redacted(value));
    };
    check_redacted_label(label, map)?;
    let boxed = map.get("value").cloned().unwrap_or(Unknown::Null);
    let decoded = inner
        .decode(boxed.clone())
        .map_err(|_| invalid_redacted_value(&boxed))?;
    Ok(rebuild_box(map, decoded))
}

/// Encode out of a box: the box is the wire form, so the wrapped value goes
/// through the inner schema's encode and stays boxed.
///
/// The box has to hold a value the wrapped schema can read, on the way out as
/// much as on the way in — the pin refuses a box its schema cannot take, and
/// reports it at the box rather than with the schema's own words. Reading it
/// first is also what normalizes it: the value that leaves is the schema's
/// canonical wire form of what the box held.
fn redacted_encode(
    inner: &Schema<Unknown>,
    label: Option<&str>,
    value: &Unknown,
) -> Result<Unknown, SchemaError> {
    let encoded = unboxed_value(inner, label, value)?;
    match value {
        Unknown::Object(map) => Ok(rebuild_box(map, encoded)),
        // `unboxed_value` refuses anything that is not a box, so this is the
        // shape check on a value that already passed it.
        other => Err(not_redacted(other)),
    }
}

/// The value a box holds, validated and encoded: the shape and label checks a
/// box makes, then the wrapped schema's read and write of what is inside it.
fn unboxed_value(
    inner: &Schema<Unknown>,
    label: Option<&str>,
    value: &Unknown,
) -> Result<Unknown, SchemaError> {
    let Some(map) = redacted_box(value) else {
        return Err(not_redacted(value));
    };
    check_redacted_label(label, map)?;
    let boxed = map.get("value").cloned().unwrap_or(Unknown::Null);
    let decoded = inner
        .decode(boxed.clone())
        .map_err(|_| invalid_redacted_value(&boxed))?;
    inner
        .try_encode(&decoded)
        .map_err(|_| invalid_redacted_value(&boxed))
}

/// The box around a value, keeping the label the input carried — the pin's
/// `Redacted.make(value, { label: input.label })`.
fn rebuild_box(map: &std::collections::BTreeMap<String, Unknown>, value: Unknown) -> Unknown {
    let mut out = std::collections::BTreeMap::new();
    if let Some(label) = map.get("label") {
        out.insert("label".to_string(), label.clone());
    }
    out.insert("value".to_string(), value);
    Unknown::Object(out)
}

/// The schema a `flip` was built from: the pin's `flipped.schema`. An
/// annotation over a flip keeps it, so `flip(schema).annotate({}).schema`
/// answers the same thing.
pub fn flip_source(schema: &Schema<Unknown>) -> Option<Schema<Unknown>> {
    schema
        .children
        .iter()
        .find(|(name, _)| name == "flipped")
        .map(|(_, child)| child.as_ref().clone())
}

/// The schema a `Redacted` box wraps: the pin's `schema.value`. An annotation
/// over a box keeps the wrapped schema, so `schema.annotate({}).value` answers
/// the same thing.
pub fn redacted_value(schema: &Schema<Unknown>) -> Option<Schema<Unknown>> {
    schema
        .children
        .iter()
        .find(|(name, _)| name == "value")
        .map(|(_, child)| child.as_ref().clone())
}

/// Whether an amount sits between two bounds: each includes its endpoint
/// unless the check marks it exclusive, and a missing bound leaves that side
/// open.
fn bounds_hold(
    bounds: &[f64],
    exclusive_minimum: bool,
    exclusive_maximum: bool,
    amount: f64,
) -> bool {
    if let Some(minimum) = bounds.first() {
        let included = if exclusive_minimum {
            amount > *minimum
        } else {
            amount >= *minimum
        };
        if !included {
            return false;
        }
    }
    if let Some(maximum) = bounds.get(1) {
        let included = if exclusive_maximum {
            amount < *maximum
        } else {
            amount <= *maximum
        };
        if !included {
            return false;
        }
    }
    true
}

/// Whether the key schema describes a wire key. A key arrives as text, so a
/// number key schema (`Record(Int, ...)`) is also offered the JS number reading
/// of the name: `"2.2"`, `"Infinity"` and `"NaN"` are all names a number key
/// schema can consider, and the ones it refuses are unmatched like any other.
fn key_names_entry(key: &Schema<Unknown>, name: &str) -> bool {
    if key.decode(Unknown::Text(name.to_string())).is_ok() {
        return true;
    }
    match key_number(name) {
        Some(number) => key.decode(Unknown::Number(number)).is_ok(),
        None => false,
    }
}

/// The number a property name spells, the way JavaScript's `Number(name)` reads
/// one: the two infinities and `NaN` have their own spellings, an empty name is
/// zero, and anything else has to parse as a number outright.
fn key_number(name: &str) -> Option<f64> {
    match name {
        "Infinity" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        "" => Some(0.0),
        _ => name
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|_| !name.trim().is_empty()),
    }
}

/// Whether a number satisfies `isBetween`. The two bounds ride the check's/// argument list in order, and exclusivity rides its annotation, which is where
/// a check keeps the flags a repr would print. A value that is not a number is
/// left to the base schema, like every other check here.
fn between_holds(
    bounds: &[f64],
    exclusive_minimum: bool,
    exclusive_maximum: bool,
    value: &Unknown,
) -> bool {
    let Unknown::Number(number) = value else {
        return true;
    };
    bounds_hold(bounds, exclusive_minimum, exclusive_maximum, *number)
}

/// Whether an object satisfies `isBetweenProperties`: the same bounds, over its
/// entry count. A value that is not an object is left to the base schema.
fn between_properties_holds(
    bounds: &[f64],
    exclusive_minimum: bool,
    exclusive_maximum: bool,
    value: &Unknown,
) -> bool {
    let Unknown::Object(map) = value else {
        return true;
    };
    bounds_hold(
        bounds,
        exclusive_minimum,
        exclusive_maximum,
        map.len() as f64,
    )
}

fn check_holds(name: &str, limit: Option<f64>, value: &Unknown) -> bool {
    match (name, limit, value) {
        ("isFinite", _, Unknown::Number(number)) => number.is_finite(),
        // A `type: integer` import is a safe integer, not any integral
        // double: the pin's filter is `Number.isSafeInteger`, so 1e20 is an
        // integer-valued double the schema still refuses.
        ("isInt", _, Unknown::Number(number)) => {
            number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER
        }
        // A trimmed string carries no padding on either end. The check names
        // no bound, so it is matched on the value alone.
        ("isTrimmed", None, Unknown::Text(text)) => {
            text.trim().chars().count() == text.chars().count()
        }
        ("isMinLength", Some(min), Unknown::Text(text)) => text.chars().count() as f64 >= min,
        ("isMaxLength", Some(max), Unknown::Text(text)) => text.chars().count() as f64 <= max,
        ("isMinCodePoints", Some(min), Unknown::Text(text)) => {
            // TS normalizes the bound: floor then clamp to zero.
            let bound = min.floor().max(0.0);
            text.chars().count() as f64 >= bound
        }
        ("isMaxCodePoints", Some(max), Unknown::Text(text)) => {
            let bound = max.floor().max(0.0);
            text.chars().count() as f64 <= bound
        }
        ("isGreaterThan", Some(bound), Unknown::Number(number)) => *number > bound,
        ("isGreaterThanOrEqualTo", Some(bound), Unknown::Number(number)) => *number >= bound,
        ("isLessThan", Some(bound), Unknown::Number(number)) => *number < bound,
        ("isLessThanOrEqualTo", Some(bound), Unknown::Number(number)) => *number <= bound,
        // A plain quotient leaves float dust: 0.3 / 0.1 is 2.9999999999999996,
        // so its remainder is not zero and a plain check would reject a value the
        // pin accepts. The pin rounds the quotient first and compares at the
        // operands' longer decimal precision, which is what this does.
        ("isMultipleOf", Some(divisor), Unknown::Number(number)) => {
            divisor != 0.0 && {
                let precision = decimal_places(divisor).max(decimal_places(*number));
                let remainder = *number - (*number / divisor).round() * divisor;
                round_to(remainder, precision) == 0.0
            }
        }
        ("isMinLength", Some(min), Unknown::Array(items)) => items.len() as f64 >= min,
        ("isMaxLength", Some(max), Unknown::Array(items)) => items.len() as f64 <= max,
        ("isMinProperties", Some(min), Unknown::Object(map)) => map.len() as f64 >= min,
        ("isMaxProperties", Some(max), Unknown::Object(map)) => map.len() as f64 <= max,
        // Structural equality: two entries are duplicates when their unknown
        // values are equal, which for objects and arrays is deep equality.
        ("isUnique", _, Unknown::Array(items)) => items
            .iter()
            .enumerate()
            .all(|(index, item)| !items[..index].contains(item)),
        _ => true,
    }
}

impl<T> Schema<T> {
    /// The Source-style `code` for this schema, the shape the pin's
    /// `SchemaRepresentation.toCodeDocument` emits (`Schema.String`,
    /// `Schema.Array(Schema.Never)`, `Schema.Union([...])`).
    pub fn to_code(&self) -> String {
        // A re-typing keeps its own kind opaque, so its code is the code of
        // the schema it wraps.
        match &self.wrapped {
            Some(boxed) => boxed.code.clone(),
            None => code_of(&self.kind, &self.children),
        }
    }

    /// The TypeScript-style `type` for this schema, the second half of a
    /// `Code` entry (`string`, `ReadonlyArray<never>`, `never | string`).
    pub fn to_type(&self) -> String {
        match &self.wrapped {
            Some(boxed) => boxed.ty.clone(),
            None => type_of(&self.kind, &self.children),
        }
    }
}

fn code_of(kind: &SchemaKind, children: &[(String, Arc<Schema<Unknown>>)]) -> String {
    match kind {
        SchemaKind::String => "Schema.String".to_string(),
        SchemaKind::Number => "Schema.Number".to_string(),
        SchemaKind::Boolean => "Schema.Boolean".to_string(),
        SchemaKind::Object => "Schema.Object".to_string(),
        SchemaKind::Opaque("never") => "Schema.Never".to_string(),
        SchemaKind::Opaque("null") => "Schema.Null".to_string(),
        SchemaKind::Opaque("any") => "Schema.Any".to_string(),
        SchemaKind::Opaque("unknown") => "Schema.Unknown".to_string(),
        SchemaKind::Json => "Schema.Json".to_string(),
        SchemaKind::Reference(name) => name.clone(),
        // Annotations riding a reference print through a `suspend`, because the
        // pin names the codec's type parameter there rather than the definition's
        // own type.
        SchemaKind::Annotated { inner, pairs }
            if matches!(&**inner, SchemaKind::Reference(_)) && !pairs.is_empty() =>
        {
            let name = match &**inner {
                SchemaKind::Reference(name) => name,
                _ => unreachable!("guarded by the arm"),
            };
            let rendered = pairs
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", json_key(key), render_annotation_value(value))
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("Schema.suspend((): Schema.Codec<{name}> => {name}).annotate({{ {rendered} }})")
        }
        SchemaKind::OptionalKey(inner) => {
            format!("Schema.optionalKey({})", code_of_at(inner, children))
        }
        SchemaKind::Literal(value) => format!("Schema.Literal({})", render_value(value)),
        SchemaKind::TemplateLiteral(parts) => {
            let inner: Vec<String> = parts
                .iter()
                .map(|part| code_of_at(part, children))
                .collect();
            format!("Schema.TemplateLiteral([{}])", inner.join(", "))
        }
        SchemaKind::Literals(values) => {
            let inner: Vec<String> = values.iter().map(render_value).collect();
            format!("Schema.Literals([{}])", inner.join(", "))
        }
        SchemaKind::StructWithRest(fields, rest) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_code())
                        .unwrap_or_else(|| code_of(field, &[]));
                    format!("{}: {}", json_key(name), child)
                })
                .collect();
            format!(
                "Schema.StructWithRest(Schema.Struct({{ {} }}), [{}])",
                inner.join(", "),
                code_of_at(rest, children)
            )
        }
        SchemaKind::TupleWithRest(elements, rest) => {
            let inner: Vec<String> = elements.iter().map(|e| code_of_at(e, children)).collect();
            let rest: Vec<String> = rest.iter().map(|r| code_of_at(r, children)).collect();
            format!(
                "Schema.TupleWithRest(Schema.Tuple([{}]), [{}])",
                inner.join(", "),
                rest.join(", ")
            )
        }
        SchemaKind::Union { members, mode } => {
            let inner: Vec<String> = members
                .iter()
                .map(|member| code_of_at(member, children))
                .collect();
            // `anyOf` is the union's own default, so the repr only prints the
            // options for the mode that departs from it.
            if *mode == "anyOf" {
                format!("Schema.Union([{}])", inner.join(", "))
            } else {
                format!(
                    "Schema.Union([{}], {{ mode: \"{}\" }})",
                    inner.join(", "),
                    mode
                )
            }
        }
        SchemaKind::Annotated { pairs, inner } => {
            if pairs.is_empty() {
                code_of(inner, children)
            } else {
                let rendered = pairs
                    .iter()
                    .map(|(key, value)| {
                        format!("{}: {}", json_key(key), render_annotation_value(value))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}.annotate({{ {} }})", code_of(inner, children), rendered)
            }
        }
        SchemaKind::Array(inner) => format!("Schema.Array({})", code_of_at(inner, children)),
        SchemaKind::Optional(inner) => format!("Schema.optional({})", code_of_at(inner, children)),
        SchemaKind::Struct(fields) if fields.is_empty() => "Schema.Struct({})".to_string(),
        SchemaKind::Struct(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_code())
                        .unwrap_or_else(|| code_of(field, &[]));
                    format!("{}: {}", json_key(name), child)
                })
                .collect();
            format!("Schema.Struct({{ {} }})", inner.join(", "))
        }
        SchemaKind::Record(key, value) => {
            // The key is only omitted when it is a plain string: a checked key
            // (a `patternProperties` shape) is what the row is about.
            let key = match &**key {
                SchemaKind::String => "Schema.String".to_string(),
                key => code_of_at(key, children),
            };
            format!("Schema.Record({key}, {})", code_of_at(value, children))
        }
        SchemaKind::Tuple(elements) => {
            let inner: Vec<String> = elements.iter().map(|e| code_of_at(e, children)).collect();
            format!("Schema.Tuple([{}])", inner.join(", "))
        }
        SchemaKind::Rest(inner) => format!("Schema.Rest({})", code_of_at(inner, children)),
        SchemaKind::Check {
            base,
            name,
            args,
            annotation,
        } => {
            let pairs = annotation
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", json_key(key), render_annotation_value(value))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let rendered = if pairs.is_empty() {
                format!("Schema.{name}({})", args.join(", "))
            } else {
                format!("Schema.{name}({}).annotate({{ {pairs} }})", args.join(", "))
            };
            // The Check's base is already the repr-visible kind: `checked()`
            // resolved through the re-typing before storing it, so no second
            // lookup happens here.
            format!("{}.check({rendered})", code_of(base, &[]))
        }
        SchemaKind::Opaque(other) => format!("Schema.{other}"),
    }
}

/// A field with no child schema of its own still needs a code, so it is
/// rendered from the kind alone.
fn code_of_at(kind: &SchemaKind, children: &[(String, Arc<Schema<Unknown>>)]) -> String {
    let _ = children;
    code_of(kind, &[])
}

fn type_of(kind: &SchemaKind, children: &[(String, Arc<Schema<Unknown>>)]) -> String {
    match kind {
        // A reference's type is its name, the same as its code.
        SchemaKind::Reference(name) => name.clone(),
        SchemaKind::String => "string".to_string(),
        SchemaKind::Number => "number".to_string(),
        SchemaKind::Boolean => "boolean".to_string(),
        SchemaKind::Object => "object".to_string(),
        SchemaKind::Opaque("never") => "never".to_string(),
        SchemaKind::Opaque("any") | SchemaKind::Opaque("unknown") => "unknown".to_string(),
        SchemaKind::Array(inner) => format!("ReadonlyArray<{}>", type_of_at(inner)),
        SchemaKind::Optional(inner) => {
            format!("{{ readonly _?: {} }}", type_of_at(inner))
        }
        SchemaKind::Struct(fields) if fields.is_empty() => "{}".to_string(),
        SchemaKind::Struct(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_type())
                        .unwrap_or_else(|| type_of(field, &[]));
                    // An exact-optional field types as an optional property,
                    // which is the same `readonly "b"?: string` a StructWithRest
                    // field gets. Only the key differs, not the shape.
                    match field {
                        SchemaKind::OptionalKey(inner_key) => {
                            format!("readonly {}?: {}", json_key(name), type_of_at(inner_key))
                        }
                        _ => format!("readonly {}: {}", json_key(name), child),
                    }
                })
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
        SchemaKind::Record(key, value) => {
            let key = match &**key {
                SchemaKind::String => "string".to_string(),
                key => type_of_at(key),
            };
            format!("{{ readonly [x: {key}]: {} }}", type_of_at(value))
        }
        SchemaKind::Tuple(elements) => {
            let inner: Vec<String> = elements.iter().map(type_of_at).collect();
            format!("readonly [{}]", inner.join(", "))
        }
        SchemaKind::Rest(inner) => format!("Array<{}>", type_of_at(inner)),
        SchemaKind::StructWithRest(fields, rest) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field)| {
                    let child = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| schema.to_type())
                        .unwrap_or_else(|| type_of(field, &[]));
                    // The intersection carries every key the rest does not
                    // claim, so an exact-optional field types as optional there
                    // exactly as it does inside a plain Struct.
                    match field {
                        SchemaKind::OptionalKey(inner_key) => {
                            format!("readonly {}?: {}", json_key(name), type_of_at(inner_key))
                        }
                        _ => format!("readonly {}: {}", json_key(name), child),
                    }
                })
                .collect();
            format!("{{ {} }} & {}", inner.join(", "), type_of_at(rest))
        }
        SchemaKind::TupleWithRest(elements, rest) => {
            let mut inner: Vec<String> = elements
                .iter()
                .map(|element| match element {
                    SchemaKind::OptionalKey(inner_key) => {
                        format!("({})?", type_of_at(inner_key))
                    }
                    other => type_of_at(other),
                })
                .collect();
            // The first rest schema repeats; the ones after it are fixed
            // positions at the end, so they type as plain elements.
            inner.push(format!("...Array<{}>", type_of_at(&rest[0])));
            inner.extend(rest[1..].iter().map(type_of_at));
            format!("readonly [{}]", inner.join(", "))
        }
        SchemaKind::Check { base, .. } => type_of_at(base),
        SchemaKind::Json => "Schema.Json".to_string(),
        SchemaKind::OptionalKey(inner) => format!("({})?", type_of_at(inner)),
        SchemaKind::Literals(values) => values
            .iter()
            .map(render_value)
            .collect::<Vec<_>>()
            .join(" | "),
        SchemaKind::Literal(value) => render_value(value),
        SchemaKind::TemplateLiteral(_) => "string".to_string(),
        SchemaKind::Union { members, .. } => members
            .iter()
            .map(|member| type_of_at(member))
            .collect::<Vec<_>>()
            .join(" | "),
        SchemaKind::Annotated { inner, .. } => type_of(inner, children),
        SchemaKind::Opaque(other) => (*other).to_string(),
    }
}

fn type_of_at(kind: &SchemaKind) -> String {
    type_of(kind, &[])
}

/// A JSON key as the pin's repr quotes it.
fn json_key(name: &str) -> String {
    serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
}

// Value formatting

/// Render a value the way `toFormatter` does: JSON shapes with `never`
/// for impossible values. Numbers print without a trailing `.0`.
pub fn format_value(value: &Unknown) -> String {
    match value {
        Unknown::Null => "null".to_string(),
        Unknown::Bool(true) => "true".to_string(),
        Unknown::Bool(false) => "false".to_string(),
        Unknown::Number(number) => format_number(*number),
        Unknown::Text(text) => serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()),
        Unknown::Array(items) => {
            let inner: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", inner.join(","))
        }
        Unknown::Object(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                        format_value(item)
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// Render a value at a composite position, recursing through `children` where
/// the owner is a Struct. `children` is empty for every other owner.
fn format_root(
    kind: &SchemaKind,
    children: &[(String, Arc<Schema<Unknown>>)],
    value: &Unknown,
) -> String {
    const NO_CHILDREN: &[(String, Arc<Schema<Unknown>>)] = &[];
    match kind {
        SchemaKind::Opaque("never") => "never".to_string(),
        SchemaKind::Struct(fields) if !fields.is_empty() => {
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, _)| {
                    let item = map.get(name).cloned().unwrap_or(Unknown::Null);
                    let key = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
                    let (child_kind, child_children) = children
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, schema)| (schema.repr_kind(), schema.children.clone()))
                        .unwrap_or_else(|| (SchemaKind::Object, Vec::new()));
                    format!(
                        "{key}: {}",
                        format_root(&child_kind, &child_children, &item)
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        SchemaKind::Record(_, value_kind) => match value {
            Unknown::Object(map) => {
                let inner: Vec<String> = map
                    .iter()
                    .map(|(key, item)| {
                        let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                        format!("{key}: {}", format_root(value_kind, NO_CHILDREN, item))
                    })
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Tuple(elements) => match value {
            Unknown::Array(items) => {
                let mut inner: Vec<String> = Vec::new();
                for (index, element_kind) in elements.iter().enumerate() {
                    match element_kind {
                        SchemaKind::Rest(rest) => {
                            for item in items.iter().skip(index) {
                                inner.push(format_root(rest, NO_CHILDREN, item));
                            }
                            break;
                        }
                        _ => match items.get(index) {
                            Some(item) => inner.push(format_root(element_kind, NO_CHILDREN, item)),
                            None => break,
                        },
                    }
                }
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Array(item_kind) => match value {
            Unknown::Array(items) => {
                let inner: Vec<String> = items
                    .iter()
                    .map(|i| format_root(item_kind, NO_CHILDREN, i))
                    .collect();
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Optional(inner) => match value {
            Unknown::Null => "null".to_string(),
            other => format_root(inner, NO_CHILDREN, other),
        },
        SchemaKind::Annotated { inner, .. } => format_root(inner, children, value),
        _ => format_value(value),
    }
}

/// A JS-style number: integers print bare.
fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else if number.abs() >= 1e21 || (number != 0.0 && number.abs() < 1e-6) {
        // JavaScript prints these in exponent form: `1e+21`, `5e-102`. Rust's
        // `{}` prints the full digits, which is what made `1e21` print as
        // `1000000000000000000000`.
        exponent_number(number)
    } else {
        format!("{number}")
    }
}

/// A number in JavaScript's exponent form: a signed exponent, and no trailing
/// `.0` on the mantissa.
fn exponent_number(number: f64) -> String {
    let text = format!("{number:e}");
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let mantissa = mantissa.trim_end_matches(".0");
            match exponent.strip_prefix('-') {
                Some(digits) => format!("{mantissa}e-{digits}"),
                None => format!("{mantissa}e+{exponent}"),
            }
        }
        None => text,
    }
}

/// Render `value` through `kind`. A Struct with declared fields renders each
/// field as `"name": value` joined by ", ", and the same rule applies to every
/// nested Struct, which is why this walks the SCHEMA tree rather than the value
/// tree: the pin's nested output is `{ "a": "a", "as": [{ "a": "b" }] }`, with
/// the spacing on the inner objects too. An EMPTY Struct has no declared
/// fields, so it falls through to the plain JSON-ish rendering, which is what
/// the pin does too (its "empty" row asserts `{"a":1}` unspaced).
fn format_with(kind: &SchemaKind, value: &Unknown) -> String {
    match kind {
        SchemaKind::Opaque("never") => "never".to_string(),
        SchemaKind::Struct(fields) if !fields.is_empty() => {
            let Unknown::Object(map) = value else {
                return format_value(value);
            };
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, field_kind)| {
                    let item = map.get(name).cloned().unwrap_or(Unknown::Null);
                    let key = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}: {}", format_with(field_kind, &item))
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        SchemaKind::Record(_, value_kind) => match value {
            Unknown::Object(map) => {
                let inner: Vec<String> = map
                    .iter()
                    .map(|(key, item)| {
                        let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                        format!("{key}: {}", format_with(value_kind, item))
                    })
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Tuple(elements) => match value {
            Unknown::Array(items) => {
                let mut inner: Vec<String> = Vec::new();
                for (index, element_kind) in elements.iter().enumerate() {
                    match element_kind {
                        SchemaKind::Rest(inner_kind) => {
                            for item in items.iter().skip(index) {
                                inner.push(format_with(inner_kind, item));
                            }
                            break;
                        }
                        _ => match items.get(index) {
                            Some(item) => inner.push(format_with(element_kind, item)),
                            None => break,
                        },
                    }
                }
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Array(item_kind) => match value {
            Unknown::Array(items) => {
                let inner: Vec<String> = items.iter().map(|i| format_with(item_kind, i)).collect();
                format!("[{}]", inner.join(", "))
            }
            other => format_value(other),
        },
        SchemaKind::Optional(inner) => match value {
            Unknown::Null => "null".to_string(),
            other => format_with(inner, other),
        },
        SchemaKind::Annotated { inner, .. } => format_with(inner, value),
        _ => format_value(value),
    }
}
