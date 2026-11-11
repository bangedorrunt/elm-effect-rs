//! Harel statecharts, actor-ready primitives, and XState v6 behavioural parity
//! for `elm-effect` (ADR-0007).
//!
//! A [`Machine`] is a pure value. [`Machine::step_state`] takes a
//! [`MachineState`] and an event and returns the next state plus the work the
//! host must do. Nothing here reads a clock, spawns a thread, or performs I/O:
//!
//! - Entry and exit actions mutate extended context and enqueue commands in an
//!   [`ActionQueue`].
//! - `after` delays leave as [`TimerIntent`] values; the host schedules them and
//!   returns a [`TimerFired`] to [`Machine::step_timer`].
//! - Transitions can be recorded to a durable log with [`TransitionRecord`].
//!
//! Compound and parallel states share one algorithm. The active states are a
//! [`Configuration`], so a parallel region contributes its own leaf and events
//! are broadcast to every region.

pub mod action;
pub mod actor;
pub mod adapter;
pub mod configuration;
pub mod durable;
pub mod dynamic;
pub mod export;
pub mod machine;
pub mod state_node;
pub mod transition;
pub mod types;

pub use action::ActionQueue;
pub use actor::{
    ActorDirective, ActorError, ActorId, ActorLogic, ActorSnapshotEntry, ActorState, ActorSystem,
    ActorTask, ActorTreeSnapshot, CallbackLogic, CallbackSender, EffectLogic, Reducer, StreamLogic,
};
pub use adapter::{StatechartDefinition, StatechartModel, StatechartProgram};
pub use configuration::Configuration;
pub use durable::{ReplayError, TransitionRecord, TransitionStamp};
pub use dynamic::{
    ActionList, DelaySpec, DynamicActionFn, DynamicContext, DynamicDelayFn, DynamicEvent,
    DynamicGuardFn, DynamicInputFn, DynamicMachine, DynamicMachineBuilder, DynamicOutputFn,
    InvokeConfig, InvokeList, MachineConfig, StateNodeConfig, TargetList, TransitionConfig,
    TransitionRule,
};
pub use machine::{Machine, MachineBuilder};
pub use state_node::StateNode;
pub use transition::Transition;
pub use types::{ArmedTimer, MachineState, StateNodeType, StepOutcome, TimerFired, TimerIntent};

pub mod prelude {
    pub use crate::action::ActionQueue;
    pub use crate::actor::{
        ActorDirective, ActorError, ActorId, ActorLogic, ActorSnapshotEntry, ActorState,
        ActorSystem, ActorTask, ActorTreeSnapshot, CallbackLogic, CallbackSender, EffectLogic,
        Reducer, StreamLogic,
    };
    pub use crate::adapter::{StatechartDefinition, StatechartModel, StatechartProgram};
    pub use crate::configuration::Configuration;
    pub use crate::durable::{ReplayError, TransitionRecord, TransitionStamp};
    pub use crate::dynamic::{
        ActionList, DelaySpec, DynamicActionFn, DynamicContext, DynamicDelayFn, DynamicEvent,
        DynamicGuardFn, DynamicMachine, DynamicMachineBuilder, InvokeConfig, InvokeList,
        MachineConfig, StateNodeConfig, TargetList, TransitionConfig, TransitionRule,
    };
    pub use crate::machine::{Machine, MachineBuilder};
    pub use crate::state_node::StateNode;
    pub use crate::transition::Transition;
    pub use crate::types::{
        ArmedTimer, MachineState, StateNodeType, StepOutcome, TimerFired, TimerIntent,
    };
}
