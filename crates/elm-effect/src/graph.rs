//! `Graph.ts` behavior port: persistent directed/undirected graphs with
//! stable indexes, scoped mutation, queries, traversals, paths, cycles,
//! connectivity, spanning forests, bipartite matching, flow, and
//! GraphViz/Mermaid serialization.
//!
//! The immutable [`Graph`] is ordinary cloned data. [`MutableGraph`] is a
//! shared handle (`Rc<RefCell<..>>`): retained handles see finalization,
//! and a depth guard rejects nested mutation inside transformation
//! callbacks, so every lifecycle error row ports exactly.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::rc::Rc;

/// A node identifier. Stable across removals.
pub type NodeIndex = usize;
/// An edge identifier. Stable across removals.
pub type EdgeIndex = usize;

/// Directed or undirected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Edges have orientation.
    Directed,
    /// Edges ignore orientation.
    Undirected,
}

impl Kind {
    /// Parse `"directed"` / `"undirected"`; anything else is a [`GraphError`].
    pub fn parse(kind: &str) -> Result<Self, GraphError> {
        match kind {
            "directed" => Ok(Self::Directed),
            "undirected" => Ok(Self::Undirected),
            _ => Err(GraphError("Graph type must be directed or undirected".into())),
        }
    }

    /// The spec kind string.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Directed => "directed",
            Self::Undirected => "undirected",
        }
    }
}

/// A graph failure: invalid kinds, dead handles, missing nodes, kind
/// mismatches, bad traversal radii, flow/path validation. Ordinary data;
/// [`Display`](std::fmt::Display) renders the spec message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphError(pub String);

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GraphError {}

/// One stored edge: endpoints plus payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StoredEdge<E> {
    /// Source endpoint.
    pub source: NodeIndex,
    /// Target endpoint.
    pub target: NodeIndex,
    /// Edge payload.
    pub data: E,
}

/// An immutable graph: node/edge payloads keyed by stable indexes.
/// Iteration and snapshots visit nodes and edges in index order.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph<N, E> {
    /// Directed or undirected.
    pub kind: Kind,
    /// Live nodes by stable index.
    pub nodes: BTreeMap<NodeIndex, N>,
    /// Live edges by stable index.
    pub edges: BTreeMap<EdgeIndex, StoredEdge<E>>,
    next_node: NodeIndex,
    next_edge: EdgeIndex,
}

impl<N, E> Graph<N, E> {
    /// Empty graph of a parsed kind string.
    pub fn make(kind: &str) -> Result<Self, GraphError> {
        Ok(Self { kind: Kind::parse(kind)?, nodes: BTreeMap::new(), edges: BTreeMap::new(), next_node: 0, next_edge: 0 })
    }

    /// Empty directed graph.
    pub fn directed() -> Self {
        Self { kind: Kind::Directed, nodes: BTreeMap::new(), edges: BTreeMap::new(), next_node: 0, next_edge: 0 }
    }

    /// Empty undirected graph.
    pub fn undirected() -> Self {
        Self { kind: Kind::Undirected, nodes: BTreeMap::new(), edges: BTreeMap::new(), next_node: 0, next_edge: 0 }
    }

    /// Always false: this handle is immutable.
    pub fn is_mutable(&self) -> bool {
        false
    }

    /// Node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Edge count.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Nodes in index order.
    pub fn node_vec(&self) -> Vec<(NodeIndex, &N)> {
        self.nodes.iter().map(|(i, n)| (*i, n)).collect()
    }

    /// Rebuild from a snapshot, preserving sparse indexes. New identifiers
    /// continue after the greatest active index. Index lists must be
    /// strictly increasing, and every endpoint must reference a live node.
    pub fn from_snapshot(snap: Snapshot<N, E>) -> Result<Self, GraphError> {
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        let mut next_node = 0;
        let mut next_edge = 0;
        let mut last_node: Option<NodeIndex> = None;
        for n in snap.nodes {
            if last_node.is_some_and(|last| n.index <= last) {
                return Err(GraphError("Node indexes must be strictly increasing".into()));
            }
            last_node = Some(n.index);
            next_node = next_node.max(n.index + 1);
            nodes.insert(n.index, n.data);
        }
        let mut last_edge: Option<EdgeIndex> = None;
        for e in snap.edges {
            if last_edge.is_some_and(|last| e.index <= last) {
                return Err(GraphError("Edge indexes must be strictly increasing".into()));
            }
            last_edge = Some(e.index);
            if !nodes.contains_key(&e.source) || !nodes.contains_key(&e.target) {
                return Err(GraphError(format!("Node {} does not exist", if !nodes.contains_key(&e.source) { e.source } else { e.target })));
            }
            next_edge = next_edge.max(e.index + 1);
            edges.insert(e.index, StoredEdge { source: e.source, target: e.target, data: e.data });
        }
        Ok(Self { kind: snap.kind, nodes, edges, next_node, next_edge })
    }

    /// Snapshot with active nodes/edges in index order (fresh clones, never
    /// stored records).
    pub fn to_snapshot(&self) -> Snapshot<N, E>
    where
        N: Clone,
        E: Clone,
    {
        Snapshot {
            kind: self.kind,
            nodes: self.nodes.iter().map(|(i, n)| IndexedNode { index: *i, data: n.clone() }).collect(),
            edges: self
                .edges
                .iter()
                .map(|(i, e)| IndexedEdge { index: *i, source: e.source, target: e.target, data: e.data.clone() })
                .collect(),
        }
    }
}

impl<N, E> std::fmt::Display for Graph<N, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Graph({}, {}, {})", self.kind.name(), self.nodes.len(), self.edges.len())
    }
}

impl<N, E> std::hash::Hash for Graph<N, E>
where
    N: std::hash::Hash,
    E: std::hash::Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.kind == Kind::Directed).hash(state);
        self.nodes.len().hash(state);
        self.edges.len().hash(state);
        for (i, n) in &self.nodes {
            i.hash(state);
            n.hash(state);
        }
        for (i, e) in &self.edges {
            i.hash(state);
            e.source.hash(state);
            e.target.hash(state);
            e.data.hash(state);
        }
    }
}

/// A node with its stable index, for snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedNode<N> {
    /// Stable node index.
    pub index: NodeIndex,
    /// Node payload.
    pub data: N,
}

/// An edge with its stable index, for snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedEdge<E> {
    /// Stable edge index.
    pub index: EdgeIndex,
    /// Source endpoint.
    pub source: NodeIndex,
    /// Target endpoint.
    pub target: NodeIndex,
    /// Edge payload.
    pub data: E,
}

/// Active indexed structure: kind plus nodes/edges in index order.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot<N, E> {
    /// Graph kind.
    pub kind: Kind,
    /// Active nodes in index order.
    pub nodes: Vec<IndexedNode<N>>,
    /// Active edges in index order.
    pub edges: Vec<IndexedEdge<E>>,
}

struct MutInner<N, E> {
    graph: Graph<N, E>,
    live: bool,
    depth: usize,
}

/// A live mutation handle. Clones share one finalization: ending any copy
/// retires them all, and a depth guard rejects nested mutation inside
/// transformation callbacks.
#[derive(Clone)]
pub struct MutableGraph<N, E> {
    inner: Rc<RefCell<MutInner<N, E>>>,
}

impl<N, E> MutableGraph<N, E> {
    fn check(&self) -> Result<std::cell::RefMut<'_, MutInner<N, E>>, GraphError> {
        let borrow = self.inner.borrow_mut();
        if !borrow.live {
            return Err(GraphError("Graph is not mutable".into()));
        }
        if borrow.depth > 0 {
            return Err(GraphError("Cannot mutate graph during a transformation".into()));
        }
        Ok(borrow)
    }

    fn transforming<T>(&self, f: impl FnOnce(&mut Graph<N, E>) -> T) -> Result<T, GraphError> {
        {
            let mut borrow = self.inner.borrow_mut();
            if !borrow.live {
                return Err(GraphError("Graph is not mutable".into()));
            }
            if borrow.depth > 0 {
                return Err(GraphError("Cannot mutate graph during a transformation".into()));
            }
            borrow.depth += 1;
        }
        // The guard borrow is released before running `f` so transformation
        // callbacks may query this same handle (reads observe fresh state).
        // Nested mutation is still rejected: every entry point re-checks
        // `depth` first. Single-threaded scope; no guard is held here, so no
        // RefCell double-borrow can fire.
        let graph_ptr: *mut Graph<N, E> = unsafe { &mut (*self.inner.as_ptr()).graph };
        // SAFETY: `graph_ptr` addresses live `Rc` contents owned by `self`;
        // `self` outlives `f`, and nested access re-borrows (never aliasing
        // this `&mut`, which callbacks cannot retain).
        let graph_ptr: *mut Graph<N, E> = unsafe { &mut (*self.inner.as_ptr()).graph };
        let out = f(unsafe { &mut *graph_ptr });
        self.inner.borrow_mut().depth -= 1;
        Ok(out)
    }

    /// True while no copy has been finalized.
    pub fn is_live(&self) -> bool {
        self.inner.borrow().live
    }

    /// The graph kind.
    pub fn kind(&self) -> Kind {
        self.inner.borrow().graph.kind
    }

    /// Node count.
    pub fn node_count(&self) -> usize {
        self.inner.borrow().graph.nodes.len()
    }

    /// Edge count.
    pub fn edge_count(&self) -> usize {
        self.inner.borrow().graph.edges.len()
    }

    /// Allocate a node, returning its stable index.
    pub fn add_node(&self, data: N) -> Result<NodeIndex, GraphError> {
        let mut borrow = self.check()?;
        let index = borrow.graph.next_node;
        borrow.graph.next_node += 1;
        borrow.graph.nodes.insert(index, data);
        Ok(index)
    }

    /// Allocate an edge, returning its stable index. Missing endpoints are
    /// [`GraphError`] (`Node {i} does not exist`).
    pub fn add_edge(&self, source: NodeIndex, target: NodeIndex, data: E) -> Result<EdgeIndex, GraphError> {
        let mut borrow = self.check()?;
        for endpoint in [source, target] {
            if !borrow.graph.nodes.contains_key(&endpoint) {
                return Err(GraphError(format!("Node {endpoint} does not exist")));
            }
        }
        let index = borrow.graph.next_edge;
        borrow.graph.next_edge += 1;
        borrow.graph.edges.insert(index, StoredEdge { source, target, data });
        Ok(index)
    }

    /// Get a cloned node payload, if present.
    pub fn get_node(&self, index: NodeIndex) -> Option<N>
    where
        N: Clone,
    {
        self.inner.borrow().graph.nodes.get(&index).cloned()
    }

    /// Get a cloned edge record, if present.
    pub fn get_edge(&self, index: EdgeIndex) -> Option<StoredEdge<E>>
    where
        E: Clone,
    {
        self.inner.borrow().graph.edges.get(&index).cloned()
    }

    /// Rewrite one node payload in place; missing indexes are silent no-ops.
    pub fn update_node(&self, index: NodeIndex, f: impl FnOnce(&N) -> N) -> Result<(), GraphError>
    where
        N: Clone,
    {
        self.transforming(|g| {
            if let Some(node) = g.nodes.get(&index).cloned() {
                g.nodes.insert(index, f(&node));
            }
        })
    }

    /// Rewrite one edge payload in place; missing indexes are silent no-ops.
    pub fn update_edge(&self, index: EdgeIndex, f: impl FnOnce(&E) -> E) -> Result<(), GraphError>
    where
        E: Clone,
    {
        self.transforming(|g| {
            if let Some(edge) = g.edges.get(&index).cloned() {
                g.edges.insert(index, StoredEdge { data: f(&edge.data), ..edge });
            }
        })
    }

    /// Remove one node and its incident edges; missing indexes are silent.
    pub fn remove_node(&self, index: NodeIndex) -> Result<(), GraphError> {
        self.transforming(|g| {
            if g.nodes.remove(&index).is_some() {
                g.edges.retain(|_, e| e.source != index && e.target != index);
            }
        })
    }

    /// Remove nodes (collected first, so callers may pass live queries).
    pub fn remove_nodes(&self, indexes: &[NodeIndex]) -> Result<(), GraphError> {
        let owned = indexes.to_vec();
        self.transforming(|g| {
            for index in &owned {
                if g.nodes.remove(index).is_some() {
                    g.edges.retain(|_, e| e.source != *index && e.target != *index);
                }
            }
        })
    }

    /// Remove one edge; missing indexes are silent no-ops.
    pub fn remove_edge(&self, index: EdgeIndex) -> Result<(), GraphError> {
        self.transforming(|g| {
            g.edges.remove(&index);
        })
    }

    /// Remove edges (collected first, so callers may pass live queries).
    pub fn remove_edges(&self, indexes: &[EdgeIndex]) -> Result<(), GraphError> {
        let owned = indexes.to_vec();
        self.transforming(|g| {
            for index in &owned {
                g.edges.remove(index);
            }
        })
    }

    /// Map every node payload, preserving indexes and structure.
    pub fn map_nodes(&self, f: impl Fn(&N) -> N) -> Result<(), GraphError> {
        self.transforming(|g| {
            for node in g.nodes.values_mut() {
                let next = f(node);
                *node = next;
            }
        })
    }

    /// Map every edge payload, preserving indexes and structure.
    pub fn map_edges(&self, f: impl Fn(&E) -> E) -> Result<(), GraphError> {
        self.transforming(|g| {
            for edge in g.edges.values_mut() {
                let next = f(&edge.data);
                edge.data = next;
            }
        })
    }

    /// Keep nodes selected by `f`, dropping the rest with incident edges.
    pub fn filter_map_nodes(&self, f: impl Fn(&N) -> Option<N>) -> Result<(), GraphError> {
        self.transforming(|g| {
            let mut keep: Vec<(NodeIndex, N)> = vec![];
            let mut drop: Vec<NodeIndex> = vec![];
            for (index, node) in g.nodes.iter() {
                match f(node) {
                    Some(next) => keep.push((*index, next)),
                    None => drop.push(*index),
                }
            }
            for (index, next) in keep {
                g.nodes.insert(index, next);
            }
            for index in drop {
                g.nodes.remove(&index);
                g.edges.retain(|_, e| e.source != index && e.target != index);
            }
        })
    }

    /// Keep edges selected by `f`; nodes always survive.
    pub fn filter_map_edges(&self, f: impl Fn(&E) -> Option<E>) -> Result<(), GraphError> {
        self.transforming(|g| {
            let mut keep: Vec<(EdgeIndex, E)> = vec![];
            let mut drop: Vec<EdgeIndex> = vec![];
            for (index, edge) in g.edges.iter() {
                match f(&edge.data) {
                    Some(next) => keep.push((*index, next)),
                    None => drop.push(*index),
                }
            }
            for (index, next) in keep {
                if let Some(slot) = g.edges.get_mut(&index) {
                    slot.data = next;
                }
            }
            for index in drop {
                g.edges.remove(&index);
            }
        })
    }

    /// Keep nodes passing `pred`, dropping the rest with incident edges.
    pub fn filter_nodes(&self, pred: impl Fn(&N) -> bool) -> Result<(), GraphError>
    where
        N: Clone,
    {
        self.filter_map_nodes(|n| pred(n).then(|| n.clone()))
    }

    /// Keep edges passing `pred`; nodes always survive.
    pub fn filter_edges(&self, pred: impl Fn(&E) -> bool) -> Result<(), GraphError>
    where
        E: Clone,
    {
        self.filter_map_edges(|e| pred(e).then(|| e.clone()))
    }

    /// Flip every directed edge; undirected graphs are unchanged.
    pub fn reverse(&self) -> Result<(), GraphError> {
        self.transforming(|g| {
            if g.kind == Kind::Directed {
                for edge in g.edges.values_mut() {
                    std::mem::swap(&mut edge.source, &mut edge.target);
                }
            }
        })
    }

    /// Snapshot with active nodes/edges in index order.
    pub fn to_snapshot(&self) -> Snapshot<N, E>
    where
        N: Clone,
        E: Clone,
    {
        self.inner.borrow().graph.to_snapshot()
    }

    /// Finalize into a new immutable graph, retiring every shared handle.
    pub fn end_mutation(&self) -> Result<Graph<N, E>, GraphError>
    where
        N: Clone,
        E: Clone,
    {
        let mut borrow = self.inner.borrow_mut();
        if !borrow.live {
            return Err(GraphError("Graph is not mutable".into()));
        }
        if borrow.depth > 0 {
            return Err(GraphError("Cannot mutate graph during a transformation".into()));
        }
        borrow.live = false;
        Ok(borrow.graph.clone())
    }
}

