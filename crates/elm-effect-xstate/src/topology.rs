use crate::machine::Machine;
use crate::types::StateNodeType;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Debug;

/// One state of a [`Topology`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyNode<S> {
    /// The kind of state node.
    pub node_type: StateNodeType,
    /// The enclosing compound or parallel state, if any.
    pub parent: Option<S>,
    /// The states a compound node enters first.
    pub initial: Vec<S>,
    /// Every target of the node's event, `always`, `after`, and `onDone`
    /// transitions. Targetless transitions add nothing.
    pub targets: BTreeSet<S>,
}

/// The states and edges of a chart with every guard treated as enabled.
///
/// Checks over this view over-approximate behaviour. A state reported
/// unreachable is unreachable whatever the guards decide, and a leaf reported
/// unable to reach a goal cannot reach it whatever the guards decide. An empty
/// report does not prove that the guards allow a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topology<S: Ord> {
    /// The declared initial state.
    pub initial: S,
    /// Every top-level node is an active region when true.
    pub root_parallel: bool,
    /// Every declared state.
    pub nodes: BTreeMap<S, TopologyNode<S>>,
}

impl<S, E, C, CMD> Machine<S, E, C, CMD>
where
    S: Ord + Clone + Debug + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
    CMD: Clone + Send + Sync + 'static,
{
    /// The chart's states and guard-free edges, for static checks.
    #[must_use]
    pub fn topology(&self) -> Topology<S> {
        let nodes = self
            .nodes
            .values()
            .map(|node| {
                let targets = node
                    .transitions
                    .iter()
                    .chain(node.always.iter())
                    .chain(node.after.iter().map(|(_, transition)| transition))
                    .chain(node.on_done.iter())
                    .filter(|transition| !transition.is_targetless())
                    .flat_map(|transition| transition.target_states().iter().cloned())
                    .collect();
                let initial = if node.initial_targets.is_empty() {
                    node.initial.iter().cloned().collect()
                } else {
                    node.initial_targets.clone()
                };
                let entry = TopologyNode {
                    node_type: node.node_type,
                    parent: node.parent.clone(),
                    initial,
                    targets,
                };
                (node.id.clone(), entry)
            })
            .collect();
        Topology {
            initial: self.initial_state.clone(),
            root_parallel: self.root_parallel,
            nodes,
        }
    }
}

impl<S: Ord + Clone> Topology<S> {
    /// Declared states that no path from the initial configuration enters.
    /// History pseudo-states are never active, so they are not listed.
    #[must_use]
    pub fn unreachable(&self) -> Vec<S> {
        let active = self.reachable();
        self.nodes
            .iter()
            .filter(|(id, node)| !is_history(node.node_type) && !active.contains(*id))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// States some path from the initial configuration enters: reachable
    /// leaves and every ancestor of one.
    #[must_use]
    pub fn reachable(&self) -> BTreeSet<S> {
        let mut active = BTreeSet::new();
        for leaf in self.reachable_leaves() {
            active.extend(self.ancestors(&leaf));
            active.insert(leaf);
        }
        active
    }

    /// Reachable leaves from which no path enters a leaf for which `goal`
    /// holds. A leaf that is itself a goal is never listed.
    #[must_use]
    pub fn cannot_reach(&self, goal: impl Fn(&S) -> bool) -> Vec<S> {
        let leaves = self.reachable_leaves();
        let mut predecessors: BTreeMap<S, BTreeSet<S>> = BTreeMap::new();
        for leaf in &leaves {
            for next in self.moves(leaf) {
                predecessors.entry(next).or_default().insert(leaf.clone());
            }
        }
        let mut good: BTreeSet<S> = leaves.iter().filter(|s| goal(s)).cloned().collect();
        let mut queue: VecDeque<S> = good.iter().cloned().collect();
        while let Some(state) = queue.pop_front() {
            for previous in predecessors.get(&state).into_iter().flatten() {
                if good.insert(previous.clone()) {
                    queue.push_back(previous.clone());
                }
            }
        }
        leaves.into_iter().filter(|s| !good.contains(s)).collect()
    }

    /// Reachable leaves that cannot reach a top-level final state.
    #[must_use]
    pub fn cannot_finish(&self) -> Vec<S> {
        self.cannot_reach(|state| {
            self.nodes
                .get(state)
                .is_some_and(|node| node.node_type == StateNodeType::Final && node.parent.is_none())
        })
    }

    fn reachable_leaves(&self) -> BTreeSet<S> {
        let mut seen = BTreeSet::new();
        if self.root_parallel {
            for top in self.nodes.iter().filter(|(_, n)| n.parent.is_none()) {
                self.enter(top.0, &mut seen);
            }
        } else {
            self.enter(&self.initial, &mut seen);
        }
        let mut queue: VecDeque<S> = seen.iter().cloned().collect();
        while let Some(leaf) = queue.pop_front() {
            for next in self.moves(&leaf) {
                if seen.insert(next.clone()) {
                    queue.push_back(next);
                }
            }
        }
        seen
    }

    fn moves(&self, leaf: &S) -> BTreeSet<S> {
        let mut out = BTreeSet::new();
        let sources = std::iter::once(leaf.clone()).chain(self.ancestors(leaf));
        for source in sources {
            if let Some(node) = self.nodes.get(&source) {
                for target in &node.targets {
                    self.enter(target, &mut out);
                }
            }
        }
        out
    }

    fn enter(&self, state: &S, out: &mut BTreeSet<S>) {
        let Some(node) = self.nodes.get(state) else {
            out.insert(state.clone());
            return;
        };
        match node.node_type {
            StateNodeType::Atomic | StateNodeType::Final | StateNodeType::Choice => {
                out.insert(state.clone());
            }
            StateNodeType::Compound => {
                if node.initial.is_empty() {
                    for child in self.children(state) {
                        self.enter(&child, out);
                    }
                } else {
                    for initial in &node.initial {
                        self.enter(initial, out);
                    }
                }
            }
            StateNodeType::Parallel => {
                for child in self.children(state) {
                    self.enter(&child, out);
                }
            }
            StateNodeType::HistoryShallow | StateNodeType::HistoryDeep => {
                if let Some(parent) = &node.parent {
                    for sibling in self.children(parent) {
                        if !is_history(self.nodes[&sibling].node_type) {
                            self.enter(&sibling, out);
                        }
                    }
                }
            }
        }
    }

    fn children(&self, state: &S) -> Vec<S> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.parent.as_ref() == Some(state))
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn ancestors(&self, state: &S) -> Vec<S> {
        let mut out = Vec::new();
        let mut current = self.nodes.get(state).and_then(|n| n.parent.clone());
        while let Some(parent) = current {
            current = self.nodes.get(&parent).and_then(|n| n.parent.clone());
            out.push(parent);
        }
        out
    }
}

fn is_history(node_type: StateNodeType) -> bool {
    matches!(
        node_type,
        StateNodeType::HistoryShallow | StateNodeType::HistoryDeep
    )
}
