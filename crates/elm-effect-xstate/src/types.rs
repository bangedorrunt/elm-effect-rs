/// The structural classification of a statechart node.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum StateNodeType {
    /// Leaf state node with no children.
    Atomic,
    /// Hierarchical state node containing child states and an initial sub-state.
    Compound,
    /// Orthogonal state node with concurrently active child regions.
    Parallel,
    /// History state node restoring the immediate substate active prior to exit.
    HistoryShallow,
    /// History state node restoring the deepest nested leaf state configuration.
    HistoryDeep,
    /// Terminal state node signifying completion of its parent compound state.
    Final,
}

/// The result of processing an event through a statechart machine step.
#[derive(Debug, Clone)]
pub struct StepOutcome<S, C, CMD, E> {
    /// The active state after the step (and any eventless transitions) settled.
    pub current_state: S,
    /// The updated extended state context.
    pub context: C,
    /// Outgoing commands emitted for execution by the runtime.
    pub commands: Vec<CMD>,
    /// Internal events raised during execution.
    pub raised_events: Vec<E>,
    /// Whether any transition was taken.
    pub transitioned: bool,
}