/// Read-only view shared by [`Graph`] and [`MutableGraph`]: every query
/// below accepts either handle.
pub trait View<N, E> {
    /// Directed or undirected.
    fn kind(&self) -> Kind;
    /// Nodes in index order (cloned; never stored records).
    fn node_vec(&self) -> Vec<(NodeIndex, N)>
    where
        N: Clone;
    /// Edges in index order (cloned).
    fn edge_vec(&self) -> Vec<(EdgeIndex, StoredEdge<E>)>
    where
        E: Clone;
}

impl<N, E> View<N, E> for Graph<N, E> {
    fn kind(&self) -> Kind {
        self.kind
    }
    fn node_vec(&self) -> Vec<(NodeIndex, N)>
    where
        N: Clone,
    {
        self.node_vec().into_iter().map(|(i, n)| (i, n.clone())).collect()
    }
    fn edge_vec(&self) -> Vec<(EdgeIndex, StoredEdge<E>)>
    where
        E: Clone,
    {
        self.edges.iter().map(|(i, e)| (*i, e.clone())).collect()
    }
}

impl<N, E> View<N, E> for MutableGraph<N, E> {
    fn kind(&self) -> Kind {
        self.kind()
    }
    fn node_vec(&self) -> Vec<(NodeIndex, N)>
    where
        N: Clone,
    {
        self.inner.borrow().graph.node_vec().into_iter().map(|(i, n)| (i, n.clone())).collect()
    }
    fn edge_vec(&self) -> Vec<(EdgeIndex, StoredEdge<E>)>
    where
        E: Clone,
    {
        self.inner.borrow().graph.edges.iter().map(|(i, e)| (*i, e.clone())).collect()
    }
}

/// Start a mutation scope from an immutable graph (cloned: the source is
/// isolated from every later change).
pub fn begin_mutation<N, E>(graph: &Graph<N, E>) -> MutableGraph<N, E>
where
    N: Clone,
    E: Clone,
{
    MutableGraph { inner: Rc::new(RefCell::new(MutInner { graph: graph.clone(), live: true, depth: 0 })) }
}

/// Run a builder to a finalized immutable graph.
pub fn build<N, E>(kind: Kind, f: impl FnOnce(&MutableGraph<N, E>) -> Result<(), GraphError>) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone,
{
    let base = Graph { kind, nodes: BTreeMap::new(), edges: BTreeMap::new(), next_node: 0, next_edge: 0 };
    let mutable = begin_mutation(&base);
    scoped(&mutable, f)
}

/// Run `f` to finalization: the handle retires on return, on error, and on
/// panic (which resumes after retiring, so retained handles stay dead).
fn scoped<N, E>(mutable: &MutableGraph<N, E>, f: impl FnOnce(&MutableGraph<N, E>) -> Result<(), GraphError>) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone,
{
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(mutable)));
    match outcome {
        Ok(Ok(())) => mutable.end_mutation(),
        Ok(Err(e)) => {
            let _ = mutable.end_mutation();
            Err(e)
        }
        Err(panicked) => {
            let mut borrow = mutable.inner.borrow_mut();
            borrow.live = false;
            borrow.depth = 0;
            std::panic::resume_unwind(panicked)
        }
    }
}

/// Run a scoped mutation over an immutable graph to a new immutable graph.
pub fn mutate<N, E>(graph: &Graph<N, E>, f: impl FnOnce(&MutableGraph<N, E>) -> Result<(), GraphError>) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone,
{
    let mutable = begin_mutation(graph);
    scoped(&mutable, f)
}

// ---------------------------------------------------------------------------
// Queries (free functions over any View: Graph or MutableGraph)
// ---------------------------------------------------------------------------

/// Successor targets of `node` (directed), deduped in first-edge order.
pub fn successors<N, E, V>(graph: &V, node: NodeIndex) -> Result<Vec<NodeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot get successors of undirected graph".into()));
    }
    Ok(succ_impl(graph, node))
}

/// Successor targets ignoring kind checks (internal traversals).
fn succ_impl<N, E, V>(graph: &V, node: NodeIndex) -> Vec<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for (id, edge) in edge_list(graph) {
        let _ = id;
        if edge.source == node {
            push_seen(&mut seen, &mut out, edge.target);
        }
    }
    out
}

/// Predecessor sources of `node` (directed), deduped in first-edge order.
pub fn predecessors<N, E, V>(graph: &V, node: NodeIndex) -> Result<Vec<NodeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot get predecessors of undirected graph".into()));
    }
    Ok(pred_impl(graph, node))
}

/// Predecessor sources ignoring kind checks (internal traversals).
fn pred_impl<N, E, V>(graph: &V, node: NodeIndex) -> Vec<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for (_, edge) in edge_list(graph) {
        if edge.target == node {
            push_seen(&mut seen, &mut out, edge.source);
        }
    }
    out
}

fn push_seen(seen: &mut BTreeSet<NodeIndex>, out: &mut Vec<NodeIndex>, node: NodeIndex) {
    if seen.insert(node) {
        out.push(node);
    }
}

/// Both-kind adjacency ignoring kind checks (internal traversals).
fn neighbors_both<N, E, V>(graph: &V, node: NodeIndex) -> Vec<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() == Kind::Undirected {
        adj_impl(graph, node, Direction::Undirected)
    } else {
        succ_impl(graph, node)
    }
}

/// Neighbors of `node`: successors when directed, either side when
/// undirected (self-loops contribute once), deduped in first-edge order.
pub fn neighbors<N, E, V>(graph: &V, node: NodeIndex) -> Vec<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() == Kind::Undirected {
        let mut seen = BTreeSet::new();
        let mut out = vec![];
        for (_, edge) in edge_list(graph) {
            if edge.source == node {
                push_seen(&mut seen, &mut out, edge.target);
            }
        }
        for (_, edge) in edge_list(graph) {
            if edge.target == node {
                push_seen(&mut seen, &mut out, edge.source);
            }
        }
        out
    } else {
        succ_impl(graph, node)
    }
}

/// Directional neighbors: outgoing, incoming, or undirected adjacency.
pub fn neighbors_directed<N, E, V>(graph: &V, node: NodeIndex, dir: Direction) -> Result<Vec<NodeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed && dir != Direction::Undirected {
        return Err(GraphError("Cannot get directed neighbors of undirected graph".into()));
    }
    Ok(match dir {
        Direction::Outgoing => succ_impl(graph, node),
        Direction::Incoming => pred_impl(graph, node),
        Direction::Undirected => {
            let mut seen = BTreeSet::new();
            let mut out = vec![];
            for (_, edge) in edge_list(graph) {
                if edge.source == node {
                    push_seen(&mut seen, &mut out, edge.target);
                }
            }
            for (_, edge) in edge_list(graph) {
                if edge.target == node {
                    push_seen(&mut seen, &mut out, edge.source);
                }
            }
            out
        }
    })
}

/// Directional adjacency ignoring kind checks (internal traversals).
fn adj_impl<N, E, V>(graph: &V, node: NodeIndex, dir: Direction) -> Vec<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    match dir {
        Direction::Outgoing => succ_impl(graph, node),
        Direction::Incoming => pred_impl(graph, node),
        Direction::Undirected => {
            let mut seen = BTreeSet::new();
            let mut out = vec![];
            for (_, edge) in edge_list(graph) {
                if edge.source == node {
                    push_seen(&mut seen, &mut out, edge.target);
                }
            }
            for (_, edge) in edge_list(graph) {
                if edge.target == node {
                    push_seen(&mut seen, &mut out, edge.source);
                }
            }
            out
        }
    }
}

/// Edge direction selector for [`neighbors_directed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Follow stored orientation forward.
    Outgoing,
    /// Follow stored orientation backward.
    Incoming,
    /// Ignore stored orientation.
    Undirected,
}

/// Edge indexes touching `node`, in index order.
pub fn incident_edges<N, E, V>(graph: &V, node: NodeIndex) -> Vec<EdgeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    edge_list(graph)
        .into_iter()
        .filter(|(_, e)| e.source == node || e.target == node)
        .map(|(i, _)| i)
        .collect()
}

/// Outgoing edge indexes of `node`, in index order.
pub fn outgoing_edges<N, E, V>(graph: &V, node: NodeIndex) -> Result<Vec<EdgeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot get outgoing edges of undirected graph".into()));
    }
    Ok(out_impl(graph, node))
}

/// Outgoing edge indexes ignoring kind checks (internal use).
fn out_impl<N, E, V>(graph: &V, node: NodeIndex) -> Vec<EdgeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    edge_list(graph).into_iter().filter(|(_, e)| e.source == node).map(|(i, _)| i).collect()
}

/// Incoming edge indexes of `node`, in index order.
pub fn incoming_edges<N, E, V>(graph: &V, node: NodeIndex) -> Result<Vec<EdgeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot get incoming edges of undirected graph".into()));
    }
    Ok(in_impl(graph, node))
}

/// Incoming edge indexes ignoring kind checks (internal use).
fn in_impl<N, E, V>(graph: &V, node: NodeIndex) -> Vec<EdgeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    edge_list(graph).into_iter().filter(|(_, e)| e.target == node).map(|(i, _)| i).collect()
}

/// Edge indexes from `a` to `b` (either direction when undirected).
pub fn edges_between<N, E, V>(graph: &V, a: NodeIndex, b: NodeIndex) -> Vec<EdgeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let directed = graph.kind() == Kind::Directed;
    edge_list(graph)
        .into_iter()
        .filter(|(_, e)| {
            (e.source == a && e.target == b) || (!directed && e.source == b && e.target == a)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Outgoing edge count (loops count once here).
pub fn out_degree<N, E, V>(graph: &V, node: NodeIndex) -> Result<usize, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    Ok(out_impl(graph, node).len())
}

/// Incoming edge count (loops count once here).
pub fn in_degree<N, E, V>(graph: &V, node: NodeIndex) -> Result<usize, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    Ok(in_impl(graph, node).len())
}

/// Total degree: out plus in (loops count twice).
pub fn degree<N, E, V>(graph: &V, node: NodeIndex) -> Result<usize, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot get degree of directed graph".into()));
    }
    Ok(out_impl(graph, node).len() + in_impl(graph, node).len())
}

/// Cloned node payload, if the index is live.
pub fn get_node<N, E, V>(graph: &V, index: NodeIndex) -> Option<N>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    graph.node_vec().into_iter().find(|(i, _)| *i == index).map(|(_, n)| n)
}

/// Cloned edge record, if the index is live.
pub fn get_edge<N, E, V>(graph: &V, index: EdgeIndex) -> Option<StoredEdge<E>>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    graph.edge_vec().into_iter().find(|(i, _)| *i == index).map(|(_, e)| e)
}

/// True when the node index is live.
pub fn has_node<N, E, V>(graph: &V, node: NodeIndex) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    node_list(graph).iter().any(|(i, _)| *i == node)
}

/// True when an `a` to `b` edge exists (either direction when undirected).
/// Missing nodes report false, never an error.
pub fn has_edge<N, E, V>(graph: &V, a: NodeIndex, b: NodeIndex) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    !edges_between(graph, a, b).is_empty()
}

/// First node index in index order satisfying `pred`.
pub fn find_node<N, E, V>(graph: &V, pred: impl Fn(&N) -> bool) -> Option<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    node_list(graph).into_iter().find(|(_, n)| pred(n)).map(|(i, _)| i)
}

/// Every node index in index order satisfying `pred`.
pub fn find_nodes<N, E, V>(graph: &V, pred: impl Fn(&N) -> bool) -> Vec<NodeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    node_list(graph).into_iter().filter(|(_, n)| pred(n)).map(|(i, _)| i).collect()
}

