//! `ConfigProvider.ts` port: layered key/value lookup.
//!
//! A [`Provider`] answers dotted paths from an in-memory map. Providers
//! compose: [`Provider::nested`] scopes every lookup under a prefix,
//! [`Provider::or_else`] falls through to a second provider on absence,
//! [`Provider::from_env`] snapshots the process environment once (parsed
//! at startup; domain code never reads it). [`Provider::single`] builds
//! the one-shot provider the array-element path uses internally.

use std::collections::BTreeMap;
use std::rc::Rc;

/// Key/value lookup backing [`Config`](super::Config) descriptors.
/// Implemented by [`Provider`] and its views so custom sources
/// (lookup functions, path transforms) feed `Config::load` directly.
pub trait Lookup {
    /// Value at `path`, or `None` when absent (empty reads missing
    /// unless the source preserves them).
    fn get(&self, path: &[String]) -> Option<String>;

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
            values: pairs
                .into_iter()
                .map(|(k, v)| (k.split('.').map(str::to_string).collect(), v))
                .collect(),
        }
    }

    /// Build from pairs keeping empty values (`preserveEmptyStrings`
    /// opt-out of the missing-by-default rule).
    pub fn from_pairs_preserving(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            preserve_empty: true,
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
        Fallback { first: self, second: other }
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
        Nested { inner: self.inner, prefix }
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
}

impl Lookup for Fallback<'_> {
    fn get(&self, path: &[String]) -> Option<String> {
        self.first.get(path).or_else(|| self.second.get(path))
    }
}

/// Path-transformed view of a lookup source (`mapInput`): every lookup
/// runs against the mapped path.
pub struct Mapped<'a, F> {
    inner: &'a dyn Lookup,
    map: F,
}

impl<F> Mapped<'_, F>
where
    F: Fn(&[String]) -> Vec<String>,
{
    /// Look up through the mapped path (inherent alias of [`Lookup::get`]).
    pub fn get(&self, path: &[String]) -> Option<String> {
        Lookup::get(self, path)
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
}

impl EnvProvider {
    /// Build from `(VARIABLE_NAME, value)` pairs.
    pub fn from_env_map(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: pairs.into_iter().collect(),
        }
    }

    /// Parse `.env` text (`KEY=value` lines; blanks, `#` comments, and a
    /// leading `export ` are skipped; surrounding quotes stripped).
    pub fn from_dotenv(text: &str) -> Self {
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
            let unquoted = value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| {
                    value
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                })
                .unwrap_or(value);
            values.insert(key, unquoted.to_string());
        }
        Self { values }
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
            Some(v) if !v.is_empty() => Some(v.clone()),
            _ => None,
        }
    }
}
