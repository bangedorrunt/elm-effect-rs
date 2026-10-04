//! Weighted consistent-hash ring port (`HashRing.ts` at the pin).
//!
//! Nodes are matched by [`PrimaryKey`](super::primary_key::PrimaryKey)
//! value; adding/removing a node only remaps inputs near its points.
//! Virtual points mirror the tag (`"${key}:${i}"`, `round(weight *
//! base_weight)` points, sorted by hash).
//!
//! Deliberate divergence: point/input hashing uses FNV-1a (deterministic
//! within this port) instead of the tag's string hash, so exact
//! input-to-node assignments differ from TypeScript while the ring
//! properties (stability, weight proportionality, shard caps) hold.

use super::primary_key::PrimaryKey;
use std::collections::{HashMap, HashSet};

/// FNV-1a 64-bit fold of a string. Deterministic across runs.
fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Weighted consistent-hash ring. Mutated in place, like the tag.
#[derive(Clone, Debug)]
pub struct HashRing<A> {
    base_weight: u32,
    total_weight: f64,
    nodes: HashMap<String, (A, f64)>,
    ring: Vec<(u64, String)>,
}

impl<A> HashRing<A>
where
    A: PrimaryKey + Clone,
{
    /// Empty ring. `base_weight` defaults to 128, clamped to at least 1.
    pub fn make(base_weight: Option<u32>) -> Self {
        Self {
            base_weight: base_weight.unwrap_or(128).max(1),
            total_weight: 0.0,
            nodes: HashMap::new(),
            ring: Vec::new(),
        }
    }

    fn add_points(&mut self, keys: &[String], points: u32) {
        for i in (1..=points).rev() {
            for key in keys {
                self.ring
                    .push((hash_str(&format!("{key}:{i}")), key.clone()));
            }
        }
        self.ring.sort_by_key(|(h, _)| *h);
    }

    /// Register nodes at `weight` (default 1, clamped to at least 0.1).
    /// Existing nodes are updated in place.
    pub fn add_many(&mut self, nodes: impl IntoIterator<Item = A>, weight: Option<f64>) {
        let weight = weight.unwrap_or(1.0).max(0.1);
        let mut fresh = Vec::new();
        let mut changed = HashSet::new();
        for node in nodes {
            let key = node.primary_key();
            match self.nodes.get_mut(&key) {
                Some(entry) => {
                    entry.0 = node;
                    if entry.1 != weight {
                        changed.insert(key.clone());
                        self.total_weight -= entry.1;
                        self.total_weight += weight;
                        entry.1 = weight;
                        fresh.push(key);
                    }
                }
                None => {
                    self.nodes.insert(key.clone(), (node, weight));
                    self.total_weight += weight;
                    fresh.push(key);
                }
            }
        }
        // Re-point only new or reweighted nodes.
        if !changed.is_empty() {
            self.ring.retain(|(_, n)| !changed.contains(n));
        }
        // `round(weight * base)` points per key, like the tag.
        let points = (weight * f64::from(self.base_weight)).round() as u32;
        self.add_points(&fresh, points);
    }

    /// Register one node.
    pub fn add(&mut self, node: A, weight: Option<f64>) {
        self.add_many([node], weight);
    }

    /// Remove a node by primary key. No-op when absent.
    pub fn remove(&mut self, node: &A) {
        let key = node.primary_key();
        if let Some((_, w)) = self.nodes.remove(&key) {
            self.ring.retain(|(_, n)| *n != key);
            self.total_weight -= w;
        }
    }

    /// `true` when a node with the same primary key is registered.
    pub fn has(&self, node: &A) -> bool {
        self.nodes.contains_key(&node.primary_key())
    }

    /// Iterate registered nodes.
    pub fn nodes(&self) -> impl Iterator<Item = &A> {
        self.nodes.values().map(|(n, _)| n)
    }

    /// Nearest-point lookup. Mirrors the tag: first point at or past `hash`
    /// (wrapping to the last point), ties and predecessors compared by
    /// absolute distance; `exclude` spirals outward.
    fn index_for(&self, hash: u64, exclude: Option<&HashSet<String>>) -> (usize, u64) {
        let ring = &self.ring;
        let len = ring.len();
        // Lower bound: first point with hash >= input. (Non-empty: len >= 1.)
        let mut lo = 0usize;
        let mut hi = len;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if ring[mid].0 >= hash {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let a = if lo == len { len - 1 } else { lo };
        let dist_a = ring[a].0.abs_diff(hash);
        match exclude {
            None => {
                if lo == 0 {
                    return (a, dist_a);
                }
                let b = lo - 1;
                let dist_b = ring[b].0.abs_diff(hash);
                if dist_a <= dist_b {
                    (a, dist_a)
                } else {
                    (b, dist_b)
                }
            }
            Some(skip) => {
                if !skip.contains(&ring[a].1) {
                    return (a, dist_a);
                }
                let range = lo.max(len - lo);
                for i in 1..=range {
                    if let Some(idx) = lo.checked_sub(i)
                        && idx < len
                        && !skip.contains(&ring[idx].1)
                    {
                        return (idx, ring[idx].0.abs_diff(hash));
                    }
                    let idx = lo + i;
                    if idx < len && !skip.contains(&ring[idx].1) {
                        return (idx, ring[idx].0.abs_diff(hash));
                    }
                }
                (a, dist_a)
            }
        }
    }

    /// Node responsible for `input`. `None` on an empty ring.
    pub fn get(&self, input: &str) -> Option<A> {
        if self.ring.is_empty() {
            return None;
        }
        let (index, _) = self.index_for(hash_str(input), None);
        let key = &self.ring[index].1;
        self.nodes.get(key).map(|(n, _)| n.clone())
    }

    /// Assign `count` shard indexes to nodes with per-node caps
    /// (`max(1, floor(count * weight / total))`), mirroring the tag's
    /// two-pass allocation. `None` on an empty ring.
    pub fn get_shards(&self, count: usize) -> Option<Vec<A>> {
        if self.ring.is_empty() {
            return None;
        }
        let mut shards: Vec<Option<A>> = vec![None; count];
        let mut allocations: HashMap<String, usize> = HashMap::new();
        let mut remaining: HashSet<usize> = (0..count).collect();
        let mut exclude: HashSet<String> = HashSet::new();
        let mut hashes: Vec<Option<u64>> = vec![None; count];

        // First pass: nearest node per shard, skipping capped nodes.
        let mut order: Vec<(usize, String, u64)> = Vec::with_capacity(count);
        for shard in 0..count {
            let h = *hashes[shard].get_or_insert_with(|| hash_str(&format!("shard-{shard}")));
            let (index, dist) = self.index_for(h, None);
            order.push((shard, self.ring[index].1.clone(), dist));
        }
        order.sort_by_key(|(_, _, d)| *d);
        for (shard, key, _) in order {
            if exclude.contains(&key) {
                continue;
            }
            let (node, weight) = &self.nodes[&key];
            shards[shard] = Some(node.clone());
            remaining.remove(&shard);
            let n = allocations.get(&key).copied().unwrap_or(0) + 1;
            allocations.insert(key.clone(), n);
            let max = (count as f64 * (*weight / self.total_weight)).floor() as usize;
            if n >= max.max(1) {
                exclude.insert(key);
            }
        }

        // Second pass: remaining shards, still skipping capped nodes unless
        // every node is capped.
        let mut all_capped = exclude.len() == self.nodes.len() && !self.nodes.is_empty();
        let rest: Vec<usize> = remaining.iter().copied().collect();
        for shard in rest {
            let h = hashes[shard].expect("hashed in first pass");
            let skip = if all_capped { None } else { Some(&exclude) };
            let (index, _) = self.index_for(h, skip);
            let key = self.ring[index].1.clone();
            let (node, weight) = &self.nodes[&key];
            shards[shard] = Some(node.clone());
            remaining.remove(&shard);
            if !all_capped {
                let n = allocations.get(&key).copied().unwrap_or(0) + 1;
                allocations.insert(key.clone(), n);
                let max = (count as f64 * (*weight / self.total_weight)).floor() as usize;
                if n >= max.max(1) {
                    exclude.insert(key);
                    if exclude.len() == self.nodes.len() {
                        all_capped = true;
                    }
                }
            }
        }

        debug_assert!(remaining.is_empty());
        Some(shards.into_iter().map(|s| s.expect("allocated")).collect())
    }
}
