//! Mutable `HashMap` port (`MutableHashMap.ts` at the pin).
//!
//! Unlike [`super::hash_map`], updates happen in place behind shared
//! ownership. Reads clone values out (`V: Clone`), which keeps borrowing
//! simple at behavior-test granularity.

use std::cell::RefCell;
use std::collections::HashMap as StdMap;
use std::hash::Hash;
use std::rc::Rc;

/// In-place map behind shared ownership.
#[derive(Clone, Debug, Default)]
pub struct MutableHashMap<K, V> {
    inner: Rc<RefCell<StdMap<K, V>>>,
}

impl<K, V> MutableHashMap<K, V>
where
    K: Hash + Eq,
{
    /// Empty map.
    pub fn empty() -> Self {
        Self {
            inner: Rc::new(RefCell::new(StdMap::new())),
        }
    }

    /// Map of the given entries.
    pub fn make(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(entries.into_iter().collect())),
        }
    }

    /// Cloned value for `key`, if present.
    pub fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        self.inner.borrow().get(key).cloned()
    }

    /// `true` when `key` is present.
    pub fn has(&self, key: &K) -> bool {
        self.inner.borrow().contains_key(key)
    }

    /// Set `key` in place.
    pub fn set(&self, key: K, value: V) {
        self.inner.borrow_mut().insert(key, value);
    }

    /// Remove `key` in place, returning the old cloned value.
    pub fn remove(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        self.inner.borrow_mut().remove(key)
    }

    /// Number of entries.
    pub fn size(&self) -> usize {
        self.inner.borrow().len()
    }

    /// `true` with no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Snapshot of all keys.
    pub fn keys(&self) -> Vec<K>
    where
        K: Clone,
    {
        self.inner.borrow().keys().cloned().collect()
    }

    /// Snapshot of all values.
    pub fn values(&self) -> Vec<V>
    where
        V: Clone,
    {
        self.inner.borrow().values().cloned().collect()
    }
}
