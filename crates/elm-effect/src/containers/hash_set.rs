//! Persistent `HashSet` port (`HashSet.ts` at the pin).
//!
//! Same rule as [`super::hash_map`]: updates return new sets; the backing
//! table is cloned. Keys use [`Hash`] + [`Eq`].

use super::hash_map::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Persistent set. Updates return new sets.
#[derive(Clone, Debug, Default)]
pub struct HashSet<T> {
    inner: HashMap<T, ()>,
}

impl<T> HashSet<T>
where
    T: Hash + Eq,
{
    /// Empty set.
    pub fn empty() -> Self {
        Self {
            inner: HashMap::empty(),
        }
    }

    /// Set of the given values.
    pub fn make(values: impl IntoIterator<Item = T>) -> Self {
        Self::from_iter(values)
    }

    /// Alias of [`make`](HashSet::make), mirroring `fromIterable`.
    pub fn from_iter(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            inner: HashMap::make(values.into_iter().map(|v| (v, ()))),
        }
    }

    /// `true` when `value` is present.
    pub fn has(&self, value: &T) -> bool {
        self.inner.has(value)
    }

    /// New set with `value` added; `self` is unchanged.
    pub fn add(&self, value: T) -> Self
    where
        T: Clone,
    {
        Self {
            inner: self.inner.set(value, ()),
        }
    }

    /// New set with `value` removed; `self` is unchanged.
    pub fn remove(&self, value: &T) -> Self
    where
        T: Clone,
    {
        Self {
            inner: self.inner.remove(value),
        }
    }

    /// Number of values.
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    /// `true` with no values.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// All values.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.inner.keys()
    }

    /// Union of both sets.
    pub fn union(&self, other: &Self) -> Self
    where
        T: Clone,
    {
        Self {
            inner: self.inner.union(&other.inner),
        }
    }

    /// Values present in both sets.
    pub fn intersection(&self, other: &Self) -> Self
    where
        T: Clone,
    {
        Self {
            inner: self.inner.filter(|k, _| other.has(k)),
        }
    }

    /// Values present here but not in `other`.
    pub fn difference(&self, other: &Self) -> Self
    where
        T: Clone,
    {
        Self {
            inner: self.inner.filter(|k, _| !other.has(k)),
        }
    }

    /// New set with each value mapped.
    pub fn map<U>(&self, f: impl Fn(&T) -> U) -> HashSet<U>
    where
        T: Clone,
        U: Hash + Eq,
    {
        HashSet::from_iter(self.inner.keys().map(f))
    }

    /// New set keeping values that satisfy `f`.
    pub fn filter(&self, f: impl Fn(&T) -> bool) -> Self
    where
        T: Clone,
    {
        Self {
            inner: self.inner.filter(|k, _| f(k)),
        }
    }

    /// `true` when some value satisfies `f`.
    pub fn some(&self, f: impl Fn(&T) -> bool) -> bool {
        self.inner.keys().any(f)
    }

    /// `true` when every value satisfies `f`.
    pub fn every(&self, f: impl Fn(&T) -> bool) -> bool {
        self.inner.keys().all(f)
    }

    /// `true` when every value here is also present in `other`.
    pub fn is_subset(&self, other: &Self) -> bool {
        self.inner.keys().all(|v| other.has(v))
    }

    /// Folds every value into an accumulator.
    pub fn reduce<B>(&self, init: B, f: impl Fn(B, &T) -> B) -> B {
        self.inner.keys().fold(init, f)
    }

    /// Borrows an iterator over the values; order is not promised.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.values()
    }
}

impl<T> PartialEq for HashSet<T>
where
    T: Hash + Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T> Eq for HashSet<T> where T: Hash + Eq {}

impl<T> Hash for HashSet<T>
where
    T: Hash + Eq,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Commutative combine: insertion order never leaks into the hash.
        self.size().hash(state);
        let mut acc: u64 = 0;
        for v in self.inner.keys() {
            let mut h = DefaultHasher::new();
            v.hash(&mut h);
            acc = acc.wrapping_add(h.finish());
        }
        acc.hash(state);
    }
}

impl<'a, T> IntoIterator for &'a HashSet<T>
where
    T: Hash + Eq,
{
    type Item = &'a T;
    type IntoIter = Box<dyn Iterator<Item = &'a T> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.values())
    }
}
