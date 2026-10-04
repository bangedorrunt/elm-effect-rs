//! Persistent `Trie` port (`Trie.ts` at the pin).
//!
//! Byte-wise prefix tree over string keys. Updates return new tries; the
//! original is unchanged (clone-on-write, same observable rule as the other
//! persistent containers here).

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Node<V> {
    value: Option<V>,
    children: BTreeMap<u8, Node<V>>,
}

/// Persistent string-keyed prefix tree.
#[derive(Clone, Debug)]
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

    /// Entries whose key starts with `prefix`.
    pub fn keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        match self.walk(prefix) {
            None => Vec::new(),
            Some(node) => {
                let mut out = Vec::new();
                Self::collect(node, &mut prefix.as_bytes().to_vec(), &mut out);
                out.into_iter().map(|(k, _)| k).collect()
            }
        }
    }
}
