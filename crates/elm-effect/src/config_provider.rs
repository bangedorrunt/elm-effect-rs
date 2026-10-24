//! Layered key/value lookup.
//!
//! A [`Provider`] answers dotted paths from an in-memory map. Providers
//! compose: [`Provider::nested`] scopes every lookup under a prefix,
//! [`Provider::or_else`] falls through to a second provider on absence,
//! [`Provider::from_env`] snapshots the process environment once (parsed
//! at startup; domain code never reads it). [`Provider::single`] builds
//! the one-shot provider the array-element path uses internally.

use std::collections::BTreeMap;
use std::rc::Rc;

// The crate root re-exports `Defect`, which is the one name the path-included
// test hosts have to provide for this module to build outside the library.
use crate::Defect;

/// A source failure, with the two kinds kept apart.
///
/// The separation is the whole point (`Config.test.ts:65`): [`Source`] is a
/// typed `ConfigProvider.SourceError` that `Config.orElse` may recover, while
/// [`Defect`] is a provider that died, which no default, option, or fallback
/// may answer for. A single `String` channel could not tell them apart, so
/// every provider failure looked recoverable.
///
/// [`Source`]: ProviderError::Source
/// [`Defect`]: ProviderError::Defect
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderError {
    /// A typed read failure (`ConfigProvider.SourceError`).
    Source(String),
    /// The provider itself failed: the defect travels instead of a result.
    Defect(Defect),
}

impl ProviderError {
    /// A typed source failure.
    pub fn source(message: impl Into<String>) -> Self {
        Self::Source(message.into())
    }

    /// A provider that died while answering.
    pub fn defect(defect: Defect) -> Self {
        Self::Defect(defect)
    }
}

/// A container node a source stores explicitly, with no value of its own.
///
/// A flat map cannot hold one: `{ a: {} }` and `{}` store the same nothing, so
/// an explicit empty container and an absent path are one state. `Config.schema`
/// validates a present container as a whole (Config.test.ts:729, :877), so the
/// difference has to be representable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// An object node, which may have no children.
    Object,
    /// An array node with its declared length. A flat map cannot tell a hole
    /// from the end of the array, and Effect 4's node can: `makeArray(2)` with
    /// only position 0 stored still has position 1, holding `undefined`
    /// (Config.test.ts:1873).
    Array(usize),
}

/// Key/value lookup backing [`Config`](super::Config) descriptors.
/// Implemented by [`Provider`] and its views so custom sources
/// (lookup functions, path transforms) feed `Config::load` directly.
pub trait Lookup {
    /// Value at `path`, or `None` when absent (empty reads missing
    /// unless the source preserves them).
    fn get(&self, path: &[String]) -> Option<String>;

    /// True when anything is stored strictly under `path`.
    ///
    /// A schema config reads a node's shape from what is beneath its path, so
    /// it needs to tell "an object with no matching fields" from "nothing
    /// here" (Config.test.ts:2253). The default keeps sources that cannot
    /// enumerate their keys on the old rule.
    fn has_children(&self, path: &[String]) -> bool {
        let _ = path;
        false
    }

    /// The keys directly under `path`, in the source's own order.
    ///
    /// `Config.Record` and a schema `Record` decode whatever keys the source
    /// has, so they need the list rather than one known path. A source that
    /// cannot enumerate keeps the old rule (no keys).
    fn child_keys(&self, path: &[String]) -> Vec<String> {
        let _ = path;
        Vec::new()
    }

    /// The explicit container at `path`, when the source keeps nodes. `None`
    /// means no container is recorded there; sources that cannot enumerate
    /// keep that answer for every path.
    fn node(&self, path: &[String]) -> Option<Node> {
        let _ = path;
        None
    }

    /// Value at `path` with the failure channel a source can carry
    /// ([`ProviderError`]): `Ok(None)` is absence, `Err(Source)` is a read
    /// failure a fallback chain must propagate rather than skip, and
    /// `Err(Defect)` is the provider dying, which nothing recovers. The default
    /// is the infallible shape every value source has.
    fn try_get(&self, path: &[String]) -> Result<Option<String>, ProviderError> {
        Ok(self.get(path))
    }

