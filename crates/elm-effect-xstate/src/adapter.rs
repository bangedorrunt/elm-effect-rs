use crate::configuration::Configuration;
use crate::machine::Machine;
use crate::types::{MachineState, TimerIntent};
use elm_effect_elm::{Boot, Cmd, Failure, Program, Update};
use std::collections::BTreeMap;
use std::marker::PhantomData;

/// Statechart model holding the active configuration, history, and context.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "S: serde::Serialize + Ord + Clone, C: serde::Serialize",
    deserialize = "S: serde::Deserialize<'de> + Ord + Clone, C: serde::Deserialize<'de>"
))]
pub struct StatechartModel<S, C> {
    /// Active leaf states.
    pub configuration: Configuration<S>,
    /// Configuration each history node remembered at its last exit.
    pub history: BTreeMap<S, Configuration<S>>,
    /// Quantitative context data.
    pub context: C,
}

impl<S: Ord + Clone, C: Clone> StatechartModel<S, C> {
    fn machine_state(&self) -> MachineState<S, C> {
        MachineState {
            configuration: self.configuration.clone(),
            history: self.history.clone(),
            context: self.context.clone(),
        }
    }

    /// The primary active leaf, for a chart with one region.
    #[must_use]
    pub fn state(&self) -> Option<&S> {
        self.configuration.primary()
    }
}

/// Domain definition for embedding a statechart into the Elm loop.
pub trait StatechartDefinition {
    /// Mutually exclusive state nodes.
    type State: Ord + Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static;
    /// Incoming domain events.
    type Event: Clone + Send + Sync + 'static;
    /// Extended quantitative context data.
    type Context: Clone + PartialEq + Default + Send + Sync + 'static;
    /// Outgoing commands.
    type Cmd: Cmd + Send + Sync + 'static;
    /// Application loop messages.
    type Msg: Send + 'static + serde::Serialize + for<'de> serde::Deserialize<'de> + Clone;
    /// Typed interpretation errors.
    type Err: Send + 'static;

    /// Build the static machine topology.
    fn machine() -> Machine<Self::State, Self::Event, Self::Context, Self::Cmd>;

    /// Extract a domain event from an incoming message, if matched.
    fn extract_event(msg: &Self::Msg) -> Option<Self::Event>;

    /// Map a command interpretation failure to an incoming message.
    fn failure(failure: &Failure<Self::Err>) -> Self::Msg;

    /// Turn a timer the machine armed into a command the host can run.
    ///
    /// The engine never reads a clock (ADR-0007 law L3). Entering a state with
    /// `after` produces a [`TimerIntent::Start`]; returning a command from here
    /// hands it to the host, whose delay message must come back through
    /// [`StatechartDefinition::extract_event`]. The default ignores time, so a
    /// machine with no `after` transition needs no clock.
    fn timer_cmd(timer: &TimerIntent<Self::State>) -> Option<Self::Cmd> {
        let _ = timer;
        None
    }
}

/// An [`elm_effect_elm::Program`] adapter driven by a [`StatechartDefinition`].
pub struct StatechartProgram<D: StatechartDefinition>(PhantomData<D>);

impl<D: StatechartDefinition> Program for StatechartProgram<D> {
    type Model = StatechartModel<D::State, D::Context>;
    type Msg = D::Msg;
    type Cmd = D::Cmd;
    type Err = D::Err;

    fn init(_boot: &Boot) -> Update<Self::Model, Self::Cmd> {
        let machine = D::machine();
        let outcome = machine.enter(&D::Context::default());
        let mut commands = outcome.commands;
        commands.extend(outcome.timers.iter().filter_map(D::timer_cmd));
        Update::with_commands(
            StatechartModel {
                configuration: outcome.configuration,
                history: outcome.history,
                context: outcome.context,
            },
            commands,
        )
    }

    fn update(model: &Self::Model, msg: &Self::Msg) -> Update<Self::Model, Self::Cmd> {
        let Some(event) = D::extract_event(msg) else {
            return Update::quiet(model.clone());
        };
        let machine = D::machine();
        let outcome = machine.step_state(&model.machine_state(), &event);
        let mut commands = outcome.commands;
        commands.extend(outcome.timers.iter().filter_map(D::timer_cmd));
        Update::with_commands(
            StatechartModel {
                configuration: outcome.configuration,
                history: outcome.history,
                context: outcome.context,
            },
            commands,
        )
    }

    fn failure(failure: &Failure<Self::Err>) -> Self::Msg {
        D::failure(failure)
    }
}
