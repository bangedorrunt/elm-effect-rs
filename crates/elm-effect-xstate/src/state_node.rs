use crate::action::ActionQueue;
use crate::transition::Transition;
use crate::types::StateNodeType;
use elm_effect::Duration;
use std::sync::Arc;

/// A lifecycle action: entry or exit, with the event that caused it.
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
    pub(crate) after: Vec<(Duration, Transition<S, E, C, CMD>)>,
    pub(crate) on_done: Vec<Transition<S, E, C, CMD>>,
    pub(crate) entry_actions: Vec<LifecycleFn<C, CMD, E>>,
    pub(crate) exit_actions: Vec<LifecycleFn<C, CMD, E>>,
}

impl<S: Clone, E: 'static, C: 'static, CMD: Send + Sync + 'static> StateNode<S, E, C, CMD> {
    fn empty(id: S, node_type: StateNodeType) -> Self {
        Self {
            id,
            node_type,
            parent: None,
            initial: None,
            transitions: Vec::new(),
            always: Vec::new(),
            after: Vec::new(),
            on_done: Vec::new(),
            entry_actions: Vec::new(),
            exit_actions: Vec::new(),
        }
    }

    /// Create a new atomic state node.
    pub fn atomic(id: S) -> Self {
        Self::empty(id, StateNodeType::Atomic)
    }

    /// Create a compound (hierarchical) state node with a default initial sub-state.
    pub fn compound(id: S, initial: S) -> Self {
        Self {
            initial: Some(initial),
            ..Self::empty(id, StateNodeType::Compound)
        }
    }

    /// Create an orthogonal state node whose children are all entered together.
    pub fn parallel(id: S) -> Self {
        Self::empty(id, StateNodeType::Parallel)
    }

    /// Create a final terminal state node.
    pub fn final_state(id: S) -> Self {
        Self::empty(id, StateNodeType::Final)
    }

    /// Create a history node. `fallback` is entered when nothing is remembered.
    pub fn history(id: S, deep: bool, fallback: S) -> Self {
        let node_type = if deep {
            StateNodeType::HistoryDeep
        } else {
            StateNodeType::HistoryShallow
        };
        Self {
            initial: Some(fallback),
            ..Self::empty(id, node_type)
        }
    }

    /// Set the parent compound state in the hierarchy.
    pub fn with_parent(mut self, parent: S) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Set the initial child entered when this compound node is entered.
    pub fn with_initial(mut self, initial: S) -> Self {
        self.initial = Some(initial);
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

    /// Add a delayed transition, armed on entry and disarmed on exit.
    pub fn after(mut self, duration: Duration, transition: Transition<S, E, C, CMD>) -> Self {
        self.after.push((duration, transition));
        self
    }

    /// Add a transition taken when this node completes: its child reached a
    /// final state (compound) or every region did (parallel).
    pub fn on_done(mut self, transition: Transition<S, E, C, CMD>) -> Self {
        self.on_done.push(transition);
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

    /// Access the parent, when this node is nested.
    #[must_use]
    pub fn parent(&self) -> Option<&S> {
        self.parent.as_ref()
    }

    /// Access the initial child, when this node is compound or a history node.
    #[must_use]
    pub fn initial(&self) -> Option<&S> {
        self.initial.as_ref()
    }
}