/// Every edge index satisfying `pred`, in index order.
pub fn find_edges<N, E, V>(graph: &V, mut pred: impl FnMut(&E, NodeIndex, NodeIndex) -> bool) -> Vec<EdgeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    edge_list(graph).into_iter().filter(|(_, e)| pred(&e.data, e.source, e.target)).map(|(i, _)| i).collect()
}

/// First edge index whose stored endpoints and data satisfy `pred`.
pub fn find_edge<N, E, V>(graph: &V, mut pred: impl FnMut(&E, NodeIndex, NodeIndex) -> bool) -> Option<EdgeIndex>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    edge_list(graph).into_iter().find(|(_, e)| pred(&e.data, e.source, e.target)).map(|(i, _)| i)
}

fn node_list<N, E, V>(graph: &V) -> Vec<(NodeIndex, N)>
where
    V: View<N, E>,
    N: Clone,
{
    graph.node_vec()
}

fn edge_list<N, E, V>(graph: &V) -> Vec<(EdgeIndex, StoredEdge<E>)>
where
    V: View<N, E>,
    E: Clone,
{
    graph.edge_vec()
}

// ---------------------------------------------------------------------------
// Set operations, complements, neighborhoods, sums
// ---------------------------------------------------------------------------

fn same_kind(a: Kind, b: Kind) -> Result<Kind, GraphError> {
    if a == b {
        Ok(a)
    } else {
        Err(GraphError("Cannot combine directed and undirected graphs".into()))
    }
}

/// Edge identity under a node projection: endpoints by key plus payload.
/// Undirected pairs normalize by key order.
fn edge_ident<N, E, K>(
    directed: bool,
    nodes: &BTreeMap<NodeIndex, N>,
    e: &StoredEdge<E>,
    node_identity: &dyn Fn(&N) -> K,
) -> Option<(K, K, E)>
where
    N: Clone,
    E: Clone,
    K: PartialEq + PartialOrd,
{
    let a = node_identity(nodes.get(&e.source)?);
    let b = node_identity(nodes.get(&e.target)?);
    let ends = if !directed && b < a { (b, a) } else { (a, b) };
    Some((ends.0, ends.1, e.data.clone()))
}

/// Nodes by projected identity that appear in both graphs (keeping `a`'s
/// payloads), plus edges both graphs share.
pub fn intersection<N, E, K>(
    a: &Graph<N, E>,
    b: &Graph<N, E>,
    node_identity: impl Fn(&N) -> K,
) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone + PartialEq,
    K: PartialEq + PartialOrd,
{
    same_kind(a.kind, b.kind)?;
    let directed = a.kind == Kind::Directed;
    let node_identity = &node_identity;
    let mut out = Graph::directed();
    out.kind = a.kind;
    let mut index_of: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    for (i, n) in &a.nodes {
        let key = node_identity(n);
        if b.nodes.values().any(|m| node_identity(m) == key) {
            let ni = out.next_node;
            out.next_edge = 0;
            out.next_node += 1;
            out.nodes.insert(ni, n.clone());
            index_of.insert(*i, ni);
        }
    }
    let b_keys: Vec<(K, K, E)> = b
        .edges
        .values()
        .filter_map(|e| edge_ident(directed, &b.nodes, e, node_identity))
        .collect();
    for e in a.edges.values() {
        let Some(key) = edge_ident(directed, &a.nodes, e, node_identity) else { continue };
        if !b_keys.iter().any(|k| k.0 == key.0 && k.1 == key.1 && k.2 == key.2) {
            continue;
        }
        let (Some(s), Some(t)) = (index_of.get(&e.source), index_of.get(&e.target)) else { continue };
        let ei = out.next_edge;
        out.next_edge += 1;
        out.edges.insert(ei, StoredEdge { source: *s, target: *t, data: e.data.clone() });
    }
    Ok(out)
}

/// All nodes of `a` plus the edges of `a` that `b` does not share.
pub fn difference<N, E, K>(
    a: &Graph<N, E>,
    b: &Graph<N, E>,
    node_identity: impl Fn(&N) -> K,
) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone + PartialEq,
    K: PartialEq + PartialOrd,
{
    same_kind(a.kind, b.kind)?;
    let directed = a.kind == Kind::Directed;
    let node_identity = &node_identity;
    let mut out = Graph::directed();
    out.kind = a.kind;
    let mut index_of: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    for (i, n) in &a.nodes {
        let ni = out.next_node;
        out.next_node += 1;
        out.nodes.insert(ni, n.clone());
        index_of.insert(*i, ni);
    }
    let b_keys: Vec<(K, K, E)> = b
        .edges
        .values()
        .filter_map(|e| edge_ident(directed, &b.nodes, e, node_identity))
        .collect();
    for e in a.edges.values() {
        let Some(key) = edge_ident(directed, &a.nodes, e, node_identity) else { continue };
        if b_keys.iter().any(|k| k.0 == key.0 && k.1 == key.1 && k.2 == key.2) {
            continue;
        }
        let (Some(s), Some(t)) = (index_of.get(&e.source), index_of.get(&e.target)) else { continue };
        let ei = out.next_edge;
        out.next_edge += 1;
        out.edges.insert(ei, StoredEdge { source: *s, target: *t, data: e.data.clone() });
    }
    Ok(out)
}

/// Union of nodes (right payloads win) with edges in exactly one side.
pub fn symmetric_difference<N, E, K>(
    a: &Graph<N, E>,
    b: &Graph<N, E>,
    node_identity: impl Fn(&N) -> K,
) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone + PartialEq,
    K: PartialEq + PartialOrd + Clone,
{
    same_kind(a.kind, b.kind)?;
    let directed = a.kind == Kind::Directed;
    let node_identity = &node_identity;
    let mut out = Graph::directed();
    out.kind = a.kind;
    let mut index_of_a: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut index_of_b: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut by_key: HashMap<NodeIndex, K> = HashMap::new();
    for (i, n) in &a.nodes {
        let ni = out.next_node;
        out.next_node += 1;
        out.nodes.insert(ni, n.clone());
        by_key.insert(ni, node_identity(n));
        index_of_a.insert(*i, ni);
    }
    for (i, n) in &b.nodes {
        let key = node_identity(n);
        match by_key.iter().find(|(_, k)| **k == key).map(|(ni, _)| *ni) {
            Some(ni) => {
                out.nodes.insert(ni, n.clone());
                index_of_b.insert(*i, ni);
            }
            None => {
                let ni = out.next_node;
                out.next_node += 1;
                out.nodes.insert(ni, n.clone());
                by_key.insert(ni, key);
                index_of_b.insert(*i, ni);
            }
        }
    }
    let keys_a: Vec<(K, K, E)> = a
        .edges
        .values()
        .filter_map(|e| edge_ident(directed, &a.nodes, e, node_identity))
        .collect();
    let keys_b: Vec<(K, K, E)> = b
        .edges
        .values()
        .filter_map(|e| edge_ident(directed, &b.nodes, e, node_identity))
        .collect();
    let mut emit = |e: &StoredEdge<E>, map: &HashMap<NodeIndex, NodeIndex>, out: &mut Graph<N, E>| {
        let (Some(s), Some(t)) = (map.get(&e.source), map.get(&e.target)) else { return };
        let ei = out.next_edge;
        out.next_edge += 1;
        out.edges.insert(ei, StoredEdge { source: *s, target: *t, data: e.data.clone() });
    };
    for e in a.edges.values() {
        let Some(key) = edge_ident(directed, &a.nodes, e, node_identity) else { continue };
        if keys_b.iter().any(|k| k.0 == key.0 && k.1 == key.1 && k.2 == key.2) {
            continue;
        }
        emit(e, &index_of_a, &mut out);
    }
    for e in b.edges.values() {
        let Some(key) = edge_ident(directed, &b.nodes, e, node_identity) else { continue };
        if keys_a.iter().any(|k| k.0 == key.0 && k.1 == key.1 && k.2 == key.2) {
            continue;
        }
        emit(e, &index_of_b, &mut out);
    }
    Ok(out)
}

/// Disjoint union: `b`'s nodes and edges reindexed after `a`'s greatest
/// active index, so equal payloads stay disjoint.
pub fn sum<N, E>(a: &Graph<N, E>, b: &Graph<N, E>) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone,
{
    same_kind(a.kind, b.kind)?;
    let mut out = a.clone();
    let mut remap: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    for (i, n) in &b.nodes {
        let ni = out.next_node;
        out.next_node += 1;
        out.nodes.insert(ni, n.clone());
        remap.insert(*i, ni);
    }
    for e in b.edges.values() {
        let ei = out.next_edge;
        out.next_edge += 1;
        out.edges.insert(
            ei,
            StoredEdge { source: remap[&e.source], target: remap[&e.target], data: e.data.clone() },
        );
    }
    Ok(out)
}

/// Every absent pair as an edge (never self-loops), built by `create`.
/// Directed pairs cover both orientations; undirected pairs list once.
pub fn complement<N, E>(graph: &Graph<N, E>, create: impl Fn(&N, &N) -> E) -> Graph<N, E>
where
    N: Clone,
    E: Clone,
{
    let mut out = Graph {
        kind: graph.kind,
        nodes: graph.nodes.clone(),
        edges: BTreeMap::new(),
        next_node: graph.next_node,
        next_edge: 0,
    };
    let ids: Vec<NodeIndex> = graph.nodes.keys().copied().collect();
    let mut pairs: Vec<(NodeIndex, NodeIndex)> = vec![];
    for (ia, &a) in ids.iter().enumerate() {
        for &b in &ids {
            if a == b {
                continue;
            }
            if graph.kind == Kind::Undirected && ids.iter().position(|x| *x == b).unwrap_or(0) < ia {
                continue;
            }
            pairs.push((a, b));
        }
    }
    for (a, b) in pairs {
        let exists = graph.edges.values().any(|e| {
            (e.source == a && e.target == b)
                || (graph.kind == Kind::Undirected && e.source == b && e.target == a)
        });
        if exists {
            continue;
        }
        let ei = out.next_edge;
        out.next_edge += 1;
        out.edges.insert(
            ei,
            StoredEdge { source: a, target: b, data: create(&graph.nodes[&a], &graph.nodes[&b]) },
        );
    }
    out
}

/// Traversal radius: a non-negative hop count, or infinity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Radius {
    /// At most this many hops.
    Hops(usize),
    /// No limit.
    Infinity,
}

/// Parse a float radius: non-negative integers and infinity only.
pub fn radius_from_f64(value: f64) -> Result<Radius, GraphError> {
    if value.is_infinite() && value > 0.0 {
        Ok(Radius::Infinity)
    } else if value.is_finite() && value >= 0.0 && value.fract() == 0.0 {
        Ok(Radius::Hops(value as usize))
    } else {
        Err(GraphError("Traversal radius must be a non-negative integer or Infinity".into()))
    }
}

/// Nodes within `radius` hops of `node` (BFS order), as a fresh graph with
/// dense indexes. `direction` ignores orientation when [`Direction::Undirected`].
pub fn neighborhood<N, E, V>(graph: &V, node: NodeIndex, radius: f64, direction: Direction) -> Result<Graph<N, E>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if !has_node(graph, node) {
        return Err(GraphError(format!("Node {node} does not exist")));
    }
    let limit = match radius_from_f64(radius)? {
        Radius::Hops(h) => h,
        Radius::Infinity => usize::MAX,
    };
    let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
    let mut queue = VecDeque::from([node]);
    dist.insert(node, 0);
    while let Some(current) = queue.pop_front() {
        let d = dist[&current];
        if d == limit {
            continue;
        }
        for next in adj_impl(graph, current, direction) {
            if !dist.contains_key(&next) {
                dist.insert(next, d + 1);
                queue.push_back(next);
            }
        }
    }
    let mut order: Vec<NodeIndex> = dist.keys().copied().collect();
    order.sort_by_key(|n| (dist[n], *n));
    let mut remap: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut out = Graph { kind: graph.kind(), nodes: BTreeMap::new(), edges: BTreeMap::new(), next_node: 0, next_edge: 0 };
    for (fresh, old) in order.iter().enumerate() {
        remap.insert(*old, fresh);
        out.nodes.insert(fresh, node_data(graph, *old));
        out.next_node += 1;
    }
    for (id, edge) in edge_list(graph) {
        let _ = id;
        if dist.contains_key(&edge.source) && dist.contains_key(&edge.target) {
            let ei = out.next_edge;
            out.next_edge += 1;
            out.edges.insert(
                ei,
                StoredEdge { source: remap[&edge.source], target: remap[&edge.target], data: edge.data.clone() },
            );
        }
    }
    Ok(out)
}

fn node_data<N, E, V>(graph: &V, node: NodeIndex) -> N
where
    V: View<N, E>,
    N: Clone,
{
    graph.node_vec().into_iter().find(|(i, _)| *i == node).map(|(_, n)| n).expect("visited nodes exist")
}

