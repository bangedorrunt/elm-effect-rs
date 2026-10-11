//! Persistent `HashMap` port (`HashMap.ts` at the pin).
//!
//! Observable behavior is the spec: every update returns a new map and the
//! old one is unchanged. Structural sharing is an implementation detail; this
//! port clones the backing table (same observable semantics, no new
//! dependency).
//!
//! Deliberate divergence: keys use Rust's [`Hash`] + [`Eq`] (std is the
//! substrate) instead of the tag's `Equal`. `beginMutation` / `endMutation`
//! are residuals (no transient-update path in this port).
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::collections::HashMap as StdMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// `HashMap.mutate` at the pin: apply a batch of in-place updates and return
/// the resulting map, leaving the source unchanged.
///
/// The pin's `beginMutation` returns a structurally shared editable map and
/// `endMutation` freezes it, with `mutate` as the composition. This port has no
/// structural sharing, so [`Mutation`] holds the map and each update replaces
/// it: the observable shape is the same, the source is never modified, and no
/// `unsafe` is needed to hand out `&self` updaters.
impl<K, V> HashMap<K, V>
where
    K: Hash + Eq + Clone,
{
    pub fn mutate<F>(&self, f: F) -> Self
    where
        F: FnOnce(&Mutation<K, V>),
        V: Clone,
    {
        let mutable = Mutation::new(self.clone());
        f(&mutable);
        mutable.into_map()
    }

    /// `HashMap.beginMutation` at the pin: a distinct map the caller can update
    /// in place.
    pub fn begin_mutation(&self) -> Mutation<K, V>
    where
        V: Clone,
    {
        Mutation::new(self.clone())
    }
}

/// A map being mutated in place (the pin's editable `HashMap`).
///
/// The held map is behind a `Mutex` because the pin hands the same mutable
/// value to several calls in one batch and offers the updaters on `&self`.
pub struct Mutation<K, V> {
    map: Mutex<HashMap<K, V>>,
}

impl<K, V> Mutation<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    fn new(map: HashMap<K, V>) -> Self {
        Self {
            map: Mutex::new(map),
        }
    }

    /// The current map (a clone), which is what `into_map` freezes.
    fn snapshot(&self) -> HashMap<K, V> {
        self.map.lock().unwrap().clone()
    }

    /// `HashMap.set` on the mutable map: replaces the held map.
    pub fn set(&self, key: K, value: V) {
        let updated = self.snapshot().set(key, value);
        *self.map.lock().unwrap() = updated;
    }

    /// `HashMap.remove` on the mutable map: replaces the held map.
    pub fn remove(&self, key: &K) {
        let updated = self.snapshot().remove(key);
        *self.map.lock().unwrap() = updated;
    }

    /// The map's size.
    pub fn size(&self) -> usize {
        self.map.lock().unwrap().size()
    }

    /// `HashMap.endMutation` at the pin: freeze and hand back the map.
    pub fn into_map(self) -> HashMap<K, V> {
        self.map.into_inner().unwrap()
    }
}

/// Persistent string-keyed-or-any-key map. Updates return new maps.
#[derive(Clone, Debug, Default)]
pub struct HashMap<K, V> {
    inner: StdMap<K, V>,
}

