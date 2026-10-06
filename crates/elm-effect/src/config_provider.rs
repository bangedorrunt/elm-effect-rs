//! `ConfigProvider.ts` port: layered key/value lookup.
//!
//! A [`Provider`] answers dotted paths from an in-memory map. Providers
//! compose: [`Provider::nested`] scopes every lookup under a prefix,
//! [`Provider::or_else`] falls through to a second provider on absence,
//! [`Provider::from_env`] snapshots the process environment once (parsed
//! at startup; domain code never reads it). [`Provider::single`] builds
//! the one-shot provider the array-element path uses internally.

use std::collections::BTreeMap;

/// Key/value lookup backing [`Config`](super::Config) descriptors.
/// Implemented by [`Provider`] and its views so custom sources
/// (lookup functions, path transforms) feed `Config::load` directly.
pub trait Lookup {
    /// Value at `path`, or `None` when absent (empty reads missing
    /// unless the source preserves them).
    fn get(&self, path: &[String]) -> Option<String>;
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
    pub fn constant_case(&self) -> Mapped<'_, fn(&[String]) -> Vec<String>> {
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