    /// Ordered transform pipeline over this source (pipe order: each
    /// step sees the previous step's path).
    fn chain(&self) -> Chain<'_>
    where
        Self: Sized,
    {
        Chain {
            inner: self,
            ops: Vec::new(),
        }
    }

    /// Fall through to `other` on absence (works on any lookup, so
    /// views compose uniformly).
    fn or_else<'a>(&'a self, other: &'a dyn Lookup) -> Fallback<'a>
    where
        Self: Sized,
    {
        Fallback {
            first: self,
            second: other,
        }
    }

    /// Scope lookups under literal `prefix` segments.
    fn nested(&self, prefix: Vec<String>) -> Nested<'_>
    where
        Self: Sized,
    {
        Nested {
            inner: self,
            prefix,
        }
    }

    /// Transform every lookup path before delegating.
    fn mapped<F>(&self, map: F) -> Mapped<'_, F>
    where
        Self: Sized,
        F: Fn(&[String]) -> Vec<String>,
    {
        Mapped { inner: self, map }
    }
}

fn constant_case_path(path: &[String]) -> Vec<String> {
    path.iter().map(|s| constant_case_segment(s)).collect()
}

fn constant_case_segment(segment: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, words: &mut Vec<String>| {
        if !current.is_empty() {
            words.push(std::mem::take(current));
        }
    };
    let chars: Vec<char> = segment.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !c.is_alphanumeric() {
            flush(&mut current, &mut words);
            i += 1;
            continue;
        }
        let boundary = if c.is_uppercase() && !current.is_empty() {
            let prev = current.chars().next_back().unwrap_or('\0');
            let next = chars.get(i + 1).copied();
            prev.is_lowercase()
                || prev.is_numeric()
                || (prev.is_uppercase() && matches!(next, Some(n) if n.is_lowercase()))
        } else {
            false
        };
        if boundary {
            flush(&mut current, &mut words);
        }
        current.push(c);
        i += 1;
    }
    flush(&mut current, &mut words);
    words.join("_").to_uppercase()
}

/// Path-mapping function: whole-path in, whole-path out.
pub type PathMapFn = fn(&[String]) -> Vec<String>;
/// Shared path-mapping function (pipelines).
pub type PathMap = Rc<dyn Fn(&[String]) -> Vec<String>>;

/// Layered key/value lookup for [`Config`](super::Config) descriptors.
/// (Loaded by path; `super::` resolves under both layouts.)
#[derive(Clone, Debug, Default)]
pub struct Provider {
    values: BTreeMap<Vec<String>, String>,
    nodes: BTreeMap<Vec<String>, Node>,
    preserve_empty: bool,
}

