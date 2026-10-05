//! `ConfigProvider.ts` port: layered key/value lookup.
//!
//! A [`Provider`] answers dotted paths from an in-memory map. Providers
//! compose: [`Provider::nested`] scopes every lookup under a prefix,
//! [`Provider::or_else`] falls through to a second provider on absence,
//! [`Provider::from_env`] snapshots the process environment once (parsed
//! at startup; domain code never reads it). [`Provider::single`] builds
//! the one-shot provider the array-element path uses internally.

use std::collections::BTreeMap;

/// Layered key/value lookup for [`Config`](super::Config) descriptors.
/// (Loaded by path; `super::` resolves under both layouts.)
#[derive(Clone, Debug, Default)]
pub struct Provider {
    values: BTreeMap<Vec<String>, String>,
}

impl Provider {
    /// Empty provider.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from `(dotted.path, value)` pairs. Empty values are absent
    /// (ignored), matching the spec default and `from_env`.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: pairs
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| (k.split('.').map(str::to_string).collect(), v))
                .collect(),
        }
    }

    /// Snapshot the process environment once. Empty values are
    /// absent (ignored), matching the spec default.
    pub fn from_env() -> Self {
        Self {
            values: std::env::vars()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| (vec![k], v))
                .collect(),
        }
    }

    /// One-shot single-path provider (array elements, tests). Empty values
    /// are absent (ignored), matching the spec default.
    pub fn single(path: Vec<String>, value: Option<String>) -> Self {
        let mut values = BTreeMap::new();
        if let Some(v) = value {
            if !v.is_empty() {
                values.insert(path, v);
            }
        }
        Self { values }
    }

    /// Look up a path.
    pub fn get(&self, path: &[String]) -> Option<String> {
        self.values.get(path).cloned()
    }

    /// Scope every lookup under `prefix` (dotted).
    pub fn nested(&self, prefix: &str) -> Nested<'_> {
        Nested {
            inner: self,
            prefix: prefix.split('.').map(str::to_string).collect(),
        }
    }

    /// Fall through to `other` on absence.
    pub fn or_else<'a>(&'a self, other: &'a Provider) -> Fallback<'a> {
        Fallback { first: self, second: other }
    }
}

/// Prefixed view of a provider.
pub struct Nested<'a> {
    inner: &'a Provider,
    prefix: Vec<String>,
}

impl Nested<'_> {
    /// Look up under the prefix.
    pub fn get(&self, path: &[String]) -> Option<String> {
        let mut full = self.prefix.clone();
        full.extend(path.iter().cloned());
        self.inner.get(&full)
    }

    /// Compose another prefix outside this one.
    pub fn nested(&self, outer: &str) -> Nested<'_> {
        let mut prefix: Vec<String> = outer.split('.').map(str::to_string).collect();
        prefix.extend(self.prefix.iter().cloned());
        Nested { inner: self.inner, prefix }
    }
}

/// Two providers tried in order.
pub struct Fallback<'a> {
    first: &'a Provider,
    second: &'a Provider,
}

impl Fallback<'_> {
    /// First hit wins.
    pub fn get(&self, path: &[String]) -> Option<String> {
        self.first.get(path).or_else(|| self.second.get(path))
    }
}