/// Induced subgraph on `indexes`, preserving sparse indexes. Missing nodes
/// are [`GraphError`] (`Node {i} does not exist`).
pub fn induced_subgraph<N, E, V>(graph: &V, indexes: &[NodeIndex]) -> Result<Graph<N, E>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    for index in indexes {
        if !has_node(graph, *index) {
            return Err(GraphError(format!("Node {index} does not exist")));
        }
    }
    let keep: BTreeSet<NodeIndex> = indexes.iter().copied().collect();
    let mut out = Graph {
        kind: graph.kind(),
        nodes: BTreeMap::new(),
        edges: BTreeMap::new(),
        next_node: 0,
        next_edge: 0,
    };
    for (i, n) in graph.node_vec() {
        if keep.contains(&i) {
            out.next_node = out.next_node.max(i + 1);
            out.nodes.insert(i, n);
        }
    }
    for (i, e) in graph.edge_vec() {
        if keep.contains(&e.source) && keep.contains(&e.target) {
            out.next_edge = out.next_edge.max(i + 1);
            out.edges.insert(i, e);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Traversal: Walker, BFS / DFS / postorder, radius, topo
// ---------------------------------------------------------------------------

/// A repeatable walk over items of type `T`: custom-built from a fixed
/// list, or computed live from a graph on every projection (so fresh runs
/// observe later mutation while in-flight vectors stay isolated).
#[derive(Clone)]
pub struct Walker<T> {
    run: std::rc::Rc<dyn Fn() -> Result<Vec<T>, GraphError>>,
}

impl<T> Walker<T>
where
    T: Clone + 'static,
{
    /// Build from a fixed entry list (the Rust spelling of `new Walker`).
    pub fn custom(items: Vec<T>) -> Self {
        Self { run: std::rc::Rc::new(move || Ok(items.clone())) }
    }

    fn run(&self) -> Result<Vec<T>, GraphError> {
        (self.run)()
    }

    /// Length of one full run.
    pub fn len(&self) -> Result<usize, GraphError> {
        Ok(self.run()?.len())
    }

    /// True when one full run yields nothing.
    pub fn is_empty(&self) -> Result<bool, GraphError> {
        Ok(self.run()?.is_empty())
    }
}

impl<N> Walker<(NodeIndex, N)>
where
    N: Clone + 'static,
{
    /// Indexes in walk order.
    pub fn indices(&self) -> Result<Vec<NodeIndex>, GraphError> {
        Ok(self.run()?.into_iter().map(|(i, _)| i).collect())
    }

    /// Values in walk order.
    pub fn values(&self) -> Result<Vec<N>, GraphError> {
        Ok(self.run()?.into_iter().map(|(_, n)| n).collect())
    }

    /// Entries in walk order.
    pub fn entries(&self) -> Result<Vec<(NodeIndex, N)>, GraphError> {
        self.run()
    }

    /// Map entries through `f` (the Rust spelling of `walker.visit`).
    pub fn visit<T>(&self, f: impl Fn(NodeIndex, &N) -> T) -> Result<Vec<T>, GraphError> {
        Ok(self.run()?.iter().map(|(i, n)| f(*i, n)).collect())
    }
}

impl<E> Walker<(EdgeIndex, StoredEdge<E>)>
where
    E: Clone + 'static,
{
    /// Edge indexes in walk order.
    pub fn edge_indices(&self) -> Result<Vec<EdgeIndex>, GraphError> {
        Ok(self.run()?.into_iter().map(|(i, _)| i).collect())
    }

    /// Edge records in walk order.
    pub fn edge_values(&self) -> Result<Vec<StoredEdge<E>>, GraphError> {
        Ok(self.run()?.into_iter().map(|(_, e)| e).collect())
    }

    /// Edge entries in walk order.
    pub fn edge_entries(&self) -> Result<Vec<(EdgeIndex, StoredEdge<E>)>, GraphError> {
        self.run()
    }
}

/// Indexes of a node walk.
pub fn indices<N>(walker: &Walker<(NodeIndex, N)>) -> Result<Vec<NodeIndex>, GraphError>
where
    N: Clone + 'static,
{
    walker.indices()
}

/// Values of a node walk.
pub fn values<N>(walker: &Walker<(NodeIndex, N)>) -> Result<Vec<N>, GraphError>
where
    N: Clone + 'static,
{
    walker.values()
}

/// Entries of a node walk.
pub fn entries<N>(walker: &Walker<(NodeIndex, N)>) -> Result<Vec<(NodeIndex, N)>, GraphError>
where
    N: Clone + 'static,
{
    walker.entries()
}

/// Edge records of an edge walk.
pub fn edge_values<E>(walker: &Walker<(EdgeIndex, StoredEdge<E>)>) -> Result<Vec<StoredEdge<E>>, GraphError>
where
    E: Clone + 'static,
{
    walker.edge_values()
}

/// A live edge walk over a graph: re-resolved per projection.
pub fn edges<N, E, V>(graph: &V) -> Walker<(EdgeIndex, StoredEdge<E>)>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    let view = graph.clone();
    Walker { run: std::rc::Rc::new(move || Ok(view.edge_vec())) }
}

/// A live node walk over a graph in index order.
pub fn nodes<N, E, V>(graph: &V) -> Walker<(NodeIndex, N)>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    let view = graph.clone();
    Walker { run: std::rc::Rc::new(move || Ok(view.node_vec())) }
}

fn check_starts<N, E, V>(graph: &V, starts: &[NodeIndex]) -> Result<(), GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    for start in starts {
        if !has_node(graph, *start) {
            return Err(GraphError(format!("Node {start} does not exist")));
        }
    }
    Ok(())
}

/// Breadth-first walk: `starts` (graph order when empty), `radius` as a
/// float (non-negative integers and infinity; anything else errors here),
/// following `direction`. Within a level, neighbors visit in edge order.
/// Starts and radius validate eagerly; the walk runs live per projection.
pub fn bfs<N, E, V>(graph: &V, starts: &[NodeIndex], radius: f64, direction: Direction) -> Result<Walker<(NodeIndex, N)>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    check_starts(graph, starts)?;
    let limit = radius_from_f64(radius)?;
    let seeds: Vec<NodeIndex> = starts.to_vec();
    let view = graph.clone();
    Ok(Walker {
        run: std::rc::Rc::new(move || {
            let full = bfs_run(&view, &seeds, direction)?;
            Ok(filter_radius(&view, &full, &seeds, limit, direction))
        }),
    })
}

fn bfs_run<N, E, V>(graph: &V, seeds: &[NodeIndex], direction: Direction) -> Result<Vec<(NodeIndex, N)>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    check_starts(graph, seeds)?;
    if seeds.is_empty() {
        return Ok(vec![]);
    }
    let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut order: Vec<(NodeIndex, N)> = vec![];
    let mut queue: VecDeque<NodeIndex> = seeds.to_vec().into();
    while let Some(current) = queue.pop_front() {
        if !seen.insert(current) {
            continue;
        }
        order.push((current, node_data(graph, current)));
        for next in adj_impl(graph, current, direction) {
            if !seen.contains(&next) {
                queue.push_back(next);
            }
        }
    }
    Ok(order)
}

/// Shortest-hop membership filter: keep nodes within `limit` hops of a
/// seed (BFS distance), preserving walk order.
fn filter_radius<N, E, V>(
    graph: &V,
    walk: &[(NodeIndex, N)],
    seeds: &[NodeIndex],
    limit: Radius,
    direction: Direction,
) -> Vec<(NodeIndex, N)>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let bound = match limit {
        Radius::Hops(h) => h,
        Radius::Infinity => usize::MAX,
    };
    let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
    let mut queue = VecDeque::new();
    let start_ids: Vec<NodeIndex> = if seeds.is_empty() {
        graph.node_vec().into_iter().map(|(i, _)| i).collect()
    } else {
        seeds.to_vec()
    };
    for seed in start_ids {
        dist.insert(seed, 0);
        queue.push_back(seed);
    }
    while let Some(current) = queue.pop_front() {
        let d = dist[&current];
        if d == bound {
            continue;
        }
        for next in adj_impl(graph, current, direction) {
            if !dist.contains_key(&next) {
                dist.insert(next, d + 1);
                queue.push_back(next);
            }
        }
    }
    walk.iter().filter(|(i, _)| dist.get(i).is_some_and(|d| *d <= bound)).cloned().collect()
}

/// Depth-first walk (neighbors in edge order, distinct roots prioritized
/// in the order given). Radius filters membership by shortest distance.
pub fn dfs<N, E, V>(graph: &V, starts: &[NodeIndex], radius: f64, direction: Direction) -> Result<Walker<(NodeIndex, N)>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    check_starts(graph, starts)?;
    let limit = radius_from_f64(radius)?;
    let seeds: Vec<NodeIndex> = starts.to_vec();
    let view = graph.clone();
    Ok(Walker {
        run: std::rc::Rc::new(move || {
            let full = dfs_run(&view, &seeds, direction)?;
            Ok(filter_radius(&view, &full, &seeds, limit, direction))
        }),
    })
}

fn dfs_run<N, E, V>(graph: &V, seeds: &[NodeIndex], direction: Direction) -> Result<Vec<(NodeIndex, N)>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    check_starts(graph, seeds)?;
    if seeds.is_empty() {
        return Ok(vec![]);
    }
    let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut order: Vec<(NodeIndex, N)> = vec![];
    for seed in seeds.to_vec() {
        let mut stack = vec![seed];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            order.push((current, node_data(graph, current)));
            let mut nexts = adj_impl(graph, current, direction);
            nexts.reverse();
            for next in nexts {
                if !seen.contains(&next) {
                    stack.push(next);
                }
            }
        }
    }
    Ok(order)
}

/// Postorder walk: a node lands after its successors. Radius filters
/// membership by shortest distance.
pub fn dfs_post_order<N, E, V>(graph: &V, starts: &[NodeIndex], radius: f64, direction: Direction) -> Result<Walker<(NodeIndex, N)>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    check_starts(graph, starts)?;
    let limit = radius_from_f64(radius)?;
    let seeds: Vec<NodeIndex> = starts.to_vec();
    let view = graph.clone();
    Ok(Walker {
        run: std::rc::Rc::new(move || {
            let full = postorder_run(&view, &seeds, direction)?;
            Ok(filter_radius(&view, &full, &seeds, limit, direction))
        }),
    })
}

fn postorder_run<N, E, V>(graph: &V, seeds: &[NodeIndex], direction: Direction) -> Result<Vec<(NodeIndex, N)>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    check_starts(graph, seeds)?;
    if seeds.is_empty() {
        return Ok(vec![]);
    }
    let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut order: Vec<(NodeIndex, N)> = vec![];
    for seed in seeds.to_vec() {
        let mut stack: Vec<(NodeIndex, bool)> = vec![(seed, false)];
        while let Some((current, expanded)) = stack.pop() {
            if expanded {
                order.push((current, node_data(graph, current)));
                continue;
            }
            if !seen.insert(current) {
                continue;
            }
            stack.push((current, true));
            let mut nexts = adj_impl(graph, current, direction);
            nexts.reverse();
            for next in nexts {
                if !seen.contains(&next) {
                    stack.push((next, false));
                }
            }
        }
    }
    Ok(order)
}

/// Nodes within `radius` hops of any start, in BFS order.
pub fn within_radius<N, E, V>(graph: &V, starts: &[NodeIndex], radius: f64, direction: Direction) -> Result<Walker<(NodeIndex, N)>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    bfs(graph, starts, radius, direction)
}

/// Topological order (Kahn, lowest index first, `initials` prioritized).
/// Missing initials error eagerly; cycles and non-source initials error
/// per projection, so fresh iterations track live mutation.
pub fn topo<N, E, V>(graph: &V, initials: &[NodeIndex]) -> Result<Walker<(NodeIndex, N)>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    for initial in initials {
        if !has_node(graph, *initial) {
            return Err(GraphError(format!("Node {initial} does not exist")));
        }
    }
    let seeds: Vec<NodeIndex> = initials.to_vec();
    let view = graph.clone();
    Ok(Walker {
        run: std::rc::Rc::new(move || topo_run(&view, &seeds)),
    })
}

fn topo_run<N, E, V>(graph: &V, initials: &[NodeIndex]) -> Result<Vec<(NodeIndex, N)>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot perform topological sort on undirected graph".into()));
    }
    for initial in initials {
        let incoming = in_impl(graph, *initial);
        if !incoming.is_empty() {
            return Err(GraphError(format!("Initial node {initial} has incoming edges")));
        }
    }
    let mut indeg: HashMap<NodeIndex, usize> = HashMap::new();
    for (i, _) in graph.node_vec() {
        indeg.insert(i, 0);
    }
    for (_, e) in graph.edge_vec() {
        *indeg.entry(e.target).or_insert(0) += 1;
    }
    // FIFO with initials first: zero-indegree initials (given order), then
    // zero-indegree rest (ascending); newly freed nodes append in edge order.
    let mut inits: Vec<NodeIndex> = vec![];
    let mut rest: Vec<NodeIndex> = vec![];
    for (node, degree) in &indeg {
        if *degree == 0 {
            if initials.contains(node) {
                inits.push(*node);
            } else {
                rest.push(*node);
            }
        }
    }
    inits.sort();
    rest.sort();
    // Initials keep their given order among themselves.
    let mut ordered_inits: Vec<NodeIndex> = initials.iter().filter(|n| inits.contains(n)).copied().collect();
    ordered_inits.extend(rest);
    let mut queue: VecDeque<NodeIndex> = ordered_inits.into();
    let mut queued: BTreeSet<NodeIndex> = queue.iter().copied().collect();
    let mut order: Vec<(NodeIndex, N)> = vec![];
    let mut remaining = indeg.clone();
    while let Some(current) = queue.pop_front() {
        order.push((current, node_data(graph, current)));
        let mut freed: Vec<NodeIndex> = vec![];
        for (_, e) in graph.edge_vec() {
            if e.source == current {
                let entry = remaining.entry(e.target).or_insert(0);
                *entry = entry.saturating_sub(1);
                if *entry == 0 && queued.insert(e.target) {
                    freed.push(e.target);
                }
            }
        }
        freed.sort();
        queue.extend(freed);
    }
    if order.len() != graph.node_vec().len() {
        return Err(GraphError("Cannot perform topological sort on cyclic graph".into()));
    }
    Ok(order)
}
// ---------------------------------------------------------------------------
// Paths: Dijkstra / A* / Bellman-Ford / Floyd-Warshall / enumeration
// ---------------------------------------------------------------------------

/// One shortest-path answer: node path, edge path, total distance, and the
/// original edge payloads along the way (not the numeric weights).
#[derive(Clone, Debug, PartialEq)]
pub struct PathResult<E> {
    /// Node indexes from source to target.
    pub path: Vec<NodeIndex>,
    /// Edge indexes from source to target.
    pub edges: Vec<EdgeIndex>,
    /// Total numeric distance.
    pub distance: f64,
    /// Original edge payloads along the path.
    pub costs: Vec<E>,
}

/// Validate a weight value: finite numbers only.
fn check_weight(edge: EdgeIndex, w: f64) -> Result<f64, GraphError> {
    if !w.is_finite() {
        return Err(GraphError(format!("Edge {edge} weight must be a finite number")));
    }
    Ok(w)
}