impl Provider {
    /// Empty provider.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from `(dotted.path, value)` pairs. Empty values read as
    /// absent at lookup (see `get`) but stay stored, preserving structure.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            preserve_empty: false,
            nodes: BTreeMap::new(),
            values: pairs
                .into_iter()
                .map(|(k, v)| (k.split('.').map(str::to_string).collect(), v))
                .collect(),
        }
    }

    /// Build from `(key, value)` pairs whose keys are literal segments, not
    /// dotted paths. Effect 4's object provider keeps `service.name` as one
    /// key, which [`from_pairs`](Self::from_pairs) cannot: it would split it
    /// into two path segments.
    pub fn record(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self::record_at(Vec::new(), pairs)
    }

    /// [`record`](Self::record) under a base path: `base` is path segments and
    /// each key stays one literal segment under it.
    pub fn record_at(base: Vec<String>, pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            preserve_empty: false,
            nodes: BTreeMap::new(),
            values: pairs
                .into_iter()
                .map(|(k, v)| {
                    let mut path = base.clone();
                    path.push(k);
                    (path, v)
                })
                .collect(),
        }
    }

    /// Record an explicit container at `path` (segments), so the source can
    /// hold `{ a: {} }` or `{ a: [] }`: the path exists although nothing is
    /// stored under it (Config.test.ts:729).
    pub fn with_node(mut self, path: &[&str], node: Node) -> Self {
        self.nodes
            .insert(path.iter().map(|s| s.to_string()).collect(), node);
        self
    }

    /// Build from pairs keeping empty values (`preserveEmptyStrings`
    /// opt-out of the missing-by-default rule).
    pub fn from_pairs_preserving(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            preserve_empty: true,
            nodes: BTreeMap::new(),
            values: pairs
                .into_iter()
                .map(|(k, v)| (k.split('.').map(str::to_string).collect(), v))
                .collect(),
        }
    }

    /// Snapshot the process environment once. Empty values read as absent
    /// (see `get`); stored keys keep structure for listings.
    pub fn from_env() -> Self {
        Self {
            preserve_empty: false,
            nodes: BTreeMap::new(),
            values: std::env::vars().map(|(k, v)| (vec![k], v)).collect(),
        }
    }

    /// One-shot single-path provider (array elements, tests). Empty values
    /// read as absent at lookup (see `get`).
    pub fn single(path: Vec<String>, value: Option<String>) -> Self {
        let mut values = BTreeMap::new();
        if let Some(v) = value {
            values.insert(path, v);
        }
        Self {
            values,
            nodes: BTreeMap::new(),
            preserve_empty: false,
        }
    }

    /// Look up a path (inherent alias of [`Lookup::get`]; kept so plain
    /// `Provider` callers need no trait import).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }

    /// First path segments of every stored key, in order (structure
    /// listing: keys with empty values are included).
    pub fn root_keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for key in self.values.keys() {
            if let Some(first) = key.first()
                && out.last() != Some(first)
            {
                out.push(first.clone());
            }
        }
        out
    }

    /// Scope every lookup under `prefix` (dotted).
    pub fn nested(&self, prefix: &str) -> Nested<'_> {
        Nested {
            inner: self,
            prefix: prefix.split('.').map(str::to_string).collect(),
        }
    }

    /// Fall through to `other` on absence.
    pub fn or_else<'a>(&'a self, other: &'a dyn Lookup) -> Fallback<'a> {
        Fallback {
            first: self,
            second: other,
        }
    }

    /// Transform every lookup path before delegating (`mapInput`).
    pub fn map_input<F>(&self, map: F) -> Mapped<'_, F>
    where
        F: Fn(&[String]) -> Vec<String>,
    {
        Mapped { inner: self, map }
    }

    /// Map lookup paths to CONSTANT_CASE (`constantCase`): segments split
    /// on non-alphanumerics and camel boundaries (numeric groups stay
    /// attached), then join with `_` uppercased.
    pub fn constant_case(&self) -> Mapped<'_, PathMapFn> {
        Mapped {
            inner: self,
            map: constant_case_path,
        }
    }

    /// Build from a lookup function (`make`): `None` reads as absent.
    pub fn from_fn<F>(lookup: F) -> FnProvider<F>
    where
        F: Fn(&[String]) -> Option<String>,
    {
        FnProvider { lookup }
    }
}

impl Lookup for Provider {
    fn get(&self, path: &[String]) -> Option<String> {
        match self.values.get(path) {
            Some(v) if v.is_empty() && !self.preserve_empty => None,
            found => found.cloned(),
        }
    }

    fn has_children(&self, path: &[String]) -> bool {
        self.values
            .keys()
            .any(|key| key.len() > path.len() && key.starts_with(path))
    }

    fn child_keys(&self, path: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for key in self.values.keys() {
            if key.len() == path.len() + 1 && key.starts_with(path) {
                let name = key[path.len()].clone();
                if out.last() != Some(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    fn node(&self, path: &[String]) -> Option<Node> {
        self.nodes.get(path).copied()
    }
}

/// Prefixed view of a lookup source.
pub struct Nested<'a> {
    inner: &'a dyn Lookup,
    prefix: Vec<String>,
}

impl Nested<'_> {
    /// Look up under the prefix (inherent alias of [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }

    /// Compose another prefix outside this one.
    pub fn nested(&self, outer: &str) -> Nested<'_> {
        let mut prefix: Vec<String> = outer.split('.').map(str::to_string).collect();
        prefix.extend(self.prefix.iter().cloned());
        Nested {
            inner: self.inner,
            prefix,
        }
    }
}

/// Two lookup sources tried in order.
pub struct Fallback<'a> {
    first: &'a dyn Lookup,
    second: &'a dyn Lookup,
}

