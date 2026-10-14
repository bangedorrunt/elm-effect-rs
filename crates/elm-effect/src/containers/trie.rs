//! Persistent `Trie` port (`Trie.ts` at the pin).
//!
//! Byte-wise prefix tree over string keys. Updates return new tries; the
//! original is unchanged (clone-on-write, same observable rule as the other
//! persistent containers here).
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Node<V> {
    value: Option<V>,
    children: BTreeMap<u8, Node<V>>,
}

/// Persistent string-keyed prefix tree.
///
/// `Display` and `Debug` are written by hand rather than derived: the pin
/// renders the trie as `Trie([["a",0],["b",1]])`, and the derived `Debug`
/// would print the node tree instead. `Debug` follows
/// [`MutableHashMap`](super::mutable_hash_map::MutableHashMap), which reads its
/// own `inspect` row as the same rendering as `toString`.
#[derive(Clone)]
pub struct Trie<V> {
    root: Node<V>,
    len: usize,
}

impl<V> Trie<V> {
    /// Empty trie.
    pub fn empty() -> Self {
        Self {
            root: Node {
                value: None,
                children: BTreeMap::new(),
            },
            len: 0,
        }
    }

    /// Trie of the given entry pairs.
    pub fn make(entries: impl IntoIterator<Item = (String, V)>) -> Self {
        Self::from_iter(entries)
    }

    /// Alias of [`make`](Trie::make), mirroring `fromIterable`.
    pub fn from_iter(entries: impl IntoIterator<Item = (String, V)>) -> Self {
        let mut trie = Self::empty();
        for (k, v) in entries {
            trie.insert_mut(&k, v);
        }
        trie
    }

    fn walk(&self, key: &str) -> Option<&Node<V>> {
        let mut node = &self.root;
        for b in key.bytes() {
            node = node.children.get(&b)?;
        }
        Some(node)
    }