/// Scan every edge weight first: NaN and negative fail Dijkstra-style
/// algorithms with their message, before any early return.
fn prescan_nonnegative<N, E, V>(graph: &V, weight: impl Fn(&E) -> f64, message: &str) -> Result<(), GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    for (id, e) in &edges {
        let w = weight(&e.data);
        if w.is_nan() || w < 0.0 {
            return Err(GraphError(message.into()));
        }
        let _ = id;
    }
    Ok(())
}

/// Scan every edge weight first: NaN and -Infinity fail Bellman-Ford-style
/// algorithms with their message. Positive infinity stays traversable.
fn prescan_no_nan_neg_inf<N, E, V>(graph: &V, weight: impl Fn(&E) -> f64, message: &str) -> Result<(), GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    for (id, e) in &edges {
        let w = weight(&e.data);
        if w.is_nan() || w == f64::NEG_INFINITY {
            return Err(GraphError(message.into()));
        }
        let _ = id;
    }
    Ok(())
}

/// Parse an enumeration limit: non-negative integers and infinity only.
pub fn parse_limit(limit: f64) -> Result<Option<usize>, GraphError> {
    if limit.is_infinite() && limit > 0.0 {
        Ok(None)
    } else if limit.is_finite() && limit >= 0.0 && limit.fract() == 0.0 {
        Ok(Some(limit as usize))
    } else {
        Err(GraphError("Path enumeration limit must be a non-negative integer or Infinity".into()))
    }
}

fn check_endpoints<N, E, V>(graph: &V, source: NodeIndex, target: NodeIndex) -> Result<(), GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    for endpoint in [source, target] {
        if !has_node(graph, endpoint) {
            return Err(GraphError(format!("Node {endpoint} does not exist")));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Open {
    dist: f64,
    seq: u64,
    node: NodeIndex,
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist && self.seq == other.seq
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.dist.total_cmp(&self.dist).then_with(|| other.seq.cmp(&self.seq))
    }
}

/// Dijkstra shortest path. Negative or non-finite weights are [`GraphError`].
/// Unreachable targets are `Ok(None)`; positive infinity counts as unreachable.
pub fn dijkstra<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    weight: impl Fn(&E) -> f64,
) -> Result<Option<PathResult<E>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    check_endpoints(graph, source, target)?;
    prescan_nonnegative(graph, &weight, "Dijkstra's algorithm requires non-negative edge weights")?;
    if source == target {
        return Ok(Some(PathResult { path: vec![source], edges: vec![], distance: 0.0, costs: vec![] }));
    }
    let mut dist: HashMap<NodeIndex, f64> = HashMap::new();
    let mut prev: HashMap<NodeIndex, (NodeIndex, EdgeIndex)> = HashMap::new();
    let mut heap = std::collections::BinaryHeap::new();
    let mut seq = 0u64;
    dist.insert(source, 0.0);
    heap.push(Open { dist: 0.0, seq, node: source });
    while let Some(Open { dist: d, node, .. }) = heap.pop() {
        if d > dist[&node] {
            continue;
        }
        if node == target {
            break;
        }
        let mut outs: Vec<(EdgeIndex, StoredEdge<E>)> = edge_list(graph)
            .into_iter()
            .filter(|(_, e)| e.source == node)
            .collect();
        outs.sort_by_key(|(i, _)| *i);
        for (id, edge) in outs {
            let w = weight(&edge.data);
            let nd = d + w;
            if nd < *dist.get(&edge.target).unwrap_or(&f64::INFINITY) {
                dist.insert(edge.target, nd);
                prev.insert(edge.target, (node, id));
                seq += 1;
                heap.push(Open { dist: nd, seq, node: edge.target });
            }
            let _ = id;
        }
    }
    finish_path(graph, source, target, &dist, &prev, "Dijkstra distance calculation exceeded the finite number range")
}

fn finish_path<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    dist: &HashMap<NodeIndex, f64>,
    prev: &HashMap<NodeIndex, (NodeIndex, EdgeIndex)>,
    overflow: &str,
) -> Result<Option<PathResult<E>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let distance = match dist.get(&target) {
        Some(d) if d.is_finite() => *d,
        Some(_) => {
            // Reached through arithmetic that left the finite range.
            if prev.contains_key(&target) {
                return Err(GraphError(overflow.into()));
            }
            return Ok(None);
        }
        None => return Ok(None),
    };
    let mut path = vec![target];
    let mut edges = vec![];
    let mut costs = vec![];
    let mut current = target;
    let mut walked: BTreeSet<NodeIndex> = BTreeSet::from([target]);
    while current != source {
        let (parent, edge) = prev.get(&current).ok_or_else(|| GraphError(overflow.into()))?;
        if !walked.insert(*parent) {
            return Err(GraphError(overflow.into()));
        }
        let data = graph.edge_vec().into_iter().find(|(i, _)| i == edge).map(|(_, e)| e.data);
        let Some(data) = data else { return Err(GraphError(overflow.into())) };
        edges.push(*edge);
        costs.push(data);
        path.push(*parent);
        current = *parent;
    }
    path.reverse();
    edges.reverse();
    costs.reverse();
    Ok(Some(PathResult { path, edges, distance, costs }))
}

/// A* shortest path: Dijkstra guided by `heuristic` (must be finite and
/// non-negative). Closed nodes never reopen; stale queue entries skip.
pub fn astar<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    weight: impl Fn(&E) -> f64,
    heuristic: impl Fn(NodeIndex) -> f64,
) -> Result<Option<PathResult<E>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    check_endpoints(graph, source, target)?;
    prescan_nonnegative(graph, &weight, "A* algorithm requires non-negative edge weights")?;
    let h0 = heuristic(source);
    if !h0.is_finite() || h0 < 0.0 {
        return Err(GraphError("A* algorithm requires finite heuristic values".into()));
    }
    if source == target {
        return Ok(Some(PathResult { path: vec![source], edges: vec![], distance: 0.0, costs: vec![] }));
    }
    let mut dist: HashMap<NodeIndex, f64> = HashMap::new();
    let mut prev: HashMap<NodeIndex, (NodeIndex, EdgeIndex)> = HashMap::new();
    let mut closed: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut heap = std::collections::BinaryHeap::new();
    let mut seq = 0u64;
    dist.insert(source, 0.0);
    heap.push(Open { dist: h0, seq, node: source });
    while let Some(Open { dist: est, node, .. }) = heap.pop() {
        let h = heuristic(node);
        if !h.is_finite() || h < 0.0 {
            return Err(GraphError("A* algorithm requires finite heuristic values".into()));
        }
        let d = est - h;
        if d > *dist.get(&node).unwrap_or(&f64::INFINITY) {
            continue;
        }
        if !closed.insert(node) {
            continue;
        }
        if node == target {
            break;
        }
        let mut outs: Vec<(EdgeIndex, StoredEdge<E>)> = edge_list(graph)
            .into_iter()
            .filter(|(_, e)| e.source == node)
            .collect();
        outs.sort_by_key(|(i, _)| *i);
        for (id, edge) in outs {
            let w = weight(&edge.data);
            let nd = d + w;
            if nd < *dist.get(&edge.target).unwrap_or(&f64::INFINITY) {
                dist.insert(edge.target, nd);
                prev.insert(edge.target, (node, id));
                seq += 1;
                let ht = heuristic(edge.target);
                if !ht.is_finite() || ht < 0.0 {
                    return Err(GraphError("A* algorithm requires finite heuristic values".into()));
                }
                let est2 = nd + ht;
                if !est2.is_finite() && nd.is_finite() && ht.is_finite() {
                    return Err(GraphError("A* priority calculation exceeded the finite number range".into()));
                }
                heap.push(Open { dist: est2, seq, node: edge.target });
            }
            let _ = id;
        }
    }
    finish_path(graph, source, target, &dist, &prev, "A* distance calculation exceeded the finite number range")
}

/// Bellman-Ford shortest path: negative edges allowed. A negative cycle on
/// some source-to-target route is [`GraphError`]; cycles elsewhere are
/// ignored.
pub fn bellman_ford<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    weight: impl Fn(&E) -> f64,
) -> Result<Option<PathResult<E>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    check_endpoints(graph, source, target)?;
    prescan_no_nan_neg_inf(graph, &weight, "Bellman-Ford algorithm does not support NaN or -Infinity edge weights")?;
    let nodes: Vec<NodeIndex> = graph.node_vec().into_iter().map(|(i, _)| i).collect();
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    let mut dist: HashMap<NodeIndex, f64> = nodes.iter().map(|n| (*n, f64::INFINITY)).collect();
    let mut prev: HashMap<NodeIndex, (NodeIndex, EdgeIndex)> = HashMap::new();
    dist.insert(source, 0.0);
    for _ in 0..nodes.len().saturating_sub(1) {
        let mut moved = false;
        for (id, e) in &edges {
            let w = weight(&e.data);
            let d = dist[&e.source];
            if d.is_finite() && d + w < dist[&e.target] {
                dist.insert(e.target, d + w);
                prev.insert(e.target, (e.source, *id));
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    // One more pass: a relaxable edge feeding the target poisons the answer.
    let mut bad: BTreeSet<NodeIndex> = BTreeSet::new();
    for (_, e) in &edges {
        let w = weight(&e.data);
        let d = dist[&e.source];
        if d.is_finite() && d + w < dist[&e.target] {
            bad.insert(e.target);
        }
    }
    // Propagate taint forward along edges.
    let mut queue: Vec<NodeIndex> = bad.iter().copied().collect();
    while let Some(current) = queue.pop() {
        for (_, e) in &edges {
            if e.source == current && bad.insert(e.target) {
                queue.push(e.target);
            }
        }
    }
    if bad.contains(&target) {
        return Err(GraphError(format!("Negative cycle affects path to node {target}")));
    }
    // Finite-range overflow surfaces when the witness left finite arithmetic.
    if let Some(d) = dist.get(&target) {
        if !d.is_finite() && prev.contains_key(&target) {
            return Err(GraphError("Bellman-Ford distance calculation exceeded the finite number range".into()));
        }
    }
    finish_path(graph, source, target, &dist, &prev, "Bellman-Ford distance calculation exceeded the finite number range")
}

/// All-pairs shortest paths (Floyd-Warshall): nested maps of distances,
/// node paths, edge paths, and original payloads. Unreachable pairs absent.
#[derive(Clone, Debug, PartialEq)]
pub struct FloydResult<E> {
    /// Minimum distances by (source, target).
    pub distances: BTreeMap<NodeIndex, BTreeMap<NodeIndex, f64>>,
    /// Node paths by (source, target).
    pub paths: BTreeMap<NodeIndex, BTreeMap<NodeIndex, Vec<NodeIndex>>>,
    /// Edge paths by (source, target).
    pub edges: BTreeMap<NodeIndex, BTreeMap<NodeIndex, Vec<EdgeIndex>>>,
    /// Original payloads along each path.
    pub costs: BTreeMap<NodeIndex, BTreeMap<NodeIndex, Vec<E>>>,
}

pub fn floyd_warshall<N, E, V>(
    graph: &V,
    weight: impl Fn(&E) -> f64,
) -> Result<FloydResult<E>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    prescan_no_nan_neg_inf(graph, &weight, "Floyd-Warshall algorithm does not support NaN or -Infinity edge weights")?;
    let nodes: Vec<NodeIndex> = graph.node_vec().into_iter().map(|(i, _)| i).collect();
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    let mut dist: HashMap<(NodeIndex, NodeIndex), f64> = HashMap::new();
    let mut via: HashMap<(NodeIndex, NodeIndex), Vec<EdgeIndex>> = HashMap::new();
    for n in &nodes {
        dist.insert((*n, *n), 0.0);
        via.insert((*n, *n), vec![]);
    }
    for (id, e) in &edges {
        let w = weight(&e.data);
        let key = (e.source, e.target);
        if w < *dist.get(&key).unwrap_or(&f64::INFINITY) {
            dist.insert(key, w);
            via.insert(key, vec![*id]);
        }
    }
    for k in &nodes {
        for i in &nodes {
            for j in &nodes {
                let dik = *dist.get(&(*i, *k)).unwrap_or(&f64::INFINITY);
                let dkj = *dist.get(&(*k, *j)).unwrap_or(&f64::INFINITY);
                if dik.is_finite() && dkj.is_finite() {
                    let nd = dik + dkj;
                    if !nd.is_finite() {
                        return Err(GraphError(
                            "Floyd-Warshall distance calculation exceeded the finite number range".into(),
                        ));
                    }
                    if nd < *dist.get(&(*i, *j)).unwrap_or(&f64::INFINITY) {
                        let mut p = via.get(&(*i, *k)).cloned().unwrap_or_default();
                        p.extend(via.get(&(*k, *j)).cloned().unwrap_or_default());
                        dist.insert((*i, *j), nd);
                        via.insert((*i, *j), p);
                    }
                }
            }
        }
    }
    let mut ordered: Vec<NodeIndex> = nodes.clone();
    ordered.sort();
    for n in ordered {
        if dist.get(&(n, n)).is_some_and(|d| *d < 0.0) {
            return Err(GraphError(format!("Negative cycle detected involving node {n}")));
        }
    }
    let by_id: HashMap<EdgeIndex, StoredEdge<E>> = edges.into_iter().collect();
    let mut out = FloydResult {
        distances: BTreeMap::new(),
        paths: BTreeMap::new(),
        edges: BTreeMap::new(),
        costs: BTreeMap::new(),
    };
    let mut pairs: Vec<((NodeIndex, NodeIndex), f64)> =
        dist.into_iter().filter(|(_, d)| d.is_finite()).collect();
    pairs.sort_by_key(|((i, j), _)| (*i, *j));
    for ((i, j), d) in pairs {
        if i == j {
            continue;
        }
        let edge_ids = via.remove(&(i, j)).unwrap_or_default();
        let mut node_path = vec![i];
        let mut costs = vec![];
        for id in &edge_ids {
            let e = &by_id[id];
            node_path.push(e.target);
            costs.push(e.data.clone());
        }
        out.distances.entry(i).or_default().insert(j, d);
        out.paths.entry(i).or_default().insert(j, node_path);
        out.edges.entry(i).or_default().insert(j, edge_ids);
        out.costs.entry(i).or_default().insert(j, costs);
    }
    Ok(out)
}

/// Every shortest path from `source` to `target` (all tied predecessors),
/// each as a [`SimplePath`], enumerated per run over an isolated snapshot.
/// Endpoints validate eagerly; weights, limits, and ranges validate per
/// iteration with the spec messages.
pub fn all_shortest_paths<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    weight: impl Fn(&E) -> f64 + Clone + 'static,
    limit: f64,
) -> Result<PathWalker<E>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    check_endpoints(graph, source, target)?;
    let max = parse_limit(limit)?;
    let view = graph.clone();
    Ok(PathWalker {
        run: std::rc::Rc::new(move || {
            check_endpoints(&view, source, target)?;
            enumerate_all_shortest(&view, source, target, &weight, max)
        }),
    })
}

