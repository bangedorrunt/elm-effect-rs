use elm_effect_elm::{Boot, Cmd, Durability, Failure, Program};
use elm_effect_xstate::prelude::*;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
enum GateState {
    Closed,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum GateEvent {
    Push,
    Coin,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct GateContext {
    coins_collected: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum AppCmd {
    UnlockTurnstile,
    LockTurnstile,
}

impl Cmd for AppCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum AppMsg {
    UserEvent(GateEvent),
}

struct GateProgramDef;

impl StatechartDefinition for GateProgramDef {
    type State = GateState;
    type Event = GateEvent;
    type Context = GateContext;
    type Cmd = AppCmd;
    type Msg = AppMsg;
    type Err = String;

    fn machine() -> Machine<Self::State, Self::Event, Self::Context, Self::Cmd> {
        Machine::builder(GateState::Closed, GateContext::default())
            .state(GateState::Closed, |node| {
                let to_open = node
                    .transition(GateState::Open)
                    .on(|e| matches!(e, GateEvent::Coin))
                    .assign(|ctx, _| ctx.coins_collected += 1)
                    .exec(AppCmd::UnlockTurnstile);
                node.on(to_open)
            })
            .state(GateState::Open, |node| {
                let to_closed = node
                    .transition(GateState::Closed)
                    .on(|e| matches!(e, GateEvent::Push))
                    .exec(AppCmd::LockTurnstile);
                node.on(to_closed)
            })
            .build()
    }

    fn extract_event(msg: &Self::Msg) -> Option<Self::Event> {
        match msg {
            AppMsg::UserEvent(e) => Some(e.clone()),
        }
    }

    fn failure(_failure: &Failure<Self::Err>) -> Self::Msg {
        AppMsg::UserEvent(GateEvent::Push)
    }
}

#[test]
fn statechart_program_runs_in_elm_fold() {
    type ProgramType = StatechartProgram<GateProgramDef>;

    let boot = Boot::default();
    let init_update = ProgramType::init(&boot);
    assert_eq!(init_update.model.state, GateState::Closed);
    assert_eq!(init_update.model.context.coins_collected, 0);

    let update1 = ProgramType::update(&init_update.model, &AppMsg::UserEvent(GateEvent::Coin));
    assert_eq!(update1.model.state, GateState::Open);
    assert_eq!(update1.model.context.coins_collected, 1);
    assert_eq!(update1.commands, vec![AppCmd::UnlockTurnstile]);

    let update2 = ProgramType::update(&update1.model, &AppMsg::UserEvent(GateEvent::Push));
    assert_eq!(update2.model.state, GateState::Closed);
    assert_eq!(update2.model.context.coins_collected, 1);
    assert_eq!(update2.commands, vec![AppCmd::LockTurnstile]);
}
