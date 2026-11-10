use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Spot {
    Root,
    Active,
    Idle,
    Paused,
    Stopped,
    Shallow,
    Deep,
}

#[derive(Debug, Clone)]
enum Signal {
    Play,
    Stop,
    ResumeShallow,
    ResumeDeep,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Trace {
    log: Vec<String>,
}

fn machine(initial: Spot) -> Machine<Spot, Signal, Trace, ()> {
    Machine::builder(initial, Trace::default())
        .compound(Spot::Root, Spot::Active, |node| {
            let stop = node
                .transition(Spot::Stopped)
                .on(|e| matches!(e, Signal::Stop));
            node.on(stop)
        })
        .compound(Spot::Active, Spot::Idle, |node| {
            node.with_parent(Spot::Root)
                .entry(|ctx, _| ctx.log.push("enter_Active".into()))
                .exit(|ctx, _| ctx.log.push("exit_Active".into()))
        })
        .state(Spot::Idle, |node| {
            let play = node
                .transition(Spot::Paused)
                .on(|e| matches!(e, Signal::Play));
            node.with_parent(Spot::Active)
                .entry(|ctx, _| ctx.log.push("enter_Idle".into()))
                .exit(|ctx, _| ctx.log.push("exit_Idle".into()))
                .on(play)
        })
        .state(Spot::Paused, |node| {
            node.with_parent(Spot::Active)
                .entry(|ctx, _| ctx.log.push("enter_Paused".into()))
                .exit(|ctx, _| ctx.log.push("exit_Paused".into()))
        })
        .state(Spot::Stopped, |node| {
            let shallow = node
                .transition(Spot::Shallow)
                .on(|e| matches!(e, Signal::ResumeShallow));
            let deep = node
                .transition(Spot::Deep)
                .on(|e| matches!(e, Signal::ResumeDeep));
            node.entry(|ctx, _| ctx.log.push("enter_Stopped".into()))
                .on(shallow)
                .on(deep)
        })
        .history(Spot::Shallow, Spot::Idle, |node| {
            node.with_parent(Spot::Root)
        })
        .deep_history(Spot::Deep, Spot::Idle, |node| node.with_parent(Spot::Root))
        .build()
}

fn run_to_paused(machine: &Machine<Spot, Signal, Trace, ()>) -> MachineState<Spot, Trace> {
    let entered = machine.enter(&Trace::default());
    let state = MachineState::new(entered.configuration, entered.context);
    let played = machine.step_state(&state, &Signal::Play);
    MachineState {
        configuration: played.configuration,
        history: played.history,
        context: played.context,
    }
}

#[test]
fn shallow_history_restores_the_child_and_resolves_its_initial() {
    let machine = machine(Spot::Root);
    let paused = run_to_paused(&machine);
    assert_eq!(paused.state(), Some(&Spot::Paused));

    let stopped = machine.step_state(&paused, &Signal::Stop);
    assert_eq!(stopped.state(), Some(&Spot::Stopped));
    assert!(stopped.history.contains_key(&Spot::Shallow));

    let resumed = machine.step_state(
        &MachineState {
            configuration: stopped.configuration,
            history: stopped.history,
            context: stopped.context,
        },
        &Signal::ResumeShallow,
    );
    assert_eq!(resumed.state(), Some(&Spot::Idle));
}

#[test]
fn deep_history_restores_the_exact_leaf() {
    let machine = machine(Spot::Root);
    let paused = run_to_paused(&machine);

    let stopped = machine.step_state(&paused, &Signal::Stop);
    let resumed = machine.step_state(
        &MachineState {
            configuration: stopped.configuration,
            history: stopped.history,
            context: stopped.context,
        },
        &Signal::ResumeDeep,
    );

    assert_eq!(resumed.state(), Some(&Spot::Paused));
    assert_eq!(
        &resumed.context.log[resumed.context.log.len() - 2..],
        ["enter_Active", "enter_Paused"]
    );
}

#[test]
fn history_without_a_record_uses_its_fallback() {
    let machine = machine(Spot::Stopped);
    let entered = machine.enter(&Trace::default());
    assert_eq!(
        entered.configuration.leaves().copied().collect::<Vec<_>>(),
        [Spot::Stopped]
    );

    let resumed = machine.step_state(
        &MachineState::new(entered.configuration, entered.context),
        &Signal::ResumeDeep,
    );

    assert_eq!(resumed.state(), Some(&Spot::Idle));
    assert_eq!(
        resumed.configuration.leaves().copied().collect::<Vec<_>>(),
        [Spot::Idle]
    );
}