fn enumerate_all_shortest<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    weight: &dyn Fn(&E) -> f64,
    limit: Option<usize>,
) -> Result<Vec<SimplePath<E>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let nodes: Vec<NodeIndex> = graph.node_vec().into_iter().map(|(i, _)| i).collect();
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    // Non-negative weights only; overflow past finite range errors.
    for (id, e) in &edges {
        let w = weight(&e.data);
        if w.is_nan() || w < 0.0 {
            return Err(GraphError("All shortest paths requires non-negative edge weights".into()));
        }
        let _ = id;
    }
    let mut dist: HashMap<NodeIndex, f64> = nodes.iter().map(|n| (*n, f64::INFINITY)).collect();
    dist.insert(source, 0.0);
    let mut heap = std::collections::BinaryHeap::new();
    heap.push(Open { dist: 0.0, seq: 0, node: source });
    while let Some(Open { dist: d, node, .. }) = heap.pop() {
        if d > dist[&node] {
            continue;
        }
        for (id, e) in edges.iter().filter(|(_, e)| e.source == node) {
            let nd = d + weight(&e.data);
            if !nd.is_finite() && d.is_finite() {
                return Err(GraphError(
                    "All shortest paths distance calculation exceeded the finite number range".into(),
                ));
            }
            if nd < dist[&e.target] {
                dist.insert(e.target, nd);
                heap.push(Open { dist: nd, seq: 0, node: e.target });
            }
            let _ = id;
        }
    }
    if !dist.get(&target).is_some_and(|d| d.is_finite()) {
        return Ok(vec![]);
    }
    // Enumerate every tight-edge route in edge order.
    let mut out: Vec<SimplePath<E>> = vec![];
    let mut stack: Vec<(NodeIndex, Vec<NodeIndex>, Vec<EdgeIndex>)> = vec![(target, vec![target], vec![])];
    while let Some((current, node_path, edge_path)) = stack.pop() {
        if let Some(max) = limit {
            if out.len() >= max {
                break;
            }
        }
        if current == source {
            let mut p = node_path.clone();
            p.reverse();
            let mut e = edge_path.clone();
            e.reverse();
            let costs: Vec<E> = e
                .iter()
                .map(|id| graph.edge_vec().into_iter().find(|(i, _)| i == id).map(|(_, x)| x.data))
                .collect::<Option<Vec<E>>>()
                .unwrap_or_default();
            out.push(SimplePath { nodes: p, edges: e, costs });
            continue;
        }
        let mut preds: Vec<(EdgeIndex, NodeIndex)> = vec![];
        for (id, e) in &edges {
            if e.target != current {
                continue;
            }
            let w = weight(&e.data);
            if (dist[&e.source] + w - dist[&current]).abs() < 1e-9 {
                preds.push((*id, e.source));
            }
        }
        preds.sort_by_key(|(i, _)| *i);
        for (id, parent) in preds.into_iter().rev() {
            // Zero-cost predecessor cycles terminate: never revisit a node.
            if node_path.contains(&parent) {
                continue;
            }
            let mut n2 = node_path.clone();
            let mut e2 = edge_path.clone();
            n2.push(parent);
            e2.push(id);
            stack.push((parent, n2, e2));
        }
    }
    out.sort_by(|a, b| a.nodes.cmp(&b.nodes).then_with(|| a.edges.cmp(&b.edges)));
    Ok(out)
}

/// One simple (node-acyclic) path: node and edge index sequences.
#[derive(Clone, Debug, PartialEq)]
pub struct SimplePath<E> {
    /// Node indexes from source to target.
    pub nodes: Vec<NodeIndex>,
    /// Edge indexes from source to target.
    pub edges: Vec<EdgeIndex>,
    /// Original edge payloads.
    pub costs: Vec<E>,
}

/// Lazily-enumerated simple paths: endpoints revalidate against the live
/// graph per run, enumeration runs over an isolated snapshot, and every run
/// is repeatable and independent of later mutation.
#[derive(Clone)]
pub struct PathWalker<E> {
    run: std::rc::Rc<dyn Fn() -> Result<Vec<SimplePath<E>>, GraphError>>,
}

impl<E> PathWalker<E>
where
    E: Clone + 'static,
{
    /// All enumerated paths for one run.
    pub fn paths(&self) -> Result<Vec<SimplePath<E>>, GraphError> {
        (self.run)()
    }

    /// Path count for one run.
    pub fn len(&self) -> Result<usize, GraphError> {
        Ok((self.run)()?.len())
    }

    /// True when one run enumerates nothing.
    pub fn is_empty(&self) -> Result<bool, GraphError> {
        Ok((self.run)()?.is_empty())
    }
}

/// Every simple path from `source` to `target` in edge order (at most
/// `limit`, unbounded when `None`). Zero-cost cycles terminate: nodes never
/// repeat, so only simple paths emit.
pub fn simple_paths<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    limit: f64,
) -> Result<PathWalker<E>, GraphError>
where
    V: View<N, E> + Clone + 'static,
    N: Clone + 'static,
    E: Clone + 'static,
{
    check_endpoints(graph, source, target)?;
    let max = parse_limit(limit)?;
    let view = graph.clone();
    Ok(PathWalker {
        run: std::rc::Rc::new(move || {
            check_endpoints(&view, source, target)?;
            Ok(enumerate_simple(&view, source, target, max))
        }),
    })
}

fn enumerate_simple<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    limit: Option<usize>,
) -> Vec<SimplePath<E>>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    let mut out: Vec<SimplePath<E>> = vec![];
    // Stack of (current, node-path, edge-path, visited-set).
    let mut stack: Vec<(NodeIndex, Vec<NodeIndex>, Vec<EdgeIndex>, BTreeSet<NodeIndex>)> = vec![];
    let mut start_visited = BTreeSet::new();
    start_visited.insert(source);
    stack.push((source, vec![source], vec![], start_visited));
    while let Some((current, nodes, edge_ids, visited)) = stack.pop() {
        if let Some(max) = limit {
            if out.len() >= max {
                break;
            }
        }
        if current == target && nodes.len() > 1 {
            let costs: Vec<E> = edge_ids
                .iter()
                .map(|id| graph.edge_vec().into_iter().find(|(i, _)| i == id).map(|(_, e)| e.data))
                .collect::<Option<Vec<E>>>()
                .unwrap_or_default();
            out.push(SimplePath { nodes, edges: edge_ids, costs });
            continue;
        }
        if current == target {
            continue;
        }
        let mut nexts: Vec<(EdgeIndex, NodeIndex)> = vec![];
        for (id, e) in &edges {
            if e.source == current && !visited.contains(&e.target) {
                nexts.push((*id, e.target));
            }
        }
        nexts.reverse();
        for (id, next) in nexts {
            let mut v2 = visited.clone();
            v2.insert(next);
            let mut n2 = nodes.clone();
            n2.push(next);
            let mut e2 = edge_ids.clone();
            e2.push(id);
            stack.push((next, n2, e2, v2));
        }
    }
    // Source-equals-target with no movement enumerates nothing.
    out
}

/// Unweighted hop distances from `source` (BFS). Unreachable nodes absent.
pub fn unweighted_distances<N, E, V>(graph: &V, source: NodeIndex) -> Result<HashMap<NodeIndex, usize>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if !has_node(graph, source) {
        return Err(GraphError(format!("Node {source} does not exist")));
    }
    let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
    let mut queue = VecDeque::from([source]);
    dist.insert(source, 0);
    while let Some(current) = queue.pop_front() {
        let d = dist[&current];
        let mut outs: Vec<(EdgeIndex, StoredEdge<E>)> = edge_list(graph)
            .into_iter()
            .filter(|(_, e)| e.source == current)
            .collect();
        outs.sort_by_key(|(i, _)| *i);
        for (_, e) in outs {
            if !dist.contains_key(&e.target) {
                dist.insert(e.target, d + 1);
                queue.push_back(e.target);
            }
        }
    }
    Ok(dist)
}

/// True when a directed-or-undirected route (per `direction`) joins them.
pub fn has_path<N, E, V>(graph: &V, source: NodeIndex, target: NodeIndex, direction: Direction) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if source == target {
        return has_node(graph, source);
    }
    let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut queue = VecDeque::from([source]);
    seen.insert(source);
    while let Some(current) = queue.pop_front() {
        for next in adj_impl(graph, current, direction) {
            if next == target {
                return true;
            }
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Cycles and connectivity
// ---------------------------------------------------------------------------

/// One directed-or-undirected cycle witness: closed node loop plus edges.
/// Self-loops witness as single-edge cycles; parallel edges as two-edge ones.
#[derive(Clone, Debug, PartialEq)]
pub struct CycleResult {
    /// Node loop (first node repeats at the end).
    pub path: Vec<NodeIndex>,
    /// Edge indexes around the loop.
    pub edges: Vec<EdgeIndex>,
}

/// One cycle if any exists (DFS with colors; parallel edges and self-loops
/// count). Undirected graphs use orientation-insensitive traversal.
pub fn find_cycle<N, E, V>(graph: &V) -> Option<CycleResult>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let undirected = graph.kind() == Kind::Undirected;
    // 0 = white, 1 = gray, 2 = black.
    let mut color: HashMap<NodeIndex, u8> = graph.node_vec().into_iter().map(|(i, _)| (i, 0)).collect();
    let mut stack: Vec<NodeIndex> = vec![];
    let mut stack_edges: Vec<EdgeIndex> = vec![];
    // (node, neighbor-iterator state) manual recursion.
    for (root, _) in graph.node_vec() {
        if color[&root] != 0 {
            continue;
        }
        let mut work: Vec<(NodeIndex, Vec<(EdgeIndex, NodeIndex)>, usize)> = vec![];
        let first = ordered_neighbors(graph, root, undirected);
        work.push((root, first, 0));
        color.insert(root, 1);
        stack.push(root);
        while let Some((current, nexts, pos)) = work.pop() {
            if pos < nexts.len() {
                work.push((current, nexts.clone(), pos + 1));
                let (edge_id, next) = nexts[pos].clone();
                if color[&next] == 0 {
                    color.insert(next, 1);
                    stack.push(next);
                    stack_edges.push(edge_id);
                    let more = ordered_neighbors(graph, next, undirected);
                    work.push((next, more, 0));
                } else if color[&next] == 1 {
                    // Gray hit: close the loop. For undirected graphs the
                    // arrival edge back is not a cycle.
                    if undirected && Some(edge_id) == stack_edges.last().copied() {
                        continue;
                    }
                    let start = stack.iter().position(|n| *n == next).unwrap_or(0);
                    let mut path: Vec<NodeIndex> = stack[start..].to_vec();
                    path.push(next);
                    let mut edges: Vec<EdgeIndex> = stack_edges[start..].to_vec();
                    edges.push(edge_id);
                    return Some(CycleResult { path, edges });
                }
            } else {
                color.insert(current, 2);
                stack.pop();
                stack_edges.pop();
            }
        }
    }
    None
}

fn ordered_neighbors<N, E, V>(graph: &V, node: NodeIndex, undirected: bool) -> Vec<(EdgeIndex, NodeIndex)>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut out: Vec<(EdgeIndex, NodeIndex)> = vec![];
    for (id, e) in graph.edge_vec() {
        if e.source == node {
            out.push((id, e.target));
        } else if undirected && e.target == node {
            out.push((id, e.source));
        }
    }
    out.sort_by_key(|(i, _)| *i);
    out
}

/// True when no cycle exists.
pub fn is_acyclic<N, E, V>(graph: &V) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    find_cycle(graph).is_none()
}

/// Undirected connected components: partitions in first-node order, members
/// ascending. Directed graphs are rejected.
pub fn connected_components<N, E, V>(graph: &V) -> Result<Vec<Vec<NodeIndex>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot find connected components of directed graph".into()));
    }
    Ok(components_undirected(graph))
}

fn components_undirected<N, E, V>(graph: &V) -> Vec<Vec<NodeIndex>>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut parts: Vec<Vec<NodeIndex>> = vec![];
    for (root, _) in graph.node_vec() {
        if !seen.insert(root) {
            continue;
        }
        let mut part = vec![root];
        let mut queue = VecDeque::from([root]);
        while let Some(current) = queue.pop_front() {
            for next in adj_impl(graph, current, Direction::Undirected) {
                if seen.insert(next) {
                    part.push(next);
                    queue.push_back(next);
                }
            }
        }
        part.sort();
        parts.push(part);
    }
    parts
}

/// Weak components of a directed graph (orientation ignored).
pub fn weakly_connected_components<N, E, V>(graph: &V) -> Result<Vec<Vec<NodeIndex>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot find weakly connected components of undirected graph".into()));
    }
    Ok(components_undirected(graph))
}

