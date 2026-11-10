pub mod action;
pub mod adapter;
pub mod dynamic;
pub mod machine;
pub mod state_node;
pub mod transition;
pub mod types;

pub use action::ActionQueue;
pub use adapter::{StatechartDefinition, StatechartModel, StatechartProgram};
pub use dynamic::{
    ActionList, DynamicActionFn, DynamicContext, DynamicEvent, DynamicGuardFn, DynamicMachine,
    DynamicMachineBuilder, MachineConfig, StateNodeConfig, TransitionConfig, TransitionRule,
};
pub use machine::{Machine, MachineBuilder};
pub use state_node::StateNode;
pub use transition::Transition;
pub use types::{StateNodeType, StepOutcome};

pub mod prelude {
    pub use crate::action::ActionQueue;
    pub use crate::adapter::{StatechartDefinition, StatechartModel, StatechartProgram};
    pub use crate::dynamic::{
        ActionList, DynamicActionFn, DynamicContext, DynamicEvent, DynamicGuardFn, DynamicMachine,
        DynamicMachineBuilder, MachineConfig, StateNodeConfig, TransitionConfig, TransitionRule,
    };
    pub use crate::machine::{Machine, MachineBuilder};
    pub use crate::state_node::StateNode;
    pub use crate::transition::Transition;
    pub use crate::types::{StateNodeType, StepOutcome};
}
