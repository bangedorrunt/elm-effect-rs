//! `Context.ts` portable core: an ordered service map.
//!
//! A [`Context`] maps service tags to values with `Map` semantics:
//! insertion-ordered keys, replacement keeps position, merges are
//! right-biased. All operations consume and return owned values, so the
//! source is never mutated. Keys are tag strings; values are heterogeneous
//! (`Send + Sync + 'static`, shared by [`Arc`](std::sync::Arc)).
//!
//! Out of scope: fiber-cache invalidation, `Redactable` fallbacks,
//! `undefined`-vs-absent distinctions, overlay chains and lazy flattening,
//! reference defaults, structural equality/JSON, and the `Effect` service
//! machinery (those rows stay unported with reasons in the close report).

use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

/// A service tag: a stable key plus the service type it addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tag<T> {
    /// Stable lookup key (`"ContextTest/A"` style).
    pub key: &'static str,
    marker: PhantomData<fn() -> T>,
}

impl<T> Tag<T> {
    /// A tag for `key`.
    pub const fn new(key: &'static str) -> Self {
        Self {
            key,
            marker: PhantomData,
        }
    }
}

/// A service tag with a lazy default (`Context.Reference`): the first
/// resolution computes the default and caches it in the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContextReference<T> {
    /// Stable lookup key.
    pub key: &'static str,
    /// Computes the value on first resolution.
    pub default: fn() -> T,
    marker: PhantomData<fn() -> T>,
}

impl<T> ContextReference<T> {
    /// A reference tag with its default factory.
    pub const fn new(key: &'static str, default: fn() -> T) -> Self {
        Self {
            key,
            default,
            marker: PhantomData,
        }
    }
}

/// An ordered service map.
#[derive(Clone, Debug, Default)]
pub struct Context {
    entries: Vec<(String, Arc<dyn Any + Send + Sync>)>,
}

impl Context {
    /// No services.
    pub fn empty() -> Self {
        Self::default()
    }

    /// One service.
    pub fn make<T: Send + Sync + 'static>(tag: Tag<T>, value: T) -> Self {
        Self::empty().add(tag, value)
    }

    /// Add or replace `tag` (replacement keeps its position; fresh tags
    /// append). Returns a new map; `self` is unchanged.
    pub fn add<T: Send + Sync + 'static>(mut self, tag: Tag<T>, value: T) -> Self {
        let value: Arc<dyn Any + Send + Sync> = Arc::new(value);
        match self.entries.iter_mut().find(|(k, _)| k == tag.key) {
            Some((_, slot)) => *slot = value,
            None => self.entries.push((tag.key.to_string(), value)),
        }
        self
    }

    /// Add `Some(value)`, or remove `tag` for `None` (`addOrOmit`).
    pub fn add_or_omit<T: Send + Sync + 'static>(self, tag: Tag<T>, value: Option<T>) -> Self {
        match value {
            Some(v) => self.add(tag, v),
            None => self.omit(tag),
        }
    }

    /// Remove `tag` when present (otherwise unchanged).
    pub fn omit<T>(mut self, tag: Tag<T>) -> Self {
        self.entries.retain(|(k, _)| k != tag.key);
        self
    }

    /// Read a service (`None` when absent or a different type was stored,
    /// which cannot happen through this API).
    pub fn get<T: 'static>(&self, tag: Tag<T>) -> Option<&T> {
        self.entries
            .iter()
            .find(|(k, _)| k == tag.key)
            .and_then(|(_, v)| v.downcast_ref())
    }

    /// Merge with right bias: `other`'s values win, positions follow first
    /// appearance (left order, then other's fresh keys in order).
    pub fn merge(mut self, other: Self) -> Self {
        for (key, value) in other.entries {
            match self.entries.iter_mut().find(|(k, _)| *k == key) {
                Some((_, slot)) => *slot = value,
                None => self.entries.push((key, value)),
            }
        }
        self
    }

    /// Keep only `tags`, in source order (`pick`).
    pub fn pick(mut self, tags: &[&str]) -> Self {
        self.entries.retain(|(k, _)| tags.contains(&k.as_str()));
        self
    }

    /// Keys in insertion order.
    pub fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|(k, _)| k.as_str()).collect()
    }

    /// Resolve a reference: the stored service when present, otherwise
    /// the default, computed once and cached in the map for later
    /// resolutions (shared by [`Arc`](std::sync::Arc)).
    pub fn resolve<T: Send + Sync + 'static>(&mut self, tag: ContextReference<T>) -> Arc<T> {
        if let Some(hit) = self
            .entries
            .iter()
            .find(|(k, _)| k == tag.key)
            .and_then(|(_, v)| v.clone().downcast::<T>().ok())
        {
            return hit;
        }
        let value: Arc<T> = Arc::new((tag.default)());
        self.entries.push((tag.key.to_string(), value.clone()));
        value
    }
}
