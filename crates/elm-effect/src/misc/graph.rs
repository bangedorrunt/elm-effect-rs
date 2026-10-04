//! `Graph` core port (`Graph.ts` at the pin).
//!
//! Directed and undirected graphs over hashable nodes with edge values:
//! construction, node/edge operations, neighborhoods, BFS/DFS traversal,
//! shortest paths, cycle detection, and connected components.
//!
//! Residuals (full algorithm catalog, out of timebox): snapshots and
//! serialization, equality/hashing suites, mutation lifecycle,
//! set-operation combinators (union/intersection/difference/complement),
//! induced subgraphs, bipartite checks, low-link articulation, flow,
//! spanning forests, path enumeration, and Walker.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

/// Directed or undirected graph with values on nodes and edges.
#[derive(Clone, Debug)]
pub struct Graph<N, E> {
    directed: bool,
    nodes: HashSet<N>,
    edges: HashMap<(N, N), E>,
}

/// Node payload marker: nodes are values themselves.
impl<N, E> Graph<N, E>
where
    N: Hash + Eq + Clone,
{
    /// Empty directed graph.
    pub fn directed() -> Self {
        Self {
            directed: true,
            nodes: HashSet::new(),
            edges: HashMap::new(),
        }
    }

    /// Empty undirected graph (every edge reads both ways).
    pub fn undirected() -> Self {
        Self {
            directed: false,
            nodes: HashSet::new(),
            edges: HashMap::new(),
        }
    }

    /// `true` for a directed graph.
    pub fn is_directed(&self) -> bool {
        self.directed
    }

    /// Add a node; `false` when already present.
    pub fn add_node(&mut self, node: N) -> bool {
        self.nodes.insert(node)
    }

    /// `true` when the node is present.
    pub fn has_node(&self, node: &N) -> bool {
        self.nodes.contains(node)
    }

    /// Remove a node and its incident edges; `false` when absent.
    pub fn remove_node(&mut self, node: &N) -> bool {
        if !self.nodes.remove(node) {
            return false;
        }
        self.edges.retain(|(a, b), _| a != node && b != node);
        true
    }

    /// Add an edge (inserting missing endpoints); overwrites the edge value.
    pub fn add_edge(&mut self, from: N, to: N, value: E) {
        self.nodes.insert(from.clone());
        self.nodes.insert(to.clone());
        self.edges.insert((from, to), value);
    }

    /// `true` when the edge is present (either direction for undirected).
    pub fn has_edge(&self, from: &N, to: &N) -> bool {
        self.edges.contains_key(&(from.clone(), to.clone()))
            || (!self.directed && self.edges.contains_key(&(to.clone(), from.clone())))
    }

    /// Remove an edge; `false` when absent.
    pub fn remove_edge(&mut self, from: &N, to: &N) -> bool {
        let key = (from.clone(), to.clone());
        if self.edges.remove(&key).is_some() {
            return true;
        }
        if !self.directed {
            let rev = (to.clone(), from.clone());
            return self.edges.remove(&rev).is_some();
        }
        false
    }

    /// Edge value, if present.
    pub fn edge_value(&self, from: &N, to: &N) -> Option<&E> {
        let key = (from.clone(), to.clone());
        self.edges.get(&key).or_else(|| {
            if self.directed {
                None
            } else {
                self.edges.get(&(to.clone(), from.clone()))
            }
        })
    }

    /// Out-neighbors (all neighbors for undirected), deduplicated.
    pub fn neighbors(&self, node: &N) -> Vec<N> {
        let mut out = Vec::new();
        for (a, b) in self.edges.keys() {
            if a == node && !out.contains(b) {
                out.push(b.clone());
            } else if !self.directed && b == node && !out.contains(a) {
                out.push(a.clone());
            }
        }
        out
    }

    /// Out-degree (full degree for undirected).
    pub fn degree(&self, node: &N) -> usize {
        self.neighbors(node).len()
    }

    /// Node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Edge count (stored direction only).
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// All nodes.
    pub fn node_list(&self) -> Vec<N> {
        self.nodes.iter().cloned().collect()
    }

    /// Breadth-first visit from `start` (includes `start`).
    pub fn bfs(&self, start: &N) -> Vec<N> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut queue = VecDeque::new();
        if !self.has_node(start) {
            return order;
        }
        seen.insert(start.clone());
        queue.push_back(start.clone());
        while let Some(cur) = queue.pop_front() {
            order.push(cur.clone());
            for next in self.neighbors(&cur) {
                if seen.insert(next.clone()) {
                    queue.push_back(next);
                }
            }
        }
        order
    }

    /// Depth-first visit from `start` (includes `start`).
    pub fn dfs(&self, start: &N) -> Vec<N> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![start.clone()];
        if !self.has_node(start) {
            return order;
        }
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            order.push(cur.clone());
            let mut nexts = self.neighbors(&cur);
            nexts.reverse();
            for next in nexts {
                if !seen.contains(&next) {
                    stack.push(next);
                }
            }
        }
        order
    }

    /// Shortest node path from `start` to `goal` (BFS). `None` when
    /// unreachable.
    pub fn shortest_path(&self, start: &N, goal: &N) -> Option<Vec<N>> {
        if !self.has_node(start) || !self.has_node(goal) {
            return None;
        }
        let mut prev: HashMap<N, N> = HashMap::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        seen.insert(start.clone());
        queue.push_back(start.clone());
        while let Some(cur) = queue.pop_front() {
            if cur == *goal {
                let mut path = vec![cur];
                while let Some(p) = prev.get(path.last().expect("nonempty")) {
                    path.push(p.clone());
                }
                path.reverse();
                return Some(path);
            }
            for next in self.neighbors(&cur) {
                if seen.insert(next.clone()) {
                    prev.insert(next.clone(), cur.clone());
                    queue.push_back(next);
                }
            }
        }
        None
    }

    /// `true` when a directed cycle (any cycle for undirected) exists.
    pub fn has_cycle(&self) -> bool {
        // Colors: absent = white, true = on stack (gray), finished removed.
        let mut on_stack = HashSet::new();
        let mut done = HashSet::new();
        for root in self.nodes.iter() {
            if done.contains(root) {
                continue;
            }
            let mut work: Vec<(N, bool)> = vec![(root.clone(), false)];
            while let Some((cur, exiting)) = work.pop() {
                if exiting {
                    on_stack.remove(&cur);
                    done.insert(cur);
                    continue;
                }
                if !on_stack.insert(cur.clone()) {
                    return true;
                }
                work.push((cur.clone(), true));
                for next in self.neighbors(&cur) {
                    if on_stack.contains(&next) {
                        return true;
                    }
                    if !done.contains(&next) {
                        work.push((next, false));
                    }
                }
            }
        }
        false
    }

    /// Connected components (weakly connected for directed).
    pub fn components(&self) -> Vec<Vec<N>> {
        let mut seen = HashSet::new();
        let mut parts = Vec::new();
        for root in self.nodes.iter() {
            if !seen.insert(root.clone()) {
                continue;
            }
            let mut part = vec![root.clone()];
            let mut queue = VecDeque::new();
            queue.push_back(root.clone());
            while let Some(cur) = queue.pop_front() {
                for next in self.all_adjacent(&cur) {
                    if seen.insert(next.clone()) {
                        part.push(next.clone());
                        queue.push_back(next);
                    }
                }
            }
            parts.push(part);
        }
        parts
    }

    fn all_adjacent(&self, node: &N) -> Vec<N> {
        let mut out = self.neighbors(node);
        if self.directed {
            for (a, b) in self.edges.keys() {
                if b == node && *a != *node && !out.contains(a) {
                    out.push(a.clone());
                }
            }
        }
        out
    }
}