impl<K, V> HashMap<K, V>
where
    K: Hash + Eq,
{
    /// Empty map.
    pub fn empty() -> Self {
        Self {
            inner: StdMap::new(),
        }
    }

    /// Map of the given entries.
    pub fn make(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        Self {
            inner: entries.into_iter().collect(),
        }
    }

    /// Alias of [`make`](HashMap::make), mirroring `fromIterable`.
    pub fn from_iter(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        Self::make(entries)
    }

    /// Value for `key`, if present.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.inner.get(key)
    }

    /// Value for `key`, panicking with a tagged message when absent.
    pub fn get_unsafe(&self, key: &K) -> &V {
        self.get(key)
            .unwrap_or_else(|| panic!("HashMap.getUnsafe: key not found"))
    }

    /// `true` when `key` is present.
    pub fn has(&self, key: &K) -> bool {
        self.inner.contains_key(key)
    }

    /// New map with `key` set to `value`; `self` is unchanged.
    pub fn set(&self, key: K, value: V) -> Self
    where
        K: Clone,
        V: Clone,
    {
        let mut next = self.inner.clone();
        next.insert(key, value);
        Self { inner: next }
    }

    /// New map with `key` removed; `self` is unchanged.
    pub fn remove(&self, key: &K) -> Self
    where
        K: Clone,
        V: Clone,
    {
        let mut next = self.inner.clone();
        next.remove(key);
        Self { inner: next }
    }

    /// New map with each key removed.
    pub fn remove_many(&self, keys: impl IntoIterator<Item = K>) -> Self
    where
        K: Clone,
        V: Clone,
    {
        let mut next = self.inner.clone();
        for key in keys {
            next.remove(&key);
        }
        Self { inner: next }
    }

    /// New map with each entry set.
    pub fn set_many(&self, entries: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Clone,
        V: Clone,
    {
        let mut next = self.inner.clone();
        next.extend(entries);
        Self { inner: next }
    }

    /// Number of entries.
    pub fn size(&self) -> usize {
        self.inner.len()
    }

    /// `HashMap.modifyAt` at the pin: update the entry at `key` with
    /// `Option<V> -> Option<V>`. `Some` replaces the value, `None` removes an
    /// existing key and is a no-op on a missing one.
    ///
    /// This is the primitive `modify` is expressed through, exactly as the
    /// pin's `internal.modify` calls `modifyAt` with `Option.map(f)`.
    pub fn modify_at(&self, key: K, f: impl FnOnce(Option<&V>) -> Option<V>) -> Self
    where
        K: Hash + Eq + Clone,
        V: Clone,
    {
        let updated = f(self.get(&key));
        match updated {
            None => {
                if self.has(&key) {
                    self.remove(&key)
                } else {
                    self.clone()
                }
            }
            Some(value) => self.set(key, value),
        }
    }

    /// `HashMap.modify` at the pin: map the value at an existing `key`.
    ///
    /// A missing key is left missing, because `f` is applied to the present
    /// value only, which is the pin's `Option.map` over the same update
    /// function.
    pub fn modify(&self, key: K, f: impl FnOnce(&V) -> V) -> Self
    where
        K: Hash + Eq + Clone,
        V: Clone,
    {
        self.modify_at(key, |current| current.map(f))
    }

    /// `HashMap.reduce` at the pin: fold (accumulator, value, key) over every
    /// entry.
    pub fn reduce<Z>(&self, zero: Z, f: impl Fn(Z, &V, &K) -> Z) -> Z {
        let mut result = zero;
        for (key, value) in self.entries() {
            result = f(result, value, key);
        }
        result
    }

    /// `HashMap.forEach` at the pin: visit every entry as `(value, key)`.
    pub fn for_each(&self, mut f: impl FnMut(&V, &K)) {
        for (key, value) in self.entries() {
            f(value, key);
        }
    }

    /// `true` with no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// All keys.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.inner.keys()
    }

    /// All values.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.inner.values()
    }

    /// All entries.
    pub fn entries(&self) -> impl Iterator<Item = (&K, &V)> {
        self.inner.iter()
    }

    /// Union: entries of `other` win on collision.
    pub fn union(&self, other: &Self) -> Self
    where
        K: Clone,
        V: Clone,
    {
        let mut next = self.inner.clone();
        for (k, v) in other.inner.iter() {
            next.insert(k.clone(), v.clone());
        }
        Self { inner: next }
    }

    /// New map with each value mapped.
    pub fn map<W>(&self, f: impl Fn(&V) -> W) -> HashMap<K, W>
    where
        K: Clone,
    {
        HashMap {
            inner: self.inner.iter().map(|(k, v)| (k.clone(), f(v))).collect(),
        }
    }

    /// New map merging one map per entry.
    pub fn flat_map<W>(&self, f: impl Fn(&V, &K) -> HashMap<K, W>) -> HashMap<K, W> {
        let mut out = HashMap {
            inner: StdMap::new(),
        };
        for (key, value) in self.inner.iter() {
            for (key, value) in f(value, key).inner {
                out.inner.insert(key, value);
            }
        }
        out
    }

    /// New map keeping entries that satisfy `f`.
    pub fn filter(&self, f: impl Fn(&K, &V) -> bool) -> Self
    where
        K: Clone,
        V: Clone,
    {
        Self {
            inner: self
                .inner
                .iter()
                .filter(|(k, v)| f(k, v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    /// `true` when some entry satisfies `f`.
    pub fn some(&self, f: impl Fn(&K, &V) -> bool) -> bool {
        self.inner.iter().any(|(k, v)| f(k, v))
    }

    /// `true` when every entry satisfies `f`.
    pub fn every(&self, f: impl Fn(&K, &V) -> bool) -> bool {
        self.inner.iter().all(|(k, v)| f(k, v))
    }

    /// `true` when some entry satisfies `f`.
    pub fn has_by(&self, f: impl Fn(&K, &V) -> bool) -> bool {
        self.some(f)
    }

    /// First entry satisfying `f`, if any.
    pub fn find_first(&self, f: impl Fn(&K, &V) -> bool) -> Option<(&K, &V)> {
        self.inner.iter().find(|(k, v)| f(k, v))
    }

    /// Keep entries whose mapped value is `Some`.
    pub fn filter_map<W>(&self, f: impl Fn(&K, &V) -> Option<W>) -> HashMap<K, W>
    where
        K: Clone,
    {
        HashMap {
            inner: self
                .inner
                .iter()
                .filter_map(|(k, v)| f(k, v).map(|w| (k.clone(), w)))
                .collect(),
        }
    }
}

impl<K, A> HashMap<K, Option<A>>
where
    K: Hash + Eq + Clone,
    A: Clone,
{
    /// Drop entries holding `None`, unwrapping the rest.
    pub fn compact(&self) -> HashMap<K, A> {
        HashMap {
            inner: self
                .inner
                .iter()
                .filter_map(|(k, v)| v.as_ref().map(|a| (k.clone(), a.clone())))
                .collect(),
        }
    }
}

impl<K, V> PartialEq for HashMap<K, V>
where
    K: Hash + Eq,
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<K, V> Eq for HashMap<K, V>
where
    K: Hash + Eq,
    V: Eq,
{
}
