use crate::machine::Machine;
use elm_effect_elm::{Boot, Cmd, Failure, Program, Update};
use std::marker::PhantomData;

/// Statechart model holding the active leaf state and extended context.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatechartModel<S, C> {
    /// Active state configuration.
    pub state: S,
    /// Quantitative context data.
    pub context: C,
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
}

/// An [`elm_effect_elm::Program`] adapter driven by a [`StatechartDefinition`].
pub struct StatechartProgram<D: StatechartDefinition>(PhantomData<D>);

impl<D: StatechartDefinition> Program for StatechartProgram<D> {
    type Model = StatechartModel<D::State, D::Context>;
    type Msg = D::Msg;
    type Cmd = D::Cmd;
    type Err = D::Err;

    fn init(_boot: &Boot) -> Update<Self::Model, Self::Cmd> {
        let m = D::machine();
        let initial_leaf = m.resolve_leaf(m.initial_state());
        Update::quiet(StatechartModel {
            state: initial_leaf,
            context: m.initial_context().clone(),
        })
    }

    fn update(model: &Self::Model, msg: &Self::Msg) -> Update<Self::Model, Self::Cmd> {
        let Some(event) = D::extract_event(msg) else {
            return Update::quiet(model.clone());
        };
        let m = D::machine();
        let outcome = m.step(&model.state, &model.context, &event);
        Update::with_commands(
            StatechartModel {
                state: outcome.current_state,
                context: outcome.context,
            },
            outcome.commands,
        )
    }

    fn failure(failure: &Failure<Self::Err>) -> Self::Msg {
        D::failure(failure)
    }
}
