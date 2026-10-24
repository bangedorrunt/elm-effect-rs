//! Every command in a window is a fiber of the loop's runtime. The old
//! `run_effects` used scoped OS threads, so `with_fiber` and `fork_child`
//! died with "outside a fiber" even though the command was an `Effect`.

use std::sync::Arc;

use elm_effect::{Effect, Exit, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::MemLog;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Model {
    fiber_ids: Vec<u64>,
    child_values: Vec<u64>,
    failures: Vec<Failure<InterpretError>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    /// Emits two probes, so the window takes the concurrent path.
    Probe,
    /// Emits two fork-and-join commands.
    Fork,
    SawFiber(u64),
    SawChild(u64),
    Failed(Failure<InterpretError>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum ProbeCmd {
    Probe,
    Fork,
}

impl Cmd for ProbeCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

struct App;

impl Program for App {
    type Model = Model;
    type Msg = Msg;
    type Cmd = ProbeCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<Model, ProbeCmd> {
        Update::quiet(Model::default())
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, ProbeCmd> {
        match msg {
            Msg::Probe => {
                Update::with_commands(model.clone(), vec![ProbeCmd::Probe, ProbeCmd::Probe])
            }
            Msg::Fork => Update::with_commands(model.clone(), vec![ProbeCmd::Fork, ProbeCmd::Fork]),
            Msg::SawFiber(id) => {
                let mut next = model.clone();
                next.fiber_ids.push(*id);
                Update::quiet(next)
            }
            Msg::SawChild(value) => {
                let mut next = model.clone();
                next.child_values.push(*value);
                Update::quiet(next)
            }
            Msg::Failed(failure) => {
                let mut next = model.clone();
                next.failures.push(failure.clone());
                Update::quiet(next)
            }
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> Msg {
        Msg::Failed(failure.clone())
    }
}

struct Run;

impl Interpret<App> for Run {
    fn interpret(cmd: &ProbeCmd) -> Effect<Msg, InterpretError> {
        match cmd {
            ProbeCmd::Probe => Effect::with_fiber(|fiber| {
                Effect::<Msg, InterpretError>::succeed(Msg::SawFiber(fiber.id().0))
            }),
            ProbeCmd::Fork => Effect::with_fiber(|_| {
                Effect::<u64, InterpretError>::succeed(41)
                    .fork_child()
                    .map(|handle| match handle.join() {
                        Exit::Success(value) => Msg::SawChild(value),
                        Exit::Failure(cause) => Msg::Failed(Failure::of_cause(cause)),
                    })
            }),
        }
    }
}

fn boot() -> Loop<App, Run> {
    Loop::<App, Run>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("fiber-test"),
    )
    .expect("boot")
}

#[test]
fn every_command_in_a_window_has_a_fiber() {
    let mut app = boot();
    app.send(Msg::Probe).expect("append");
    assert!(
        app.model().failures.is_empty(),
        "a command on a fiber must not report a defect: {:?}",
        app.model().failures
    );
    assert_eq!(app.model().fiber_ids.len(), 2);
    assert!(app.model().fiber_ids.iter().all(|id| *id > 0));
}

#[test]
fn a_command_can_fork_a_child() {
    let mut app = boot();
    app.send(Msg::Fork).expect("append");
    assert!(
        app.model().failures.is_empty(),
        "fork_child must work inside a command: {:?}",
        app.model().failures
    );
    assert_eq!(app.model().child_values, vec![41, 41]);
}