impl Fallback<'_> {
    /// First hit wins (inherent alias of [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }
}

impl Lookup for Nested<'_> {
    fn get(&self, path: &[String]) -> Option<String> {
        let mut full = self.prefix.clone();
        full.extend(path.iter().cloned());
        self.inner.get(&full)
    }

    fn has_children(&self, path: &[String]) -> bool {
        let mut full = self.prefix.clone();
        full.extend(path.iter().cloned());
        self.inner.has_children(&full)
    }

    fn child_keys(&self, path: &[String]) -> Vec<String> {
        let mut full = self.prefix.clone();
        full.extend(path.iter().cloned());
        self.inner.child_keys(&full)
    }

    fn node(&self, path: &[String]) -> Option<Node> {
        let mut full = self.prefix.clone();
        full.extend(path.iter().cloned());
        self.inner.node(&full)
    }
}

impl Lookup for Fallback<'_> {
    fn get(&self, path: &[String]) -> Option<String> {
        self.first.get(path).or_else(|| self.second.get(path))
    }

    fn has_children(&self, path: &[String]) -> bool {
        self.first.has_children(path) || self.second.has_children(path)
    }

    fn child_keys(&self, path: &[String]) -> Vec<String> {
        let mut out = self.first.child_keys(path);
        for key in self.second.child_keys(path) {
            if !out.contains(&key) {
                out.push(key);
            }
        }
        out
    }

    fn node(&self, path: &[String]) -> Option<Node> {
        self.first.node(path).or_else(|| self.second.node(path))
    }

    fn try_get(&self, path: &[String]) -> Result<Option<String>, ProviderError> {
        // Absence falls through; a FAILED source poisons the chain, so the
        // fallback is never consulted — Effect 4's `SourceError` rule, and a
        // defect poisons it the same way.
        match self.first.try_get(path)? {
            Some(hit) => Ok(Some(hit)),
            None => self.second.try_get(path),
        }
    }
}

/// A source that fails every lookup with a `SourceError`-shaped message
/// (`ConfigProvider.make(() => Effect.fail(...))`): the crate's failure
/// channel for providers, which `Fallback` propagates instead of skipping.
pub struct Failing {
    message: String,
}

impl Failing {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Lookup for Failing {
    fn get(&self, _path: &[String]) -> Option<String> {
        None
    }

    fn try_get(&self, _path: &[String]) -> Result<Option<String>, ProviderError> {
        Err(ProviderError::source(self.message.clone()))
    }
}

/// Path-transformed view of a lookup source (`mapInput`): every lookup
/// runs against the mapped path.
pub struct Mapped<'a, F> {
    inner: &'a dyn Lookup,
    map: F,
}

impl<'a, F> Mapped<'a, F>
where
    F: Fn(&[String]) -> Vec<String>,
{
    /// Look up through the mapped path (inherent alias of [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }

    /// Compose another transform applied AFTER this one. Chained
    /// `mapInput` applies in application order (first-added runs first),
    /// which nesting alone would invert, so the two maps fuse into one.
    pub fn then_map<G>(self, next: G) -> Mapped<'a, impl Fn(&[String]) -> Vec<String> + 'a>
    where
        F: 'a,
        G: Fn(&[String]) -> Vec<String> + 'a,
    {
        let Mapped { inner, map } = self;
        Mapped {
            inner,
            map: move |path: &[String]| next(&map(path)),
        }
    }
}

impl<F> Lookup for Mapped<'_, F>
where
    F: Fn(&[String]) -> Vec<String>,
{
    fn get(&self, path: &[String]) -> Option<String> {
        self.inner.get(&(self.map)(path))
    }
}

/// Lookup source backed by a function (`make` with a lookup closure):
/// missing maps to `None`.
pub struct FnProvider<F> {
    lookup: F,
}

impl<F> FnProvider<F>
where
    F: Fn(&[String]) -> Option<String>,
{
    /// Look up through the closure (inherent alias of [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }
}

impl<F> Lookup for FnProvider<F>
where
    F: Fn(&[String]) -> Option<String>,
{
    fn get(&self, path: &[String]) -> Option<String> {
        (self.lookup)(path)
    }
}

