use crate::action::ActionQueue;
use crate::transition::Transition;
use crate::types::StateNodeType;
use std::sync::Arc;

pub type LifecycleFn<C, CMD, E> = Arc<dyn Fn(&mut C, &mut ActionQueue<CMD, E>) + Send + Sync>;

/// A node within the statechart hierarchy.
#[derive(Clone)]
pub struct StateNode<S, E, C, CMD> {
    pub(crate) id: S,
    pub(crate) node_type: StateNodeType,
    pub(crate) parent: Option<S>,
    pub(crate) initial: Option<S>,
    pub(crate) transitions: Vec<Transition<S, E, C, CMD>>,
    pub(crate) always: Vec<Transition<S, E, C, CMD>>,
    pub(crate) entry_actions: Vec<LifecycleFn<C, CMD, E>>,
    pub(crate) exit_actions: Vec<LifecycleFn<C, CMD, E>>,
}

impl<S: Clone, E: 'static, C: 'static, CMD: Send + Sync + 'static> StateNode<S, E, C, CMD> {
    /// Create a new atomic state node.
    pub fn atomic(id: S) -> Self {
        Self {
            id,
            node_type: StateNodeType::Atomic,
            parent: None,
            initial: None,
            transitions: Vec::new(),
            always: Vec::new(),
            entry_actions: Vec::new(),
            exit_actions: Vec::new(),
        }
    }

    /// Create a compound (hierarchical) state node with a default initial sub-state.
    pub fn compound(id: S, initial: S) -> Self {
        Self {
            id,
            node_type: StateNodeType::Compound,
            parent: None,
            initial: Some(initial),
            transitions: Vec::new(),
            always: Vec::new(),
            entry_actions: Vec::new(),
            exit_actions: Vec::new(),
        }
    }

    /// Create a final terminal state node.
    pub fn final_state(id: S) -> Self {
        Self {
            id,
            node_type: StateNodeType::Final,
            parent: None,
            initial: None,
            transitions: Vec::new(),
            always: Vec::new(),
            entry_actions: Vec::new(),
            exit_actions: Vec::new(),
        }
    }

    /// Set the parent compound state in the hierarchy.
    pub fn with_parent(mut self, parent: S) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Create a transition targeting `target` sharing this node's type parameters.
    pub fn transition(&self, target: S) -> Transition<S, E, C, CMD> {
        Transition::target(target)
    }

    /// Add a reactive event transition.
    pub fn on(mut self, transition: Transition<S, E, C, CMD>) -> Self {
        self.transitions.push(transition);
        self
    }

    /// Add an eventless (transient) transition evaluated upon entering the state.
    pub fn always(mut self, transition: Transition<S, E, C, CMD>) -> Self {
        self.always.push(transition);
        self
    }

    /// Register an action guaranteed to execute when entering this state.
    pub fn entry(
        mut self,
        action: impl Fn(&mut C, &mut ActionQueue<CMD, E>) + Send + Sync + 'static,
    ) -> Self {
        self.entry_actions.push(Arc::new(action));
        self
    }

    /// Register an action guaranteed to execute when exiting this state.
    pub fn exit(
        mut self,
        action: impl Fn(&mut C, &mut ActionQueue<CMD, E>) + Send + Sync + 'static,
    ) -> Self {
        self.exit_actions.push(Arc::new(action));
        self
    }

    /// Access the node identifier.
    #[must_use]
    pub fn id(&self) -> &S {
        &self.id
    }

    /// Access the node type.
    #[must_use]
    pub fn node_type(&self) -> StateNodeType {
        self.node_type
    }
}
