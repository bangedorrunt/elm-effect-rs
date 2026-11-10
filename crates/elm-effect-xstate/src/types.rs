use crate::actor::ActorDirective;
use crate::configuration::Configuration;
use elm_effect::Duration;
use std::collections::BTreeMap;

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

/// One chart run: the active configuration, recorded history, and context.
///
/// The engine is pure, so a run is a value. [`step`](crate::Machine::step_state)
/// takes it and returns the next one. History lives here rather than in the
/// machine, because two runs of one machine must not share remembered states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "S: serde::Serialize + Ord, C: serde::Serialize",
    deserialize = "S: serde::Deserialize<'de> + Ord, C: serde::Deserialize<'de>"
))]
pub struct MachineState<S, C> {
    /// The states active now.
    pub configuration: Configuration<S>,
    /// The configuration each history node remembered at its last exit.
    pub history: BTreeMap<S, Configuration<S>>,
    /// Extended quantitative context.
    pub context: C,
}

impl<S: Ord + Clone, C> MachineState<S, C> {
    /// A run from an explicit configuration.
    #[must_use]
    pub fn new(configuration: Configuration<S>, context: C) -> Self {
        Self {
            configuration,
            history: BTreeMap::new(),
            context,
        }
    }

    /// A run from one active state, with no recorded history.
    #[must_use]
    pub fn at(state: S, context: C) -> Self {
        Self::new(Configuration::single(state), context)
    }

    /// The first active leaf, which is the stable answer for one region.
    #[must_use]
    pub fn state(&self) -> Option<&S> {
        self.configuration.primary()
    }
}

/// A request from the engine to the host's clock.
///
/// The engine never reads a clock. Entering a state that declares `after`
/// yields [`TimerIntent::Start`]; leaving it yields [`TimerIntent::Cancel`].
/// The host schedules the delay (or advances a [`TestClock`](elm_effect::TestClock))
/// and calls [`Machine::step_timer`](crate::Machine::step_timer) with a
/// [`TimerFired`] when it elapses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerIntent<S> {
    /// Arm a delay for this state.
    Start {
        /// The state that declares the delay.
        state: S,
        /// The delay to arm.
        duration: Duration,
    },
    /// Disarm the delay for this state, because the state was exited.
    Cancel {
        /// The state that declares the delay.
        state: S,
        /// The delay that is no longer relevant.
        duration: Duration,
    },
}

/// A delay that elapsed. Pass it to
/// [`Machine::step_timer`](crate::Machine::step_timer) to take the matching
/// `after` transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerFired<S> {
    /// The state whose `after` delay elapsed.
    pub state: S,
    /// The delay that elapsed.
    pub duration: Duration,
}

impl<S> TimerFired<S> {
    /// A fired timer for `state`'s `duration` delay.
    pub fn new(state: S, duration: Duration) -> Self {
        Self { state, duration }
    }
}

impl<S: Ord + Clone, C, CMD, E> StepOutcome<S, C, CMD, E> {
    /// The primary active leaf, for a chart with one region.
    #[must_use]
    pub fn state(&self) -> Option<&S> {
        self.configuration.primary()
    }
}

/// The result of processing one event through a statechart machine.
#[derive(Debug, Clone)]
pub struct StepOutcome<S, C, CMD, E> {
    /// The first active leaf after the step settled. For a parallel machine
    /// this is one region's leaf; read [`StepOutcome::configuration`] for all.
    pub current_state: S,
    /// Every active leaf after the step settled.
    pub configuration: Configuration<S>,
    /// History recorded by states this step exited.
    pub history: BTreeMap<S, Configuration<S>>,
    /// The updated extended state context.
    pub context: C,
    /// Outgoing commands emitted for execution by the runtime.
    pub commands: Vec<CMD>,
    /// Internal events raised but not consumed before the micro-step bound.
    pub raised_events: Vec<E>,
    /// Inspection payloads emitted with `enq.emit`.
    pub emitted: Vec<serde_json::Value>,
    /// Actor work the actions asked the host to do, in action order.
    pub directives: Vec<ActorDirective>,
    /// Clock requests this step produced, in entry and exit order.
    pub timers: Vec<TimerIntent<S>>,
    /// Whether any transition was taken.
    pub transitioned: bool,
    /// Whether the root node reached a final state.
    pub done: bool,
}