/// One path operation inside a [`Chain`]: prepend literal segments, or
/// map the whole path.
pub enum PathOp {
    /// Prepend literal segments (nested prefix, unmapped).
    Prepend(Vec<String>),
    /// Replace the path with the mapped one.
    Map(PathMap),
}

/// Ordered transform pipeline: operations apply in pipe order, then the
/// final path reads from the inner source. Single views apply one
/// operation; chains preserve multi-step order (map-then-prepend differs
/// from prepend-then-map).
pub struct Chain<'a> {
    inner: &'a dyn Lookup,
    ops: Vec<PathOp>,
}

impl<'a> Chain<'a> {
    /// Append a literal prefix (nested).
    pub fn prepended(mut self, prefix: Vec<String>) -> Self {
        self.ops.push(PathOp::Prepend(prefix));
        self
    }

    /// Append a path map.
    pub fn mapped<F>(mut self, map: F) -> Self
    where
        F: Fn(&[String]) -> Vec<String> + 'static,
    {
        self.ops.push(PathOp::Map(Rc::new(map)));
        self
    }

    /// Append the CONSTANT_CASE map.
    pub fn constant_cased(self) -> Self {
        self.mapped(constant_case_path)
    }
}

impl Lookup for Chain<'_> {
    fn get(&self, path: &[String]) -> Option<String> {
        let mut current: Vec<String> = path.to_vec();
        for op in &self.ops {
            match op {
                PathOp::Prepend(prefix) => {
                    let mut next = prefix.clone();
                    next.extend(current);
                    current = next;
                }
                PathOp::Map(map) => {
                    current = map(&current);
                }
            }
        }
        self.inner.get(&current)
    }
}

/// Flat environment-variable source (`fromEnv`): lookups join segments
/// with `_` and match whole variable names exactly (empty reads missing).
/// Injectable form is [`EnvProvider::from_env_map`]; [`Provider::from_env`]
/// snapshots the process environment into a plain [`Provider`].
pub struct EnvProvider {
    values: BTreeMap<String, String>,
    preserve_empty: bool,
}

impl EnvProvider {
    /// Build from `(VARIABLE_NAME, value)` pairs.
    pub fn from_env_map(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: pairs.into_iter().collect(),
            preserve_empty: false,
        }
    }

    /// Build from pairs keeping empty values (`preserveEmptyStrings`).
    pub fn from_env_map_preserving(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: pairs.into_iter().collect(),
            preserve_empty: true,
        }
    }

    /// Top-level key segments, splitting names on `_` (structure
    /// listing: `B_C` surfaces as `B`).
    pub fn root_keys(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut keys: Vec<&str> = self.values.keys().map(|s| s.as_str()).collect();
        keys.sort();
        for key in keys {
            let first = key.split('_').next().unwrap_or("");
            if out.last().map(|s| s.as_str()) != Some(first) {
                out.push(first.to_string());
            }
        }
        out
    }

    /// Parse `.env` text (`KEY=value` lines; blanks, `#` comments, and a
    /// leading `export ` are skipped; surrounding quotes stripped; empty
    /// values read missing; `$VAR` stays literal).
    pub fn from_dotenv(text: &str) -> Self {
        Self::from_dotenv_with(text, false, false)
    }

    /// Parse `.env` keeping empty values (`preserveEmptyStrings`).
    pub fn from_dotenv_preserving(text: &str) -> Self {
        Self::from_dotenv_with(text, false, true)
    }

    /// Parse `.env` expanding `$VAR` and `${VAR}` from earlier entries
    /// (`expandVariables`).
    pub fn from_dotenv_expanding(text: &str) -> Self {
        Self::from_dotenv_with(text, true, false)
    }

    fn from_dotenv_with(text: &str, expand: bool, preserve_empty: bool) -> Self {
        let mut values = BTreeMap::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let body = line.strip_prefix("export ").unwrap_or(line).trim();
            let Some((k, v)) = body.split_once('=') else {
                continue;
            };
            let key = k.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let value = v.trim();
            let decoded =
                if let Some(inner) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    unescape_double(inner)
                } else if let Some(inner) = value
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
                    .or_else(|| value.strip_prefix('`').and_then(|s| s.strip_suffix('`')))
                {
                    inner.to_string()
                } else {
                    strip_inline_comment(value).trim().to_string()
                };
            let decoded = if expand {
                expand_refs(&decoded, &values)
            } else {
                decoded
            };
            values.insert(key, decoded);
        }
        Self {
            values,
            preserve_empty,
        }
    }

    /// Read and parse a `.env` file (`fromDotEnv({ path })`); a missing
    /// or unreadable file is an I/O error, never an empty provider.
    pub fn from_dotenv_file(path: &str) -> Result<Self, std::io::Error> {
        std::fs::read_to_string(path).map(|text| Self::from_dotenv(&text))
    }

    /// Look up by joining segments with `_` (inherent alias of
    /// [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }
}