/// Strong components (Kosaraju): second pass over the reversed graph in
/// descending finish order; members list in visit order. Directed only.
pub fn strongly_connected_components<N, E, V>(graph: &V) -> Result<Vec<Vec<NodeIndex>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot find strongly connected components of undirected graph".into()));
    }
    let nodes: Vec<NodeIndex> = graph.node_vec().into_iter().map(|(i, _)| i).collect();
    // Pass one: finishing order over outgoing edges.
    let mut finished: Vec<NodeIndex> = vec![];
    let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
    for root in &nodes {
        if !seen.insert(*root) {
            continue;
        }
        let mut work: Vec<(NodeIndex, Vec<NodeIndex>, usize)> = vec![(*root, succ_impl(graph, *root), 0)];
        while let Some((current, nexts, pos)) = work.pop() {
            if pos < nexts.len() {
                work.push((current, nexts.clone(), pos + 1));
                let next = nexts[pos];
                if seen.insert(next) {
                    work.push((next, succ_impl(graph, next), 0));
                }
            } else {
                finished.push(current);
            }
        }
    }
    // Pass two: reversed graph in descending finish order.
    let mut assigned: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut parts: Vec<Vec<NodeIndex>> = vec![];
    let mut order: Vec<NodeIndex> = finished;
    order.reverse();
    for root in order {
        if !assigned.insert(root) {
            continue;
        }
        let mut part = vec![root];
        let mut queue = VecDeque::from([root]);
        while let Some(current) = queue.pop_front() {
            for next in pred_impl(graph, current) {
                if assigned.insert(next) {
                    part.push(next);
                    queue.push_back(next);
                }
            }
        }
        parts.push(part);
    }
    Ok(parts)
}

/// True when every pair joins (undirected). Empty graphs count as connected.
pub fn is_connected<N, E, V>(graph: &V) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    components_undirected(graph).len() <= 1
}

/// True when every pair joins ignoring orientation. Empty counts as weakly joined.
pub fn is_weakly_connected<N, E, V>(graph: &V) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    components_undirected(graph).len() <= 1
}

/// True when every pair joins both ways. Empty counts as strongly joined.
pub fn is_strongly_connected<N, E, V>(graph: &V) -> bool
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() == Kind::Undirected {
        return components_undirected(graph).len() <= 1;
    }
    strongly_connected_components(graph).map(|parts| parts.len() <= 1).unwrap_or(false)
}

/// True for a connected acyclic undirected graph (empty counts as a tree).
pub fn is_tree<N, E, V>(graph: &V) -> Result<bool, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot determine tree status of directed graph".into()));
    }
    if graph.node_vec().is_empty() {
        return Ok(false);
    }
    let parts = components_undirected(graph);
    if parts.len() > 1 {
        return Ok(false);
    }
    Ok(is_acyclic(graph))
}

struct LowLink {
    articulation: BTreeSet<NodeIndex>,
    bridges: Vec<EdgeIndex>,
    biconnected: Vec<Vec<NodeIndex>>,
}

fn low_link<N, E, V>(graph: &V) -> LowLink
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut disc: HashMap<NodeIndex, usize> = HashMap::new();
    let mut low: HashMap<NodeIndex, usize> = HashMap::new();
    let mut parent_edge: HashMap<NodeIndex, Option<EdgeIndex>> = HashMap::new();
    let mut articulation: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut bridges: Vec<EdgeIndex> = vec![];
    let mut edge_stack: Vec<EdgeIndex> = vec![];
    let mut biconnected: Vec<Vec<NodeIndex>> = vec![];
    let mut time = 0usize;
    let nodes: Vec<NodeIndex> = graph.node_vec().into_iter().map(|(i, _)| i).collect();
    for root in &nodes {
        if disc.contains_key(root) {
            continue;
        }
        let mut children = 0usize;
        // Iterative DFS with explicit frames.
        let mut work: Vec<(NodeIndex, Vec<(EdgeIndex, NodeIndex)>, usize)> = vec![];
        disc.insert(*root, time);
        low.insert(*root, time);
        time += 1;
        parent_edge.insert(*root, None);
        work.push((*root, ordered_neighbors(graph, *root, true), 0));
        while let Some((current, nexts, pos)) = work.pop() {
            if pos < nexts.len() {
                work.push((current, nexts.clone(), pos + 1));
                let (id, next) = nexts[pos].clone();
                if next == current {
                    // A self-loop is always its own biconnected component.
                    biconnected.push(vec![current]);
                    let _ = id;
                    continue;
                }
                if !disc.contains_key(&next) {
                    if current == *root {
                        children += 1;
                    }
                    disc.insert(next, time);
                    low.insert(next, time);
                    time += 1;
                    parent_edge.insert(next, Some(id));
                    edge_stack.push(id);
                    work.push((next, ordered_neighbors(graph, next, true), 0));
                } else if Some(id) != parent_edge.get(&current).copied().flatten() && disc[&next] < disc[&current] {
                    // Back edge: update low, track the cycle's edges.
                    let lo = low[&current].min(disc[&next]);
                    low.insert(current, lo);
                    edge_stack.push(id);
                }
            } else {
                // Finished: propagate to parent, check articulation/bridges.
                if let Some(frame) = work.last() {
                    let parent = frame.0;
                    let lo = low[&parent].min(low[&current]);
                    low.insert(parent, lo);
                    if low[&current] >= disc[&parent] && Some(parent) != Some(*root) {
                        articulation.insert(parent);
                    }
                    if low[&current] > disc[&parent] {
                        if let Some(eid) = parent_edge.get(&current).copied().flatten() {
                            bridges.push(eid);
                        }
                    }
                    // Pop one biconnected component when the subtree closes.
                    if low[&current] >= disc[&parent] {
                        let mut members: BTreeSet<NodeIndex> = BTreeSet::new();
                        while let Some(eid) = edge_stack.pop() {
                            if let Some(e) = graph.edge_vec().into_iter().find(|(i, _)| *i == eid) {
                                members.insert(e.1.source);
                                members.insert(e.1.target);
                            }
                            if Some(eid) == parent_edge.get(&current).copied().flatten() {
                                break;
                            }
                        }
                        if !members.is_empty() {
                            biconnected.push(members.into_iter().collect());
                        }
                    }
                } else if children > 1 {
                    articulation.insert(*root);
                }
            }
        }
        // Drain leftover edges (isolated root component).
        if !edge_stack.is_empty() {
            let mut members: BTreeSet<NodeIndex> = BTreeSet::new();
            for eid in edge_stack.drain(..) {
                if let Some((_, e)) = graph.edge_vec().into_iter().find(|(i, _)| *i == eid) {
                    members.insert(e.source);
                    members.insert(e.target);
                }
            }
            if !members.is_empty() {
                biconnected.push(members.into_iter().collect());
            }
        }
    }
    bridges.sort();
    biconnected.sort();
    LowLink { articulation, bridges, biconnected }
}

/// Articulation points (undirected): removal grows the component count.
pub fn articulation_points<N, E, V>(graph: &V) -> Result<Vec<NodeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot analyze undirected connectivity of directed graph".into()));
    }
    Ok(low_link(graph).articulation.into_iter().collect())
}

/// Bridges (undirected): edges whose removal grows the component count.
pub fn bridges<N, E, V>(graph: &V) -> Result<Vec<EdgeIndex>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot analyze undirected connectivity of directed graph".into()));
    }
    Ok(low_link(graph).bridges)
}

/// Biconnected components (undirected): maximal node sets sharing no
/// articulation split, in sorted order.
pub fn biconnected_components<N, E, V>(graph: &V) -> Result<Vec<Vec<NodeIndex>>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot analyze undirected connectivity of directed graph".into()));
    }
    Ok(low_link(graph).biconnected)
}

// ---------------------------------------------------------------------------
// Bipartite coloring and maximum matching
// ---------------------------------------------------------------------------

/// Two-color an undirected graph: `None` on conflict (odd cycle, self-loop).
fn bipartite_colors<N, E, V>(graph: &V) -> Option<HashMap<NodeIndex, bool>>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut color: HashMap<NodeIndex, bool> = HashMap::new();
    for (root, _) in graph.node_vec() {
        if color.contains_key(&root) {
            continue;
        }
        color.insert(root, false);
        let mut queue = VecDeque::from([root]);
        while let Some(current) = queue.pop_front() {
            for next in neighbors_both(graph, current) {
                if next == current {
                    return None;
                }
                match color.get(&next) {
                    Some(existing) if *existing == color[&current] => return None,
                    Some(_) => {}
                    None => {
                        color.insert(next, !color[&current]);
                        queue.push_back(next);
                    }
                }
            }
        }
    }
    Some(color)
}

/// True for bipartite undirected graphs (even, disconnected, and empty
/// graphs pass; odd cycles and self-loops fail).
pub fn is_bipartite<N, E, V>(graph: &V) -> Result<bool, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot determine bipartite status of directed graph".into()));
    }
    Ok(bipartite_colors(graph).is_some())
}

/// One matched pair: left/right endpoints plus the edge joining them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BipartiteMatch {
    /// Endpoint on the first-colored side.
    pub left: NodeIndex,
    /// Endpoint on the other side.
    pub right: NodeIndex,
    /// The joining edge (first parallel edge wins ties).
    pub edge: EdgeIndex,
}

/// Maximum bipartite matching (Kuhn, deterministic): left side in index
/// order, first parallel edge preferred.
pub fn maximum_bipartite_matching<N, E, V>(graph: &V) -> Result<Vec<BipartiteMatch>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot find bipartite matching of directed graph".into()));
    }
    let Some(colors) = bipartite_colors(graph) else {
        return Err(GraphError("Cannot find bipartite matching of non-bipartite graph".into()));
    };
    let mut lefts: Vec<NodeIndex> = colors.iter().filter(|(_, c)| !**c).map(|(n, _)| *n).collect();
    lefts.sort();
    // Adjacency: left node -> sorted edge indexes to right nodes.
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    let mut adjacent: HashMap<NodeIndex, Vec<(EdgeIndex, NodeIndex)>> = HashMap::new();
    for (id, e) in &edges {
        let (l, r) = if colors.get(&e.source) == Some(&false) && colors.get(&e.target) == Some(&true) {
            (e.source, e.target)
        } else if colors.get(&e.source) == Some(&true) && colors.get(&e.target) == Some(&false) {
            (e.target, e.source)
        } else {
            continue;
        };
        adjacent.entry(l).or_default().push((*id, r));
    }
    for list in adjacent.values_mut() {
        list.sort_by_key(|(i, _)| *i);
    }
    let mut match_right: HashMap<NodeIndex, (NodeIndex, EdgeIndex)> = HashMap::new();
    for left in lefts {
        let mut seen = BTreeSet::new();
        augment(left, &adjacent, &mut match_right, &mut seen);
    }
    let mut out: Vec<BipartiteMatch> = match_right
        .into_iter()
        .map(|(right, (left, edge))| BipartiteMatch { left, right, edge })
        .collect();
    out.sort_by_key(|m| (m.left, m.right, m.edge));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Spanning forests and transitive reduction
// ---------------------------------------------------------------------------

fn augment(
    left: NodeIndex,
    adjacent: &HashMap<NodeIndex, Vec<(EdgeIndex, NodeIndex)>>,
    match_right: &mut HashMap<NodeIndex, (NodeIndex, EdgeIndex)>,
    seen: &mut BTreeSet<NodeIndex>,
) -> bool {
    let list = adjacent.get(&left).cloned().unwrap_or_default();
    for (id, right) in list {
        if !seen.insert(right) {
            continue;
        }
        let incumbent = match_right.get(&right).map(|(other, _)| *other);
        let steal = match incumbent {
            Some(other) if other != left => augment(other, adjacent, match_right, seen),
            _ => false,
        };
        if incumbent.is_none() || steal {
            match_right.insert(right, (left, id));
            return true;
        }
    }
    false
}

/// Minimum spanning forest (Kruskal): edges by ascending weight, ties broken
/// by first edge order. Undirected graphs only.
pub fn minimum_spanning_forest<N, E, V>(graph: &V, weight: impl Fn(&E) -> f64) -> Result<Graph<N, E>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Undirected {
        return Err(GraphError("Cannot find minimum spanning forest of directed graph".into()));
    }
    let mut weighted: Vec<(EdgeIndex, f64)> = vec![];
    for (id, e) in graph.edge_vec() {
        let w = weight(&e.data);
        if w.is_nan() || w == f64::NEG_INFINITY {
            return Err(GraphError(
                "Minimum spanning forest does not support NaN or -Infinity edge weights".into(),
            ));
        }
        weighted.push((id, w));
        let _ = id;
    }
    weighted.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    // Union-find over live nodes.
    let mut parent: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    for (i, _) in graph.node_vec() {
        parent.insert(i, i);
    }
    fn root_of(parent: &mut HashMap<NodeIndex, NodeIndex>, mut node: NodeIndex) -> NodeIndex {
        while parent[&node] != node {
            let grand = parent[&parent[&node]];
            parent.insert(node, grand);
            node = parent[&node];
        }
        node
    }
    let mut out = Graph {
        kind: Kind::Undirected,
        nodes: graph.node_vec().into_iter().collect(),
        edges: BTreeMap::new(),
        next_node: 0,
        next_edge: 0,
    };
    out.next_node = out.nodes.keys().copied().max().map(|m| m + 1).unwrap_or(0);
    let by_id: HashMap<EdgeIndex, StoredEdge<E>> = graph.edge_vec().into_iter().collect();
    for (id, _) in weighted {
        let e = &by_id[&id];
        let (mut a, mut b) = (e.source, e.target);
        a = root_of(&mut parent, a);
        b = root_of(&mut parent, b);
        if a != b {
            parent.insert(a, b);
            out.next_edge = out.next_edge.max(id + 1);
            out.edges.insert(id, e.clone());
        }
    }
    Ok(out)
}

