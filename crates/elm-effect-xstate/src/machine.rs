use crate::action::ActionQueue;
use crate::state_node::StateNode;
use crate::transition::Transition;
use crate::types::{StateNodeType, StepOutcome};
use std::collections::BTreeMap;

/// A compiled Harel statechart machine.
#[derive(Clone)]
pub struct Machine<S: Ord + Clone, E, C: Clone, CMD> {
    pub(crate) nodes: BTreeMap<S, StateNode<S, E, C, CMD>>,
    pub(crate) initial_state: S,
    pub(crate) initial_context: C,
}

impl<S, E, C, CMD> Machine<S, E, C, CMD>
where
    S: Ord + Clone + std::fmt::Debug + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
    CMD: Clone + Send + Sync + 'static,
{
    /// Start building a machine with the specified root initial state and context.
    pub fn builder(initial_state: S, initial_context: C) -> MachineBuilder<S, E, C, CMD> {
        MachineBuilder::new(initial_state, initial_context)
    }

    /// Access the initial root state.
    #[must_use]
    pub fn initial_state(&self) -> &S {
        &self.initial_state
    }

    /// Access the initial extended context.
    #[must_use]
    pub fn initial_context(&self) -> &C {
        &self.initial_context
    }

    /// Resolve a state to its active leaf node, following initial sub-states if compound.
    #[must_use]
    pub fn resolve_leaf(&self, state: &S) -> S {
        let mut curr = state.clone();
        while let Some(node) = self.nodes.get(&curr) {
            if node.node_type == StateNodeType::Compound
                && let Some(ref init) = node.initial
            {
                curr = init.clone();
                continue;
            }
            break;
        }
        curr
    }

    /// Compute ancestry path from state up to root: `[state, parent, grandparent, ...]`.
    fn ancestry(&self, state: &S) -> Vec<S> {
        let mut path = Vec::new();
        let mut curr = Some(state.clone());
        while let Some(s) = curr {
            path.push(s.clone());
            curr = self.nodes.get(&s).and_then(|n| n.parent.clone());
        }
        path
    }

    /// Compute path from root down to state: `[root, ..., parent, state]`.
    fn path_from_root(&self, state: &S) -> Vec<S> {
        let mut path = self.ancestry(state);
        path.reverse();
        path
    }

    /// Find the least common ancestor of two states, if any.
    fn lca(&self, s1: &S, s2: &S) -> Option<S> {
        let p1 = self.path_from_root(s1);
        let p2 = self.path_from_root(s2);
        let mut last_common = None;
        for (a, b) in p1.iter().zip(p2.iter()) {
            if a == b {
                last_common = Some(a.clone());
            } else {
                break;
            }
        }
        last_common
    }

    /// Execute a single transition from `source_leaf` to `target_state` obeying Law L1.
    fn execute_transition(
        &self,
        source_leaf: &S,
        target_state: &S,
        transition: &Transition<S, E, C, CMD>,
        event: &E,
        ctx: &mut C,
        enq: &mut ActionQueue<CMD, E>,
    ) -> S {
        let target_leaf = self.resolve_leaf(target_state);

        let lca = if source_leaf == &target_leaf {
            self.nodes.get(source_leaf).and_then(|n| n.parent.clone())
        } else {
            self.lca(source_leaf, &target_leaf)
        };

        let source_ancestry = self.ancestry(source_leaf);
        for s in &source_ancestry {
            if Some(s) == lca.as_ref() {
                break;
            }
            if let Some(node) = self.nodes.get(s) {
                for exit_fn in &node.exit_actions {
                    exit_fn(ctx, enq);
                }
            }
        }

        for action in &transition.actions {
            action(ctx, event, enq);
        }

        let target_path = self.path_from_root(&target_leaf);
        let mut enter = false;
        for s in &target_path {
            if !enter {
                if Some(s) == lca.as_ref() {
                    enter = true;
                    continue;
                } else if lca.is_none() {
                    enter = true;
                }
            }
            if enter && let Some(node) = self.nodes.get(s) {
                for entry_fn in &node.entry_actions {
                    entry_fn(ctx, enq);
                }
            }
        }

        target_leaf
    }

    /// Process an event through the statechart, following hierarchy bubbling and Law L4 eventless loops.
    pub fn step(&self, current: &S, context: &C, event: &E) -> StepOutcome<S, C, CMD, E> {
        let mut ctx = context.clone();
        let mut enq = ActionQueue::new();
        let current_leaf = self.resolve_leaf(current);

        let mut selected_transition = None;
        let active_ancestry = self.ancestry(&current_leaf);

        for state in &active_ancestry {
            if let Some(node) = self.nodes.get(state) {
                for trans in &node.transitions {
                    if trans.is_enabled(&ctx, event) {
                        selected_transition = Some(trans.clone());
                        break;
                    }
                }
            }
            if selected_transition.is_some() {
                break;
            }
        }

        let Some(trans) = selected_transition else {
            return StepOutcome {
                current_state: current_leaf,
                context: ctx,
                commands: enq.take_commands(),
                raised_events: enq.take_raised(),
                transitioned: false,
            };
        };

        let mut active_state = self.execute_transition(
            &current_leaf,
            &trans.target,
            &trans,
            event,
            &mut ctx,
            &mut enq,
        );

        let mut micro_step_count = 0;
        const MAX_MICRO_STEPS: usize = 100;

        while micro_step_count < MAX_MICRO_STEPS {
            let mut always_trans = None;
            let current_ancestry = self.ancestry(&active_state);

            for state in &current_ancestry {
                if let Some(node) = self.nodes.get(state) {
                    for trans in &node.always {
                        if trans.is_enabled(&ctx, event) {
                            always_trans = Some(trans.clone());
                            break;
                        }
                    }
                }
                if always_trans.is_some() {
                    break;
                }
            }

            let Some(always) = always_trans else {
                break;
            };

            active_state = self.execute_transition(
                &active_state,
                &always.target,
                &always,
                event,
                &mut ctx,
                &mut enq,
            );
            micro_step_count += 1;
        }

        StepOutcome {
            current_state: active_state,
            context: ctx,
            commands: enq.take_commands(),
            raised_events: enq.take_raised(),
            transitioned: true,
        }
    }
}

/// Builder for constructing a [`Machine`].
pub struct MachineBuilder<S: Ord + Clone, E, C: Clone, CMD> {
    nodes: BTreeMap<S, StateNode<S, E, C, CMD>>,
    initial_state: S,
    initial_context: C,
}

impl<S, E, C, CMD> MachineBuilder<S, E, C, CMD>
where
    S: Ord + Clone + std::fmt::Debug + Send + Sync + 'static,
    E: 'static,
    C: Clone + 'static,
    CMD: Send + Sync + 'static,
{
    pub fn new(initial_state: S, initial_context: C) -> Self {
        Self {
            nodes: BTreeMap::new(),
            initial_state,
            initial_context,
        }
    }

    /// Add an atomic state node configured through a closure.
    pub fn state(
        mut self,
        id: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::atomic(id));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a compound state node configured through a closure.
    pub fn compound(
        mut self,
        id: S,
        initial: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::compound(id, initial));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a final terminal state node configured through a closure.
    pub fn final_state(
        mut self,
        id: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::final_state(id));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a pre-constructed state node directly.
    pub fn node(mut self, node: StateNode<S, E, C, CMD>) -> Self {
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Build the finalized [`Machine`].
    #[must_use]
    pub fn build(self) -> Machine<S, E, C, CMD> {
        Machine {
            nodes: self.nodes,
            initial_state: self.initial_state,
            initial_context: self.initial_context,
        }
    }
}