impl Lookup for EnvProvider {
    fn get(&self, path: &[String]) -> Option<String> {
        match self.values.get(&path.join("_")) {
            Some(v) if v.is_empty() && !self.preserve_empty => None,
            found => found.cloned(),
        }
    }
}

/// Expand `$NAME` and `${NAME}` from already-parsed entries; unknown
/// names stay literal.
fn expand_refs(s: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'{') {
            // Braced `${NAME}` or `${NAME:-fallback}`; an unterminated
            // `${` stays literal. Unknown or empty names expand to empty
            // (missing at lookup unless preserved), and `:-` supplies the
            // default for empty or unset names.
            let rest = &chars[i + 2..];
            match rest.iter().position(|&c| c == '}') {
                None => {
                    out.push('$');
                    i += 1;
                }
                Some(close) => {
                    let inner: String = rest[..close].iter().collect();
                    let (name, default) = match inner.split_once(":-") {
                        Some((n, d)) => (n, Some(d)),
                        None => (inner.as_str(), None),
                    };
                    if name.is_empty() && default.is_none() {
                        out.push('$');
                        i += 1;
                    } else {
                        let current = values.get(name).filter(|v| !v.is_empty());
                        match (current, default) {
                            (Some(v), _) => out.push_str(v),
                            (None, Some(d)) => out.push_str(d),
                            (None, None) => {}
                        }
                        i += 2 + close + 1;
                    }
                }
            }
            continue;
        }
        // Unbraced `$NAME`: unknown names expand to empty (missing at
        // lookup unless preserved); a bare `$` stays literal.
        let mut end = i + 1;
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        let name: String = chars[i + 1..end].iter().collect();
        if name.is_empty() {
            out.push('$');
            i += 1;
        } else {
            if let Some(v) = values.get(&name) {
                out.push_str(v);
            }
            i = end;
        }
    }
    out
}

/// Decode backslash escapes inside double-quoted dotenv values:
/// `\n` `\r` `\t` `\\` `\"` `\'`; anything else keeps the backslash.
fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Cut an unquoted dotenv value at the first ` #` comment marker.
fn strip_inline_comment(s: &str) -> &str {
    match s.find(" #") {
        Some(i) => s[..i].trim_end(),
        None => s,
    }
}

/// Filesystem directory source (`fromDir`, leaf values only): each path
/// segment descends one level; file contents read trimmed. Directories
/// and unreadable files read as absent; empty-after-trim reads missing
/// unless preserving.
pub struct DirProvider {
    root: std::path::PathBuf,
    preserve_empty: bool,
}

impl DirProvider {
    /// Read leaves under `root` (`fromDir`); empty files read missing.
    pub fn from_dir(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root: root.into(),
            preserve_empty: false,
        }
    }

    /// Read leaves under `root`, keeping empty files
    /// (`preserveEmptyStrings`).
    pub fn from_dir_preserving(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root: root.into(),
            preserve_empty: true,
        }
    }

    /// Look up a file under the root (inherent alias of [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
    }
}

impl Lookup for DirProvider {
    fn get(&self, path: &[String]) -> Option<String> {
        let mut full = self.root.clone();
        for seg in path {
            if seg.is_empty() || seg == ".." {
                return None;
            }
            full.push(seg);
        }
        let text = std::fs::read_to_string(&full).ok()?;
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() && !self.preserve_empty {
            None
        } else {
            Some(trimmed)
        }
    }
}