/// Transitive reduction of a DAG: drop every edge implied by a longer route.
/// Cyclic graphs are [`GraphError`].
pub fn transitive_reduction<N, E, V>(graph: &V) -> Result<Graph<N, E>, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot transitively reduce undirected graph".into()));
    }
    if !is_acyclic(graph) {
        return Err(GraphError("Cannot transitively reduce cyclic graph".into()));
    }
    let mut out = Graph {
        kind: Kind::Directed,
        nodes: graph.node_vec().into_iter().collect(),
        edges: BTreeMap::new(),
        next_node: 0,
        next_edge: 0,
    };
    out.next_node = out.nodes.keys().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    for (id, e) in &edges {
        // Implied when another route joins source to target without this edge.
        let mut seen: BTreeSet<NodeIndex> = BTreeSet::new();
        let mut queue = VecDeque::from([e.source]);
        seen.insert(e.source);
        let mut implied = false;
        while let Some(current) = queue.pop_front() {
            for (oid, o) in &edges {
                if *oid == *id || o.source != current {
                    continue;
                }
                if o.target == e.target {
                    implied = true;
                    break;
                }
                if seen.insert(o.target) {
                    queue.push_back(o.target);
                }
            }
            if implied {
                break;
            }
        }
        if !implied {
            out.next_edge = out.next_edge.max(id + 1);
            out.edges.insert(*id, e.clone());
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Flow: maximum flow and minimum cut (Edmonds-Karp)
// ---------------------------------------------------------------------------

/// Maximum flow value, per-edge flows (every edge present, zeros included),
/// and the crossing edges of the corresponding minimum cut.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowResult {
    /// Total flow value.
    pub value: f64,
    /// Flow per edge index (all edges present).
    pub flows: BTreeMap<EdgeIndex, f64>,
    /// Crossing edges of the minimum cut.
    pub cut: Vec<EdgeIndex>,
}

/// Minimum cut value, crossing edges, and the residual partitions.
#[derive(Clone, Debug, PartialEq)]
pub struct CutResult {
    /// Cut value (equals the maximum flow).
    pub value: f64,
    /// Crossing edge indexes.
    pub edges: Vec<EdgeIndex>,
    /// Nodes residual-reachable from the source, ascending.
    pub source: Vec<NodeIndex>,
    /// Remaining nodes, ascending.
    pub target: Vec<NodeIndex>,
}

fn flow_solve<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    capacity: &dyn Fn(&E) -> f64,
) -> Result<(f64, BTreeMap<EdgeIndex, f64>, BTreeSet<NodeIndex>), GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    if graph.kind() != Kind::Directed {
        return Err(GraphError("Cannot compute flow of undirected graph".into()));
    }
    if !has_node(graph, source) {
        return Err(GraphError(format!("Node {source} does not exist")));
    }
    if !has_node(graph, target) {
        return Err(GraphError(format!("Node {target} does not exist")));
    }
    if source == target {
        return Err(GraphError("Flow source and target must be different nodes".into()));
    }
    let mut edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    edges.sort_by_key(|(i, _)| *i);
    let mut cap: HashMap<EdgeIndex, f64> = HashMap::new();
    for (id, e) in &edges {
        let c = capacity(&e.data);
        if !c.is_finite() || c < 0.0 {
            return Err(GraphError(format!("Edge {id} capacity must be a finite non-negative number")));
        }
        cap.insert(*id, c);
    }
    // Residual: (from, edge-id-or-None, forward?) adjacency per node pair.
    let mut flow: HashMap<EdgeIndex, f64> = edges.iter().map(|(i, _)| (*i, 0.0)).collect();
    loop {
        // BFS augmenting path over residual capacity.
        let mut prev: HashMap<NodeIndex, (NodeIndex, EdgeIndex, bool)> = HashMap::new();
        let mut queue = VecDeque::from([source]);
        prev.insert(source, (source, 0, true));
        while let Some(current) = queue.pop_front() {
            if current == target {
                break;
            }
            for (id, e) in &edges {
                // Forward residual.
                if e.source == current && flow[id] < cap[id] && !prev.contains_key(&e.target) {
                    prev.insert(e.target, (current, *id, true));
                    queue.push_back(e.target);
                }
                // Reverse residual.
                if e.target == current && flow[id] > 0.0 && !prev.contains_key(&e.source) {
                    prev.insert(e.source, (current, *id, false));
                    queue.push_back(e.source);
                }
            }
        }
        if !prev.contains_key(&target) {
            break;
        }
        let mut bottleneck = f64::INFINITY;
        let mut node = target;
        while node != source {
            let (parent, id, forward) = prev[&node];
            let room = if forward { cap[&id] - flow[&id] } else { flow[&id] };
            bottleneck = bottleneck.min(room);
            node = parent;
        }
        if !bottleneck.is_finite() || bottleneck <= 0.0 {
            break;
        }
        let mut node = target;
        while node != source {
            let (parent, id, forward) = prev[&node];
            if forward {
                *flow.get_mut(&id).unwrap() += bottleneck;
            } else {
                *flow.get_mut(&id).unwrap() -= bottleneck;
            }
            node = parent;
        }
        let total: f64 = edges.iter().filter(|(_, e)| e.source == source).map(|(i, _)| flow[i]).sum();
        if !total.is_finite() {
            return Err(GraphError("Maximum flow exceeds the finite number range".into()));
        }
    }
    let value: f64 = edges.iter().filter(|(_, e)| e.source == source).map(|(i, _)| flow[i]).sum();
    if !value.is_finite() {
        return Err(GraphError("Maximum flow exceeds the finite number range".into()));
    }
    // Residual-reachable side.
    let mut side: BTreeSet<NodeIndex> = BTreeSet::new();
    let mut queue = VecDeque::from([source]);
    side.insert(source);
    while let Some(current) = queue.pop_front() {
        for (id, e) in &edges {
            if e.source == current && flow[id] < cap[id] && side.insert(e.target) {
                queue.push_back(e.target);
            }
            if e.target == current && flow[id] > 0.0 && side.insert(e.source) {
                queue.push_back(e.source);
            }
        }
    }
    Ok((value, flow.into_iter().collect(), side))
}

/// Maximum flow (Edmonds-Karp) with full validation.
pub fn maximum_flow<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    capacity: impl Fn(&E) -> f64,
) -> Result<FlowResult, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let (value, flows, side) = flow_solve(graph, source, target, &capacity)?;
    let edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    let mut cut: Vec<EdgeIndex> = edges
        .iter()
        .filter(|(_, e)| side.contains(&e.source) && !side.contains(&e.target))
        .map(|(i, _)| *i)
        .collect();
    cut.sort();
    Ok(FlowResult { value, flows, cut })
}

/// Minimum cut: value, crossing edges, and both partitions.
pub fn minimum_cut<N, E, V>(
    graph: &V,
    source: NodeIndex,
    target: NodeIndex,
    capacity: impl Fn(&E) -> f64,
) -> Result<CutResult, GraphError>
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let (value, flows, side) = flow_solve(graph, source, target, &capacity)?;
    let edges: Vec<(EdgeIndex, StoredEdge<E>)> = graph.edge_vec();
    let mut cut: Vec<EdgeIndex> = edges
        .iter()
        .filter(|(_, e)| side.contains(&e.source) && !side.contains(&e.target))
        .map(|(i, _)| *i)
        .collect();
    cut.sort();
    let _ = flows;
    let source_side: Vec<NodeIndex> = side.into_iter().collect();
    let target_side: Vec<NodeIndex> = graph.node_vec().into_iter().map(|(i, _)| i).filter(|n| !source_side.contains(n)).collect();
    Ok(CutResult { value, edges: cut, source: source_side, target: target_side })
}

// ---------------------------------------------------------------------------
// Serialization: GraphViz DOT and Mermaid
// ---------------------------------------------------------------------------

fn graphviz_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// GraphViz DOT: `digraph "G" {` (or `graph`), indexed nodes with labels,
/// edges with stored orientation and labels.
pub fn to_graphviz<N, E, V>(
    graph: &V,
    name: &str,
    node_label: impl Fn(&N) -> String,
    edge_label: impl Fn(&E) -> String,
) -> String
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut out = String::new();
    let keyword = if graph.kind() == Kind::Directed { "digraph" } else { "graph" };
    let link = if graph.kind() == Kind::Directed { "->" } else { "--" };
    out.push_str(&format!("{keyword} \"{}\" {{\n", graphviz_escape(name)));
    for (i, n) in graph.node_vec() {
        out.push_str(&format!("  \"{i}\" [label=\"{}\"];\n", graphviz_escape(&node_label(&n))));
    }
    for (_, e) in graph.edge_vec() {
        out.push_str(&format!(
            "  \"{}\" {} \"{}\" [label=\"{}\"];\n",
            e.source,
            link,
            e.target,
            graphviz_escape(&edge_label(&e.data))
        ));
    }
    out.push('}');
    out
}

fn mermaid_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '#' => out.push_str("#35;"),
            '"' => out.push_str("#quot;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            '[' => out.push_str("#91;"),
            ']' => out.push_str("#93;"),
            '{' => out.push_str("#123;"),
            '}' => out.push_str("#125;"),
            '(' => out.push_str("#40;"),
            ')' => out.push_str("#41;"),
            '|' => out.push_str("#124;"),
            '\\' => out.push_str("#92;"),
            '\r' => out.push_str("<br/>"),
            '\n' => out.push_str("<br/>"),
            c => out.push(c),
        }
    }
    out
}

/// Mermaid node shapes (all eight).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MermaidShape {
    /// `A["label"]`.
    Rectangle,
    /// `A("label")`.
    Rounded,
    /// `A(("label"))`.
    Circle,
    /// `A{"label"}`.
    Diamond,
    /// `A{{"label"}}`.
    Hexagon,
    /// `A(["label"])`.
    Stadium,
    /// `A[["label"]]`.
    Subroutine,
    /// `A[("label")]`.
    Cylindrical,
}

fn mermaid_node(index: NodeIndex, label: &str, shape: MermaidShape) -> String {
    let escaped = mermaid_escape(label);
    match shape {
        MermaidShape::Rectangle => format!("{index}[\"{escaped}\"]"),
        MermaidShape::Rounded => format!("{index}(\"{escaped}\")"),
        MermaidShape::Circle => format!("{index}((\"{escaped}\"))"),
        MermaidShape::Diamond => format!("{index}{{\"{escaped}\"}}"),
        MermaidShape::Hexagon => format!("{index}{{{{\"{escaped}\"}}}}"),
        MermaidShape::Stadium => format!("{index}([\"{escaped}\"])"),
        MermaidShape::Subroutine => format!("{index}[[\"{escaped}\"]]"),
        MermaidShape::Cylindrical => format!("{index}[(\"{escaped}\")]"),
    }
}

/// Mermaid flowchart/graph: header, shaped nodes, labeled edges. Empty edge
/// labels render bare links (`---` / `-->`); otherwise `---|"label"|`.
pub fn to_mermaid<N, E, V>(
    graph: &V,
    direction: &str,
    flowchart: bool,
    node_label: impl Fn(&N) -> String,
    edge_label: impl Fn(&E) -> String,
    shape: impl Fn(&N) -> MermaidShape,
) -> String
where
    V: View<N, E>,
    N: Clone,
    E: Clone,
{
    let mut out = String::new();
    out.push_str(&format!("{} {direction}\n", if flowchart { "flowchart" } else { "graph" }));
    for (i, n) in graph.node_vec() {
        out.push_str(&format!("  {}\n", mermaid_node(i, &node_label(&n), shape(&n))));
    }
    for (_, e) in graph.edge_vec() {
        let label = edge_label(&e.data);
        if graph.kind() == Kind::Directed {
            if label.is_empty() {
                out.push_str(&format!("  {} --> {}\n", e.source, e.target));
            } else {
                out.push_str(&format!("  {} -->|\"{}\"| {}\n", e.source, mermaid_escape(&label), e.target));
            }
        } else if label.is_empty() {
            out.push_str(&format!("  {} --- {}\n", e.source, e.target));
        } else {
            out.push_str(&format!("  {} ---|\"{}\"| {}\n", e.source, mermaid_escape(&label), e.target));
        }
    }
    out.pop();
    out
}

// Immutable transforms: reverse and relational composition
// ---------------------------------------------------------------------------


/// Overlay composition: union of nodes by projected identity (right
/// payloads win conflicts) with the union of edges, deduplicated by
/// endpoint identity plus payload. Kinds must match.
pub fn compose<N, E, K>(
    a: &Graph<N, E>,
    b: &Graph<N, E>,
    node_identity: impl Fn(&N) -> K,
) -> Result<Graph<N, E>, GraphError>
where
    N: Clone,
    E: Clone + PartialEq,
    K: PartialEq + PartialOrd + Clone,
{
    same_kind(a.kind, b.kind)?;
    let directed = a.kind == Kind::Directed;
    let node_identity = &node_identity;
    let mut out = Graph::directed();
    out.kind = a.kind;
    let mut index_of_a: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut index_of_b: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut by_key: HashMap<NodeIndex, K> = HashMap::new();
    for (i, n) in &a.nodes {
        let ni = out.next_node;
        out.next_node += 1;
        out.nodes.insert(ni, n.clone());
        by_key.insert(ni, node_identity(n));
        index_of_a.insert(*i, ni);
    }
    for (i, n) in &b.nodes {
        let key = node_identity(n);
        match by_key.iter().find(|(_, k)| **k == key).map(|(ni, _)| *ni) {
            Some(ni) => {
                out.nodes.insert(ni, n.clone());
                index_of_b.insert(*i, ni);
            }
            None => {
                let ni = out.next_node;
                out.next_node += 1;
                out.nodes.insert(ni, n.clone());
                by_key.insert(ni, key);
                index_of_b.insert(*i, ni);
            }
        }
    }
    let mut seen: Vec<(K, K, E)> = vec![];
    let mut emit = |e: &StoredEdge<E>, nodes: &BTreeMap<NodeIndex, N>, map: &HashMap<NodeIndex, NodeIndex>, out: &mut Graph<N, E>| {
        let Some(key) = edge_ident(directed, nodes, e, node_identity) else { return };
        if seen.iter().any(|k| k.0 == key.0 && k.1 == key.1 && k.2 == key.2) {
            return;
        }
        seen.push(key);
        let (Some(s), Some(t)) = (map.get(&e.source), map.get(&e.target)) else { return };
        let ei = out.next_edge;
        out.next_edge += 1;
        out.edges.insert(ei, StoredEdge { source: *s, target: *t, data: e.data.clone() });
    };
    for e in a.edges.values() {
        emit(e, &a.nodes, &index_of_a, &mut out);
    }
    for e in b.edges.values() {
        emit(e, &b.nodes, &index_of_b, &mut out);
    }
    Ok(out)
}