    /// Value for `key`, if present.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.walk(key)?.value.as_ref()
    }

    /// `true` when `key` is present.
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    fn insert_mut(&mut self, key: &str, value: V) {
        let mut node = &mut self.root;
        for b in key.bytes() {
            node = node.children.entry(b).or_insert_with(|| Node {
                value: None,
                children: BTreeMap::new(),
            });
        }
        if node.value.is_none() {
            self.len += 1;
        }
        node.value = Some(value);
    }

    /// New trie with `key` set; `self` is unchanged.
    pub fn insert(&self, key: &str, value: V) -> Self
    where
        V: Clone,
    {
        let mut next = self.clone();
        next.insert_mut(key, value);
        next
    }

    fn remove_mut(&mut self, key: &str) -> bool {
        fn go<V>(node: &mut Node<V>, bytes: &[u8]) -> bool {
            if bytes.is_empty() {
                return node.value.take().is_some();
            }
            let Some(child) = node.children.get_mut(&bytes[0]) else {
                return false;
            };
            let removed = go(child, &bytes[1..]);
            if removed && child.value.is_none() && child.children.is_empty() {
                node.children.remove(&bytes[0]);
            }
            removed
        }
        let removed = go(&mut self.root, key.as_bytes());
        if removed {
            self.len -= 1;
        }
        removed
    }

    /// New trie with `key` removed; `self` is unchanged.
    pub fn remove(&self, key: &str) -> Self
    where
        V: Clone,
    {
        let mut next = self.clone();
        next.remove_mut(key);
        next
    }

    /// Update the value at `key`; missing keys are left alone.
    pub fn modify(&self, key: &str, f: impl FnOnce(&V) -> V) -> Self
    where
        V: Clone,
    {
        match self.get(key) {
            None => self.clone(),
            Some(current) => self.insert(key, f(current)),
        }
    }

    /// Insert every entry, left to right.
    pub fn insert_many<K, V2>(&self, entries: impl IntoIterator<Item = (K, V2)>) -> Self
    where
        K: AsRef<str>,
        V: Clone,
        V2: Into<V>,
    {
        entries
            .into_iter()
            .fold(self.clone(), |trie: Self, (key, value)| {
                trie.insert(key.as_ref(), value.into())
            })
    }

    /// Remove every key.
    pub fn remove_many<K: AsRef<str>>(&self, keys: impl IntoIterator<Item = K>) -> Self
    where
        V: Clone,
    {
        keys.into_iter()
            .fold(self.clone(), |trie: Self, key| trie.remove(key.as_ref()))
    }

    /// Longest stored key that prefixes `input`, paired with its value.
    pub fn longest_prefix_of(&self, input: &str) -> Option<(String, &V)>
    where
        V: Clone,
    {
        let bytes = input.as_bytes();
        let mut best: Option<(String, &V)> = None;
        for split in (1..=bytes.len()).rev() {
            let candidate = &input[..split];
            if !input.is_char_boundary(split) {
                continue;
            }
            if let Some(value) = self.get(candidate) {
                best = Some((candidate.to_owned(), value));
                break;
            }
        }
        best
    }

    /// Number of keys.
    pub fn size(&self) -> usize {
        self.len
    }

    /// `true` with no keys.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn collect<'a>(node: &'a Node<V>, prefix: &mut Vec<u8>, out: &mut Vec<(String, &'a V)>) {
        if let Some(v) = node.value.as_ref() {
            out.push((String::from_utf8_lossy(prefix).into_owned(), v));
        }
        for (b, child) in node.children.iter() {
            prefix.push(*b);
            Self::collect(child, prefix, out);
            prefix.pop();
        }
    }

    /// All entries in byte order.
    pub fn entries(&self) -> Vec<(String, &V)> {
        let mut out = Vec::with_capacity(self.len);
        Self::collect(&self.root, &mut Vec::new(), &mut out);
        out
    }

    /// All keys in byte order.
    pub fn keys(&self) -> Vec<String> {
        self.entries().into_iter().map(|(k, _)| k).collect()
    }

    /// All values in key byte order.
    pub fn values(&self) -> Vec<&V> {
        self.entries().into_iter().map(|(_, v)| v).collect()
    }

    /// Entries whose key starts with `prefix`, in byte order.
    pub fn entries_with_prefix(&self, prefix: &str) -> Vec<(String, &V)> {
        match self.walk(prefix) {
            None => Vec::new(),
            Some(node) => {
                let mut out = Vec::new();
                Self::collect(node, &mut prefix.as_bytes().to_vec(), &mut out);
                out
            }
        }
    }

    /// Keys whose key starts with `prefix`, in byte order.
    pub fn keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.entries_with_prefix(prefix)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    }

    /// Values whose key starts with `prefix`, in key byte order.
    pub fn values_with_prefix(&self, prefix: &str) -> Vec<&V> {
        self.entries_with_prefix(prefix)
            .into_iter()
            .map(|(_, v)| v)
            .collect()
    }

    /// Value for `key`, panicking when the key is missing.
    pub fn get_unsafe(&self, key: &str) -> &V {
        self.get(key)
            .unwrap_or_else(|| panic!("Trie.getUnsafe: no entry for key {key:?}"))
    }

    /// New trie with each value mapped.
    pub fn map<W>(&self, f: impl Fn(&V) -> W) -> Trie<W> {
        Trie::from_iter(self.entries().into_iter().map(|(k, v)| (k, f(v))))
    }

    /// New trie with each value mapped, with the key in hand.
    ///
    /// The pin's callback is variadic (value, or value and key). A Rust closure
    /// has one signature, so the key-aware shape is its own method rather than
    /// an optional argument.
    pub fn map_with_key<W>(&self, f: impl Fn(&V, &str) -> W) -> Trie<W> {
        Trie::from_iter(self.entries().into_iter().map(|(k, v)| {
            let mapped = f(v, &k);
            (k, mapped)
        }))
    }

    /// New trie keeping the entries that satisfy `f`.
    pub fn filter(&self, f: impl Fn(&str, &V) -> bool) -> Self
    where
        V: Clone,
    {
        Trie::from_iter(
            self.entries()
                .into_iter()
                .filter(|(k, v)| f(k, *v))
                .map(|(k, v)| (k, v.clone())),
        )
    }

    /// New trie keeping the entries `f` maps to `Some`.
    pub fn filter_map<W>(&self, f: impl Fn(&str, &V) -> Option<W>) -> Trie<W> {
        Trie::from_iter(
            self.entries()
                .into_iter()
                .filter_map(|(k, v)| f(&k, v).map(|w| (k, w))),
        )
    }

    /// Fold every entry in key order.
    pub fn reduce<Z>(&self, zero: Z, f: impl Fn(Z, &V, &str) -> Z) -> Z {
        self.entries()
            .into_iter()
            .fold(zero, |acc, (k, v)| f(acc, v, &k))
    }

    /// Visit every entry in key order.
    pub fn for_each(&self, mut f: impl FnMut(&V, &str)) {
        for (k, v) in self.entries() {
            f(v, &k);
        }
    }
}

impl<A: Clone> Trie<Option<A>> {
    /// Drop entries holding `None`, unwrapping the rest.
    pub fn compact(&self) -> Trie<A> {
        Trie::from_iter(
            self.entries()
                .into_iter()
                .filter_map(|(k, v)| v.as_ref().map(|a| (k, a.clone()))),
        )
    }
}

/// `[[key,value],...]` in key order, as the pin renders entries.
fn rendered_entries<V: std::fmt::Debug>(trie: &Trie<V>) -> Vec<String> {
    trie.entries()
        .into_iter()
        .map(|(key, value)| format!("[{key:?},{value:?}]"))
        .collect()
}

impl<V: std::fmt::Debug> Trie<V> {
    /// `toJSON` shape: `{"_id":"Trie","values":[["a",0]]}`.
    pub fn json_shape(&self) -> String {
        format!(
            "{{\"_id\":\"Trie\",\"values\":[{}]}}",
            rendered_entries(self).join(",")
        )
    }
}

impl<V: std::fmt::Debug> std::fmt::Display for Trie<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Trie([{}])", rendered_entries(self).join(","))
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for Trie<V> {
    /// The pin's `inspect` shape: the class rendering, not the node tree.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Trie([{}])", rendered_entries(self).join(","))
    }
}

impl<V: PartialEq> PartialEq for Trie<V> {
    /// Equality is by entries. The size is checked first, so two tries that
    /// hold different numbers of keys are never equal.
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        self.entries()
            .into_iter()
            .all(|(key, value)| other.get(&key) == Some(value))
    }
}
