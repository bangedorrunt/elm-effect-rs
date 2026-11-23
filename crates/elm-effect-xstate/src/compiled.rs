// ADR-0008: Pre-order compiled statechart topology index (Law L7)
// See: docs/decisions/0008-harden-elm-effect-xstate-with-compiled-topology-timer-tokens-and-macro-dx.md

use crate::state_node::StateNode;
use std::collections::{BTreeMap, BTreeSet};

/// Dense pre-order rank assigned to a declared state node at build time.
///
/// Pre-order numbering paired with `subtree_end` enables $O(1)$ ancestor/descendant
/// checks without walking parent pointers (`ancestor < child < subtree_end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateIdx(pub u32);

#[derive(Debug, Clone)]
pub(crate) struct CompiledNode<S> {
    pub(crate) idx: StateIdx,
    pub(crate) subtree_end: StateIdx,
    pub(crate) depth: usize,
    pub(crate) children: Vec<S>,
    pub(crate) ancestry: Vec<S>,
    pub(crate) path_from_root: Vec<S>,
}

/// Precomputed immutable topology index built once when [`Machine`](crate::Machine) is constructed.
#[derive(Debug, Clone)]
pub(crate) struct CompiledIndex<S: Ord + Clone> {
    by_state: BTreeMap<S, CompiledNode<S>>,
    top_level: Vec<S>,
}

impl<S: Ord + Clone> CompiledIndex<S> {
    /// Compile pre-order state indices, subtree bounds, child slices, and ancestry paths.
    pub(crate) fn build<E, C, CMD>(nodes: &BTreeMap<S, StateNode<S, E, C, CMD>>) -> Self {
        let mut top_level = Vec::new();
        let mut children_map: BTreeMap<S, Vec<S>> = BTreeMap::new();

        for (id, node) in nodes {
            match &node.parent {
                None => top_level.push(id.clone()),
                Some(parent) => {
                    if nodes.contains_key(parent) {
                        children_map
                            .entry(parent.clone())
                            .or_default()
                            .push(id.clone());
                    }
                }
            }
        }

        let mut by_state: BTreeMap<S, CompiledNode<S>> = BTreeMap::new();
        let mut visited: BTreeSet<S> = BTreeSet::new();
        let mut counter: u32 = 0;

        for root in &top_level {
            Self::visit_preorder(
                root,
                nodes,
                &children_map,
                &mut visited,
                &mut counter,
                &mut by_state,
            );
        }

        for id in nodes.keys() {
            if !visited.contains(id) {
                Self::visit_preorder(
                    id,
                    nodes,
                    &children_map,
                    &mut visited,
                    &mut counter,
                    &mut by_state,
                );
            }
        }

        Self {
            by_state,
            top_level,
        }
    }

    fn visit_preorder<E, C, CMD>(
        id: &S,
        nodes: &BTreeMap<S, StateNode<S, E, C, CMD>>,
        children_map: &BTreeMap<S, Vec<S>>,
        visited: &mut BTreeSet<S>,
        counter: &mut u32,
        by_state: &mut BTreeMap<S, CompiledNode<S>>,
    ) {
        if !visited.insert(id.clone()) {
            return;
        }
        let idx = StateIdx(*counter);
        *counter += 1;

        let children = children_map.get(id).cloned().unwrap_or_default();
        for child in &children {
            Self::visit_preorder(child, nodes, children_map, visited, counter, by_state);
        }
        let subtree_end = StateIdx(*counter);

        let mut ancestry = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = Some(id.clone());
        while let Some(current) = cursor {
            if !seen.insert(current.clone()) {
                break;
            }
            ancestry.push(current.clone());
            cursor = nodes.get(&current).and_then(|n| n.parent.clone());
        }
        let depth = ancestry.len();
        let mut path_from_root = ancestry.clone();
        path_from_root.reverse();

        by_state.insert(
            id.clone(),
            CompiledNode {
                idx,
                subtree_end,
                depth,
                children,
                ancestry,
                path_from_root,
            },
        );
    }

    /// Pre-order rank of `id`, if declared in the machine.
    #[inline]
    pub(crate) fn pre_order_idx(&self, id: &S) -> Option<StateIdx> {
        self.by_state.get(id).map(|node| node.idx)
    }

    /// Exclusive pre-order end bound of `id`'s subtree, if declared in the machine.
    #[inline]
    pub(crate) fn subtree_end(&self, id: &S) -> Option<StateIdx> {
        self.by_state.get(id).map(|node| node.subtree_end)
    }

    /// $O(1)$ strict descendant check via pre-order interval containment.
    #[inline]
    pub(crate) fn is_strict_descendant(&self, child: &S, ancestor: &S) -> bool {
        if let (Some(c), Some(a)) = (self.by_state.get(child), self.by_state.get(ancestor)) {
            return a.idx < c.idx && c.idx < a.subtree_end;
        }
        self.by_state
            .get(child)
            .is_some_and(|c| c.ancestry.iter().skip(1).any(|p| p == ancestor))
    }

    /// Direct children in deterministic identifier order.
    #[inline]
    pub(crate) fn children(&self, id: &S) -> &[S] {
        self.by_state
            .get(id)
            .map_or(&[], |node| node.children.as_slice())
    }

    /// Nodes directly under the implicit machine root.
    #[inline]
    pub(crate) fn top_level_children(&self) -> &[S] {
        &self.top_level
    }

    /// `[state, parent, grandparent, ...]`.
    #[inline]
    pub(crate) fn ancestry(&self, id: &S) -> Option<&[S]> {
        self.by_state.get(id).map(|node| node.ancestry.as_slice())
    }

    /// `[root, ..., parent, state]`.
    #[inline]
    pub(crate) fn path_from_root(&self, id: &S) -> Option<&[S]> {
        self.by_state
            .get(id)
            .map(|node| node.path_from_root.as_slice())
    }

    /// Distance from a top-level node, counted in nodes.
    #[inline]
    pub(crate) fn depth(&self, id: &S) -> Option<usize> {
        self.by_state.get(id).map(|node| node.depth)
    }
}
